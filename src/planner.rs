//! Conservative, deterministic optimisation planning. No PDF bytes are changed here.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{
    model::{AnalysisResult, ImageAnalysis, ImagePlacement, PdfPage},
    optimisation::{
        validate_raster_invariants, ExplicitPolicyPermissions, RasterFormat, RasterState,
        GEOMETRY_TOLERANCE,
    },
};

#[derive(Debug, Clone)]
pub struct PlannerConfig {
    pub target_dpi: u16,
    /// A downsample is proposed only when both axes exceed target by this ratio.
    pub oversampling_threshold: f64,
    pub document_target: Option<DocumentTargetProfile>,
}

impl Default for PlannerConfig {
    fn default() -> Self {
        Self {
            target_dpi: 300,
            oversampling_threshold: 1.25,
            document_target: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "profile", rename_all = "snake_case")]
pub enum DocumentTargetProfile {
    Original,
    Screen4k,
    Screen1440p,
    Screen1080p,
    Screen720p,
    Custom { long_dimension_px: u32 },
}

impl DocumentTargetProfile {
    pub fn long_dimension_px(&self) -> Option<u32> {
        match self {
            Self::Original => None,
            Self::Screen4k => Some(3840),
            Self::Screen1440p => Some(2560),
            Self::Screen1080p => Some(1920),
            Self::Screen720p => Some(1280),
            Self::Custom { long_dimension_px } => Some(*long_dimension_px),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Original => "original",
            Self::Screen4k => "4k",
            Self::Screen1440p => "1440p",
            Self::Screen1080p => "1080p",
            Self::Screen720p => "720p",
            Self::Custom { .. } => "custom",
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OptimisationPlan {
    pub schema_version: String,
    pub source: PlanSource,
    pub config: PlanConfigReport,
    pub document_target: Option<DocumentTargetReport>,
    pub summary: PlanSummary,
    pub candidates: Vec<OptimisationCandidate>,
    pub duplicates: Vec<DuplicateCandidate>,
    pub images: Vec<ImagePlan>,
    pub safety_invariants: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PlanSource {
    pub path: String,
    pub size_bytes: u64,
    pub page_count: usize,
    pub image_count: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PlanConfigReport {
    pub target_dpi: Option<u16>,
    pub oversampling_threshold: f64,
    pub optimisation_model: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DocumentTargetReport {
    pub profile: String,
    pub long_dimension_px: Option<u32>,
    pub aspect_ratio_locked: bool,
    pub page_render_dimensions: Vec<PageRenderDimensions>,
    pub geometry_policy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageRenderDimensions {
    pub page_number: u32,
    pub width_px: Option<u32>,
    pub height_px: Option<u32>,
    pub aspect_ratio: f64,
    pub width_pt: f64,
    pub height_pt: f64,
    pub rotation_degrees: i32,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct PlanSummary {
    pub oversampled_images: usize,
    pub optimal_images: usize,
    pub undersampled_images: usize,
    pub unknown_geometry_images: usize,
    pub duplicate_assets: usize,
    pub do_not_modify_images: usize,
    pub current_image_bytes: u64,
    pub estimated_resolution_saving_bytes: u64,
    pub potential_duplicate_saving_bytes: u64,
    pub estimated_encoding_saving_bytes_range: [u64; 2],
    pub estimated_final_size_bytes_range: [u64; 2],
    pub confidence: Confidence,
    pub savings_note: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionClassification {
    Oversampled,
    Optimal,
    Undersampled,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    High,
    Medium,
    Low,
    None,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    High,
    Medium,
    #[default]
    Low,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncodingKind {
    Jpeg,
    Jpeg2000,
    FlateLossless,
    Ccitt,
    Jbig2,
    Raw,
    Other,
    Unknown,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImagePlan {
    pub object_id: String,
    pub pages: Vec<u32>,
    pub current_size_bytes: u64,
    pub current_pixels: [u32; 2],
    pub classification: ResolutionClassification,
    pub target_analysis: Vec<TargetResolutionAnalysis>,
    pub target_screen_pixel_dimensions: Option<[u32; 2]>,
    pub target_screen_effective_dpi_x: Option<f64>,
    pub target_screen_effective_dpi_y: Option<f64>,
    pub resolution_optimisation: ResolutionOptimisation,
    pub encoding_optimisation: EncodingOptimisation,
    pub recommendation: String,
    pub reason: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TargetResolutionAnalysis {
    pub target_dpi: u16,
    pub aspect_ratio_preserving_pixels: Option<[u32; 2]>,
    pub oversampling_ratio_x: Option<f64>,
    pub oversampling_ratio_y: Option<f64>,
    pub undersampling_ratio_x: Option<f64>,
    pub undersampling_ratio_y: Option<f64>,
    pub would_require_upsampling: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResolutionOptimisation {
    pub target_dpi: u16,
    pub limiting_effective_dpi_x: Option<f64>,
    pub limiting_effective_dpi_y: Option<f64>,
    pub oversampling_ratio_x: Option<f64>,
    pub oversampling_ratio_y: Option<f64>,
    pub undersampling_ratio_x: Option<f64>,
    pub undersampling_ratio_y: Option<f64>,
    pub target_pixels: Option<[u32; 2]>,
    pub current_pixel_count: u64,
    pub target_pixel_count: Option<u64>,
    pub estimated_bytes_after: Option<u64>,
    pub estimated_saving_bytes: Option<u64>,
    pub confidence: Confidence,
    pub estimate_note: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EncodingOptimisation {
    pub current_encoding: EncodingKind,
    pub recommendation: String,
    pub preserves_format: bool,
    pub estimated_bytes_after_range: Option<[u64; 2]>,
    pub estimated_saving_bytes_range: Option<[u64; 2]>,
    pub confidence: Confidence,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OptimisationCandidate {
    pub object_id: String,
    pub pages: Vec<u32>,
    pub current_size_bytes: u64,
    pub category: String,
    pub severity: Severity,
    pub recommendation: String,
    pub current_pixels: [u32; 2],
    pub target_pixels: Option<[u32; 2]>,
    pub current_effective_dpi_x: Option<f64>,
    pub current_effective_dpi_y: Option<f64>,
    pub target_effective_dpi_x: Option<f64>,
    pub target_effective_dpi_y: Option<f64>,
    pub target_dpi: u16,
    pub estimated_saving_bytes: Option<u64>,
    pub confidence: Confidence,
    pub reason: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DuplicateCandidate {
    pub duplicate_object_id: String,
    pub original_object_id: String,
    pub pages: Vec<u32>,
    pub reference_count: usize,
    pub bytes_currently_consumed: u64,
    pub potential_saving_bytes: u64,
    pub confidence: Confidence,
    pub recommendation: String,
}

pub fn create_plan(inspection: &AnalysisResult, config: &PlannerConfig) -> OptimisationPlan {
    let mut summary = PlanSummary {
        current_image_bytes: inspection.summary.image_bytes,
        confidence: Confidence::Medium,
        savings_note: "Pixel-count reduction is mathematical; byte savings are conservative estimates based on current bytes-per-pixel. Encoding savings are speculative ranges and are kept separate.".into(),
        ..PlanSummary::default()
    };
    let mut images = Vec::with_capacity(inspection.images.len());
    let mut candidates = Vec::new();
    let by_id = inspection
        .images
        .iter()
        .map(|image| (image.id.as_str(), image))
        .collect::<HashMap<_, _>>();
    let mut duplicates = Vec::new();
    let page_dimensions = inspection
        .pages
        .iter()
        .filter_map(|page| {
            page_render_dimensions(page, config.document_target.as_ref())
                .map(|dimensions| (page.page_number, dimensions))
        })
        .collect::<HashMap<_, _>>();

    for image in &inspection.images {
        let image_plan = plan_image(image, config, &page_dimensions);
        match image_plan.classification {
            ResolutionClassification::Oversampled => summary.oversampled_images += 1,
            ResolutionClassification::Optimal => summary.optimal_images += 1,
            ResolutionClassification::Undersampled => summary.undersampled_images += 1,
            ResolutionClassification::Unknown => summary.unknown_geometry_images += 1,
        }
        if image_plan.classification != ResolutionClassification::Oversampled {
            summary.do_not_modify_images += 1;
        }
        if let Some(saving) = image_plan.resolution_optimisation.estimated_saving_bytes {
            summary.estimated_resolution_saving_bytes = summary
                .estimated_resolution_saving_bytes
                .saturating_add(saving);
        }
        if image_plan.classification == ResolutionClassification::Oversampled {
            candidates.push(candidate_from_plan(&image_plan));
        }
        images.push(image_plan);

        if let Some(original) = image.duplicate_of.as_deref().and_then(|id| by_id.get(id)) {
            duplicates.push(DuplicateCandidate {
                duplicate_object_id: image.id.clone(),
                original_object_id: original.id.clone(),
                pages: image.pages.clone(),
                reference_count: image.reference_count,
                bytes_currently_consumed: image.encoded_bytes,
                potential_saving_bytes: image.encoded_bytes,
                confidence: Confidence::Medium,
                recommendation: "Evaluate safe indirect-object deduplication; do not assume equivalent usage semantics.".into(),
            });
        }
    }
    summary.duplicate_assets = duplicates.len();
    summary.potential_duplicate_saving_bytes = duplicates
        .iter()
        .map(|duplicate| duplicate.potential_saving_bytes)
        .sum();
    // No automatic encoding recommendation yet: source quality cannot be inferred safely.
    summary.estimated_encoding_saving_bytes_range = [0, 0];
    // Duplicate and resolution candidates can refer to the same bytes. Do not add them together.
    let final_size = inspection
        .file
        .size_bytes
        .saturating_sub(summary.estimated_resolution_saving_bytes);
    summary.estimated_final_size_bytes_range = [final_size, final_size];

    let document_target = config.document_target.clone().map(|profile| {
        let page_render_dimensions = inspection
            .pages
            .iter()
            .map(|page| page_render_report(page, &profile))
            .collect();
        DocumentTargetReport {
            long_dimension_px: profile.long_dimension_px(),
            profile: profile.label().into(),
            aspect_ratio_locked: true,
            page_render_dimensions,
            geometry_policy: "Planning only: MediaBox, CropBox, TrimBox, BleedBox, ArtBox, rotation, and physical page dimensions remain unchanged.".into(),
        }
    });
    OptimisationPlan {
        schema_version: "0.1.0".into(),
        source: PlanSource {
            path: inspection.file.path.clone(),
            size_bytes: inspection.file.size_bytes,
            page_count: inspection.file.page_count,
            image_count: inspection.images.len(),
        },
        config: PlanConfigReport {
            target_dpi: config
                .document_target
                .is_none()
                .then_some(config.target_dpi),
            oversampling_threshold: config.oversampling_threshold,
            optimisation_model: if config.document_target.is_some() {
                "screen_render_occupancy".into()
            } else {
                "print_effective_dpi".into()
            },
        },
        document_target,
        summary,
        candidates,
        duplicates,
        images,
        safety_invariants: vec![
            "Never change aspect ratio beyond integer-pixel rounding tolerance.".into(),
            "Never change placement matrix, rendered bounds, rotation, or crop.".into(),
            "Never upsample an undersampled image.".into(),
            "Never convert format or colour representation without explicit policy and validation."
                .into(),
            "If geometry is unknown, do not modify the image.".into(),
        ],
    }
}

fn plan_image(
    image: &ImageAnalysis,
    config: &PlannerConfig,
    page_dimensions: &HashMap<u32, (f64, f64)>,
) -> ImagePlan {
    let limiting = limiting_dpi(&image.placements);
    let screen_target = config.document_target.as_ref().and_then(|profile| {
        profile
            .long_dimension_px()
            .and_then(|_| screen_target_pixels(image, page_dimensions))
    });
    let classification = if matches!(
        config.document_target,
        Some(DocumentTargetProfile::Original)
    ) {
        ResolutionClassification::Optimal
    } else if config.document_target.is_some() {
        classify_screen(image, screen_target, config.oversampling_threshold)
    } else {
        classify(limiting, config)
    };
    let target = if classification == ResolutionClassification::Oversampled {
        let proposed = if config.document_target.is_some() {
            screen_target
        } else {
            conservative_target_pixels(image, config.target_dpi)
        };
        proposed.filter(|&dimensions| target_passes_invariants(image, dimensions))
    } else {
        None
    };
    let current_pixels = u64::from(image.pixel_width) * u64::from(image.pixel_height);
    let target_pixel_count = target.map(|d| u64::from(d[0]) * u64::from(d[1]));
    let estimated_after = target_pixel_count.map(|pixels| {
        if current_pixels == 0 {
            image.encoded_bytes
        } else {
            ((image.encoded_bytes as u128 * pixels as u128) / current_pixels as u128) as u64
        }
    });
    let estimated_saving = estimated_after.map(|after| image.encoded_bytes.saturating_sub(after));
    let encoding = encoding_kind(&image.filters);
    let (recommendation, reason) = if config.document_target.is_some() {
        screen_recommendation(classification, target, screen_target, image, config)
    } else {
        recommendation(classification, target, limiting, image, config)
    };
    let target_analysis = [96, 150, 200, 300, 600]
        .into_iter()
        .map(|dpi| {
            let dimensions = aspect_preserving_target_pixels(image, dpi, false);
            TargetResolutionAnalysis {
                target_dpi: dpi,
                aspect_ratio_preserving_pixels: dimensions,
                oversampling_ratio_x: limiting.map(|v| v.0 / f64::from(dpi)),
                oversampling_ratio_y: limiting.map(|v| v.1 / f64::from(dpi)),
                undersampling_ratio_x: limiting.map(|v| f64::from(dpi) / v.0),
                undersampling_ratio_y: limiting.map(|v| f64::from(dpi) / v.1),
                would_require_upsampling: dimensions
                    .is_some_and(|d| d[0] > image.pixel_width || d[1] > image.pixel_height),
            }
        })
        .collect();
    ImagePlan {
        object_id: image.id.clone(),
        pages: image.pages.clone(),
        current_size_bytes: image.encoded_bytes,
        current_pixels: [image.pixel_width, image.pixel_height],
        classification,
        target_analysis,
        target_screen_pixel_dimensions: screen_target,
        target_screen_effective_dpi_x: screen_target.and_then(|d| screen_effective_dpi(image, d).map(|v| v.0)),
        target_screen_effective_dpi_y: screen_target.and_then(|d| screen_effective_dpi(image, d).map(|v| v.1)),
        resolution_optimisation: ResolutionOptimisation {
            target_dpi: config.target_dpi,
            limiting_effective_dpi_x: limiting.map(|v| v.0),
            limiting_effective_dpi_y: limiting.map(|v| v.1),
            oversampling_ratio_x: limiting.map(|v| v.0 / f64::from(config.target_dpi)),
            oversampling_ratio_y: limiting.map(|v| v.1 / f64::from(config.target_dpi)),
            undersampling_ratio_x: limiting.map(|v| f64::from(config.target_dpi) / v.0),
            undersampling_ratio_y: limiting.map(|v| f64::from(config.target_dpi) / v.1),
            target_pixels: target,
            current_pixel_count: current_pixels,
            target_pixel_count,
            estimated_bytes_after: estimated_after,
            estimated_saving_bytes: estimated_saving,
            confidence: if target.is_some() { Confidence::Medium } else { Confidence::High },
            estimate_note: "Pixel dimensions are deterministic. Encoded bytes assume current bytes-per-pixel remains constant and are not a promised output size.".into(),
        },
        encoding_optimisation: EncodingOptimisation {
            current_encoding: encoding,
            recommendation: encoding_recommendation(encoding).into(),
            preserves_format: true,
            estimated_bytes_after_range: None,
            estimated_saving_bytes_range: None,
            confidence: Confidence::Low,
        },
        recommendation,
        reason,
    }
}

fn limiting_dpi(placements: &[ImagePlacement]) -> Option<(f64, f64)> {
    if placements.is_empty() {
        return None;
    }
    placements
        .iter()
        .try_fold((f64::INFINITY, f64::INFINITY), |(x, y), placement| {
            Some((
                x.min(placement.effective_dpi_x?),
                y.min(placement.effective_dpi_y?),
            ))
        })
}

fn classify(dpi: Option<(f64, f64)>, config: &PlannerConfig) -> ResolutionClassification {
    let Some((x, y)) = dpi else {
        return ResolutionClassification::Unknown;
    };
    let target = f64::from(config.target_dpi);
    if x < target || y < target {
        ResolutionClassification::Undersampled
    } else if x >= target * config.oversampling_threshold
        && y >= target * config.oversampling_threshold
    {
        ResolutionClassification::Oversampled
    } else {
        ResolutionClassification::Optimal
    }
}

fn page_render_dimensions(
    page: &PdfPage,
    profile: Option<&DocumentTargetProfile>,
) -> Option<(f64, f64)> {
    let long_px = f64::from(profile?.long_dimension_px()?);
    let long_pt = page.width_pt.max(page.height_pt);
    (long_pt.is_finite() && long_pt > 0.0).then_some((long_px / long_pt, long_px / long_pt))
}

fn page_render_report(page: &PdfPage, profile: &DocumentTargetProfile) -> PageRenderDimensions {
    let rotated = page.rotation_degrees.rem_euclid(180) == 90;
    let (width_pt, height_pt) = if rotated {
        (page.height_pt, page.width_pt)
    } else {
        (page.width_pt, page.height_pt)
    };
    let dimensions = profile
        .long_dimension_px()
        .and_then(|long_px| aspect_preserving_page_pixels(width_pt, height_pt, long_px));
    PageRenderDimensions {
        page_number: page.page_number,
        width_px: dimensions.map(|d| d[0]),
        height_px: dimensions.map(|d| d[1]),
        aspect_ratio: if height_pt > 0.0 {
            width_pt / height_pt
        } else {
            0.0
        },
        width_pt: page.width_pt,
        height_pt: page.height_pt,
        rotation_degrees: page.rotation_degrees,
    }
}

pub fn aspect_preserving_page_pixels(
    width_pt: f64,
    height_pt: f64,
    long_px: u32,
) -> Option<[u32; 2]> {
    if width_pt <= 0.0 || height_pt <= 0.0 || long_px == 0 {
        return None;
    }
    if width_pt >= height_pt {
        Some([
            long_px,
            (f64::from(long_px) * height_pt / width_pt).round().max(1.0) as u32,
        ])
    } else {
        Some([
            (f64::from(long_px) * width_pt / height_pt).round().max(1.0) as u32,
            long_px,
        ])
    }
}

fn screen_target_pixels(
    image: &ImageAnalysis,
    page_dimensions: &HashMap<u32, (f64, f64)>,
) -> Option<[u32; 2]> {
    if image.pixel_width == 0 || image.pixel_height == 0 || image.placements.is_empty() {
        return None;
    }
    let required_scale = image
        .placements
        .iter()
        .try_fold(0.0_f64, |scale, placement| {
            let page_scale = page_dimensions.get(&placement.page_number)?;
            let required_width = placement.displayed_width_pt * page_scale.0;
            let required_height = placement.displayed_height_pt * page_scale.1;
            Some(
                scale
                    .max(required_width / f64::from(image.pixel_width))
                    .max(required_height / f64::from(image.pixel_height)),
            )
        })?;
    aspect_dimensions(image.pixel_width, image.pixel_height, required_scale)
}

fn aspect_dimensions(width: u32, height: u32, scale: f64) -> Option<[u32; 2]> {
    if width == 0 || height == 0 || !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    if width >= height {
        let target_width = (f64::from(width) * scale).ceil().max(1.0) as u32;
        let target_height = (f64::from(target_width) * f64::from(height) / f64::from(width))
            .round()
            .max(1.0) as u32;
        Some([target_width, target_height])
    } else {
        let target_height = (f64::from(height) * scale).ceil().max(1.0) as u32;
        let target_width = (f64::from(target_height) * f64::from(width) / f64::from(height))
            .round()
            .max(1.0) as u32;
        Some([target_width, target_height])
    }
}

fn classify_screen(
    image: &ImageAnalysis,
    required: Option<[u32; 2]>,
    threshold: f64,
) -> ResolutionClassification {
    let Some(required) = required else {
        return ResolutionClassification::Unknown;
    };
    if image.pixel_width < required[0] || image.pixel_height < required[1] {
        ResolutionClassification::Undersampled
    } else if f64::from(image.pixel_width) / f64::from(required[0]) >= threshold
        && f64::from(image.pixel_height) / f64::from(required[1]) >= threshold
    {
        ResolutionClassification::Oversampled
    } else {
        ResolutionClassification::Optimal
    }
}

fn screen_effective_dpi(image: &ImageAnalysis, target: [u32; 2]) -> Option<(f64, f64)> {
    let max_width = image
        .placements
        .iter()
        .map(|p| p.displayed_width_pt)
        .reduce(f64::max)?;
    let max_height = image
        .placements
        .iter()
        .map(|p| p.displayed_height_pt)
        .reduce(f64::max)?;
    Some((
        f64::from(target[0]) * 72.0 / max_width,
        f64::from(target[1]) * 72.0 / max_height,
    ))
}

fn screen_recommendation(
    classification: ResolutionClassification,
    target: Option<[u32; 2]>,
    required: Option<[u32; 2]>,
    image: &ImageAnalysis,
    config: &PlannerConfig,
) -> (String, String) {
    if matches!(
        config.document_target,
        Some(DocumentTargetProfile::Original)
    ) {
        return (
            "DO NOT MODIFY — Original profile selected".into(),
            "The Original document target preserves all raster pixel dimensions.".into(),
        );
    }
    match (classification, target, required) {
        (ResolutionClassification::Oversampled, Some(target), _) => (
            "Downsample for screen occupancy while preserving all PDF and raster geometry.".into(),
            format!("Image {}×{} is substantially oversampled for the selected document target and requires approximately {}×{} pixels.", image.pixel_width, image.pixel_height, target[0], target[1]),
        ),
        (ResolutionClassification::Undersampled, _, Some(required)) => (
            "UNDERSAMPLED — DO NOT MODIFY".into(),
            format!("The selected screen target requires approximately {}×{} pixels; upsampling the {}×{} source is forbidden.", required[0], required[1], image.pixel_width, image.pixel_height),
        ),
        (ResolutionClassification::Optimal, _, Some(required)) => (
            "DO NOT MODIFY — resolution is appropriate for the screen target".into(),
            format!("The source is close to the required {}×{} screen occupancy.", required[0], required[1]),
        ),
        _ => ("DO NOT MODIFY — physical geometry is unknown".into(), "Screen pixel occupancy could not be established for every placement.".into()),
    }
}

fn conservative_target_pixels(image: &ImageAnalysis, target_dpi: u16) -> Option<[u32; 2]> {
    aspect_preserving_target_pixels(image, target_dpi, true)
}

fn aspect_preserving_target_pixels(
    image: &ImageAnalysis,
    target_dpi: u16,
    cap_to_source: bool,
) -> Option<[u32; 2]> {
    if image.pixel_width == 0 || image.pixel_height == 0 || image.placements.is_empty() {
        return None;
    }
    let required_scale = image
        .placements
        .iter()
        .try_fold(0.0_f64, |scale, placement| {
            let required_x = placement.target_analysis.get(&target_dpi.to_string())?;
            Some(
                scale
                    .max(f64::from(required_x.required_pixel_width?) / f64::from(image.pixel_width))
                    .max(
                        f64::from(required_x.required_pixel_height?)
                            / f64::from(image.pixel_height),
                    ),
            )
        })?;
    let scale = if cap_to_source {
        required_scale.min(1.0)
    } else {
        required_scale
    };
    // Derive the secondary dimension from the source ratio, never independently from page axes.
    if image.pixel_width >= image.pixel_height {
        let width = (f64::from(image.pixel_width) * scale).ceil().max(1.0) as u32;
        let height = (f64::from(width) * f64::from(image.pixel_height)
            / f64::from(image.pixel_width))
        .round()
        .max(1.0) as u32;
        Some([width, height])
    } else {
        let height = (f64::from(image.pixel_height) * scale).ceil().max(1.0) as u32;
        let width = (f64::from(height) * f64::from(image.pixel_width)
            / f64::from(image.pixel_height))
        .round()
        .max(1.0) as u32;
        Some([width, height])
    }
}

fn target_passes_invariants(image: &ImageAnalysis, target: [u32; 2]) -> bool {
    image.placements.iter().all(|placement| {
        let before = raster_state(image, placement, [image.pixel_width, image.pixel_height]);
        let after = raster_state(image, placement, target);
        validate_raster_invariants(
            &before,
            &after,
            &ExplicitPolicyPermissions::default(),
            GEOMETRY_TOLERANCE,
        )
        .valid
    })
}

fn raster_state(
    image: &ImageAnalysis,
    placement: &ImagePlacement,
    pixels: [u32; 2],
) -> RasterState {
    RasterState {
        pixel_width: pixels[0],
        pixel_height: pixels[1],
        placement_matrix: placement.matrix,
        rendered_bounding_box_pt: placement.bounding_box_pt,
        format: raster_format(encoding_kind(&image.filters)),
        colour_space: image
            .colour_space
            .clone()
            .unwrap_or_else(|| "unknown".into()),
        bits_per_component: image.bits_per_component.unwrap_or(0),
    }
}

fn encoding_kind(filters: &[String]) -> EncodingKind {
    if filters.iter().any(|f| f == "DCTDecode") {
        EncodingKind::Jpeg
    } else if filters.iter().any(|f| f == "JPXDecode") {
        EncodingKind::Jpeg2000
    } else if filters.iter().any(|f| f == "CCITTFaxDecode") {
        EncodingKind::Ccitt
    } else if filters.iter().any(|f| f == "JBIG2Decode") {
        EncodingKind::Jbig2
    } else if filters
        .iter()
        .any(|f| f == "FlateDecode" || f == "LZWDecode")
    {
        EncodingKind::FlateLossless
    } else if filters.is_empty() {
        EncodingKind::Raw
    } else {
        EncodingKind::Other
    }
}

fn raster_format(encoding: EncodingKind) -> RasterFormat {
    match encoding {
        EncodingKind::Jpeg => RasterFormat::Jpeg,
        EncodingKind::Jpeg2000 => RasterFormat::Jpeg2000,
        EncodingKind::Ccitt => RasterFormat::Ccitt,
        other => RasterFormat::Other(format!("{other:?}")),
    }
}

fn encoding_recommendation(encoding: EncodingKind) -> &'static str {
    match encoding {
        EncodingKind::Jpeg => {
            "Retain JPEG; optionally evaluate JPEG-to-JPEG recompression with visual comparison."
        }
        EncodingKind::Jpeg2000 => "Retain JPEG2000; no automatic encoding change.",
        EncodingKind::FlateLossless => "Retain lossless encoding; no automatic format conversion.",
        EncodingKind::Ccitt => "Retain CCITT monochrome encoding.",
        EncodingKind::Jbig2 => "Retain JBIG2; no automatic encoding change.",
        EncodingKind::Raw => {
            "Raw image detected; investigate lossless same-semantic encoding manually."
        }
        EncodingKind::Other | EncodingKind::Unknown => "Unknown/other encoding; do not modify.",
    }
}

fn recommendation(
    classification: ResolutionClassification,
    target: Option<[u32; 2]>,
    dpi: Option<(f64, f64)>,
    image: &ImageAnalysis,
    config: &PlannerConfig,
) -> (String, String) {
    match (classification, target, dpi) {
        (ResolutionClassification::Oversampled, Some(target), Some((x, y))) => (
            "Downsample while preserving placement matrix, bounds, rotation, crop, format and aspect ratio.".into(),
            format!("Image {}×{} is used at limiting effective DPI {:.1}×{:.1}; a {}×{} target satisfies {} DPI for every measured placement.", image.pixel_width, image.pixel_height, x, y, target[0], target[1], config.target_dpi),
        ),
        (ResolutionClassification::Undersampled, _, Some((x, y))) => ("DO NOT MODIFY — already undersampled".into(), format!("At least one axis ({x:.1}×{y:.1} DPI) is below the {} DPI target; upsampling is forbidden.", config.target_dpi)),
        (ResolutionClassification::Optimal, _, Some((x, y))) => ("DO NOT MODIFY — resolution is appropriate".into(), format!("Limiting resolution is {x:.1}×{y:.1} DPI, within the conservative target band.")),
        _ => ("DO NOT MODIFY — physical geometry is unknown".into(), "A reliable displayed size and per-axis DPI could not be established for every placement.".into()),
    }
}

fn candidate_from_plan(plan: &ImagePlan) -> OptimisationCandidate {
    let resolution = &plan.resolution_optimisation;
    let severity = match (
        resolution.oversampling_ratio_x,
        resolution.oversampling_ratio_y,
    ) {
        (Some(x), Some(y)) if x.min(y) >= 2.0 => Severity::High,
        (Some(_), Some(_)) => Severity::Medium,
        _ => Severity::Low,
    };
    OptimisationCandidate {
        object_id: plan.object_id.clone(),
        pages: plan.pages.clone(),
        current_size_bytes: plan.current_size_bytes,
        category: "resolution".into(),
        severity,
        recommendation: "downsample".into(),
        current_pixels: plan.current_pixels,
        target_pixels: resolution.target_pixels,
        current_effective_dpi_x: resolution.limiting_effective_dpi_x,
        current_effective_dpi_y: resolution.limiting_effective_dpi_y,
        target_effective_dpi_x: plan.target_screen_effective_dpi_x,
        target_effective_dpi_y: plan.target_screen_effective_dpi_y,
        target_dpi: resolution.target_dpi,
        estimated_saving_bytes: resolution.estimated_saving_bytes,
        confidence: resolution.confidence,
        reason: plan.reason.clone(),
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use crate::{analysis::analyse_placement, geometry::Matrix, model::*};

    fn matrix(width_pt: f64, height_pt: f64, rotated: bool) -> Matrix {
        if rotated {
            Matrix {
                a: 0.0,
                b: width_pt,
                c: -height_pt,
                d: 0.0,
                e: 100.0,
                f: 200.0,
            }
        } else {
            Matrix {
                a: width_pt,
                b: 0.0,
                c: 0.0,
                d: height_pt,
                e: 0.0,
                f: 0.0,
            }
        }
    }

    fn image(id: &str, pixels: [u32; 2], display_pt: [f64; 2], filters: &[&str]) -> ImageAnalysis {
        ImageAnalysis {
            id: id.into(),
            object_number: 1,
            generation: 0,
            pages: vec![1],
            placements: vec![analyse_placement(
                1,
                matrix(display_pt[0], display_pt[1], false),
                pixels[0],
                pixels[1],
            )],
            pixel_width: pixels[0],
            pixel_height: pixels[1],
            colour_space: Some("DeviceRGB".into()),
            bits_per_component: Some(8),
            filters: filters.iter().map(|s| (*s).into()).collect(),
            encoded_bytes: 1_000_000,
            estimated_raw_pixel_bytes: None,
            sha256: "a".repeat(64),
            same_object_reused: false,
            identical_data_duplicate: false,
            duplicate_of: None,
            reference_count: 1,
        }
    }

    #[test]
    fn classifies_massive_optimal_and_undersampled_resolution() {
        let config = PlannerConfig::default();
        assert_eq!(
            plan_image(
                &image(
                    "1 0 R",
                    [10_000, 10_000],
                    [141.732, 141.732],
                    &["DCTDecode"]
                ),
                &config,
                &HashMap::new()
            )
            .classification,
            ResolutionClassification::Oversampled
        );
        assert_eq!(
            plan_image(
                &image("2 0 R", [300, 300], [72.0, 72.0], &["DCTDecode"]),
                &config,
                &HashMap::new()
            )
            .classification,
            ResolutionClassification::Optimal
        );
        let under = plan_image(
            &image("3 0 R", [100, 100], [72.0, 72.0], &["DCTDecode"]),
            &config,
            &HashMap::new(),
        );
        assert_eq!(under.classification, ResolutionClassification::Undersampled);
        assert!(under.recommendation.contains("DO NOT MODIFY"));
    }

    #[test]
    fn preserves_non_square_aspect_ratio_and_handles_non_uniform_scale() {
        let config = PlannerConfig::default();
        let plan = plan_image(
            &image("1 0 R", [6000, 4000], [144.0, 72.0], &["DCTDecode"]),
            &config,
            &HashMap::new(),
        );
        let [w, h] = plan.resolution_optimisation.target_pixels.unwrap();
        assert!((f64::from(w) / f64::from(h) - 1.5).abs() < 0.002);
        assert!(
            validate_raster_invariants(
                &RasterState {
                    pixel_width: 6000,
                    pixel_height: 4000,
                    placement_matrix: Matrix::IDENTITY,
                    rendered_bounding_box_pt: [0.0, 0.0, 1.0, 1.0],
                    format: RasterFormat::Jpeg,
                    colour_space: "rgb".into(),
                    bits_per_component: 8
                },
                &RasterState {
                    pixel_width: w,
                    pixel_height: h,
                    placement_matrix: Matrix::IDENTITY,
                    rendered_bounding_box_pt: [0.0, 0.0, 1.0, 1.0],
                    format: RasterFormat::Jpeg,
                    colour_space: "rgb".into(),
                    bits_per_component: 8
                },
                &ExplicitPolicyPermissions::default(),
                GEOMETRY_TOLERANCE
            )
            .valid
        );
        let mixed = plan_image(
            &image("2 0 R", [610, 1320], [720.0, 720.0], &["FlateDecode"]),
            &PlannerConfig {
                target_dpi: 96,
                ..Default::default()
            },
            &HashMap::new(),
        );
        assert_eq!(mixed.classification, ResolutionClassification::Undersampled);
    }

    #[test]
    fn rotation_does_not_change_resolution_decision() {
        let mut rotated = image("1 0 R", [3000, 2000], [144.0, 96.0], &["DCTDecode"]);
        rotated.placements = vec![analyse_placement(1, matrix(144.0, 96.0, true), 3000, 2000)];
        assert_eq!(
            plan_image(&rotated, &PlannerConfig::default(), &HashMap::new()).classification,
            ResolutionClassification::Oversampled
        );
    }

    #[test]
    fn detects_jpeg_lossless_and_unknown_encodings() {
        assert_eq!(encoding_kind(&["DCTDecode".into()]), EncodingKind::Jpeg);
        assert_eq!(
            encoding_kind(&["FlateDecode".into()]),
            EncodingKind::FlateLossless
        );
        assert_eq!(encoding_kind(&["MadeUpDecode".into()]), EncodingKind::Other);
        assert_eq!(encoding_kind(&[]), EncodingKind::Raw);
    }

    #[test]
    fn unknown_geometry_is_never_modified() {
        let mut unknown = image("1 0 R", [6000, 4000], [144.0, 96.0], &["DCTDecode"]);
        unknown.placements.clear();
        let plan = plan_image(&unknown, &PlannerConfig::default(), &HashMap::new());
        assert_eq!(plan.classification, ResolutionClassification::Unknown);
        assert!(plan.resolution_optimisation.target_pixels.is_none());
        assert!(plan.recommendation.contains("DO NOT MODIFY"));
    }

    #[test]
    fn reports_duplicate_separately() {
        let original = image("1 0 R", [300, 300], [72.0, 72.0], &["DCTDecode"]);
        let mut duplicate = image("2 0 R", [300, 300], [72.0, 72.0], &["DCTDecode"]);
        duplicate.identical_data_duplicate = true;
        duplicate.duplicate_of = Some(original.id.clone());
        let inspection = AnalysisResult {
            schema_version: "0.1.0".into(),
            file: FileInfo {
                path: "fixture.pdf".into(),
                size_bytes: 3_000_000,
                page_count: 1,
            },
            summary: SizeSummary {
                image_bytes: 2_000_000,
                ..Default::default()
            },
            pages: vec![],
            images: vec![original, duplicate],
            fonts: vec![],
            embedded_files: vec![],
            metadata: vec![],
            warnings: vec![],
        };
        let plan = create_plan(&inspection, &PlannerConfig::default());
        assert_eq!(plan.duplicates.len(), 1);
        assert_eq!(plan.duplicates[0].potential_saving_bytes, 1_000_000);
    }

    #[test]
    fn screen_profiles_preserve_page_aspect_ratios() {
        assert_eq!(
            aspect_preserving_page_pixels(1920.0, 1080.0, 1920),
            Some([1920, 1080])
        );
        assert_eq!(
            aspect_preserving_page_pixels(595.276, 841.89, 1920),
            Some([1358, 1920])
        );
        assert_eq!(
            aspect_preserving_page_pixels(841.89, 595.276, 1920),
            Some([1920, 1358])
        );
        assert_eq!(
            aspect_preserving_page_pixels(500.0, 500.0, 1920),
            Some([1920, 1920])
        );
        assert_eq!(
            DocumentTargetProfile::Screen4k.long_dimension_px(),
            Some(3840)
        );
        assert_eq!(
            DocumentTargetProfile::Screen1440p.long_dimension_px(),
            Some(2560)
        );
        assert_eq!(
            DocumentTargetProfile::Screen720p.long_dimension_px(),
            Some(1280)
        );
        assert_eq!(
            DocumentTargetProfile::Custom {
                long_dimension_px: 1777
            }
            .long_dimension_px(),
            Some(1777)
        );
    }

    #[test]
    fn screen_occupancy_handles_full_half_tiny_and_rotated_images() {
        let page_scale = HashMap::from([(1, (2.0, 2.0))]);
        let full = image("1 0 R", [6000, 4000], [1000.0, 666.0], &["DCTDecode"]);
        let half = image("2 0 R", [6000, 4000], [500.0, 333.0], &["DCTDecode"]);
        let tiny = image("3 0 R", [6000, 4000], [50.0, 33.0], &["DCTDecode"]);
        assert_eq!(screen_target_pixels(&full, &page_scale), Some([2000, 1333]));
        assert_eq!(screen_target_pixels(&half, &page_scale), Some([1000, 667]));
        assert_eq!(screen_target_pixels(&tiny, &page_scale), Some([100, 67]));
        let mut rotated = half;
        rotated.placements = vec![analyse_placement(1, matrix(500.0, 333.0, true), 6000, 4000)];
        assert_eq!(
            screen_target_pixels(&rotated, &page_scale),
            Some([1000, 667])
        );
    }

    #[test]
    fn screen_targets_never_distort_or_upsample() {
        let page_scale = HashMap::from([(1, (2.0, 2.0))]);
        let source = image("1 0 R", [6000, 4000], [500.0, 200.0], &["DCTDecode"]);
        let target = screen_target_pixels(&source, &page_scale).unwrap();
        assert!((f64::from(target[0]) / f64::from(target[1]) - 1.5).abs() < 0.002);
        let small = image("2 0 R", [300, 200], [500.0, 333.333], &["DCTDecode"]);
        let required = screen_target_pixels(&small, &page_scale);
        assert_eq!(
            classify_screen(&small, required, 1.25),
            ResolutionClassification::Undersampled
        );
    }

    #[test]
    fn document_target_report_does_not_mutate_page_geometry() {
        let page = PdfPage {
            page_number: 1,
            width_pt: 595.276,
            height_pt: 841.89,
            width_mm: 210.0,
            height_mm: 297.0,
            rotation_degrees: 90,
            object_counts: ObjectCounts::default(),
        };
        let before = (page.width_pt, page.height_pt, page.rotation_degrees);
        let report = page_render_report(&page, &DocumentTargetProfile::Screen1080p);
        assert_eq!(
            before,
            (report.width_pt, report.height_pt, report.rotation_degrees)
        );
        assert_eq!(
            (page.width_pt, page.height_pt, page.rotation_degrees),
            before
        );
    }
}

pub fn human_summary(plan: &OptimisationPlan) -> String {
    let mb = |bytes: u64| bytes as f64 / 1_000_000.0;
    format!(
        "NoBS PDF — OPTIMISATION ANALYSIS\n\nOriginal: {:.1} MB\nPages: {}\nImages: {}\n\nHIGH PRIORITY\n{} images are conservatively classified as oversampled.\nEstimated resolution saving: {:.1} MB (pixel-ratio estimate)\n\nENCODING\nNo automatic encoding savings claimed; formats are retained by default.\n\nDUPLICATES\n{} duplicate assets detected.\nPotential saving: {:.1} MB (requires safety validation)\n\nDO NOT TOUCH\n{} images are optimal, undersampled, or have unknown geometry.\n\nEstimated final size: {:.1} MB\nConfidence: {:?}\n",
        mb(plan.source.size_bytes), plan.source.page_count, plan.source.image_count,
        plan.summary.oversampled_images, mb(plan.summary.estimated_resolution_saving_bytes),
        plan.summary.duplicate_assets, mb(plan.summary.potential_duplicate_saving_bytes),
        plan.summary.do_not_modify_images, mb(plan.summary.estimated_final_size_bytes_range[0]), plan.summary.confidence
    )
}
