use std::{collections::HashSet, path::Path};

use lopdf::{Dictionary, Document};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{geometry::Matrix, model::AnalysisResult, optimisation::GEOMETRY_TOLERANCE};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ValidationReport {
    pub passed: bool,
    pub page_count_preserved: bool,
    pub page_geometry_preserved: bool,
    pub page_rotation_preserved: bool,
    pub image_placement_preserved: bool,
    pub image_aspect_ratios_preserved: bool,
    pub image_formats_preserved: bool,
    pub no_images_upsampled: bool,
    pub non_targeted_objects_preserved: bool,
    pub text_content_preserved: bool,
    pub vector_content_preserved: bool,
    pub annotations_and_links_preserved: bool,
    pub transparency_and_other_content_preserved: bool,
    pub aspect_ratio_violations: usize,
    pub placement_changes: usize,
    pub page_geometry_changes: usize,
    pub format_changes: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("PDF validation load failed: {0}")]
    Pdf(#[from] lopdf::Error),
}

pub fn validate_export(
    input_path: &Path,
    output_path: &Path,
    before: &AnalysisResult,
    after: &AnalysisResult,
    modified_ids: &HashSet<String>,
) -> Result<ValidationReport, ValidationError> {
    let mut report = ValidationReport {
        page_count_preserved: before.pages.len() == after.pages.len(),
        page_geometry_preserved: true,
        page_rotation_preserved: true,
        image_placement_preserved: true,
        image_aspect_ratios_preserved: true,
        image_formats_preserved: true,
        no_images_upsampled: true,
        text_content_preserved: true,
        vector_content_preserved: true,
        ..ValidationReport::default()
    };
    if !report.page_count_preserved {
        report.errors.push("page count changed".into());
    }
    for (a, b) in before.pages.iter().zip(&after.pages) {
        if !close(a.width_pt, b.width_pt) || !close(a.height_pt, b.height_pt) {
            report.page_geometry_preserved = false;
            report.page_geometry_changes += 1;
        }
        if a.rotation_degrees != b.rotation_degrees {
            report.page_rotation_preserved = false;
        }
        if a.object_counts.text_operations != b.object_counts.text_operations {
            report.text_content_preserved = false;
        }
        if a.object_counts.vector_operations != b.object_counts.vector_operations {
            report.vector_content_preserved = false;
        }
    }

    for id in modified_ids {
        let Some(a) = before.images.iter().find(|image| &image.id == id) else {
            report.errors.push(format!("original image {id} missing"));
            continue;
        };
        let Some(b) = after.images.iter().find(|image| &image.id == id) else {
            report.errors.push(format!("exported image {id} missing"));
            continue;
        };
        if a.pages != b.pages
            || a.reference_count != b.reference_count
            || a.placements.len() != b.placements.len()
        {
            report.image_placement_preserved = false;
            report.placement_changes += 1;
        }
        for (pa, pb) in a.placements.iter().zip(&b.placements) {
            if !matrix_close(pa.matrix, pb.matrix)
                || !pa
                    .bounding_box_pt
                    .iter()
                    .zip(pb.bounding_box_pt)
                    .all(|(x, y)| close(*x, y))
                || !close(pa.displayed_width_pt, pb.displayed_width_pt)
                || !close(pa.displayed_height_pt, pb.displayed_height_pt)
            {
                report.image_placement_preserved = false;
                report.placement_changes += 1;
            }
        }
        let cross_a = u64::from(a.pixel_width) * u64::from(b.pixel_height);
        let cross_b = u64::from(a.pixel_height) * u64::from(b.pixel_width);
        if cross_a.abs_diff(cross_b) > u64::from(a.pixel_width.max(a.pixel_height)) {
            report.image_aspect_ratios_preserved = false;
            report.aspect_ratio_violations += 1;
        }
        if a.filters != b.filters
            || a.colour_space != b.colour_space
            || a.bits_per_component != b.bits_per_component
        {
            report.image_formats_preserved = false;
            report.format_changes += 1;
        }
        if b.pixel_width > a.pixel_width || b.pixel_height > a.pixel_height {
            report.no_images_upsampled = false;
        }
    }

    let original = Document::load(input_path)?;
    let exported = Document::load(output_path)?;
    report.non_targeted_objects_preserved =
        compare_object_graphs(&original, &exported, modified_ids, &mut report.errors);
    report.annotations_and_links_preserved = report.non_targeted_objects_preserved;
    report.transparency_and_other_content_preserved = report.non_targeted_objects_preserved;
    report.passed = report.page_count_preserved
        && report.page_geometry_preserved
        && report.page_rotation_preserved
        && report.image_placement_preserved
        && report.image_aspect_ratios_preserved
        && report.image_formats_preserved
        && report.no_images_upsampled
        && report.non_targeted_objects_preserved
        && report.text_content_preserved
        && report.vector_content_preserved
        && report.errors.is_empty();
    Ok(report)
}

fn compare_object_graphs(
    original: &Document,
    exported: &Document,
    modified_ids: &HashSet<String>,
    errors: &mut Vec<String>,
) -> bool {
    for (&id, before) in &original.objects {
        let label = format!("{} {} R", id.0, id.1);
        let Some(after) = exported.objects.get(&id) else {
            if is_bookkeeping_stream(before) {
                continue;
            }
            errors.push(format!(
                "object {label} disappeared: {}",
                object_summary(before)
            ));
            return false;
        };
        if modified_ids.contains(&label) {
            let (Ok(a), Ok(b)) = (before.as_stream(), after.as_stream()) else {
                errors.push(format!("modified image {label} is no longer a stream"));
                return false;
            };
            if format!("{:?}", sanitised_image_dict(&a.dict))
                != format!("{:?}", sanitised_image_dict(&b.dict))
            {
                errors.push(format!(
                    "non-resolution dictionary data changed for {label}"
                ));
                return false;
            }
        } else if !semantic_object_equal(before, after) {
            errors.push(format!(
                "non-targeted object {label} changed (before: {}; after: {})",
                object_summary(before),
                object_summary(after)
            ));
            return false;
        }
    }
    for (&id, after) in &exported.objects {
        if !original.objects.contains_key(&id) && !is_bookkeeping_stream(after) {
            errors.push(format!(
                "unexpected logical object {} {} R appeared",
                id.0, id.1
            ));
            return false;
        }
    }
    true
}

fn semantic_object_equal(before: &lopdf::Object, after: &lopdf::Object) -> bool {
    match (before.as_stream(), after.as_stream()) {
        (Ok(a), Ok(b)) => {
            a.content == b.content && format!("{:?}", a.dict) == format!("{:?}", b.dict)
        }
        (Err(_), Err(_)) => format!("{before:?}") == format!("{after:?}"),
        _ => false,
    }
}

fn object_summary(object: &lopdf::Object) -> String {
    let value = format!("{object:?}");
    value.chars().take(500).collect()
}

fn is_bookkeeping_stream(object: &lopdf::Object) -> bool {
    if object
        .as_dict()
        .ok()
        .is_some_and(|dictionary| dictionary.has(b"Linearized"))
    {
        return true;
    }
    object
        .as_stream()
        .ok()
        .and_then(|stream| stream.dict.get(b"Type").ok())
        .and_then(|value| value.as_name().ok())
        .is_some_and(|name| matches!(name, b"ObjStm" | b"XRef"))
}

fn sanitised_image_dict(dictionary: &Dictionary) -> Dictionary {
    let mut result = dictionary.clone();
    result.remove(b"Width");
    result.remove(b"Height");
    result.remove(b"Length");
    result
}
fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= GEOMETRY_TOLERANCE * a.abs().max(b.abs()).max(1.0)
}
fn matrix_close(a: Matrix, b: Matrix) -> bool {
    [a.a, a.b, a.c, a.d, a.e, a.f]
        .into_iter()
        .zip([b.a, b.b, b.c, b.d, b.e, b.f])
        .all(|(x, y)| close(x, y))
}
