//! Deterministic per-page raster-budget detection for the desktop Size model.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::model::{AnalysisResult, ImagePlacement, PdfPage};

const MAX_PHYSICAL_SAMPLING: f64 = 300.0;
const SUBSTANTIAL_COVERAGE: f64 = 0.15;
const CORROBORATED_COVERAGE: f64 = 0.25;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageResolutionClass {
    Digital,
    Physical,
    VectorOnly,
    Ambiguous,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageRasterBudget {
    pub page_number: u32,
    pub classification: PageResolutionClass,
    /// The useful 100% raster canvas. Hidden from the UI when confidence is insufficient.
    pub budget_100_percent: Option<[u32; 2]>,
    pub display_dimensions: bool,
    pub confidence: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentResolution {
    pub pages: Vec<PageRasterBudget>,
    pub has_raster_content: bool,
    pub mixed_page_sizes: bool,
    pub representative_100_percent: Option<[u32; 2]>,
    pub adaptive: bool,
}

impl DocumentResolution {
    pub fn scaled_budgets(&self, scale_percent: u8) -> HashMap<u32, [u32; 2]> {
        self.pages
            .iter()
            .filter_map(|page| {
                page.budget_100_percent
                    .map(|budget| (page.page_number, scale_dimensions(budget, scale_percent)))
            })
            .collect()
    }
}

pub fn detect_document_resolution(inspection: &AnalysisResult) -> DocumentResolution {
    let pages = inspection
        .pages
        .iter()
        .map(|page| detect_page_resolution(inspection, page))
        .collect::<Vec<_>>();
    let visible = pages
        .iter()
        .filter(|page| page.display_dimensions)
        .filter_map(|page| page.budget_100_percent)
        .collect::<Vec<_>>();
    let representative_100_percent = visible
        .iter()
        .copied()
        .max_by_key(|dimensions| u64::from(dimensions[0]) * u64::from(dimensions[1]));
    let mixed_page_sizes = visible
        .first()
        .is_some_and(|first| visible.iter().any(|dimensions| dimensions != first));
    DocumentResolution {
        has_raster_content: pages
            .iter()
            .any(|page| page.classification != PageResolutionClass::VectorOnly),
        mixed_page_sizes,
        representative_100_percent,
        adaptive: pages.iter().any(|page| {
            page.classification == PageResolutionClass::Ambiguous && !page.display_dimensions
        }),
        pages,
    }
}

pub fn scale_dimensions(dimensions: [u32; 2], scale_percent: u8) -> [u32; 2] {
    let scale = u32::from(scale_percent.clamp(10, 100));
    [
        ((u64::from(dimensions[0]) * u64::from(scale) + 50) / 100).max(1) as u32,
        ((u64::from(dimensions[1]) * u64::from(scale) + 50) / 100).max(1) as u32,
    ]
}

fn detect_page_resolution(inspection: &AnalysisResult, page: &PdfPage) -> PageRasterBudget {
    let placements = inspection
        .images
        .iter()
        .flat_map(|image| image.placements.iter())
        .filter(|placement| placement.page_number == page.page_number)
        .collect::<Vec<_>>();
    if placements.is_empty() {
        return page_budget(
            page,
            PageResolutionClass::VectorOnly,
            None,
            false,
            true,
            "No placed raster content was found; native text and vectors need no raster budget.",
        );
    }
    if is_digital_canvas(page) {
        return page_budget(
            page,
            PageResolutionClass::Digital,
            oriented_dimensions(
                page,
                [
                    page.width_pt.round().max(1.0) as u32,
                    page.height_pt.round().max(1.0) as u32,
                ],
            ),
            true,
            true,
            "The page geometry matches a recognised digital canvas.",
        );
    }
    let evidence = sampling_evidence(page, &placements);
    if is_physical_page(page) {
        let sampling = representative_sampling(&evidence).unwrap_or(MAX_PHYSICAL_SAMPLING);
        return page_budget(
            page,
            PageResolutionClass::Physical,
            physical_dimensions(page, sampling.min(MAX_PHYSICAL_SAMPLING)),
            true,
            true,
            "The page geometry matches a recognised physical document format.",
        );
    }
    let corroborated = evidence.len() >= 2
        && evidence.iter().map(|sample| sample.coverage).sum::<f64>() >= CORROBORATED_COVERAGE;
    let sampling = if corroborated {
        representative_sampling(&evidence).unwrap_or(MAX_PHYSICAL_SAMPLING)
    } else {
        MAX_PHYSICAL_SAMPLING
    };
    page_budget(
        page,
        PageResolutionClass::Ambiguous,
        physical_dimensions(page, sampling.min(MAX_PHYSICAL_SAMPLING)),
        corroborated,
        corroborated,
        if corroborated {
            "Multiple substantial raster placements provide consistent page-resolution evidence."
        } else {
            "Authoring intent is uncertain; an internal conservative adaptive raster ceiling is used."
        },
    )
}

fn page_budget(
    page: &PdfPage,
    classification: PageResolutionClass,
    dimensions: Option<[u32; 2]>,
    display_dimensions: bool,
    confidence: bool,
    reason: &str,
) -> PageRasterBudget {
    PageRasterBudget {
        page_number: page.page_number,
        classification,
        budget_100_percent: dimensions,
        display_dimensions,
        confidence,
        reason: reason.into(),
    }
}

fn oriented_dimensions(page: &PdfPage, dimensions: [u32; 2]) -> Option<[u32; 2]> {
    Some(if page.rotation_degrees.rem_euclid(180) == 90 {
        [dimensions[1], dimensions[0]]
    } else {
        dimensions
    })
}

fn physical_dimensions(page: &PdfPage, sampling: f64) -> Option<[u32; 2]> {
    if !sampling.is_finite() || sampling <= 0.0 {
        return None;
    }
    oriented_dimensions(
        page,
        [
            (page.width_pt / 72.0 * sampling).round().max(1.0) as u32,
            (page.height_pt / 72.0 * sampling).round().max(1.0) as u32,
        ],
    )
}

#[derive(Debug, Clone, Copy)]
struct SamplingEvidence {
    sampling: f64,
    coverage: f64,
}

fn sampling_evidence(page: &PdfPage, placements: &[&ImagePlacement]) -> Vec<SamplingEvidence> {
    let area = page.width_pt * page.height_pt;
    if !area.is_finite() || area <= 0.0 {
        return vec![];
    }
    placements
        .iter()
        .filter_map(|placement| {
            let coverage = (placement.displayed_width_pt * placement.displayed_height_pt / area)
                .clamp(0.0, 1.0);
            let sampling = placement.effective_dpi_x?.min(placement.effective_dpi_y?);
            (coverage >= SUBSTANTIAL_COVERAGE && sampling.is_finite() && sampling > 0.0)
                .then_some(SamplingEvidence { sampling, coverage })
        })
        .collect()
}

fn representative_sampling(evidence: &[SamplingEvidence]) -> Option<f64> {
    let mut samples = evidence.to_vec();
    samples.sort_by(|a, b| a.sampling.total_cmp(&b.sampling));
    let total = samples
        .iter()
        .map(|sample| sample.coverage.min(0.5))
        .sum::<f64>();
    if total <= 0.0 {
        return None;
    }
    let midpoint = total / 2.0;
    let mut accumulated = 0.0;
    for sample in samples {
        accumulated += sample.coverage.min(0.5);
        if accumulated >= midpoint {
            return Some(sample.sampling.min(MAX_PHYSICAL_SAMPLING));
        }
    }
    None
}

fn is_digital_canvas(page: &PdfPage) -> bool {
    let width = page.width_pt;
    let height = page.height_pt;
    if !near_integer(width) || !near_integer(height) || is_physical_page(page) {
        return false;
    }
    const CANVASES: &[[u32; 2]] = &[
        [640, 360],
        [800, 600],
        [1024, 768],
        [1280, 720],
        [1280, 800],
        [1366, 768],
        [1440, 900],
        [1600, 900],
        [1920, 1080],
        [1920, 1200],
        [2048, 1080],
        [2048, 1536],
        [2560, 1440],
        [2560, 1600],
        [3840, 2160],
        [4096, 2160],
        [5120, 2880],
        [7680, 4320],
    ];
    let dimensions = [width.round() as u32, height.round() as u32];
    CANVASES.iter().any(|canvas| {
        close_dimensions(dimensions, *canvas)
            || close_dimensions(dimensions, [canvas[1], canvas[0]])
    })
}

fn near_integer(value: f64) -> bool {
    (value - value.round()).abs() <= 0.5
}

fn close_dimensions(actual: [u32; 2], expected: [u32; 2]) -> bool {
    actual[0].abs_diff(expected[0]) <= 2 && actual[1].abs_diff(expected[1]) <= 2
}

fn is_physical_page(page: &PdfPage) -> bool {
    const FORMATS_MM: &[[f64; 2]] = &[
        [841.0, 1189.0],
        [594.0, 841.0],
        [420.0, 594.0],
        [297.0, 420.0],
        [210.0, 297.0],
        [148.0, 210.0],
        [105.0, 148.0],
        [74.0, 105.0],
        [215.9, 279.4],
        [215.9, 355.6],
        [279.4, 431.8],
    ];
    FORMATS_MM.iter().any(|format| {
        close_mm([page.width_mm, page.height_mm], *format)
            || close_mm([page.width_mm, page.height_mm], [format[1], format[0]])
    })
}

fn close_mm(actual: [f64; 2], expected: [f64; 2]) -> bool {
    (actual[0] - expected[0]).abs() <= 1.5 && (actual[1] - expected[1]).abs() <= 1.5
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Matrix;
    use crate::model::{FileInfo, ImageAnalysis, MeasurementStatus, ObjectCounts, SizeSummary};
    use std::collections::BTreeMap;

    fn page(width: f64, height: f64) -> PdfPage {
        PdfPage {
            page_number: 1,
            width_pt: width,
            height_pt: height,
            width_mm: width * 25.4 / 72.0,
            height_mm: height * 25.4 / 72.0,
            rotation_degrees: 0,
            object_counts: ObjectCounts::default(),
        }
    }
    fn inspection(page: PdfPage, rasters: &[(u32, u32, f64, f64)]) -> AnalysisResult {
        let images = rasters
            .iter()
            .copied()
            .enumerate()
            .map(|(index, (w, h, dw, dh))| ImageAnalysis {
                id: format!("{} 0 R", index + 1),
                object_number: (index + 1) as u32,
                generation: 0,
                pages: vec![1],
                placements: vec![ImagePlacement {
                    page_number: 1,
                    matrix: Matrix::IDENTITY,
                    bounding_box_pt: [0.0, 0.0, dw, dh],
                    displayed_width_pt: dw,
                    displayed_height_pt: dh,
                    displayed_width_mm: dw * 25.4 / 72.0,
                    displayed_height_mm: dh * 25.4 / 72.0,
                    effective_dpi_x: Some(w as f64 * 72.0 / dw),
                    effective_dpi_y: Some(h as f64 * 72.0 / dh),
                    effective_dpi: None,
                    target_analysis: BTreeMap::new(),
                    status: MeasurementStatus::Measured,
                }],
                pixel_width: w,
                pixel_height: h,
                colour_space: Some("DeviceRGB".into()),
                bits_per_component: Some(8),
                filters: vec!["DCTDecode".into()],
                encoded_bytes: 1,
                estimated_raw_pixel_bytes: None,
                sha256: String::new(),
                same_object_reused: false,
                identical_data_duplicate: false,
                duplicate_of: None,
                reference_count: 1,
            })
            .collect();
        AnalysisResult {
            schema_version: "test".into(),
            file: FileInfo {
                path: String::new(),
                size_bytes: 1,
                page_count: 1,
            },
            summary: SizeSummary::default(),
            pages: vec![page],
            images,
            fonts: vec![],
            embedded_files: vec![],
            metadata: vec![],
            warnings: vec![],
        }
    }

    #[test]
    fn digital_geometry_ignores_oversized_source() {
        let result = detect_document_resolution(&inspection(
            page(1920.0, 1080.0),
            &[(6000, 3375, 1920.0, 1080.0)],
        ));
        assert_eq!(result.pages[0].classification, PageResolutionClass::Digital);
        assert_eq!(result.pages[0].budget_100_percent, Some([1920, 1080]));
        let four_k = detect_document_resolution(&inspection(
            page(3840.0, 2160.0),
            &[(12000, 6750, 3840.0, 2160.0)],
        ));
        assert_eq!(four_k.pages[0].classification, PageResolutionClass::Digital);
        assert_eq!(four_k.pages[0].budget_100_percent, Some([3840, 2160]));
    }

    #[test]
    fn a4_full_page_raster_is_capped_at_print_ceiling() {
        let result = detect_document_resolution(&inspection(
            page(595.2756, 841.8898),
            &[(6000, 8485, 595.2756, 841.8898)],
        ));
        assert_eq!(
            result.pages[0].classification,
            PageResolutionClass::Physical
        );
        assert_eq!(result.pages[0].budget_100_percent, Some([2480, 3508]));
    }

    #[test]
    fn vector_only_has_no_invented_budget() {
        let result = detect_document_resolution(&inspection(page(1920.0, 1080.0), &[]));
        assert_eq!(
            result.pages[0].classification,
            PageResolutionClass::VectorOnly
        );
        assert_eq!(result.pages[0].budget_100_percent, None);
        assert!(!result.has_raster_content);
    }

    #[test]
    fn physical_pages_preserve_lower_representative_sampling() {
        let result = detect_document_resolution(&inspection(
            page(595.2756, 841.8898),
            &[
                (1240, 1754, 595.2756, 841.8898),
                (620, 877, 297.6378, 420.9449),
            ],
        ));
        assert_eq!(result.pages[0].budget_100_percent, Some([1240, 1754]));
    }

    #[test]
    fn tiny_logo_does_not_establish_physical_page_sampling() {
        let result = detect_document_resolution(&inspection(
            page(595.2756, 841.8898),
            &[(1200, 1200, 72.0, 72.0)],
        ));
        assert_eq!(result.pages[0].budget_100_percent, Some([2480, 3508]));
    }

    #[test]
    fn ambiguous_pages_need_corroboration_before_dimensions_are_claimed() {
        let uncertain = detect_document_resolution(&inspection(
            page(1000.0, 700.0),
            &[(10000, 7000, 1000.0, 700.0)],
        ));
        assert_eq!(
            uncertain.pages[0].classification,
            PageResolutionClass::Ambiguous
        );
        assert!(!uncertain.pages[0].display_dimensions);
        assert_eq!(uncertain.pages[0].budget_100_percent, Some([4167, 2917]));
        let corroborated = detect_document_resolution(&inspection(
            page(1000.0, 700.0),
            &[(2400, 1680, 1000.0, 700.0), (1200, 840, 500.0, 350.0)],
        ));
        assert!(corroborated.pages[0].display_dimensions);
        assert_eq!(corroborated.pages[0].budget_100_percent, Some([2400, 1680]));
    }

    #[test]
    fn percentage_scaling_is_dimensionally_exact() {
        assert_eq!(scale_dimensions([3840, 2160], 50), [1920, 1080]);
        assert_eq!(scale_dimensions([2480, 3508], 75), [1860, 2631]);
        assert_eq!(scale_dimensions([2480, 3508], 10), [248, 351]);
    }

    #[test]
    fn mixed_pages_keep_independent_budgets_and_report_the_largest() {
        let mut digital = inspection(page(1920.0, 1080.0), &[(6000, 3375, 1920.0, 1080.0)]);
        let mut physical = inspection(
            page(595.2756, 841.8898),
            &[(6000, 8485, 595.2756, 841.8898)],
        );
        physical.pages[0].page_number = 2;
        for image in &mut physical.images {
            image.pages = vec![2];
            image.object_number += 10;
            image.id = format!("{} 0 R", image.object_number);
            for placement in &mut image.placements {
                placement.page_number = 2;
            }
        }
        digital.pages.append(&mut physical.pages);
        digital.images.append(&mut physical.images);
        digital.file.page_count = 2;
        let result = detect_document_resolution(&digital);
        assert_eq!(result.pages[0].budget_100_percent, Some([1920, 1080]));
        assert_eq!(result.pages[1].budget_100_percent, Some([2480, 3508]));
        assert!(result.mixed_page_sizes);
        assert_eq!(result.representative_100_percent, Some([2480, 3508]));
    }
}
