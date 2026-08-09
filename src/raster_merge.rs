//! Conservative page-level raster merge. Text and vectors remain in their original content stream.

use std::{fs, io::Cursor, path::Path};

use flate2::{write::ZlibEncoder, Compression};
use jpeg_decoder::{Decoder, PixelFormat};
use lopdf::{
    content::{Content, Operation},
    dictionary, Document, Object, ObjectId, Stream,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::Builder;
use thiserror::Error;

use crate::{geometry::Matrix, inspect, InspectionError};

#[derive(Debug, Error)]
pub enum RasterMergeError {
    #[error("inspection failed: {0}")]
    Inspection(#[from] InspectionError),
    #[error("PDF error: {0}")]
    Pdf(#[from] lopdf::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("source and output paths must differ")]
    SourceWouldBeOverwritten,
    #[error("output already exists: {0}")]
    OutputExists(String),
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("could not persist validated output: {0}")]
    Persist(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RasterMergeReport {
    pub schema_version: String,
    pub dry_run: bool,
    pub input_path: String,
    pub output_path: Option<String>,
    pub target_profile: String,
    pub target_long_dimension_px: u32,
    pub pages: Vec<PageMergeReport>,
    pub pages_merged: usize,
    pub pages_skipped: usize,
    pub original_size_bytes: u64,
    pub output_size_bytes: Option<u64>,
    pub saved_bytes: Option<u64>,
    pub validation_passed: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageMergeReport {
    pub page_number: u32,
    pub status: String,
    pub raster_objects: usize,
    pub merged_bbox_pt: Option<[f64; 4]>,
    pub merged_pixels: Option<[u32; 2]>,
    pub skipped_reason: Option<String>,
}

#[derive(Clone)]
struct Draw {
    object_id: ObjectId,
    matrix: Matrix,
    bounds: [f64; 4],
    pixels: Vec<u8>,
    width: u32,
    height: u32,
}
struct MergePage {
    page_number: u32,
    page_id: ObjectId,
    draws: Vec<Draw>,
    union: [f64; 4],
    target: [u32; 2],
}

pub fn merge_1080p(
    input: &Path,
    output: &Path,
    dry_run: bool,
) -> Result<RasterMergeReport, RasterMergeError> {
    let input_abs = fs::canonicalize(input)?;
    let output_abs = if output.is_absolute() {
        output.to_path_buf()
    } else {
        std::env::current_dir()?.join(output)
    };
    if input_abs == output_abs {
        return Err(RasterMergeError::SourceWouldBeOverwritten);
    }
    if !dry_run && output.exists() {
        return Err(RasterMergeError::OutputExists(output.display().to_string()));
    }
    let original_hash = file_hash(input)?;
    let before = inspect(input)?;
    let mut document = Document::load(input)?;
    let mut planned = Vec::new();
    let mut pages = Vec::new();
    for (page_number, page_id) in document.get_pages() {
        match plan_page(&document, page_number, page_id) {
            Ok(Some(plan)) => {
                pages.push(PageMergeReport {
                    page_number,
                    status: if dry_run {
                        "would_merge".into()
                    } else {
                        "planned".into()
                    },
                    raster_objects: plan.draws.len(),
                    merged_bbox_pt: Some(plan.union),
                    merged_pixels: Some(plan.target),
                    skipped_reason: None,
                });
                planned.push(plan);
            }
            Ok(None) => pages.push(PageMergeReport {
                page_number,
                status: "skipped".into(),
                raster_objects: 0,
                merged_bbox_pt: None,
                merged_pixels: None,
                skipped_reason: Some("fewer_than_two_direct_rasters".into()),
            }),
            Err(reason) => pages.push(PageMergeReport {
                page_number,
                status: "skipped".into(),
                raster_objects: 0,
                merged_bbox_pt: None,
                merged_pixels: None,
                skipped_reason: Some(reason),
            }),
        }
    }
    let original_size = fs::metadata(input)?.len();
    if dry_run {
        let merged = planned.len();
        return Ok(RasterMergeReport {
            schema_version: "0.1.0".into(),
            dry_run: true,
            input_path: input.display().to_string(),
            output_path: None,
            target_profile: "1080p".into(),
            target_long_dimension_px: 1920,
            pages,
            pages_merged: merged,
            pages_skipped: before.file.page_count - merged,
            original_size_bytes: original_size,
            output_size_bytes: None,
            saved_bytes: None,
            validation_passed: None,
        });
    }
    for plan in &planned {
        apply_merge(&mut document, plan)?;
    }
    document.prune_objects();
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let temporary = Builder::new()
        .prefix(".pdfdoctor-merge-")
        .suffix(".pdf")
        .tempfile_in(parent)?;
    document.save(temporary.path())?;
    let after = inspect(temporary.path())?;
    validate(&before, &after, &planned)?;
    if file_hash(input)? != original_hash {
        return Err(RasterMergeError::Validation(
            "source changed during merge".into(),
        ));
    }
    let temporary_size = fs::metadata(temporary.path())?.len();
    if temporary_size == 0 || (!planned.is_empty() && temporary_size >= original_size) {
        return Err(RasterMergeError::Validation(format!(
            "merged output is not smaller than the source ({} bytes versus {} bytes)",
            temporary_size, original_size
        )));
    }
    temporary
        .persist_noclobber(output)
        .map_err(|e| RasterMergeError::Persist(e.error.to_string()))?;
    let output_size = fs::metadata(output)?.len();
    for page in &mut pages {
        if page.status == "planned" {
            page.status = "merged".into()
        }
    }
    let merged = planned.len();
    Ok(RasterMergeReport {
        schema_version: "0.1.0".into(),
        dry_run: false,
        input_path: input.display().to_string(),
        output_path: Some(output.display().to_string()),
        target_profile: "1080p".into(),
        target_long_dimension_px: 1920,
        pages,
        pages_merged: merged,
        pages_skipped: before.file.page_count - merged,
        original_size_bytes: original_size,
        output_size_bytes: Some(output_size),
        saved_bytes: Some(original_size.saturating_sub(output_size)),
        validation_passed: Some(true),
    })
}

fn plan_page(
    doc: &Document,
    page_number: u32,
    page_id: ObjectId,
) -> Result<Option<MergePage>, String> {
    let page = doc.get_dictionary(page_id).map_err(|e| e.to_string())?;
    let media = page
        .get(b"MediaBox")
        .map_err(|_| "missing_media_box")?
        .as_array()
        .map_err(|_| "invalid_media_box")?;
    let page_w = number(&media[2]).ok_or("invalid_media_box")?
        - number(&media[0]).ok_or("invalid_media_box")?;
    let page_h = number(&media[3]).ok_or("invalid_media_box")?
        - number(&media[1]).ok_or("invalid_media_box")?;
    let resources = page
        .get(b"Resources")
        .map_err(|_| "missing_resources")?
        .as_dict()
        .map_err(|_| "indirect_or_invalid_resources")?;
    let xobjects = resources
        .get(b"XObject")
        .map_err(|_| "no_xobjects")?
        .as_dict()
        .map_err(|_| "indirect_xobject_dictionary")?;
    let bytes = doc
        .get_page_content_with_limit(page_id, usize::MAX)
        .map_err(|e| e.to_string())?;
    let content = Content::decode(&bytes).map_err(|e| e.to_string())?;
    let mut ctm = Matrix::IDENTITY;
    let mut stack = Vec::new();
    let mut clip: Option<[f64; 4]> = None;
    let mut pending_rect: Option<[f64; 4]> = None;
    let mut clip_pending = false;
    let mut draws = Vec::new();
    let mut non_raster_painted = false;
    for op in &content.operations {
        match op.operator.as_str() {
            "q" => stack.push((ctm, clip)),
            "Q" => (ctm, clip) = stack.pop().ok_or("unbalanced_graphics_state")?,
            "cm" => {
                if op.operands.len() != 6 {
                    return Err("invalid_matrix".into());
                }
                ctm = to_matrix(&op.operands).ok_or("invalid_matrix")?.concat(ctm)
            }
            "re" => {
                if op.operands.len() != 4 {
                    return Err("complex_clipping_not_supported".into());
                }
                let x = number(&op.operands[0]).ok_or("invalid_rectangle")?;
                let y = number(&op.operands[1]).ok_or("invalid_rectangle")?;
                let w = number(&op.operands[2]).ok_or("invalid_rectangle")?;
                let h = number(&op.operands[3]).ok_or("invalid_rectangle")?;
                pending_rect = Some(rect_bounds(ctm, x, y, w, h));
            }
            "W" => {
                if pending_rect.is_none() {
                    return Err("complex_clipping_not_supported".into());
                }
                clip_pending = true;
            }
            "W*" => return Err("complex_clipping_not_supported".into()),
            "n" => {
                if clip_pending {
                    let next = pending_rect
                        .take()
                        .ok_or("complex_clipping_not_supported")?;
                    clip = Some(match clip {
                        Some(current) => intersect(current, next).ok_or("empty_clip")?,
                        None => next,
                    });
                    clip_pending = false;
                }
            }
            "gs" => {
                let name = op
                    .operands
                    .first()
                    .and_then(|o| o.as_name().ok())
                    .ok_or("invalid_extgstate")?;
                if !opaque_extgstate(doc, resources, name) {
                    return Err("transparency_or_extgstate_not_supported".into());
                }
            }
            "Do" => {
                let name = op
                    .operands
                    .first()
                    .and_then(|o| o.as_name().ok())
                    .ok_or("invalid_xobject_invocation")?;
                let reference = xobjects
                    .get(name)
                    .map_err(|_| "missing_xobject")?
                    .as_reference()
                    .map_err(|_| "direct_xobject_unsupported")?;
                let stream = doc
                    .get_object(reference)
                    .map_err(|e| e.to_string())?
                    .as_stream()
                    .map_err(|_| "invalid_xobject")?;
                if stream
                    .dict
                    .get(b"Subtype")
                    .ok()
                    .and_then(|o| o.as_name().ok())
                    != Some(b"Image")
                {
                    return Err("form_xobject_not_supported".into());
                }
                if non_raster_painted {
                    return Err("unsafe_content_order".into());
                }
                let mut draw = decode_draw(reference, stream, ctm)?;
                if let Some(clip_bounds) = clip {
                    if (ctm.b.abs() > 1e-8 || ctm.c.abs() > 1e-8)
                        && !bounds_close(clip_bounds, draw.bounds)
                    {
                        return Err("rotated_clipping_not_supported".into());
                    }
                    draw.bounds =
                        intersect(draw.bounds, clip_bounds).ok_or("image_fully_clipped")?;
                }
                draws.push(draw);
            }
            value if is_paint(value) => non_raster_painted = true,
            _ => {}
        }
    }
    if draws.len() < 2 {
        return Ok(None);
    }
    let union = draws.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |mut u, d| {
            u[0] = u[0].min(d.bounds[0]);
            u[1] = u[1].min(d.bounds[1]);
            u[2] = u[2].max(d.bounds[2]);
            u[3] = u[3].max(d.bounds[3]);
            u
        },
    );
    if !bounds_close(draws[0].bounds, union) {
        return Err("transparent_or_uncovered_union_area".into());
    }
    let scale = 1920.0 / page_w.max(page_h);
    let target = [
        ((union[2] - union[0]) * scale).ceil() as u32,
        ((union[3] - union[1]) * scale).ceil() as u32,
    ];
    if target[0] == 0 || target[1] == 0 {
        return Err("invalid_merged_dimensions".into());
    }
    Ok(Some(MergePage {
        page_number,
        page_id,
        draws,
        union,
        target,
    }))
}

fn decode_draw(id: ObjectId, stream: &Stream, matrix: Matrix) -> Result<Draw, String> {
    if stream.dict.has(b"SMask") || stream.dict.has(b"Mask") {
        return Err("transparency_not_supported".into());
    }
    let filter = stream
        .dict
        .get(b"Filter")
        .ok()
        .and_then(|o| o.as_name().ok());
    if filter != Some(b"DCTDecode") {
        return Err("unsupported_raster_encoding".into());
    }
    let colour = stream
        .dict
        .get(b"ColorSpace")
        .ok()
        .and_then(|o| o.as_name().ok())
        .ok_or("unknown_colour_space")?;
    if !matches!(colour, b"DeviceRGB" | b"DeviceGray") {
        return Err("unsupported_colour_space".into());
    }
    let mut decoder = Decoder::new(Cursor::new(&stream.content));
    let raw = decoder.decode().map_err(|_| "jpeg_decode_failed")?;
    let info = decoder.info().ok_or("jpeg_metadata_missing")?;
    let pixels = match info.pixel_format {
        PixelFormat::RGB24 => raw,
        PixelFormat::L8 => raw.into_iter().flat_map(|v| [v, v, v]).collect(),
        _ => return Err("jpeg_colour_mismatch".into()),
    };
    Ok(Draw {
        object_id: id,
        matrix,
        bounds: matrix.unit_square_bounds(),
        pixels,
        width: u32::from(info.width),
        height: u32::from(info.height),
    })
}

fn apply_merge(doc: &mut Document, plan: &MergePage) -> Result<(), RasterMergeError> {
    let pixels = composite(plan);
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    use std::io::Write;
    encoder.write_all(&pixels)?;
    let compressed = encoder.finish()?;
    let merged_id=doc.add_object(Stream::new(dictionary!{"Type"=>"XObject","Subtype"=>"Image","Width"=>plan.target[0] as i64,"Height"=>plan.target[1] as i64,"ColorSpace"=>"DeviceRGB","BitsPerComponent"=>8,"Filter"=>"FlateDecode"},compressed));
    let page = doc.get_dictionary(plan.page_id)?.clone();
    let mut resources = page.get(b"Resources")?.as_dict()?.clone();
    let mut xobjects = resources.get(b"XObject")?.as_dict()?.clone();
    let ids = plan.draws.iter().map(|d| d.object_id).collect::<Vec<_>>();
    let removed_names = xobjects
        .iter()
        .filter_map(|(name, object)| {
            object
                .as_reference()
                .ok()
                .filter(|id| ids.contains(id))
                .map(|_| name.clone())
        })
        .collect::<Vec<_>>();
    for name in &removed_names {
        xobjects.remove(name);
    }
    xobjects.set("NoBSMergedRaster", merged_id);
    resources.set("XObject", xobjects);
    doc.get_dictionary_mut(plan.page_id)?
        .set("Resources", resources);
    let bytes = doc.get_page_content_with_limit(plan.page_id, usize::MAX)?;
    let mut content = Content::decode(&bytes)?;
    content.operations.retain(|op| {
        if op.operator != "Do" {
            return true;
        }
        let Some(name) = op.operands.first().and_then(|o| o.as_name().ok()) else {
            return true;
        };
        !removed_names
            .iter()
            .any(|removed| removed.as_slice() == name)
    });
    let w = plan.union[2] - plan.union[0];
    let h = plan.union[3] - plan.union[1];
    let mut prefix = vec![
        Operation::new("q", vec![]),
        Operation::new(
            "cm",
            vec![
                w.into(),
                0.into(),
                0.into(),
                h.into(),
                plan.union[0].into(),
                plan.union[1].into(),
            ],
        ),
        Operation::new("Do", vec![Object::Name(b"NoBSMergedRaster".to_vec())]),
        Operation::new("Q", vec![]),
    ];
    prefix.extend(content.operations);
    content.operations = prefix;
    doc.change_page_content(plan.page_id, content.encode()?)?;
    Ok(())
}

fn composite(plan: &MergePage) -> Vec<u8> {
    let mut canvas = vec![0u8; plan.target[0] as usize * plan.target[1] as usize * 3];
    let sx = plan.target[0] as f64 / (plan.union[2] - plan.union[0]);
    let sy = plan.target[1] as f64 / (plan.union[3] - plan.union[1]);
    for draw in &plan.draws {
        let inv = invert(draw.matrix).expect("planned invertible matrix");
        let x0 = ((draw.bounds[0] - plan.union[0]) * sx).floor().max(0.0) as u32;
        let x1 = ((draw.bounds[2] - plan.union[0]) * sx)
            .ceil()
            .min(plan.target[0] as f64) as u32;
        let y0 = ((draw.bounds[1] - plan.union[1]) * sy).floor().max(0.0) as u32;
        let y1 = ((draw.bounds[3] - plan.union[1]) * sy)
            .ceil()
            .min(plan.target[1] as f64) as u32;
        for y in y0..y1 {
            for x in x0..x1 {
                let px = plan.union[0] + (x as f64 + 0.5) / sx;
                let py = plan.union[1] + (y as f64 + 0.5) / sy;
                let (u, v) = transform(inv, px, py);
                if (0.0..1.0).contains(&u) && (0.0..1.0).contains(&v) {
                    let dst = (((plan.target[1] - 1 - y) * plan.target[0] + x) * 3) as usize;
                    canvas[dst..dst + 3].copy_from_slice(&bilinear_sample(draw, u, 1.0 - v));
                }
            }
        }
    }
    canvas
}

fn bilinear_sample(draw: &Draw, u: f64, v: f64) -> [u8; 3] {
    let fx = (u * draw.width as f64 - 0.5).clamp(0.0, (draw.width - 1) as f64);
    let fy = (v * draw.height as f64 - 0.5).clamp(0.0, (draw.height - 1) as f64);
    let x0 = fx.floor() as u32;
    let y0 = fy.floor() as u32;
    let x1 = (x0 + 1).min(draw.width - 1);
    let y1 = (y0 + 1).min(draw.height - 1);
    let tx = fx - f64::from(x0);
    let ty = fy - f64::from(y0);
    let pixel = |x, y, channel| draw.pixels[((y * draw.width + x) * 3 + channel) as usize] as f64;
    let mut output = [0; 3];
    for channel in 0..3 {
        let top = pixel(x0, y0, channel) * (1.0 - tx) + pixel(x1, y0, channel) * tx;
        let bottom = pixel(x0, y1, channel) * (1.0 - tx) + pixel(x1, y1, channel) * tx;
        output[channel as usize] = (top * (1.0 - ty) + bottom * ty).round() as u8;
    }
    output
}

fn validate(
    before: &crate::model::AnalysisResult,
    after: &crate::model::AnalysisResult,
    plans: &[MergePage],
) -> Result<(), RasterMergeError> {
    if before.pages.len() != after.pages.len() {
        return Err(RasterMergeError::Validation("page count changed".into()));
    }
    for (a, b) in before.pages.iter().zip(&after.pages) {
        if !close(a.width_pt, b.width_pt)
            || !close(a.height_pt, b.height_pt)
            || a.rotation_degrees != b.rotation_degrees
        {
            return Err(RasterMergeError::Validation(format!(
                "page {} geometry changed",
                a.page_number
            )));
        }
        if a.object_counts.text_operations != b.object_counts.text_operations
            || a.object_counts.vector_operations != b.object_counts.vector_operations
        {
            return Err(RasterMergeError::Validation(format!(
                "page {} non-raster paint operations changed",
                a.page_number
            )));
        }
    }
    for p in plans {
        let page = after
            .pages
            .iter()
            .find(|v| v.page_number == p.page_number)
            .ok_or_else(|| RasterMergeError::Validation("merged page missing".into()))?;
        if page.object_counts.image_placements != 1 {
            return Err(RasterMergeError::Validation(format!(
                "page {} does not contain exactly one merged raster",
                p.page_number
            )));
        }
    }
    Ok(())
}
fn is_paint(op: &str) -> bool {
    matches!(
        op,
        "Tj" | "TJ" | "'" | "\"" | "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" | "sh"
    )
}
fn number(o: &Object) -> Option<f64> {
    match o {
        Object::Integer(v) => Some(*v as f64),
        Object::Real(v) => Some(*v as f64),
        _ => None,
    }
}
fn to_matrix(a: &[Object]) -> Option<Matrix> {
    Some(Matrix {
        a: number(&a[0])?,
        b: number(&a[1])?,
        c: number(&a[2])?,
        d: number(&a[3])?,
        e: number(&a[4])?,
        f: number(&a[5])?,
    })
}
fn invert(m: Matrix) -> Option<Matrix> {
    let d = m.a * m.d - m.b * m.c;
    if d.abs() < 1e-12 {
        return None;
    }
    Some(Matrix {
        a: m.d / d,
        b: -m.b / d,
        c: -m.c / d,
        d: m.a / d,
        e: (m.c * m.f - m.d * m.e) / d,
        f: (m.b * m.e - m.a * m.f) / d,
    })
}
fn transform(m: Matrix, x: f64, y: f64) -> (f64, f64) {
    m.transform_point(x, y)
}
fn rect_bounds(matrix: Matrix, x: f64, y: f64, w: f64, h: f64) -> [f64; 4] {
    let points = [
        matrix.transform_point(x, y),
        matrix.transform_point(x + w, y),
        matrix.transform_point(x, y + h),
        matrix.transform_point(x + w, y + h),
    ];
    points.into_iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |mut b, (px, py)| {
            b[0] = b[0].min(px);
            b[1] = b[1].min(py);
            b[2] = b[2].max(px);
            b[3] = b[3].max(py);
            b
        },
    )
}
fn intersect(a: [f64; 4], b: [f64; 4]) -> Option<[f64; 4]> {
    let r = [
        a[0].max(b[0]),
        a[1].max(b[1]),
        a[2].min(b[2]),
        a[3].min(b[3]),
    ];
    (r[2] > r[0] && r[3] > r[1]).then_some(r)
}
fn opaque_extgstate(doc: &Document, resources: &lopdf::Dictionary, name: &[u8]) -> bool {
    let Some(dict) = resources.get(b"ExtGState").ok().and_then(|o| match o {
        Object::Dictionary(d) => Some(d),
        Object::Reference(id) => doc.get_dictionary(*id).ok(),
        _ => None,
    }) else {
        return false;
    };
    let Some(state) = dict.get(name).ok().and_then(|o| match o {
        Object::Dictionary(d) => Some(d),
        Object::Reference(id) => doc.get_dictionary(*id).ok(),
        _ => None,
    }) else {
        return false;
    };
    if state
        .get(b"SMask")
        .ok()
        .is_some_and(|value| value.as_name().ok() != Some(b"None"))
    {
        return false;
    }
    let alpha_ok = |key: &[u8]| {
        state
            .get(key)
            .ok()
            .and_then(number)
            .is_none_or(|v| close(v, 1.0))
    };
    let blend_ok = state.get(b"BM").ok().is_none_or(|o| match o {
        Object::Name(name) => name == b"Normal",
        Object::Array(values) => values.first().and_then(|v| v.as_name().ok()) == Some(b"Normal"),
        _ => false,
    });
    alpha_ok(b"ca") && alpha_ok(b"CA") && blend_ok
}
fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}
fn bounds_close(a: [f64; 4], b: [f64; 4]) -> bool {
    a.into_iter().zip(b).all(|(x, y)| close(x, y))
}
fn file_hash(path: &Path) -> Result<String, std::io::Error> {
    Ok(format!("{:x}", Sha256::digest(fs::read(path)?)))
}
pub fn human_summary(r: &RasterMergeReport) -> String {
    if r.dry_run {
        format!("NoBS PDF — RASTER MERGE DRY RUN\n\nPages mergeable: {}\nPages skipped: {}\nNo PDF written.\n",r.pages_merged,r.pages_skipped)
    } else {
        format!("NoBS PDF — RASTER MERGE\n\nPages merged: {}\nPages skipped: {}\nOriginal: {:.1} MB\nOutput: {:.1} MB\nValidation: PASSED\n",r.pages_merged,r.pages_skipped,r.original_size_bytes as f64/1e6,r.output_size_bytes.unwrap_or(0)as f64/1e6)
    }
}
