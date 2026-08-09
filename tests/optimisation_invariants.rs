use pdfdoctor::{
    geometry::Matrix,
    optimisation::{
        validate_raster_invariants, ExplicitPolicyPermissions, InvariantViolation, RasterFormat,
        RasterState, GEOMETRY_TOLERANCE,
    },
};

fn state(width: u32, height: u32) -> RasterState {
    let matrix = Matrix {
        a: 0.0,
        b: 141.732,
        c: -85.039,
        d: 0.0,
        e: 200.0,
        f: 300.0,
    };
    RasterState {
        pixel_width: width,
        pixel_height: height,
        placement_matrix: matrix,
        rendered_bounding_box_pt: matrix.unit_square_bounds(),
        format: RasterFormat::Jpeg,
        colour_space: "DeviceRGB".into(),
        bits_per_component: 8,
    }
}

#[test]
fn proportional_downsample_preserves_all_invariants() {
    let result = validate_raster_invariants(
        &state(6000, 4000),
        &state(900, 600),
        &ExplicitPolicyPermissions::default(),
        GEOMETRY_TOLERANCE,
    );
    assert!(result.valid);
}

#[test]
fn integer_rounding_is_accepted_but_distortion_is_rejected() {
    assert!(
        validate_raster_invariants(
            &state(6000, 4000),
            &state(496, 331),
            &ExplicitPolicyPermissions::default(),
            GEOMETRY_TOLERANCE,
        )
        .valid
    );
    let result = validate_raster_invariants(
        &state(6000, 4000),
        &state(496, 300),
        &ExplicitPolicyPermissions::default(),
        GEOMETRY_TOLERANCE,
    );
    assert!(result
        .violations
        .contains(&InvariantViolation::AspectRatioChanged));
}

#[test]
fn matrix_or_bounds_changes_are_rejected() {
    let before = state(6000, 4000);
    let mut after = state(900, 600);
    after.placement_matrix.e += 0.01;
    after.rendered_bounding_box_pt[0] += 0.01;
    let result = validate_raster_invariants(
        &before,
        &after,
        &ExplicitPolicyPermissions::default(),
        GEOMETRY_TOLERANCE,
    );
    assert!(result
        .violations
        .contains(&InvariantViolation::PlacementMatrixChanged));
    assert!(result
        .violations
        .contains(&InvariantViolation::RenderedBoundingBoxChanged));
}

#[test]
fn incidental_format_and_colour_changes_are_rejected() {
    let before = state(6000, 4000);
    let mut after = state(900, 600);
    after.format = RasterFormat::Png;
    after.colour_space = "DeviceGray".into();
    after.bits_per_component = 4;
    let result = validate_raster_invariants(
        &before,
        &after,
        &ExplicitPolicyPermissions::default(),
        GEOMETRY_TOLERANCE,
    );
    assert_eq!(result.violations.len(), 3);
}
