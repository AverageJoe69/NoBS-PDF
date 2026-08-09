//! Contracts for future optimisation. This module validates proposed results; it does not mutate PDFs.

use serde::{Deserialize, Serialize};

use crate::geometry::Matrix;

/// Default tolerance for comparing PDF-space geometry and affine matrices.
pub const GEOMETRY_TOLERANCE: f64 = 1e-7;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RasterFormat {
    Jpeg,
    Png,
    Jpeg2000,
    Ccitt,
    Other(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RasterState {
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub placement_matrix: Matrix,
    pub rendered_bounding_box_pt: [f64; 4],
    pub format: RasterFormat,
    pub colour_space: String,
    pub bits_per_component: u16,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExplicitPolicyPermissions {
    pub allow_format_conversion: bool,
    pub allow_colour_space_change: bool,
    pub allow_colour_depth_change: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvariantViolation {
    InvalidPixelDimensions,
    AspectRatioChanged,
    PlacementMatrixChanged,
    RenderedBoundingBoxChanged,
    FormatChangedWithoutPermission,
    ColourSpaceChangedWithoutPermission,
    ColourDepthChangedWithoutPermission,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvariantValidation {
    pub valid: bool,
    pub violations: Vec<InvariantViolation>,
}

/// Validates the properties that can be proven from pre/post object state.
///
/// Unchanged placement matrix proves unchanged scale, translation, rotation and shear. Combined
/// with an unchanged bounding box and aspect ratio, this rejects cropping, stretching and
/// squashing represented by the rebuilt image object. A future optimiser must additionally render
/// and compare output when a policy permits colour changes.
pub fn validate_raster_invariants(
    before: &RasterState,
    after: &RasterState,
    permissions: &ExplicitPolicyPermissions,
    tolerance: f64,
) -> InvariantValidation {
    let mut violations = Vec::new();
    if before.pixel_width == 0
        || before.pixel_height == 0
        || after.pixel_width == 0
        || after.pixel_height == 0
    {
        violations.push(InvariantViolation::InvalidPixelDimensions);
    } else {
        // Cross multiplication avoids division and measures the integer rounding error in pixels.
        let lhs = u64::from(after.pixel_width) * u64::from(before.pixel_height);
        let rhs = u64::from(after.pixel_height) * u64::from(before.pixel_width);
        let integer_tolerance = u64::from(before.pixel_width.max(before.pixel_height));
        if lhs.abs_diff(rhs) > integer_tolerance {
            violations.push(InvariantViolation::AspectRatioChanged);
        }
    }
    if !matrix_approximately_equal(before.placement_matrix, after.placement_matrix, tolerance) {
        violations.push(InvariantViolation::PlacementMatrixChanged);
    }
    if !before
        .rendered_bounding_box_pt
        .iter()
        .zip(after.rendered_bounding_box_pt.iter())
        .all(|(a, b)| approximately_equal(*a, *b, tolerance))
    {
        violations.push(InvariantViolation::RenderedBoundingBoxChanged);
    }
    if before.format != after.format && !permissions.allow_format_conversion {
        violations.push(InvariantViolation::FormatChangedWithoutPermission);
    }
    if before.colour_space != after.colour_space && !permissions.allow_colour_space_change {
        violations.push(InvariantViolation::ColourSpaceChangedWithoutPermission);
    }
    if before.bits_per_component != after.bits_per_component
        && !permissions.allow_colour_depth_change
    {
        violations.push(InvariantViolation::ColourDepthChangedWithoutPermission);
    }
    InvariantValidation {
        valid: violations.is_empty(),
        violations,
    }
}

fn matrix_approximately_equal(a: Matrix, b: Matrix, tolerance: f64) -> bool {
    [a.a, a.b, a.c, a.d, a.e, a.f]
        .iter()
        .zip([b.a, b.b, b.c, b.d, b.e, b.f].iter())
        .all(|(x, y)| approximately_equal(*x, *y, tolerance))
}

fn approximately_equal(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance.max(0.0) * a.abs().max(b.abs()).max(1.0)
}
