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

use crate::{
    inspect,
    planner::aspect_preserving_page_pixels,
    resolution::{detect_document_resolution, scale_dimensions},
    InspectionError,
};

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
    #[error("hybrid foreground-text export is unavailable: {0}")]
    HybridUnavailable(String),
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
    /// Zero-based content-stream operation index immediately after the artwork
    /// prefix that was baked into the page raster.
    pub flatten_boundary_operation: Option<usize>,
    pub flatten_boundary_reason: Option<String>,
    pub text_operations_below_boundary: usize,
    pub text_operations_above_boundary: usize,
    pub native_text_operations_retained: usize,
    pub searchable_foreground_text_retained: Option<bool>,
    pub raster_artwork_dimensions: [u32; 2],
    pub visual_validation_passed: Option<bool>,
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
    pub foreground_text_extraction_preserved: bool,
    pub document_navigation_preserved: bool,
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
        None,
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
        None,
    )
}

/// Export using the detected document raster budget while baking artwork below
/// the final safe paint-order boundary and retaining only foreground text above it.
pub fn flatten_pages_preserve_foreground_text_for_scale(
    input: &Path,
    output: &Path,
    dry_run: bool,
    pdfium_library: Option<&Path>,
    scale_percent: u8,
) -> Result<FlattenReport, FlattenError> {
    let inspection = inspect(input)?;
    let source_document = Document::load(input)?;
    if source_document.catalog()?.has(b"StructTreeRoot") {
        return Err(FlattenError::HybridUnavailable(
            "tagged PDF structure requires the native fallback".into(),
        ));
    }
    if let Some(page) = inspection
        .pages
        .iter()
        .find(|page| page.rotation_degrees.rem_euclid(360) != 0)
    {
        return Err(FlattenError::HybridUnavailable(format!(
            "page {} is rotated and requires the native fallback",
            page.page_number
        )));
    }
    let resolution = detect_document_resolution(&inspection);
    let dimensions = resolution
        .pages
        .iter()
        .map(|page| {
            page.budget_100_percent
                .map(|budget| scale_dimensions(budget, scale_percent))
                .ok_or_else(|| {
                    FlattenError::HybridUnavailable(format!(
                        "page {} has no meaningful raster artwork budget",
                        page.page_number
                    ))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    flatten_pages_impl(
        input,
        output,
        dry_run,
        pdfium_library,
        dimensions.iter().map(|d| d[0].max(d[1])).max().unwrap_or(0),
        &format!("scale_{scale_percent}"),
        true,
        Some(dimensions),
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
    dimensions_override: Option<Vec<[u32; 2]>>,
) -> Result<FlattenReport, FlattenError> {
    reject_paths(input, output, dry_run)?;
    let source_hash = file_hash(input)?;
    let before = inspect(input)?;
    for page in &before.pages {
        if page.rotation_degrees.rem_euclid(360) != 0 {
            return Err(FlattenError::RotatedPage(page.page_number));
        }
    }
    let dimensions = if let Some(dimensions) = dimensions_override {
        dimensions
    } else {
        before
            .pages
            .iter()
            .map(|p| {
                aspect_preserving_page_pixels(p.width_pt, p.height_pt, long_dimension_px)
                    .ok_or_else(|| {
                        FlattenError::Validation(format!(
                            "invalid geometry on page {}",
                            p.page_number
                        ))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?
    };
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
            flatten_boundary_operation: None,
            flatten_boundary_reason: None,
            text_operations_below_boundary: 0,
            text_operations_above_boundary: 0,
            native_text_operations_retained: 0,
            searchable_foreground_text_retained: None,
            raster_artwork_dimensions: *d,
            visual_validation_passed: None,
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
            let split =
                split_at_flatten_boundary(&graphics_document, page_id, &content, page_number)?;
            let report = &mut page_reports[(page_number - 1) as usize];
            report.flatten_boundary_operation = Some(split.operation_index);
            report.flatten_boundary_reason = Some(split.reason.clone());
            report.text_operations_above_boundary = split.foreground_text_operations;
            report.text_operations_below_boundary = report
                .original_text_operations
                .saturating_sub(split.foreground_text_operations);
            let background = split.background;
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
    let expected_foreground_text = preserve_text
        .then(|| extract_foreground_text(&original_document))
        .transpose()?;
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
        expected_foreground_text.as_deref(),
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
        let foreground =
            split_at_flatten_boundary(doc, page_id, &original, page_number)?.foreground;
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

struct FlattenSplit {
    background: Content,
    foreground: Content,
    operation_index: usize,
    reason: String,
    foreground_text_operations: usize,
}

fn split_at_flatten_boundary(
    document: &Document,
    page_id: lopdf::ObjectId,
    content: &Content,
    page_number: u32,
) -> Result<FlattenSplit, FlattenError> {
    let artwork_names = page_artwork_names(document, page_id)?;
    let last_artwork = content
        .operations
        .iter()
        .rposition(|operation| is_artwork_paint_operation(operation, &artwork_names))
        .ok_or_else(|| {
            FlattenError::HybridUnavailable(format!(
                "page {page_number} has no artwork flatten boundary"
            ))
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
        if index >= last_artwork && depth == 0 && !in_text {
            boundary = Some(index + 1);
            break;
        }
    }
    let boundary = boundary.ok_or_else(|| {
        FlattenError::HybridUnavailable(format!(
            "page {page_number} has no safe boundary after its top raster layer"
        ))
    })?;
    let foreground = Content {
        operations: content.operations[boundary..].to_vec(),
    };
    let page_resources = page_resources(document, page_id)?;
    let foreground_text_operations =
        count_text_paint_operations(document, page_resources, &foreground, &mut HashSet::new());
    Ok(FlattenSplit {
        background: Content {
            operations: content.operations[..boundary].to_vec(),
        },
        foreground,
        operation_index: boundary,
        reason: format!(
            "after the final non-text artwork paint operation ({last_artwork}); advanced to operation {boundary} to close text and graphics-state scopes"
        ),
        foreground_text_operations,
    })
}

fn extract_foreground_text(document: &Document) -> Result<Vec<String>, FlattenError> {
    let mut foreground = document.clone();
    for (page_number, page_id) in foreground.get_pages() {
        let bytes = foreground.get_page_content_with_limit(page_id, usize::MAX)?;
        let content = Content::decode(&bytes)?;
        let suffix =
            split_at_flatten_boundary(&foreground, page_id, &content, page_number)?.foreground;
        foreground.change_page_content(page_id, suffix.encode()?)?;
    }
    Ok(foreground
        .get_pages()
        .keys()
        .map(|page_number| foreground.extract_text(&[*page_number]).unwrap_or_default())
        .collect())
}

fn page_resources(
    document: &Document,
    page_id: lopdf::ObjectId,
) -> Result<&lopdf::Dictionary, FlattenError> {
    let page = document.get_dictionary(page_id)?;
    match page.get(b"Resources")? {
        Object::Dictionary(resources) => Ok(resources),
        Object::Reference(id) => Ok(document.get_dictionary(*id)?),
        _ => Err(FlattenError::Validation(
            "unsupported page resources".into(),
        )),
    }
}

fn count_text_paint_operations(
    document: &Document,
    resources: &lopdf::Dictionary,
    content: &Content,
    active_forms: &mut HashSet<lopdf::ObjectId>,
) -> usize {
    let mut count = content
        .operations
        .iter()
        .filter(|operation| matches!(operation.operator.as_str(), "Tj" | "TJ" | "'" | "\""))
        .count();
    let xobjects = match resources.get(b"XObject") {
        Ok(Object::Dictionary(value)) => value,
        Ok(Object::Reference(id)) => match document.get_dictionary(*id) {
            Ok(value) => value,
            Err(_) => return count,
        },
        _ => return count,
    };
    for operation in content
        .operations
        .iter()
        .filter(|operation| operation.operator == "Do")
    {
        let Some(name) = operation
            .operands
            .first()
            .and_then(|value| value.as_name().ok())
        else {
            continue;
        };
        let Some(id) = xobjects
            .get(name)
            .ok()
            .and_then(|value| value.as_reference().ok())
        else {
            continue;
        };
        if !active_forms.insert(id) {
            continue;
        }
        let Some(stream) = document
            .get_object(id)
            .ok()
            .and_then(|value| value.as_stream().ok())
        else {
            active_forms.remove(&id);
            continue;
        };
        if stream
            .dict
            .get(b"Subtype")
            .ok()
            .and_then(|value| value.as_name().ok())
            != Some(b"Form")
        {
            active_forms.remove(&id);
            continue;
        }
        let form_content = stream
            .decompressed_content()
            .or_else(|_| Ok::<_, lopdf::Error>(stream.content.clone()))
            .and_then(|bytes| Content::decode(&bytes));
        if let Ok(form_content) = form_content {
            let form_resources = match stream.dict.get(b"Resources") {
                Ok(Object::Dictionary(value)) => value,
                Ok(Object::Reference(resource_id)) => {
                    document.get_dictionary(*resource_id).unwrap_or(resources)
                }
                _ => resources,
            };
            count +=
                count_text_paint_operations(document, form_resources, &form_content, active_forms);
        }
        active_forms.remove(&id);
    }
    count
}

fn is_artwork_paint_operation(operation: &Operation, artwork_names: &HashSet<Vec<u8>>) -> bool {
    match operation.operator.as_str() {
        "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" | "sh" | "BI" => true,
        "Do" => operation
            .operands
            .first()
            .and_then(|value| value.as_name().ok())
            .is_some_and(|name| artwork_names.contains(name)),
        _ => false,
    }
}

fn page_artwork_names(
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
            xobject_contains_artwork(document, id, &mut HashSet::new()).then(|| name.clone())
        })
        .collect())
}

fn xobject_contains_artwork(
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
            if stream.dict.has(b"Group") {
                return true;
            }
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
            let artwork_names = xobjects
                .iter()
                .filter_map(|(name, value)| {
                    let child = value.as_reference().ok()?;
                    xobject_contains_artwork(document, child, visited).then(|| name.clone())
                })
                .collect::<HashSet<_>>();
            content.operations.iter().any(|operation| {
                if is_artwork_paint_operation(operation, &artwork_names) {
                    return true;
                }
                if operation.operator != "Do" {
                    return false;
                }
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
                xobject_contains_artwork(document, child, visited)
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
    expected_foreground_text: Option<&[String]>,
    reports: &mut [FlattenPageReport],
) -> Result<FlattenValidation, FlattenError> {
    let page_count = before.pages.len() == after.pages.len();
    let mut boxes = true;
    let mut rotations = true;
    let mut annotations = true;
    let navigation = catalog_references_preserved(original, output);
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
    for ((a, b), report) in rendered.iter().zip(rerendered).zip(reports.iter_mut()) {
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
        report.visual_validation_passed = Some(mean <= 12.0);
        maximum = maximum.max(mean);
        if mean > 12.0 {
            render_ok = false
        }
    }
    for (output_page, report) in after.pages.iter().zip(reports.iter_mut()) {
        report.native_text_operations_retained = output_page.object_counts.text_operations;
        if report.native_text_operations_retained != report.text_operations_above_boundary {
            render_ok = false;
            report.visual_validation_passed = Some(false);
        }
    }
    let mut foreground_text_ok = true;
    if let Some(expected) = expected_foreground_text {
        for ((page_number, expected_text), report) in output
            .get_pages()
            .keys()
            .zip(expected)
            .zip(reports.iter_mut())
        {
            let actual = output.extract_text(&[*page_number]).unwrap_or_default();
            let retained =
                normalize_extracted_text(&actual) == normalize_extracted_text(expected_text);
            report.searchable_foreground_text_retained = Some(retained);
            foreground_text_ok &= retained;
        }
    }
    Ok(FlattenValidation {
        passed: page_count
            && boxes
            && rotations
            && annotations
            && one_image
            && render_ok
            && foreground_text_ok
            && navigation,
        page_count_preserved: page_count,
        page_boxes_preserved: boxes,
        page_rotation_preserved: rotations,
        annotations_preserved: annotations,
        exactly_one_page_image: one_image,
        render_comparison_passed: render_ok,
        foreground_text_extraction_preserved: foreground_text_ok,
        document_navigation_preserved: navigation,
        maximum_mean_render_error: maximum,
    })
}

fn catalog_references_preserved(original: &Document, output: &Document) -> bool {
    let Ok(before) = original.catalog() else {
        return false;
    };
    let Ok(after) = output.catalog() else {
        return false;
    };
    [b"Outlines".as_slice(), b"Names", b"Dests", b"PageLabels"]
        .iter()
        .all(|key| format!("{:?}", before.get(key).ok()) == format!("{:?}", after.get(key).ok()))
}

fn normalize_extracted_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
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
            vec![
                "Raster images, vector artwork, and text below the paint-order boundary are flattened into a page artwork layer.".into(),
            ]
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
                "Original foreground text operators and required font resources above the paint-order boundary remain selectable/searchable.".into()
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
    let relative = if cfg!(target_os = "windows") {
        "vendor/pdfium/bin/pdfium.dll"
    } else {
        "vendor/pdfium/lib/libpdfium.dylib"
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
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

#[cfg(test)]
mod boundary_tests {
    use super::*;

    #[test]
    fn boundary_keeps_only_text_after_final_artwork_paint() {
        let mut document = Document::with_version("1.7");
        let image = document.add_object(Stream::new(
            dictionary! {"Type"=>"XObject", "Subtype"=>"Image", "Width"=>1, "Height"=>1},
            vec![0],
        ));
        let page_id = document.add_object(dictionary! {
            "Type"=>"Page",
            "Resources"=>dictionary!{"XObject"=>dictionary!{"Im0"=>image}},
        });
        let content = Content {
            operations: vec![
                Operation::new("Do", vec![Object::Name(b"Im0".to_vec())]),
                Operation::new("BT", vec![]),
                Operation::new("Tj", vec![Object::string_literal("below")]),
                Operation::new("ET", vec![]),
                Operation::new("re", vec![0.into(), 0.into(), 10.into(), 10.into()]),
                Operation::new("f", vec![]),
                Operation::new("BT", vec![]),
                Operation::new("Tj", vec![Object::string_literal("above")]),
                Operation::new("ET", vec![]),
            ],
        };
        let split = split_at_flatten_boundary(&document, page_id, &content, 1).unwrap();
        assert_eq!(split.operation_index, 6);
        assert_eq!(split.foreground_text_operations, 1);
        assert_eq!(split.foreground.operations[1].operator, "Tj");
    }
}
pub fn human_summary(r: &FlattenReport) -> String {
    if r.dry_run {
        format!("NoBS PDF — FULL-PAGE RASTER DRY RUN\n\nPages: {}\nTarget: 1080p\nWARNING: text and vectors will become pixels.\nNo PDF written.\n",r.pages.len())
    } else {
        format!("NoBS PDF — FULL-PAGE RASTER EXPORT\n\nPages flattened: {}\nOriginal: {:.1} MB\nOutput: {:.1} MB\nSaved: {:.1} MB\nValidation: PASSED\n\nWARNING: text and vectors are rasterised.\n",r.pages.len(),r.original_size_bytes as f64/1e6,r.output_size_bytes.unwrap_or(0)as f64/1e6,r.saved_bytes.unwrap_or(0)as f64/1e6)
    }
}
