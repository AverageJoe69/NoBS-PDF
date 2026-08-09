use crate::geometry::Matrix;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Serialize, Deserialize)]
pub struct AnalysisResult {
    pub schema_version: String,
    pub file: FileInfo,
    pub summary: SizeSummary,
    pub pages: Vec<PdfPage>,
    pub images: Vec<ImageAnalysis>,
    pub fonts: Vec<FontObject>,
    pub embedded_files: Vec<EmbeddedFileObject>,
    pub metadata: Vec<MetadataObject>,
    pub warnings: Vec<AnalysisWarning>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileInfo {
    pub path: String,
    pub size_bytes: u64,
    pub page_count: usize,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SizeSummary {
    pub image_bytes: u64,
    pub font_bytes: u64,
    pub embedded_file_bytes: u64,
    pub metadata_bytes: u64,
    pub other_bytes: u64,
    pub unknown_bytes: u64,
    pub attribution: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PdfPage {
    pub page_number: u32,
    pub width_pt: f64,
    pub height_pt: f64,
    pub width_mm: f64,
    pub height_mm: f64,
    pub rotation_degrees: i32,
    pub object_counts: ObjectCounts,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ObjectCounts {
    pub image_placements: usize,
    pub text_operations: usize,
    pub vector_operations: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImageAnalysis {
    pub id: String,
    pub object_number: u32,
    pub generation: u16,
    pub pages: Vec<u32>,
    pub placements: Vec<ImagePlacement>,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub colour_space: Option<String>,
    pub bits_per_component: Option<u16>,
    pub filters: Vec<String>,
    pub encoded_bytes: u64,
    pub estimated_raw_pixel_bytes: Option<u64>,
    pub sha256: String,
    pub same_object_reused: bool,
    pub identical_data_duplicate: bool,
    pub duplicate_of: Option<String>,
    pub reference_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImagePlacement {
    pub page_number: u32,
    pub matrix: Matrix,
    pub bounding_box_pt: [f64; 4],
    pub displayed_width_pt: f64,
    pub displayed_height_pt: f64,
    pub displayed_width_mm: f64,
    pub displayed_height_mm: f64,
    pub effective_dpi_x: Option<f64>,
    pub effective_dpi_y: Option<f64>,
    pub effective_dpi: Option<f64>,
    pub target_analysis: BTreeMap<String, TargetDpiAnalysis>,
    pub status: MeasurementStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementStatus {
    Measured,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetDpiAnalysis {
    pub target_dpi: u16,
    pub oversampling_ratio_x: Option<f64>,
    pub oversampling_ratio_y: Option<f64>,
    pub required_pixel_width: Option<u32>,
    pub required_pixel_height: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FontObject {
    pub id: String,
    pub base_font: Option<String>,
    pub subtype: Option<String>,
    pub encoded_bytes: u64,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct EmbeddedFileObject {
    pub id: String,
    pub encoded_bytes: u64,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct MetadataObject {
    pub id: String,
    pub encoded_bytes: u64,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct AnalysisWarning {
    pub page_number: Option<u32>,
    pub object_id: Option<String>,
    pub message: String,
}

/// Reserved deterministic recommendation model; no PDF mutation is implemented.
#[derive(Debug, Serialize, Deserialize)]
pub struct OptimisationCandidate {
    pub object_id: String,
    pub kind: String,
    pub rationale: String,
}
