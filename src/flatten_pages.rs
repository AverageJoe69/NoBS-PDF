//! Explicit full-page raster export for screen-only copies.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use jpeg_encoder::{ColorType, Encoder};
use lopdf::{
    content::{Content, Operation},
    dictionary, Document, Object, Stream,
};
use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::Builder;
use thiserror::Error;

use crate::{inspect, planner::aspect_preserving_page_pixels, InspectionError};

#[derive(Debug, Error)]
pub enum FlattenError {
    #[error("inspection failed: {0}")]
    Inspection(#[from] InspectionError),
    #[error("PDF error: {0}")]
    Pdf(#[from] lopdf::Error),
    #[error("PDFium error: {0}")]
    Pdfium(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("source and output paths must differ")]
    SourceWouldBeOverwritten,
    #[error("output already exists: {0}")]
    OutputExists(String),
    #[error("rotated pages are not yet safe for flattening (page {0})")]
    RotatedPage(u32),
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("could not persist output: {0}")]
    Persist(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FlattenReport {
    pub schema_version: String,
    pub dry_run: bool,
    pub mode: String,
    pub input_path: String,
    pub output_path: Option<String>,
    pub document_target: String,
    pub long_dimension_px: u32,
    pub pages: Vec<FlattenPageReport>,
    pub original_size_bytes: u64,
    pub output_size_bytes: Option<u64>,
    pub saved_bytes: Option<u64>,
    pub source_sha256: String,
    pub output_sha256: Option<String>,
    pub validation: Option<FlattenValidation>,
    pub destructive_changes: Vec<String>,
    pub preserved: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlattenPageReport {
    pub page_number: u32,
    pub width_px: u32,
    pub height_px: u32,
    pub original_text_operations: usize,
    pub original_vector_operations: usize,
    pub annotations_preserved: bool,
    pub mean_render_error: Option<f64>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct FlattenValidation {
    pub passed: bool,
    pub page_count_preserved: bool,
    pub page_boxes_preserved: bool,
    pub page_rotation_preserved: bool,
    pub annotations_preserved: bool,
    pub exactly_one_page_image: bool,
    pub render_comparison_passed: bool,
    pub maximum_mean_render_error: f64,
}
#[derive(Clone)]
struct RenderedPage {
    width: u32,
    height: u32,
    rgb: Vec<u8>,
}

pub fn flatten_1080p(
    input: &Path,
    output: &Path,
    dry_run: bool,
    pdfium_library: Option<&Path>,
) -> Result<FlattenReport, FlattenError> {
    flatten_pages(input, output, dry_run, pdfium_library, 1920, "1080p")
}

pub fn flatten_pages(
    input: &Path,
    output: &Path,
    dry_run: bool,
    pdfium_library: Option<&Path>,
    long_dimension_px: u32,
    profile: &str,
) -> Result<FlattenReport, FlattenError> {
    flatten_pages_impl(
        input,
        output,
        dry_run,
        pdfium_library,
        long_dimension_px,
        profile,
        false,
    )
}

pub fn flatten_pages_preserve_text(
    input: &Path,
    output: &Path,
    dry_run: bool,
    pdfium_library: Option<&Path>,
    long_dimension_px: u32,
    profile: &str,
) -> Result<FlattenReport, FlattenError> {
    flatten_pages_impl(
        input,
        output,
        dry_run,
        pdfium_library,
        long_dimension_px,
        profile,
        true,
    )
}

fn flatten_pages_impl(
    input: &Path,
    output: &Path,
    dry_run: bool,
    pdfium_library: Option<&Path>,
    long_dimension_px: u32,
    profile: &str,
    preserve_text: bool,
) -> Result<FlattenReport, FlattenError> {
    reject_paths(input, output, dry_run)?;
    let source_hash = file_hash(input)?;
    let before = inspect(input)?;
    for page in &before.pages {
        if page.rotation_degrees.rem_euclid(360) != 0 {
            return Err(FlattenError::RotatedPage(page.page_number));
        }
    }
    let dimensions = before
        .pages
        .iter()
        .map(|p| {
            aspect_preserving_page_pixels(p.width_pt, p.height_pt, long_dimension_px).ok_or_else(
                || FlattenError::Validation(format!("invalid geometry on page {}", p.page_number)),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut page_reports = before
        .pages
        .iter()
        .zip(&dimensions)
        .map(|(p, d)| FlattenPageReport {
            page_number: p.page_number,
            width_px: d[0],
            height_px: d[1],
            original_text_operations: p.object_counts.text_operations,
            original_vector_operations: p.object_counts.vector_operations,
            annotations_preserved: true,
            mean_render_error: None,
        })
        .collect::<Vec<_>>();
    let original_size = fs::metadata(input)?.len();
    if dry_run {
        return Ok(base_report(
            input,
            None,
            true,
            original_size,
            None,
            source_hash,
            None,
            page_reports,
            None,
            long_dimension_px,
            profile,
            preserve_text,
        ));
    }
    let library = pdfium_library
        .map(PathBuf::from)
        .unwrap_or_else(default_pdfium_path);
    let bindings =
        Pdfium::bind_to_library(&library).map_err(|e| FlattenError::Pdfium(e.to_string()))?;
    let pdfium = Pdfium::new(bindings);
    let original_rendered = render_document(&pdfium, input, &dimensions)?;
    let mut document = Document::load(input)?;
    let original_document = Document::load(input)?;
    let rendered = if preserve_text {
        let mut graphics_document = Document::load(input)?;
        for (page_number, page_id) in graphics_document.get_pages() {
            let bytes = graphics_document.get_page_content_with_limit(page_id, usize::MAX)?;
            let content = Content::decode(&bytes)?;
            let (background, _) =
                split_at_top_raster(&graphics_document, page_id, &content, page_number)?;
            graphics_document.change_page_content(page_id, background.encode()?)?;
        }
        let temporary_graphics = Builder::new()
            .prefix(".nobs-graphics-")
            .suffix(".pdf")
            .tempfile()?;
        graphics_document.save(temporary_graphics.path())?;
        render_document(&pdfium, temporary_graphics.path(), &dimensions)?
    } else {
        original_rendered.clone()
    };
    for (((page_number, page_id), bitmap), dims) in document
        .get_pages()
        .into_iter()
        .zip(&rendered)
        .zip(&dimensions)
    {
        replace_page(
            &mut document,
            page_number,
            page_id,
            bitmap,
            *dims,
            preserve_text,
        )?;
    }
    document.prune_objects();
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let temporary = Builder::new()
        .prefix(".pdfdoctor-flatten-")
        .suffix(".pdf")
        .tempfile_in(parent)?;
    document.save(temporary.path())?;
    let after = inspect(temporary.path())?;
    let rerendered = render_document(&pdfium, temporary.path(), &dimensions)?;
    let validation = validate(
        &original_document,
        &Document::load(temporary.path())?,
        &before,
        &after,
        &original_rendered,
        &rerendered,
        &mut page_reports,
    )?;
    if !validation.passed {
        return Err(FlattenError::Validation(
            "one or more structural or rendered-page checks failed".into(),
        ));
    }
    if file_hash(input)? != source_hash {
        return Err(FlattenError::Validation(
            "source changed during export".into(),
        ));
    }
    let output_size = fs::metadata(temporary.path())?.len();
    if output_size >= original_size {
        return Err(FlattenError::Validation(format!(
            "flattened output would be larger than the source ({} bytes versus {} bytes)",
            output_size, original_size
        )));
    }
    temporary
        .persist_noclobber(output)
        .map_err(|e| FlattenError::Persist(e.error.to_string()))?;
    let output_hash = file_hash(output)?;
    Ok(base_report(
        input,
        Some(output),
        false,
        original_size,
        Some(output_size),
        source_hash,
        Some(output_hash),
        page_reports,
        Some(validation),
        long_dimension_px,
        profile,
        preserve_text,
    ))
}

fn render_document(
    pdfium: &Pdfium,
    path: &Path,
    dimensions: &[[u32; 2]],
) -> Result<Vec<RenderedPage>, FlattenError> {
    let document = pdfium
        .load_pdf_from_file(path, None)
        .map_err(|e| FlattenError::Pdfium(e.to_string()))?;
    if document.pages().len() as usize != dimensions.len() {
        return Err(FlattenError::Validation("PDFium page count differs".into()));
    }
    let mut pages = Vec::with_capacity(dimensions.len());
    for (index, page) in document.pages().iter().enumerate() {
        let d = dimensions[index];
        let config = PdfRenderConfig::new()
            .set_target_size(d[0] as i32, d[1] as i32)
            .render_annotations(false)
            .render_form_data(false);
        let image = page
            .render_with_config(&config)
            .map_err(|e| FlattenError::Pdfium(e.to_string()))?
            .as_image()
            .into_rgb8();
        if image.width() != d[0] || image.height() != d[1] {
            return Err(FlattenError::Validation(format!(
                "renderer dimensions differ on page {}",
                index + 1
            )));
        }
        pages.push(RenderedPage {
            width: d[0],
            height: d[1],
            rgb: image.into_raw(),
        });
    }
    Ok(pages)
}

fn replace_page(
    doc: &mut Document,
    page_number: u32,
    page_id: lopdf::ObjectId,
    bitmap: &RenderedPage,
    dims: [u32; 2],
    preserve_text: bool,
) -> Result<(), FlattenError> {
    let mut jpeg = Vec::new();
    Encoder::new(&mut jpeg, 90)
        .encode(&bitmap.rgb, dims[0] as u16, dims[1] as u16, ColorType::Rgb)
        .map_err(|e| {
            FlattenError::Validation(format!("page {page_number} JPEG encoding failed: {e}"))
        })?;
    let image_id=doc.add_object(Stream::new(dictionary!{"Type"=>"XObject","Subtype"=>"Image","Width"=>dims[0]as i64,"Height"=>dims[1]as i64,"ColorSpace"=>"DeviceRGB","BitsPerComponent"=>8,"Filter"=>"DCTDecode"},jpeg));
    let page = doc.get_dictionary(page_id)?.clone();
    let media = page.get(b"MediaBox")?.as_array()?;
    let x0 =
        number(&media[0]).ok_or_else(|| FlattenError::Validation("invalid MediaBox".into()))?;
    let y0 =
        number(&media[1]).ok_or_else(|| FlattenError::Validation("invalid MediaBox".into()))?;
    let w = number(&media[2]).unwrap() - x0;
    let h = number(&media[3]).unwrap() - y0;
    let mut operations = vec![
        Operation::new("q", vec![]),
        Operation::new(
            "cm",
            vec![w.into(), 0.into(), 0.into(), h.into(), x0.into(), y0.into()],
        ),
        Operation::new("Do", vec![Object::Name(b"NoBSFlattenedPage".to_vec())]),
        Operation::new("Q", vec![]),
    ];
    let mut resources = if preserve_text {
        let object = page.get(b"Resources")?.clone();
        match object {
            Object::Dictionary(value) => value,
            Object::Reference(id) => doc.get_dictionary(id)?.clone(),
            _ => {
                return Err(FlattenError::Validation(
                    "unsupported page resources".into(),
                ))
            }
        }
    } else {
        lopdf::Dictionary::new()
    };
    let mut foreground_xobject_names = HashSet::new();
    if preserve_text {
        let original = Content::decode(&doc.get_page_content_with_limit(page_id, usize::MAX)?)?;
        let (_, foreground) = split_at_top_raster(doc, page_id, &original, page_number)?;
        foreground_xobject_names.extend(
            foreground
                .operations
                .iter()
                .filter(|operation| operation.operator == "Do")
                .filter_map(|operation| {
                    operation
                        .operands
                        .first()
                        .and_then(|operand| operand.as_name().ok())
                        .map(Vec::from)
                }),
        );
        operations.extend(foreground.operations);
    }
    let original_xobjects = match resources.get(b"XObject").ok().cloned() {
        Some(Object::Dictionary(value)) => value,
        Some(Object::Reference(id)) => doc.get_dictionary(id)?.clone(),
        _ => lopdf::Dictionary::new(),
    };
    let mut xobjects = lopdf::Dictionary::new();
    for (name, value) in original_xobjects.iter() {
        if foreground_xobject_names.contains(name) {
            xobjects.set(name.clone(), value.clone());
        }
    }
    xobjects.set("NoBSFlattenedPage", image_id);
    resources.set("XObject", xobjects);
    let content = Content { operations };
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode()?));
    let page_mut = doc.get_dictionary_mut(page_id)?;
    page_mut.set("Contents", content_id);
    page_mut.set("Resources", resources);
    Ok(())
}

fn split_at_top_raster(
    document: &Document,
    page_id: lopdf::ObjectId,
    content: &Content,
    page_number: u32,
) -> Result<(Content, Content), FlattenError> {
    let raster_names = page_raster_names(document, page_id)?;
    let last_raster = content
        .operations
        .iter()
        .rposition(|operation| {
            operation.operator == "Do"
                && operation
                    .operands
                    .first()
                    .and_then(|value| value.as_name().ok())
                    .is_some_and(|name| raster_names.contains(name))
        })
        .ok_or_else(|| {
            FlattenError::Validation(format!("page {page_number} has no raster layer boundary"))
        })?;
    let mut depth = 0_i32;
    let mut in_text = false;
    let mut boundary = None;
    for (index, operation) in content.operations.iter().enumerate() {
        match operation.operator.as_str() {
            "q" => depth += 1,
            "Q" => {
                depth -= 1;
                if depth < 0 {
                    return Err(FlattenError::Validation(format!(
                        "page {page_number} has unbalanced graphics state"
                    )));
                }
            }
            "BT" => in_text = true,
            "ET" => in_text = false,
            _ => {}
        }
        if index >= last_raster && depth == 0 && !in_text {
            boundary = Some(index + 1);
            break;
        }
    }
    let boundary = boundary.ok_or_else(|| {
        FlattenError::Validation(format!(
            "page {page_number} has no safe boundary after its top raster layer"
        ))
    })?;
    Ok((
        Content {
            operations: content.operations[..boundary].to_vec(),
        },
        Content {
            operations: content.operations[boundary..].to_vec(),
        },
    ))
}

fn page_raster_names(
    document: &Document,
    page_id: lopdf::ObjectId,
) -> Result<HashSet<Vec<u8>>, FlattenError> {
    let page = document.get_dictionary(page_id)?;
    let resources = match page.get(b"Resources")? {
        Object::Dictionary(value) => value,
        Object::Reference(id) => document.get_dictionary(*id)?,
        _ => {
            return Err(FlattenError::Validation(
                "unsupported page resources".into(),
            ))
        }
    };
    let xobjects = match resources.get(b"XObject") {
        Ok(Object::Dictionary(value)) => value,
        Ok(Object::Reference(id)) => document.get_dictionary(*id)?,
        _ => return Ok(HashSet::new()),
    };
    Ok(xobjects
        .iter()
        .filter_map(|(name, value)| {
            let id = value.as_reference().ok()?;
            xobject_contains_raster(document, id, &mut HashSet::new()).then(|| name.clone())
        })
        .collect())
}

fn xobject_contains_raster(
    document: &Document,
    id: lopdf::ObjectId,
    visited: &mut HashSet<lopdf::ObjectId>,
) -> bool {
    if !visited.insert(id) {
        return false;
    }
    let Ok(stream) = document.get_object(id).and_then(Object::as_stream) else {
        return false;
    };
    match stream
        .dict
        .get(b"Subtype")
        .ok()
        .and_then(|value| value.as_name().ok())
    {
        Some(b"Image") => true,
        Some(b"Form") => {
            let Ok(content) = stream
                .decompressed_content()
                .or_else(|_| Ok::<_, lopdf::Error>(stream.content.clone()))
                .and_then(|bytes| Content::decode(&bytes))
            else {
                return false;
            };
            let resources = match stream.dict.get(b"Resources") {
                Ok(Object::Dictionary(value)) => value,
                Ok(Object::Reference(resource_id)) => match document.get_dictionary(*resource_id) {
                    Ok(value) => value,
                    Err(_) => return false,
                },
                _ => return false,
            };
            let xobjects = match resources.get(b"XObject") {
                Ok(Object::Dictionary(value)) => value,
                Ok(Object::Reference(resource_id)) => match document.get_dictionary(*resource_id) {
                    Ok(value) => value,
                    Err(_) => return false,
                },
                _ => return false,
            };
            content
                .operations
                .iter()
                .filter(|operation| operation.operator == "Do")
                .any(|operation| {
                    let Some(name) = operation
                        .operands
                        .first()
                        .and_then(|value| value.as_name().ok())
                    else {
                        return false;
                    };
                    let Some(child) = xobjects
                        .get(name)
                        .ok()
                        .and_then(|value| value.as_reference().ok())
                    else {
                        return false;
                    };
                    xobject_contains_raster(document, child, visited)
                })
        }
        _ => false,
    }
}

fn validate(
    original: &Document,
    output: &Document,
    before: &crate::model::AnalysisResult,
    after: &crate::model::AnalysisResult,
    rendered: &[RenderedPage],
    rerendered: &[RenderedPage],
    reports: &mut [FlattenPageReport],
) -> Result<FlattenValidation, FlattenError> {
    let page_count = before.pages.len() == after.pages.len();
    let mut boxes = true;
    let mut rotations = true;
    let mut annotations = true;
    for ((_, a), (_, b)) in original.get_pages().into_iter().zip(output.get_pages()) {
        let ad = original.get_dictionary(a)?;
        let bd = output.get_dictionary(b)?;
        for key in [
            b"MediaBox" as &[u8],
            b"CropBox",
            b"TrimBox",
            b"BleedBox",
            b"ArtBox",
        ] {
            if format!("{:?}", ad.get(key).ok()) != format!("{:?}", bd.get(key).ok()) {
                boxes = false
            }
        }
        if format!("{:?}", ad.get(b"Rotate").ok()) != format!("{:?}", bd.get(b"Rotate").ok()) {
            rotations = false
        }
        if format!("{:?}", ad.get(b"Annots").ok()) != format!("{:?}", bd.get(b"Annots").ok()) {
            annotations = false
        }
    }
    let one_image = after
        .pages
        .iter()
        .all(|p| p.object_counts.image_placements == 1);
    let mut render_ok = true;
    let mut maximum = 0.0_f64;
    for ((a, b), report) in rendered.iter().zip(rerendered).zip(reports) {
        if a.width != b.width || a.height != b.height {
            return Err(FlattenError::Validation(
                "rendered dimensions changed".into(),
            ));
        }
        let mean = a
            .rgb
            .iter()
            .zip(&b.rgb)
            .map(|(x, y)| f64::from(x.abs_diff(*y)))
            .sum::<f64>()
            / a.rgb.len() as f64;
        report.mean_render_error = Some(mean);
        maximum = maximum.max(mean);
        if mean > 12.0 {
            render_ok = false
        }
    }
    Ok(FlattenValidation {
        passed: page_count && boxes && rotations && annotations && one_image && render_ok,
        page_count_preserved: page_count,
        page_boxes_preserved: boxes,
        page_rotation_preserved: rotations,
        annotations_preserved: annotations,
        exactly_one_page_image: one_image,
        render_comparison_passed: render_ok,
        maximum_mean_render_error: maximum,
    })
}

#[allow(clippy::too_many_arguments)] // Report assembly mirrors the public schema fields.
fn base_report(
    input: &Path,
    output: Option<&Path>,
    dry: bool,
    original: u64,
    out: Option<u64>,
    source_hash: String,
    output_hash: Option<String>,
    pages: Vec<FlattenPageReport>,
    validation: Option<FlattenValidation>,
    long_dimension_px: u32,
    profile: &str,
    preserve_text: bool,
) -> FlattenReport {
    FlattenReport {
        schema_version: "0.1.0".into(),
        dry_run: dry,
        mode: if preserve_text {
            "graphics_raster_text_preserved"
        } else {
            "full_page_raster"
        }
        .into(),
        input_path: input.display().to_string(),
        output_path: output.map(|p| p.display().to_string()),
        document_target: profile.into(),
        long_dimension_px,
        pages,
        original_size_bytes: original,
        output_size_bytes: out,
        saved_bytes: out.map(|v| original.saturating_sub(v)),
        source_sha256: source_hash,
        output_sha256: output_hash,
        validation,
        destructive_changes: if preserve_text {
            vec!["Raster images and vector artwork are flattened into a page background.".into()]
        } else {
            vec![
                "Text is rasterised and is no longer selectable/searchable.".into(),
                "Vector artwork is rasterised and no longer resolution-independent.".into(),
                "Accessibility structure tied to page content may no longer function.".into(),
            ]
        },
        preserved: vec![
            "Page boxes, dimensions, aspect ratios, page count, and rotation.".into(),
            if preserve_text {
                "Original text operators and font resources remain selectable/searchable.".into()
            } else {
                "Annotation and link dictionaries; annotation appearances are not baked into page bitmaps.".into()
            },
        ],
    }
}
fn reject_paths(input: &Path, output: &Path, dry: bool) -> Result<(), FlattenError> {
    let a = fs::canonicalize(input)?;
    let b = if output.exists() {
        fs::canonicalize(output)?
    } else if output.is_absolute() {
        output.to_path_buf()
    } else {
        std::env::current_dir()?.join(output)
    };
    if a == b {
        return Err(FlattenError::SourceWouldBeOverwritten);
    }
    if !dry && output.exists() {
        return Err(FlattenError::OutputExists(output.display().to_string()));
    }
    Ok(())
}
fn default_pdfium_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vendor/pdfium/lib/libpdfium.dylib")
}
fn file_hash(path: &Path) -> Result<String, std::io::Error> {
    Ok(format!("{:x}", Sha256::digest(fs::read(path)?)))
}
fn number(o: &Object) -> Option<f64> {
    match o {
        Object::Integer(v) => Some(*v as f64),
        Object::Real(v) => Some(*v as f64),
        _ => None,
    }
}
pub fn human_summary(r: &FlattenReport) -> String {
    if r.dry_run {
        format!("NoBS PDF — FULL-PAGE RASTER DRY RUN\n\nPages: {}\nTarget: 1080p\nWARNING: text and vectors will become pixels.\nNo PDF written.\n",r.pages.len())
    } else {
        format!("NoBS PDF — FULL-PAGE RASTER EXPORT\n\nPages flattened: {}\nOriginal: {:.1} MB\nOutput: {:.1} MB\nSaved: {:.1} MB\nValidation: PASSED\n\nWARNING: text and vectors are rasterised.\n",r.pages.len(),r.original_size_bytes as f64/1e6,r.output_size_bytes.unwrap_or(0)as f64/1e6,r.saved_bytes.unwrap_or(0)as f64/1e6)
    }
}
