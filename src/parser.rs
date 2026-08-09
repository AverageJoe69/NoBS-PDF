use lopdf::{content::Content, Dictionary, Document, Object, ObjectId, Stream};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::Path,
};
use thiserror::Error;

use crate::{
    analysis::analyse_placement,
    geometry::{pt_to_mm, Matrix},
    model::*,
};

#[derive(Debug, Error)]
pub enum InspectionError {
    #[error("cannot read input: {0}")]
    Io(#[from] std::io::Error),
    #[error("cannot parse PDF: {0}")]
    Pdf(#[from] lopdf::Error),
}

pub trait PdfParser {
    fn inspect(&self, path: &Path) -> Result<AnalysisResult, InspectionError>;
}

#[derive(Default)]
pub struct LopdfParser;

#[derive(Default)]
struct ImageWork {
    width: u32,
    height: u32,
    colour_space: Option<String>,
    bpc: Option<u16>,
    filters: Vec<String>,
    bytes: u64,
    hash: String,
    placements: Vec<ImagePlacement>,
}

impl PdfParser for LopdfParser {
    fn inspect(&self, path: &Path) -> Result<AnalysisResult, InspectionError> {
        let file_size = fs::metadata(path)?.len();
        let doc = Document::load(path)?;
        let page_map = doc.get_pages();
        let mut images: BTreeMap<ObjectId, ImageWork> = BTreeMap::new();
        let mut warnings = vec![];
        let mut pages = vec![];

        for (page_number, page_id) in &page_map {
            let page_dict = doc.get_dictionary(*page_id)?;
            let (width, height) = page_size(&doc, page_dict).unwrap_or((0.0, 0.0));
            let rotation = inherited_number(&doc, page_dict, b"Rotate").unwrap_or(0.0) as i32;
            let resources = inherited_dict(&doc, page_dict, b"Resources");
            let mut counts = ObjectCounts::default();
            match doc
                .get_page_content(*page_id)
                .and_then(|bytes| Content::decode(&bytes))
            {
                Ok(content) => walk_content(
                    &doc,
                    &content,
                    resources.as_ref(),
                    Matrix::IDENTITY,
                    *page_number,
                    &mut images,
                    &mut counts,
                    &mut warnings,
                    &mut HashSet::new(),
                ),
                Err(error) => warnings.push(AnalysisWarning {
                    page_number: Some(*page_number),
                    object_id: None,
                    message: format!("page content unavailable: {error}"),
                }),
            }
            pages.push(PdfPage {
                page_number: *page_number,
                width_pt: width,
                height_pt: height,
                width_mm: pt_to_mm(width),
                height_mm: pt_to_mm(height),
                rotation_degrees: rotation,
                object_counts: counts,
            });
        }

        // Catalogue images even when never invoked by a page content stream.
        for (&id, object) in &doc.objects {
            if let Ok(stream) = object.as_stream() {
                if name(stream.dict.get(b"Subtype").ok()) == Some("Image".into()) {
                    images.entry(id).or_insert_with(|| image_work(stream));
                }
            }
        }

        let mut first_hash: HashMap<String, String> = HashMap::new();
        let mut image_models = vec![];
        for (id, work) in images {
            let object_id = format!("{} {} R", id.0, id.1);
            let duplicate_of = first_hash.get(&work.hash).cloned();
            first_hash
                .entry(work.hash.clone())
                .or_insert_with(|| object_id.clone());
            let pages_used = work
                .placements
                .iter()
                .map(|p| p.page_number)
                .collect::<HashSet<_>>();
            let refs = work.placements.len();
            image_models.push(ImageAnalysis {
                id: object_id,
                object_number: id.0,
                generation: id.1,
                pages: {
                    let mut v = pages_used.into_iter().collect::<Vec<_>>();
                    v.sort();
                    v
                },
                placements: work.placements,
                pixel_width: work.width,
                pixel_height: work.height,
                colour_space: work.colour_space,
                bits_per_component: work.bpc,
                filters: work.filters,
                encoded_bytes: work.bytes,
                estimated_raw_pixel_bytes: estimated_raw(work.width, work.height, work.bpc),
                sha256: work.hash,
                same_object_reused: refs > 1,
                identical_data_duplicate: duplicate_of.is_some(),
                duplicate_of,
                reference_count: refs,
            });
        }

        let (fonts, embedded_files, metadata) = catalogue_streams(&doc);
        let image_bytes = image_models.iter().map(|x| x.encoded_bytes).sum();
        let font_bytes = fonts.iter().map(|x| x.encoded_bytes).sum();
        let embedded_file_bytes = embedded_files.iter().map(|x| x.encoded_bytes).sum();
        let metadata_bytes = metadata.iter().map(|x| x.encoded_bytes).sum();
        let attributed = image_bytes + font_bytes + embedded_file_bytes + metadata_bytes;
        let summary = SizeSummary { image_bytes, font_bytes, embedded_file_bytes, metadata_bytes, other_bytes: 0, unknown_bytes: file_size.saturating_sub(attributed), attribution: "Encoded stream payload sizes are exact; PDF container overhead and unclassified objects remain unknown.".into() };
        Ok(AnalysisResult {
            schema_version: "0.1.0".into(),
            file: FileInfo {
                path: path.display().to_string(),
                size_bytes: file_size,
                page_count: page_map.len(),
            },
            summary,
            pages,
            images: image_models,
            fonts,
            embedded_files,
            metadata,
            warnings,
        })
    }
}

#[allow(clippy::too_many_arguments)] // Explicit traversal state keeps recursive ownership clear.
fn walk_content(
    doc: &Document,
    content: &Content,
    resources: Option<&Dictionary>,
    initial: Matrix,
    page: u32,
    images: &mut BTreeMap<ObjectId, ImageWork>,
    counts: &mut ObjectCounts,
    warnings: &mut Vec<AnalysisWarning>,
    visiting: &mut HashSet<ObjectId>,
) {
    let mut ctm = initial;
    let mut stack = vec![];
    for op in &content.operations {
        match op.operator.as_str() {
            "q" => stack.push(ctm), "Q" => if let Some(saved) = stack.pop() { ctm = saved },
            "cm" if op.operands.len() == 6 => if let Some(m) = matrix(&op.operands) { ctm = m.concat(ctm) },
            "Tj" | "TJ" | "'" | "\"" => counts.text_operations += 1,
            "m" | "l" | "c" | "v" | "y" | "re" | "S" | "s" | "f" | "F" | "f*" | "B" | "B*" => counts.vector_operations += 1,
            "Do" => if let Some(resource_name) = op.operands.first().and_then(|o| o.as_name().ok()) {
                if let Some((id, stream)) = resolve_xobject(doc, resources, resource_name) {
                    match name(stream.dict.get(b"Subtype").ok()).as_deref() {
                        Some("Image") => { let entry = images.entry(id).or_insert_with(|| image_work(stream)); entry.placements.push(analyse_placement(page, ctm, entry.width, entry.height)); counts.image_placements += 1; }
                        Some("Form") if visiting.insert(id) => {
                            let form_matrix = stream.dict.get(b"Matrix").ok().and_then(|o| o.as_array().ok()).and_then(|a| matrix(a)).unwrap_or(Matrix::IDENTITY);
                            let form_resources = stream.dict.get(b"Resources").ok().and_then(|o| resolve_dict(doc, o)).or(resources);
                            // lopdf 0.36 has no bounded decoder; skip suspiciously large encoded forms.
                            if stream.content.len() > 16 * 1024 * 1024 {
                                warnings.push(AnalysisWarning { page_number: Some(page), object_id: Some(format!("{} {} R", id.0,id.1)), message: "form content skipped: encoded stream exceeds 16 MiB safety limit".into() });
                            } else {
                                match stream.decompressed_content().and_then(|b| Content::decode(&b)) {
                                    Ok(form) => walk_content(doc, &form, form_resources, form_matrix.concat(ctm), page, images, counts, warnings, visiting),
                                    Err(error) => warnings.push(AnalysisWarning { page_number: Some(page), object_id: Some(format!("{} {} R", id.0,id.1)), message: format!("form content unavailable: {error}") }),
                                }
                            }
                            visiting.remove(&id);
                        }
                        _ => {}
                    }
                }
            },
            "BI" => warnings.push(AnalysisWarning { page_number: Some(page), object_id: None, message: "inline image encountered; this schema version does not catalogue inline image bytes".into() }),
            _ => {}
        }
    }
}

fn resolve_xobject<'a>(
    doc: &'a Document,
    resources: Option<&'a Dictionary>,
    key: &[u8],
) -> Option<(ObjectId, &'a Stream)> {
    let dict = resources?
        .get(b"XObject")
        .ok()
        .and_then(|o| resolve_dict(doc, o))?;
    let obj = dict.get(key).ok()?;
    let id = obj.as_reference().ok()?;
    Some((id, doc.get_object(id).ok()?.as_stream().ok()?))
}
fn resolve_dict<'a>(doc: &'a Document, object: &'a Object) -> Option<&'a Dictionary> {
    match object {
        Object::Dictionary(d) => Some(d),
        Object::Reference(id) => doc.get_dictionary(*id).ok(),
        _ => None,
    }
}
fn inherited_dict<'a>(doc: &'a Document, start: &'a Dictionary, key: &[u8]) -> Option<Dictionary> {
    inherited_object(doc, start, key)
        .and_then(|o| resolve_dict(doc, o))
        .cloned()
}
fn inherited_number(doc: &Document, start: &Dictionary, key: &[u8]) -> Option<f64> {
    inherited_object(doc, start, key).and_then(number)
}
fn inherited_object<'a>(
    doc: &'a Document,
    mut dict: &'a Dictionary,
    key: &[u8],
) -> Option<&'a Object> {
    loop {
        if let Ok(v) = dict.get(key) {
            return Some(v);
        };
        let parent = dict.get(b"Parent").ok()?.as_reference().ok()?;
        dict = doc.get_dictionary(parent).ok()?;
    }
}
fn page_size(doc: &Document, dict: &Dictionary) -> Option<(f64, f64)> {
    let a = inherited_object(doc, dict, b"MediaBox")?.as_array().ok()?;
    Some((
        (number(&a[2])? - number(&a[0])?).abs(),
        (number(&a[3])? - number(&a[1])?).abs(),
    ))
}
fn number(o: &Object) -> Option<f64> {
    match o {
        Object::Integer(v) => Some(*v as f64),
        Object::Real(v) => Some(*v as f64),
        _ => None,
    }
}
fn matrix(a: &[Object]) -> Option<Matrix> {
    Some(Matrix {
        a: number(&a[0])?,
        b: number(&a[1])?,
        c: number(&a[2])?,
        d: number(&a[3])?,
        e: number(&a[4])?,
        f: number(&a[5])?,
    })
}
fn name(object: Option<&Object>) -> Option<String> {
    object?
        .as_name()
        .ok()
        .map(|v| String::from_utf8_lossy(v).into_owned())
}
fn names(object: Option<&Object>) -> Vec<String> {
    match object {
        Some(Object::Name(v)) => vec![String::from_utf8_lossy(v).into_owned()],
        Some(Object::Array(v)) => v.iter().filter_map(|o| name(Some(o))).collect(),
        _ => vec![],
    }
}
fn image_work(s: &Stream) -> ImageWork {
    let width = s.dict.get(b"Width").ok().and_then(number).unwrap_or(0.0) as u32;
    let height = s.dict.get(b"Height").ok().and_then(number).unwrap_or(0.0) as u32;
    ImageWork {
        width,
        height,
        colour_space: name(s.dict.get(b"ColorSpace").ok()),
        bpc: s
            .dict
            .get(b"BitsPerComponent")
            .ok()
            .and_then(number)
            .map(|v| v as u16),
        filters: names(s.dict.get(b"Filter").ok()),
        bytes: s.content.len() as u64,
        hash: format!("{:x}", Sha256::digest(&s.content)),
        placements: vec![],
    }
}
fn estimated_raw(w: u32, h: u32, bpc: Option<u16>) -> Option<u64> {
    bpc.map(|b| u64::from(w) * u64::from(h) * u64::from(b) * 3 / 8)
}

fn catalogue_streams(
    doc: &Document,
) -> (
    Vec<FontObject>,
    Vec<EmbeddedFileObject>,
    Vec<MetadataObject>,
) {
    let mut fonts = vec![];
    let mut files = vec![];
    let mut metadata = vec![];
    for (&id, obj) in &doc.objects {
        let Ok(s) = obj.as_stream() else { continue };
        let ident = format!("{} {} R", id.0, id.1);
        let subtype = name(s.dict.get(b"Subtype").ok());
        match subtype.as_deref() {
            Some("FontFile") | Some("FontFile2") | Some("FontFile3") => fonts.push(FontObject {
                id: ident,
                base_font: None,
                subtype,
                encoded_bytes: s.content.len() as u64,
            }),
            Some("EmbeddedFile") => files.push(EmbeddedFileObject {
                id: ident,
                encoded_bytes: s.content.len() as u64,
            }),
            Some("XML") if name(s.dict.get(b"Type").ok()).as_deref() == Some("Metadata") => {
                metadata.push(MetadataObject {
                    id: ident,
                    encoded_bytes: s.content.len() as u64,
                })
            }
            _ => {}
        }
    }
    (fonts, files, metadata)
}
