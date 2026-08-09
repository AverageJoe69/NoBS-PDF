use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use lopdf::Document;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::Builder;
use thiserror::Error;

use crate::{
    inspect,
    planner::{create_plan, DocumentTargetProfile, PlannerConfig, ResolutionClassification},
    rewriter::{rewrite_pdf, ApprovedReplacement, RewriteError},
    validator::{validate_export, ValidationError, ValidationReport},
    InspectionError,
};

#[derive(Debug, Clone)]
pub struct ExportOptions {
    pub dry_run: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExportReport {
    pub schema_version: String,
    pub dry_run: bool,
    pub input: ExportFile,
    pub output: Option<ExportFile>,
    pub document_target: ExportTarget,
    pub images: ExportImageSummary,
    pub image_results: Vec<ExportImageResult>,
    pub size: ExportSize,
    pub validation: Option<ValidationReport>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExportFile {
    pub path: String,
    pub size_bytes: u64,
    pub sha256: String,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct ExportTarget {
    pub profile: String,
    pub long_dimension_px: u32,
}
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ExportImageSummary {
    pub analysed: usize,
    pub candidates: usize,
    pub modified: usize,
    pub unchanged: usize,
    pub skipped: usize,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct ExportImageResult {
    pub object_id: String,
    pub status: String,
    pub source_pixels: [u32; 2],
    pub target_pixels: Option<[u32; 2]>,
    pub original_bytes: u64,
    pub output_bytes: Option<u64>,
    pub skipped_reason: Option<String>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct ExportSize {
    pub original_bytes: u64,
    pub output_bytes: Option<u64>,
    pub saved_bytes: Option<u64>,
    pub saved_percent: Option<f64>,
    pub estimated_saving_bytes: u64,
}

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("inspection failed: {0}")]
    Inspection(#[from] InspectionError),
    #[error("PDF load failed: {0}")]
    Pdf(#[from] lopdf::Error),
    #[error("rewrite failed: {0}")]
    Rewrite(#[from] RewriteError),
    #[error("validation failed to run: {0}")]
    ValidationRun(#[from] ValidationError),
    #[error("export validation failed: {0}")]
    ValidationFailed(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("source and output paths must be different")]
    SourceWouldBeOverwritten,
    #[error(
        "output already exists; refusing to overwrite without a future explicit force option: {0}"
    )]
    OutputExists(String),
    #[error("output path has no usable parent directory")]
    InvalidOutputParent,
    #[error("could not persist validated output: {0}")]
    Persist(String),
}

pub fn export_1080p(
    input: &Path,
    output: &Path,
    options: &ExportOptions,
) -> Result<ExportReport, ExportError> {
    export_1080p_with_progress(input, output, options, |_| {})
}

pub fn export_1080p_with_progress(
    input: &Path,
    output: &Path,
    options: &ExportOptions,
    progress: impl FnMut(&str),
) -> Result<ExportReport, ExportError> {
    export_with_policy(input, output, options, Some(1920), progress)
}

pub fn export_original_resolution(
    input: &Path,
    output: &Path,
    options: &ExportOptions,
    progress: impl FnMut(&str),
) -> Result<ExportReport, ExportError> {
    export_for_target(input, output, options, None, progress)
}

pub fn export_for_target(
    input: &Path,
    output: &Path,
    options: &ExportOptions,
    long_dimension_px: Option<u32>,
    progress: impl FnMut(&str),
) -> Result<ExportReport, ExportError> {
    export_with_policy(input, output, options, long_dimension_px, progress)
}

fn export_with_policy(
    input: &Path,
    output: &Path,
    options: &ExportOptions,
    long_dimension_px: Option<u32>,
    mut progress: impl FnMut(&str),
) -> Result<ExportReport, ExportError> {
    reject_unsafe_output(input, output, options.dry_run)?;
    progress("analysing");
    let input_hash = sha256_file(input)?;
    let inspection = inspect(input)?;
    let config = PlannerConfig {
        document_target: Some(if long_dimension_px.is_none() {
            DocumentTargetProfile::Original
        } else {
            DocumentTargetProfile::Custom {
                long_dimension_px: long_dimension_px.unwrap_or(1920),
            }
        }),
        ..Default::default()
    };
    progress("planning");
    let plan = create_plan(&inspection, &config);
    let document = Document::load(input)?;
    let mut replacements = Vec::new();
    let mut approved_estimated_saving = 0_u64;
    let mut image_results = Vec::with_capacity(inspection.images.len());
    let mut summary = ExportImageSummary {
        analysed: inspection.images.len(),
        candidates: 0,
        ..Default::default()
    };

    for image_plan in &plan.images {
        let source = inspection
            .images
            .iter()
            .find(|image| image.id == image_plan.object_id)
            .expect("plan image originates from inspection");
        if long_dimension_px.is_some()
            && image_plan.classification != ResolutionClassification::Oversampled
        {
            summary.unchanged += 1;
            image_results.push(result(
                source,
                "unchanged",
                None,
                None,
                Some(image_plan.recommendation.clone()),
            ));
            continue;
        }
        let target = if long_dimension_px.is_none() {
            placement_target_pixels(source)
        } else {
            image_plan.target_screen_pixel_dimensions
        };
        let Some(target) = target else {
            summary.skipped += 1;
            image_results.push(result(
                source,
                "skipped",
                None,
                None,
                Some("target screen dimensions unavailable".into()),
            ));
            continue;
        };
        match approve_jpeg(&document, source, target) {
            Ok(replacement) => {
                let source_pixels = u64::from(source.pixel_width) * u64::from(source.pixel_height);
                let target_pixels = u64::from(target[0]) * u64::from(target[1]);
                let estimated_after = if source_pixels == 0 {
                    source.encoded_bytes
                } else {
                    ((u128::from(source.encoded_bytes) * u128::from(target_pixels))
                        / u128::from(source_pixels)) as u64
                };
                approved_estimated_saving = approved_estimated_saving
                    .saturating_add(source.encoded_bytes.saturating_sub(estimated_after));
                replacements.push(replacement);
                summary.candidates += 1;
                image_results.push(result(
                    source,
                    if options.dry_run {
                        "would_modify"
                    } else {
                        "approved"
                    },
                    Some(target),
                    None,
                    None,
                ));
            }
            Err(reason) => {
                summary.skipped += 1;
                image_results.push(result(source, "skipped", Some(target), None, Some(reason)));
            }
        }
    }
    if options.dry_run {
        summary.modified = replacements.len();
        return Ok(ExportReport {
            schema_version: "0.1.0".into(),
            dry_run: true,
            input: ExportFile {
                path: input.display().to_string(),
                size_bytes: inspection.file.size_bytes,
                sha256: input_hash,
            },
            output: None,
            document_target: ExportTarget {
                profile: target_profile(long_dimension_px),
                long_dimension_px: long_dimension_px.unwrap_or(0),
            },
            images: summary,
            image_results,
            size: ExportSize {
                original_bytes: inspection.file.size_bytes,
                output_bytes: None,
                saved_bytes: None,
                saved_percent: None,
                estimated_saving_bytes: approved_estimated_saving,
            },
            validation: None,
        });
    }

    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let temporary = Builder::new()
        .prefix(".pdfdoctor-")
        .suffix(".pdf")
        .tempfile_in(parent)?;
    let temporary_path = temporary.path().to_path_buf();
    progress("optimising");
    let encoded_sizes = rewrite_pdf(input, &temporary_path, &replacements)?;
    progress("rebuilding");
    let after = inspect(&temporary_path)?;
    let modified_ids = encoded_sizes.keys().cloned().collect::<HashSet<_>>();
    progress("validating");
    let validation = validate_export(input, &temporary_path, &inspection, &after, &modified_ids)?;
    if !validation.passed {
        return Err(ExportError::ValidationFailed(validation.errors.join("; ")));
    }
    if sha256_file(input)? != input_hash {
        return Err(ExportError::ValidationFailed(
            "source PDF changed during export".into(),
        ));
    }
    let temporary_size = fs::metadata(&temporary_path)?.len();
    if temporary_size == 0
        || (!encoded_sizes.is_empty() && temporary_size >= inspection.file.size_bytes)
    {
        return Err(ExportError::ValidationFailed(format!(
            "optimised output is not smaller than the source ({} bytes versus {} bytes)",
            temporary_size, inspection.file.size_bytes
        )));
    }
    temporary
        .persist_noclobber(output)
        .map_err(|error| ExportError::Persist(error.error.to_string()))?;
    let output_size = fs::metadata(output)?.len();
    let output_hash = sha256_file(output)?;
    summary.modified = encoded_sizes.len();
    for item in &mut image_results {
        if item.status == "approved" {
            if let Some(size) = encoded_sizes.get(&item.object_id).copied() {
                item.status = "modified".into();
                item.output_bytes = Some(size);
            } else {
                item.status = "unchanged".into();
                item.skipped_reason = Some("recompression did not reduce encoded size".into());
            }
        }
    }
    let saved = inspection.file.size_bytes.saturating_sub(output_size);
    summary.unchanged = summary
        .analysed
        .saturating_sub(summary.modified + summary.skipped);
    Ok(ExportReport {
        schema_version: "0.1.0".into(),
        dry_run: false,
        input: ExportFile {
            path: input.display().to_string(),
            size_bytes: inspection.file.size_bytes,
            sha256: input_hash,
        },
        output: Some(ExportFile {
            path: output.display().to_string(),
            size_bytes: output_size,
            sha256: output_hash,
        }),
        document_target: ExportTarget {
            profile: target_profile(long_dimension_px),
            long_dimension_px: long_dimension_px.unwrap_or(0),
        },
        images: summary,
        image_results,
        size: ExportSize {
            original_bytes: inspection.file.size_bytes,
            output_bytes: Some(output_size),
            saved_bytes: Some(saved),
            saved_percent: Some(saved as f64 * 100.0 / inspection.file.size_bytes as f64),
            estimated_saving_bytes: approved_estimated_saving,
        },
        validation: Some(validation),
    })
}

fn target_profile(long_dimension_px: Option<u32>) -> String {
    match long_dimension_px {
        None => "source".into(),
        Some(1280) => "720p".into(),
        Some(1920) => "1080p".into(),
        Some(2560) => "1440p".into(),
        Some(3840) => "4k".into(),
        Some(value) => format!("{value}px"),
    }
}

fn approve_jpeg(
    document: &Document,
    image: &crate::model::ImageAnalysis,
    target: [u32; 2],
) -> Result<ApprovedReplacement, String> {
    if target[0] >= image.pixel_width || target[1] >= image.pixel_height {
        return Err("target does not reduce both pixel dimensions".into());
    }
    if image.filters != ["DCTDecode"] {
        return Err("only direct DCTDecode JPEG streams are supported safely".into());
    }
    if image.bits_per_component != Some(8) {
        return Err("JPEG bits per component must be 8".into());
    }
    let colour = image
        .colour_space
        .as_deref()
        .ok_or("colour space is unknown")?;
    if !matches!(colour, "DeviceRGB" | "DeviceGray" | "DeviceCMYK") {
        return Err(format!("unsupported JPEG colour space: {colour}"));
    }
    let stream = document
        .get_object((image.object_number, image.generation))
        .map_err(|e| e.to_string())?
        .as_stream()
        .map_err(|_| "object is not an image stream")?;
    if stream.dict.has(b"SMask") || stream.dict.has(b"Mask") {
        return Err("image mask/alpha behaviour must remain untouched".into());
    }
    Ok(ApprovedReplacement {
        object_number: image.object_number,
        generation: image.generation,
        source_pixels: [image.pixel_width, image.pixel_height],
        target_pixels: target,
        colour_space: colour.into(),
    })
}

/// Matches the raster to its largest rendered footprint using one pixel per PDF point.
/// Shared images use their largest placement, and the source aspect ratio is retained.
fn placement_target_pixels(image: &crate::model::ImageAnalysis) -> Option<[u32; 2]> {
    if image.pixel_width == 0 || image.pixel_height == 0 || image.placements.is_empty() {
        return None;
    }
    let scale = image
        .placements
        .iter()
        .map(|placement| {
            (placement.displayed_width_pt / f64::from(image.pixel_width))
                .max(placement.displayed_height_pt / f64::from(image.pixel_height))
        })
        .fold(0.0_f64, f64::max);
    if !scale.is_finite() || scale <= 0.0 || scale >= 1.0 {
        return None;
    }
    let width = (f64::from(image.pixel_width) * scale).ceil().max(1.0) as u32;
    let height = (f64::from(width) * f64::from(image.pixel_height) / f64::from(image.pixel_width))
        .round()
        .max(1.0) as u32;
    Some([width, height])
}

fn result(
    image: &crate::model::ImageAnalysis,
    status: &str,
    target: Option<[u32; 2]>,
    output_bytes: Option<u64>,
    reason: Option<String>,
) -> ExportImageResult {
    ExportImageResult {
        object_id: image.id.clone(),
        status: status.into(),
        source_pixels: [image.pixel_width, image.pixel_height],
        target_pixels: target,
        original_bytes: image.encoded_bytes,
        output_bytes,
        skipped_reason: reason,
    }
}
fn reject_unsafe_output(input: &Path, output: &Path, dry_run: bool) -> Result<(), ExportError> {
    let input_abs = fs::canonicalize(input)?;
    let output_abs = if output.exists() {
        fs::canonicalize(output)?
    } else {
        absolute_path(output)?
    };
    if input_abs == output_abs {
        return Err(ExportError::SourceWouldBeOverwritten);
    }
    if !dry_run && output.exists() {
        return Err(ExportError::OutputExists(output.display().to_string()));
    }
    Ok(())
}
fn absolute_path(path: &Path) -> Result<PathBuf, std::io::Error> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}
fn sha256_file(path: &Path) -> Result<String, std::io::Error> {
    let bytes = fs::read(path)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub fn human_export_summary(report: &ExportReport) -> String {
    if report.dry_run {
        return format!("NoBS PDF — DRY RUN\n\nTarget: 1080p\nImages: {}\nWould optimise: {}\nWould leave unchanged: {}\nWould skip: {}\nEstimated saving: ~{:.1} MB\n\nNo PDF written.\n",report.images.analysed,report.images.modified,report.images.unchanged,report.images.skipped,report.size.estimated_saving_bytes as f64/1_000_000.0);
    }
    let output = report.output.as_ref().expect("non-dry report has output");
    let validation = report
        .validation
        .as_ref()
        .expect("non-dry report has validation");
    format!("NoBS PDF — EXPORT\n\nInput: {}\nTarget: 1080p\nOriginal: {:.1} MB\nImages analysed: {}\nImages optimised: {}\nImages unchanged: {}\nImages skipped: {}\nOutput: {:.1} MB\nSaved: {:.1} MB ({:.1}%)\n\nVALIDATION\n{} NoBS PDF validation {}\n",report.input.path,report.input.size_bytes as f64/1_000_000.0,report.images.analysed,report.images.modified,report.images.unchanged,report.images.skipped,output.size_bytes as f64/1_000_000.0,report.size.saved_bytes.unwrap_or(0)as f64/1_000_000.0,report.size.saved_percent.unwrap_or(0.0),if validation.passed{"✓"}else{"✗"},if validation.passed{"PASSED"}else{"FAILED"})
}
