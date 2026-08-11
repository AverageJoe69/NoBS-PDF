//! Application-facing API shared by desktop commands and tests.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use serde::{Deserialize, Serialize};

use crate::{
    exporter::{export_for_scale, export_for_target, ExportOptions, ExportReport},
    flatten_pages::{
        flatten_pages, flatten_pages_preserve_foreground_text_for_scale,
        flatten_pages_preserve_text, FlattenError, FlattenReport,
    },
    inspect,
    resolution::{detect_document_resolution, DocumentResolution, PageRasterBudget},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppErrorCode {
    InvalidFileType,
    InvalidPdf,
    FileUnreadable,
    OutputDirectoryMissing,
    OutputExists,
    Cancelled,
    OptimisationFailed,
    ValidationFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppError {
    pub code: AppErrorCode,
    pub message: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentSummary {
    pub path: String,
    pub filename: String,
    pub size_bytes: u64,
    pub page_count: usize,
    pub image_count: usize,
    pub resolution: DocumentResolution,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimisationEstimate {
    pub original_size_bytes: u64,
    pub estimated_output_size_bytes: Option<u64>,
    pub estimated_saving_bytes: Option<u64>,
    pub estimated_saving_percent: Option<f64>,
    pub candidate_images: usize,
    pub skipped_images: usize,
    pub profile: String,
    pub document_long_dimension_px: Option<u32>,
    pub bloated_images: Vec<BloatedImage>,
    pub scale_percent: Option<u8>,
    pub page_budgets: Vec<PageRasterBudget>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BloatedImage {
    pub object_id: String,
    pub file_pixels: [u32; 2],
    pub document_pixels: [u32; 2],
    pub original_bytes: u64,
    pub estimated_saving_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimisationResult {
    pub mode: String,
    pub output_path: String,
    pub original_size_bytes: u64,
    pub output_size_bytes: u64,
    pub saved_bytes: u64,
    pub saved_percent: f64,
    pub images_optimised: usize,
    pub images_skipped: usize,
    pub validation_passed: bool,
    pub page_layout_preserved: bool,
    pub text_preserved: bool,
    pub vectors_preserved: bool,
    pub aspect_ratios_preserved: bool,
    pub image_placement_preserved: bool,
    pub scale_percent: Option<u8>,
    pub page_budgets: Vec<PageRasterBudget>,
}

#[derive(Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);
impl CancellationToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst)
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

pub fn inspect_pdf(path: &Path) -> Result<DocumentSummary, AppError> {
    validate_pdf_path(path)?;
    let report = inspect(path).map_err(map_inspection_error)?;
    let resolution = detect_document_resolution(&report);
    Ok(DocumentSummary {
        path: path.display().to_string(),
        filename: path
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("document.pdf")
            .into(),
        size_bytes: report.file.size_bytes,
        page_count: report.file.page_count,
        image_count: report.images.len(),
        resolution,
    })
}

pub fn estimate_pdf(path: &Path, profile: &str) -> Result<OptimisationEstimate, AppError> {
    validate_pdf_path(path)?;
    if let Some(target_name) = profile
        .strip_prefix("flatten:")
        .or_else(|| profile.strip_prefix("flatten_text:"))
    {
        let long_dimension =
            target_dimensions(target_name)?.unwrap_or(source_long_dimension(path)?);
        let summary = inspect_pdf(path)?;
        return Ok(OptimisationEstimate {
            original_size_bytes: summary.size_bytes,
            estimated_output_size_bytes: None,
            estimated_saving_bytes: None,
            estimated_saving_percent: None,
            candidate_images: summary.page_count,
            skipped_images: 0,
            profile: profile.into(),
            document_long_dimension_px: Some(long_dimension),
            bloated_images: vec![],
            scale_percent: None,
            page_budgets: vec![],
        });
    }
    ensure_supported_profile(profile)?;
    let placeholder = default_output_path(path, profile);
    let resolved_target = resolved_target_dimensions(path, profile)?;
    let report = export_for_target(
        path,
        &placeholder,
        &ExportOptions { dry_run: true },
        Some(resolved_target),
        |_| {},
    )
    .map_err(map_export_error)?;
    let saving = report.size.estimated_saving_bytes;
    let output = report.input.size_bytes.checked_sub(saving);
    let bloated_images = report
        .image_results
        .iter()
        .filter(|image| image.status == "would_modify")
        .filter_map(|image| {
            let target = image.target_pixels?;
            let source_count =
                u64::from(image.source_pixels[0]) * u64::from(image.source_pixels[1]);
            let target_count = u64::from(target[0]) * u64::from(target[1]);
            let after = if let Some(output_bytes) = image.output_bytes {
                output_bytes
            } else if source_count == 0 {
                image.original_bytes
            } else {
                ((u128::from(image.original_bytes) * u128::from(target_count))
                    / u128::from(source_count)) as u64
            };
            Some(BloatedImage {
                object_id: image.object_id.clone(),
                file_pixels: image.source_pixels,
                document_pixels: target,
                original_bytes: image.original_bytes,
                estimated_saving_bytes: image.original_bytes.saturating_sub(after),
            })
        })
        .collect();
    Ok(OptimisationEstimate {
        original_size_bytes: report.input.size_bytes,
        estimated_output_size_bytes: output,
        estimated_saving_bytes: Some(saving),
        estimated_saving_percent: Some(saving as f64 * 100.0 / report.input.size_bytes as f64),
        candidate_images: report.images.modified,
        skipped_images: report.images.skipped,
        profile: profile.into(),
        document_long_dimension_px: (report.document_target.long_dimension_px > 0)
            .then_some(report.document_target.long_dimension_px),
        bloated_images,
        scale_percent: None,
        page_budgets: report.document_target.pages.clone(),
    })
}

pub fn estimate_pdf_scale(
    path: &Path,
    scale_percent: u8,
) -> Result<OptimisationEstimate, AppError> {
    validate_pdf_path(path)?;
    let placeholder = default_output_path(path, &format!("{scale_percent}pct"));
    let report = export_for_scale(
        path,
        &placeholder,
        &ExportOptions { dry_run: true },
        scale_percent,
        |_| {},
    )
    .map_err(map_export_error)?;
    estimate_from_report(report, scale_percent)
}

fn estimate_from_report(
    report: ExportReport,
    scale_percent: u8,
) -> Result<OptimisationEstimate, AppError> {
    let saving = report.size.estimated_saving_bytes;
    let output = report.input.size_bytes.checked_sub(saving);
    let bloated_images = report
        .image_results
        .iter()
        .filter(|image| image.status == "would_modify")
        .filter_map(|image| {
            let target = image.target_pixels?;
            let source_count =
                u64::from(image.source_pixels[0]) * u64::from(image.source_pixels[1]);
            let target_count = u64::from(target[0]) * u64::from(target[1]);
            let after = if source_count == 0 {
                image.original_bytes
            } else {
                ((u128::from(image.original_bytes) * u128::from(target_count))
                    / u128::from(source_count)) as u64
            };
            Some(BloatedImage {
                object_id: image.object_id.clone(),
                file_pixels: image.source_pixels,
                document_pixels: target,
                original_bytes: image.original_bytes,
                estimated_saving_bytes: image.original_bytes.saturating_sub(after),
            })
        })
        .collect();
    Ok(OptimisationEstimate {
        original_size_bytes: report.input.size_bytes,
        estimated_output_size_bytes: output,
        estimated_saving_bytes: Some(saving),
        estimated_saving_percent: Some(saving as f64 * 100.0 / report.input.size_bytes as f64),
        candidate_images: report.images.modified,
        skipped_images: report.images.skipped,
        profile: format!("scale_{scale_percent}"),
        document_long_dimension_px: None,
        bloated_images,
        scale_percent: Some(scale_percent),
        page_budgets: report.document_target.pages,
    })
}

pub fn optimise_pdf(
    path: &Path,
    profile: &str,
    output: &Path,
    cancellation: &CancellationToken,
) -> Result<OptimisationResult, AppError> {
    optimise_pdf_with_options(path, profile, output, cancellation, None, |_| {})
}

pub fn optimise_pdf_scale_with_options(
    path: &Path,
    scale_percent: u8,
    output: &Path,
    cancellation: &CancellationToken,
    mut progress: impl FnMut(&str),
) -> Result<OptimisationResult, AppError> {
    validate_pdf_path(path)?;
    if !(10..=100).contains(&scale_percent) {
        return Err(simple_error(
            AppErrorCode::OptimisationFailed,
            "Document size must be between 10% and 100%.",
        ));
    }
    if cancellation.is_cancelled() {
        return Err(simple_error(
            AppErrorCode::Cancelled,
            "Optimisation was cancelled.",
        ));
    }
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !parent.exists() {
        return Err(simple_error(
            AppErrorCode::OutputDirectoryMissing,
            "The selected output folder does not exist.",
        ));
    }
    if output.exists() {
        return Err(simple_error(
            AppErrorCode::OutputExists,
            "The selected output file already exists.",
        ));
    }
    let inspection = inspect(path).map_err(map_inspection_error)?;
    let resolution = detect_document_resolution(&inspection);
    progress("optimising");
    match flatten_pages_preserve_foreground_text_for_scale(path, output, false, None, scale_percent)
    {
        Ok(report) => {
            if cancellation.is_cancelled() {
                let _ = fs::remove_file(output);
                return Err(simple_error(
                    AppErrorCode::Cancelled,
                    "Optimisation was cancelled.",
                ));
            }
            let mut result = result_from_flatten_report(report)?;
            result.mode = format!("scale_{scale_percent}");
            result.scale_percent = Some(scale_percent);
            result.page_budgets = resolution.pages;
            return Ok(result);
        }
        Err(FlattenError::HybridUnavailable(_)) => {
            // Vector-only or structurally inseparable pages retain the existing
            // conservative native-resource path.
        }
        Err(error) => return Err(map_flatten_error(error)),
    }
    let report = export_for_scale(
        path,
        output,
        &ExportOptions { dry_run: false },
        scale_percent,
        &mut progress,
    )
    .map_err(map_export_error)?;
    if cancellation.is_cancelled() {
        let _ = fs::remove_file(output);
        return Err(simple_error(
            AppErrorCode::Cancelled,
            "Optimisation was cancelled.",
        ));
    }
    result_from_report(report)
}
pub fn optimise_pdf_with_progress(
    path: &Path,
    profile: &str,
    output: &Path,
    cancellation: &CancellationToken,
    progress: impl FnMut(&str),
) -> Result<OptimisationResult, AppError> {
    optimise_pdf_with_options(path, profile, output, cancellation, None, progress)
}
pub fn optimise_pdf_with_options(
    path: &Path,
    profile: &str,
    output: &Path,
    cancellation: &CancellationToken,
    pdfium_library: Option<&Path>,
    mut progress: impl FnMut(&str),
) -> Result<OptimisationResult, AppError> {
    ensure_supported_profile(profile)?;
    validate_pdf_path(path)?;
    if cancellation.is_cancelled() {
        return Err(simple_error(
            AppErrorCode::Cancelled,
            "Optimisation was cancelled.",
        ));
    }
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !parent.exists() {
        return Err(simple_error(
            AppErrorCode::OutputDirectoryMissing,
            "The selected output folder does not exist.",
        ));
    }
    if output.exists() {
        return Err(simple_error(
            AppErrorCode::OutputExists,
            "The selected output file already exists.",
        ));
    }
    if let Some(target_name) = profile
        .strip_prefix("flatten:")
        .or_else(|| profile.strip_prefix("flatten_text:"))
    {
        let long_dimension =
            target_dimensions(target_name)?.unwrap_or(source_long_dimension(path)?);
        progress("optimising");
        let report = if profile.starts_with("flatten_text:") {
            flatten_pages_preserve_text(
                path,
                output,
                false,
                pdfium_library,
                long_dimension,
                target_name,
            )
        } else {
            flatten_pages(
                path,
                output,
                false,
                pdfium_library,
                long_dimension,
                target_name,
            )
        }
        .map_err(map_flatten_error)?;
        progress("validating");
        if cancellation.is_cancelled() {
            let _ = fs::remove_file(output);
            return Err(simple_error(
                AppErrorCode::Cancelled,
                "Optimisation was cancelled.",
            ));
        }
        return result_from_flatten_report(report);
    }
    let resolved_target = resolved_target_dimensions(path, profile)?;
    let mut report = export_for_target(
        path,
        output,
        &ExportOptions { dry_run: false },
        Some(resolved_target),
        progress,
    )
    .map_err(map_export_error)?;
    if is_source_profile(profile) {
        report.document_target.profile = "source".into();
    }
    if cancellation.is_cancelled() {
        let _ = fs::remove_file(output);
        return Err(simple_error(
            AppErrorCode::Cancelled,
            "Optimisation was cancelled.",
        ));
    }
    result_from_report(report)
}

pub fn default_output_path(input: &Path, profile: &str) -> PathBuf {
    let stem = input
        .file_stem()
        .and_then(|v| v.to_str())
        .unwrap_or("document");
    input.with_file_name(format!("{stem}_NoBS_{profile}.pdf"))
}

fn result_from_report(report: ExportReport) -> Result<OptimisationResult, AppError> {
    let mode = report.document_target.profile.clone();
    let validation = report.validation.ok_or_else(|| {
        simple_error(
            AppErrorCode::ValidationFailed,
            "The output was not validated.",
        )
    })?;
    let output = report.output.ok_or_else(|| {
        simple_error(
            AppErrorCode::OptimisationFailed,
            "No output file was created.",
        )
    })?;
    Ok(OptimisationResult {
        mode,
        output_path: output.path,
        original_size_bytes: report.input.size_bytes,
        output_size_bytes: output.size_bytes,
        saved_bytes: report.size.saved_bytes.unwrap_or(0),
        saved_percent: report.size.saved_percent.unwrap_or(0.0),
        images_optimised: report.images.modified,
        images_skipped: report.images.skipped,
        validation_passed: validation.passed,
        page_layout_preserved: validation.page_geometry_preserved
            && validation.page_rotation_preserved,
        text_preserved: validation.text_content_preserved,
        vectors_preserved: validation.vector_content_preserved,
        aspect_ratios_preserved: validation.image_aspect_ratios_preserved,
        image_placement_preserved: validation.image_placement_preserved,
        scale_percent: report.document_target.scale_percent,
        page_budgets: report.document_target.pages,
    })
}
fn validate_pdf_path(path: &Path) -> Result<(), AppError> {
    if path
        .extension()
        .and_then(|v| v.to_str())
        .is_none_or(|v| !v.eq_ignore_ascii_case("pdf"))
    {
        return Err(simple_error(
            AppErrorCode::InvalidFileType,
            "Please choose a PDF file.",
        ));
    }
    let bytes = fs::read(path).map_err(|e| {
        app_error(
            AppErrorCode::FileUnreadable,
            "The file could not be read.",
            e,
        )
    })?;
    if !bytes.starts_with(b"%PDF-") {
        return Err(simple_error(
            AppErrorCode::InvalidPdf,
            "This file is not a valid PDF.",
        ));
    }
    Ok(())
}
fn ensure_supported_profile(profile: &str) -> Result<(), AppError> {
    let value = profile
        .strip_prefix("flatten:")
        .or_else(|| profile.strip_prefix("flatten_text:"))
        .unwrap_or(profile);
    target_dimensions(value).map(|_| ())
}
fn target_dimensions(profile: &str) -> Result<Option<u32>, AppError> {
    match profile.to_ascii_lowercase().as_str() {
        "source" | "original" => Ok(None),
        "720p" => Ok(Some(1280)),
        "1080p" => Ok(Some(1920)),
        "1440p" => Ok(Some(2560)),
        "4k" => Ok(Some(3840)),
        _ => Err(simple_error(
            AppErrorCode::OptimisationFailed,
            "Unknown resolution target.",
        )),
    }
}
fn is_source_profile(profile: &str) -> bool {
    matches!(profile.to_ascii_lowercase().as_str(), "source" | "original")
}

fn resolved_target_dimensions(path: &Path, profile: &str) -> Result<u32, AppError> {
    target_dimensions(profile)?.map_or_else(|| source_long_dimension(path), Ok)
}
fn source_long_dimension(path: &Path) -> Result<u32, AppError> {
    let report = inspect(path).map_err(|e| {
        app_error(
            AppErrorCode::InvalidPdf,
            "The source page resolution could not be determined.",
            e,
        )
    })?;
    report
        .pages
        .iter()
        .map(|page| page.width_pt.max(page.height_pt).round() as u32)
        .max()
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            simple_error(
                AppErrorCode::InvalidPdf,
                "The PDF has no usable page dimensions.",
            )
        })
}
fn result_from_flatten_report(report: FlattenReport) -> Result<OptimisationResult, AppError> {
    let text_preserved = report.mode == "graphics_raster_text_preserved";
    let validation = report.validation.ok_or_else(|| {
        simple_error(
            AppErrorCode::ValidationFailed,
            "The flattened output was not validated.",
        )
    })?;
    let output_path = report.output_path.ok_or_else(|| {
        simple_error(
            AppErrorCode::OptimisationFailed,
            "No flattened output was created.",
        )
    })?;
    let output_size = report.output_size_bytes.unwrap_or(0);
    let saved_bytes = report.saved_bytes.unwrap_or(0);
    Ok(OptimisationResult {
        mode: if text_preserved {
            "flatten_text"
        } else {
            "flatten"
        }
        .into(),
        output_path,
        original_size_bytes: report.original_size_bytes,
        output_size_bytes: output_size,
        saved_bytes,
        saved_percent: if report.original_size_bytes == 0 {
            0.0
        } else {
            saved_bytes as f64 * 100.0 / report.original_size_bytes as f64
        },
        images_optimised: report.pages.len(),
        images_skipped: 0,
        validation_passed: validation.passed,
        page_layout_preserved: validation.page_count_preserved
            && validation.page_boxes_preserved
            && validation.page_rotation_preserved,
        text_preserved,
        vectors_preserved: false,
        aspect_ratios_preserved: validation.page_boxes_preserved,
        image_placement_preserved: false,
        scale_percent: None,
        page_budgets: vec![],
    })
}
fn map_flatten_error(error: crate::flatten_pages::FlattenError) -> AppError {
    if matches!(
        &error,
        crate::flatten_pages::FlattenError::Inspection(
            crate::parser::InspectionError::EncryptedDocument
        )
    ) {
        return encrypted_pdf_error();
    }
    let message = error.to_string();
    let code = if message.contains("validation") {
        AppErrorCode::ValidationFailed
    } else if message.contains("already exists") {
        AppErrorCode::OutputExists
    } else {
        AppErrorCode::OptimisationFailed
    };
    AppError {
        code,
        message: "NoBS PDF could not create a validated flattened output.".into(),
        detail: cfg!(debug_assertions).then_some(message),
    }
}
fn map_export_error(error: crate::exporter::ExportError) -> AppError {
    if matches!(
        &error,
        crate::exporter::ExportError::Inspection(crate::parser::InspectionError::EncryptedDocument)
    ) {
        return encrypted_pdf_error();
    }
    let message = error.to_string();
    let code = if message.contains("validation") {
        AppErrorCode::ValidationFailed
    } else if message.contains("already exists") {
        AppErrorCode::OutputExists
    } else {
        AppErrorCode::OptimisationFailed
    };
    AppError {
        code,
        message: "NoBS PDF could not create a validated output.".into(),
        detail: cfg!(debug_assertions).then_some(message),
    }
}
fn map_inspection_error(error: crate::parser::InspectionError) -> AppError {
    if matches!(&error, crate::parser::InspectionError::EncryptedDocument) {
        encrypted_pdf_error()
    } else {
        app_error(
            AppErrorCode::InvalidPdf,
            "This PDF could not be inspected.",
            error,
        )
    }
}
fn encrypted_pdf_error() -> AppError {
    simple_error(
        AppErrorCode::InvalidPdf,
        "Encrypted PDFs are not supported. Decrypt the document before optimising it.",
    )
}
fn simple_error(code: AppErrorCode, message: &str) -> AppError {
    AppError {
        code,
        message: message.into(),
        detail: None,
    }
}
fn app_error(code: AppErrorCode, message: &str, error: impl std::fmt::Display) -> AppError {
    AppError {
        code,
        message: message.into(),
        detail: cfg!(debug_assertions).then(|| error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::dictionary;

    #[test]
    fn rejects_non_pdf_extensions_with_a_structured_error() {
        let error = inspect_pdf(Path::new("document.txt")).unwrap_err();
        assert_eq!(error.code, AppErrorCode::InvalidFileType);
    }

    #[test]
    fn rejects_files_that_do_not_have_a_pdf_header() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("document.pdf");
        fs::write(&path, b"not a pdf").unwrap();

        let error = inspect_pdf(&path).unwrap_err();
        assert_eq!(error.code, AppErrorCode::InvalidPdf);
    }

    #[test]
    fn honours_cancellation_before_exporting() {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("document.pdf");
        let output = directory.path().join("output.pdf");
        fs::write(&input, b"%PDF-1.7\n").unwrap();
        let cancellation = CancellationToken::default();
        cancellation.cancel();

        let error = optimise_pdf(&input, "1080p", &output, &cancellation).unwrap_err();
        assert_eq!(error.code, AppErrorCode::Cancelled);
        assert!(!output.exists());
    }

    #[test]
    fn rejects_a_missing_output_directory_before_exporting() {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("document.pdf");
        let output = directory.path().join("missing").join("output.pdf");
        fs::write(&input, b"%PDF-1.7\n").unwrap();

        let error =
            optimise_pdf(&input, "1080p", &output, &CancellationToken::default()).unwrap_err();
        assert_eq!(error.code, AppErrorCode::OutputDirectoryMissing);
        assert!(!output.exists());
    }

    #[test]
    fn original_mode_creates_a_validated_resolution_preserving_export() {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("document.pdf");
        let output = directory.path().join("copy.pdf");
        let mut document = lopdf::Document::with_version("1.7");
        let pages = document.new_object_id();
        let page = document.add_object(lopdf::dictionary! {
            "Type" => "Page",
            "Parent" => pages,
            "MediaBox" => vec![0.into(), 0.into(), 1920.into(), 1080.into()],
            "Resources" => lopdf::Dictionary::new(),
        });
        document.objects.insert(
            pages,
            lopdf::Object::Dictionary(lopdf::dictionary! {
                "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1
            }),
        );
        let catalog =
            document.add_object(lopdf::dictionary! {"Type" => "Catalog", "Pages" => pages});
        document.trailer.set("Root", catalog);
        document.save(&input).unwrap();

        let source_estimate = estimate_pdf(&input, "source").unwrap();
        let hd_estimate = estimate_pdf(&input, "1080p").unwrap();
        assert_eq!(source_estimate.document_long_dimension_px, Some(1920));
        assert_eq!(
            source_estimate.candidate_images,
            hd_estimate.candidate_images
        );
        assert_eq!(
            source_estimate.estimated_output_size_bytes,
            hd_estimate.estimated_output_size_bytes
        );

        let result =
            optimise_pdf(&input, "original", &output, &CancellationToken::default()).unwrap();
        assert_eq!(result.mode, "source");
        assert!(result.validation_passed);
        assert!(output.exists());
        assert_eq!(result.images_optimised, 0);
    }

    #[test]
    fn scale_api_rejects_encrypted_fixture_with_product_message_and_no_output() {
        let input = Path::new("tests/fixtures/encrypted_password_nobs-test.pdf");
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("encrypted-output.pdf");
        let error = optimise_pdf_scale_with_options(
            input,
            100,
            &output,
            &CancellationToken::default(),
            |_| {},
        )
        .unwrap_err();
        assert_eq!(
            error.message,
            "Encrypted PDFs are not supported. Decrypt the document before optimising it."
        );
        assert!(!output.exists());
    }
}
