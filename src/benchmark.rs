//! Repeatable regression benchmark around the frozen selective-flattening engine.

use std::{fs, path::Path};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::Builder;
use thiserror::Error;

use crate::{
    flatten_pages::{flatten_pages_preserve_text, FlattenError},
    inspect, InspectionError,
};

#[derive(Debug, Error)]
pub enum BenchmarkError {
    #[error("inspection failed: {0}")]
    Inspection(#[from] InspectionError),
    #[error("optimisation failed: {0}")]
    Optimisation(#[from] FlattenError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("only the frozen 1080p benchmark target is supported")]
    UnsupportedTarget,
    #[error("benchmark validation failed")]
    ValidationFailed,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BenchmarkReport {
    pub schema_version: String,
    pub input: BenchmarkInput,
    pub target: BenchmarkTarget,
    pub analysis: BenchmarkAnalysis,
    pub output: BenchmarkOutput,
    pub operations: BenchmarkOperations,
    pub validation: BenchmarkValidation,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BenchmarkInput {
    pub path: String,
    pub size_bytes: u64,
    pub page_count: usize,
    pub sha256: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BenchmarkTarget {
    pub profile: String,
    pub resolution: String,
    pub long_dimension_px: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BenchmarkAnalysis {
    pub raster_objects: usize,
    pub raster_placements: usize,
    pub vector_operations_before: usize,
    pub vector_operations_after: usize,
    pub text_operations_before: usize,
    pub text_operations_after: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BenchmarkOutput {
    pub size_bytes: u64,
    pub reduction_bytes: u64,
    pub reduction_percent: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BenchmarkOperations {
    /// Selective flattening composites placements; it cannot truthfully attribute
    /// which source images were independently downsampled.
    pub images_downsampled: Option<usize>,
    pub raster_placements_composited: usize,
    pub raster_objects_after: usize,
    pub pages_raster_composited: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BenchmarkValidation {
    pub passed: bool,
    pub output_valid: bool,
    pub output_non_zero: bool,
    pub output_smaller: bool,
    pub page_count: bool,
    pub page_geometry: bool,
    pub page_rotation: bool,
    pub page_boxes: bool,
    pub annotations: bool,
    pub aspect_ratios: bool,
    pub selectable_text_present: bool,
    pub native_vectors_present: bool,
    pub rendered_pages: bool,
    pub maximum_mean_render_error: f64,
}

pub fn run_1080p(
    input: &Path,
    pdfium_library: Option<&Path>,
) -> Result<BenchmarkReport, BenchmarkError> {
    let before = inspect(input)?;
    let temporary = Builder::new()
        .prefix("nobs-benchmark-")
        .suffix(".pdf")
        .tempfile()?;
    // flatten_pages persists without overwriting, so reserve a distinct path.
    let output = temporary.path().with_extension("output.pdf");
    drop(temporary);
    let flatten =
        flatten_pages_preserve_text(input, &output, false, pdfium_library, 1920, "1080p")?;
    let after_result = inspect(&output);
    let report = (|| {
        let after = after_result?;
        let output_size = fs::metadata(&output)?.len();
        let text_before = sum_text(&before);
        let text_after = sum_text(&after);
        let vectors_before = sum_vectors(&before);
        let vectors_after = sum_vectors(&after);
        let raster_placements_before = sum_rasters(&before);
        let structural = flatten
            .validation
            .as_ref()
            .expect("non-dry run is validated");
        let output_non_zero = output_size > 0;
        let output_smaller = output_size < before.file.size_bytes;
        let selectable_text_present = text_after > 0;
        let native_vectors_present = vectors_after > 0;
        let aspect_ratios = before.pages.iter().zip(&after.pages).all(|(a, b)| {
            let lhs = a.width_pt * b.height_pt;
            let rhs = a.height_pt * b.width_pt;
            (lhs - rhs).abs() <= 1e-7 * lhs.abs().max(rhs.abs()).max(1.0)
        });
        let passed = structural.passed
            && output_non_zero
            && output_smaller
            && selectable_text_present
            && native_vectors_present
            && aspect_ratios;
        Ok(BenchmarkReport {
            schema_version: "1.0.0".into(),
            input: BenchmarkInput {
                path: input.display().to_string(),
                size_bytes: before.file.size_bytes,
                page_count: before.file.page_count,
                sha256: format!("{:x}", Sha256::digest(fs::read(input)?)),
            },
            target: BenchmarkTarget {
                profile: "screen".into(),
                resolution: "1080p".into(),
                long_dimension_px: 1920,
            },
            analysis: BenchmarkAnalysis {
                raster_objects: before.images.len(),
                raster_placements: raster_placements_before,
                vector_operations_before: vectors_before,
                vector_operations_after: vectors_after,
                text_operations_before: text_before,
                text_operations_after: text_after,
            },
            output: BenchmarkOutput {
                size_bytes: output_size,
                reduction_bytes: before.file.size_bytes - output_size,
                reduction_percent: (before.file.size_bytes - output_size) as f64 * 100.0
                    / before.file.size_bytes as f64,
            },
            operations: BenchmarkOperations {
                images_downsampled: None,
                raster_placements_composited: raster_placements_before,
                raster_objects_after: after.images.len(),
                pages_raster_composited: flatten.pages.len(),
            },
            validation: BenchmarkValidation {
                passed,
                output_valid: true,
                output_non_zero,
                output_smaller,
                page_count: structural.page_count_preserved,
                page_geometry: structural.page_boxes_preserved,
                page_rotation: structural.page_rotation_preserved,
                page_boxes: structural.page_boxes_preserved,
                annotations: structural.annotations_preserved,
                aspect_ratios,
                selectable_text_present,
                native_vectors_present,
                rendered_pages: structural.render_comparison_passed,
                maximum_mean_render_error: structural.maximum_mean_render_error,
            },
        })
    })();
    let _ = fs::remove_file(&output);
    report
}

fn sum_text(report: &crate::model::AnalysisResult) -> usize {
    report
        .pages
        .iter()
        .map(|p| p.object_counts.text_operations)
        .sum()
}
fn sum_vectors(report: &crate::model::AnalysisResult) -> usize {
    report
        .pages
        .iter()
        .map(|p| p.object_counts.vector_operations)
        .sum()
}
fn sum_rasters(report: &crate::model::AnalysisResult) -> usize {
    report
        .pages
        .iter()
        .map(|p| p.object_counts.image_placements)
        .sum()
}

pub fn human_summary(report: &BenchmarkReport) -> String {
    format!(
        "NoBS PDF — OPTIMISATION BENCHMARK\n\nInput: {:.1} MB\nPages: {}\nRaster objects: {}\nTarget: {}\nOutput: {:.1} MB\nReduction: {:.1}%\nRaster placements composited: {}\nPages raster-composited: {}\nSelectable text present: {}\nNative vectors present: {}\nAspect ratios: {}\nPage geometry: {}\nValidation: {}\n",
        report.input.size_bytes as f64 / 1e6,
        report.input.page_count,
        report.analysis.raster_objects,
        report.target.resolution,
        report.output.size_bytes as f64 / 1e6,
        report.output.reduction_percent,
        report.operations.raster_placements_composited,
        report.operations.pages_raster_composited,
        yes(report.validation.selectable_text_present),
        yes(report.validation.native_vectors_present),
        pass(report.validation.aspect_ratios),
        pass(report.validation.page_geometry),
        pass(report.validation.passed),
    )
}

fn yes(value: bool) -> &'static str {
    if value {
        "YES"
    } else {
        "NO"
    }
}
fn pass(value: bool) -> &'static str {
    if value {
        "PASS"
    } else {
        "FAIL"
    }
}
