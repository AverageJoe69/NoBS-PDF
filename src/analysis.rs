use crate::{
    geometry::{effective_dpi, pt_to_mm, target_pixels, Matrix},
    model::{ImagePlacement, MeasurementStatus, TargetDpiAnalysis},
};
use std::collections::BTreeMap;

pub const TARGET_DPIS: [u16; 5] = [96, 150, 200, 300, 600];

pub fn analyse_placement(
    page_number: u32,
    matrix: Matrix,
    pixel_width: u32,
    pixel_height: u32,
) -> ImagePlacement {
    let (width_pt, height_pt) = matrix.axis_lengths_pt();
    let dpi_x = effective_dpi(pixel_width, width_pt);
    let dpi_y = effective_dpi(pixel_height, height_pt);
    let dpi = dpi_x.zip(dpi_y).map(|(x, y)| x.min(y));
    let targets = TARGET_DPIS
        .into_iter()
        .map(|target| {
            (
                target.to_string(),
                TargetDpiAnalysis {
                    target_dpi: target,
                    oversampling_ratio_x: dpi_x.map(|v| v / f64::from(target)),
                    oversampling_ratio_y: dpi_y.map(|v| v / f64::from(target)),
                    required_pixel_width: target_pixels(width_pt, target),
                    required_pixel_height: target_pixels(height_pt, target),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    ImagePlacement {
        page_number,
        matrix,
        bounding_box_pt: matrix.unit_square_bounds(),
        displayed_width_pt: width_pt,
        displayed_height_pt: height_pt,
        displayed_width_mm: pt_to_mm(width_pt),
        displayed_height_mm: pt_to_mm(height_pt),
        effective_dpi_x: dpi_x,
        effective_dpi_y: dpi_y,
        effective_dpi: dpi,
        target_analysis: targets,
        status: if dpi.is_some() {
            MeasurementStatus::Measured
        } else {
            MeasurementStatus::Unknown
        },
    }
}
