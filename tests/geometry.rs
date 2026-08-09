use pdfdoctor::{
    analysis::analyse_placement,
    geometry::{pt_to_mm, Matrix},
};

fn scale_mm(w: f64, h: f64) -> Matrix {
    Matrix {
        a: w * 72.0 / 25.4,
        b: 0.0,
        c: 0.0,
        d: h * 72.0 / 25.4,
        e: 0.0,
        f: 0.0,
    }
}

#[test]
fn a4_units() {
    assert!((pt_to_mm(595.2756) - 210.0).abs() < 0.001);
    assert!((pt_to_mm(841.8898) - 297.0).abs() < 0.001);
}
#[test]
fn huge_image_at_50mm() {
    let p = analyse_placement(1, scale_mm(50.0, 50.0), 10_000, 10_000);
    assert!((p.effective_dpi.unwrap() - 5080.0).abs() < 0.01);
    assert_eq!(p.target_analysis["300"].required_pixel_width, Some(591));
}
#[test]
fn thousand_pixels_at_250mm() {
    let p = analyse_placement(1, scale_mm(250.0, 250.0), 1000, 1000);
    assert!((p.effective_dpi.unwrap() - 101.6).abs() < 0.01);
}
#[test]
fn non_square_and_non_uniform() {
    let p = analyse_placement(1, scale_mm(100.0, 50.0), 2000, 500);
    assert!((p.effective_dpi_x.unwrap() - 508.0).abs() < 0.01);
    assert!((p.effective_dpi_y.unwrap() - 254.0).abs() < 0.01);
}
#[test]
fn rotation_preserves_dimensions() {
    let p = analyse_placement(
        1,
        Matrix {
            a: 0.0,
            b: 100.0,
            c: -50.0,
            d: 0.0,
            e: 10.0,
            f: 20.0,
        },
        1000,
        500,
    );
    assert_eq!(p.displayed_width_pt, 100.0);
    assert_eq!(p.displayed_height_pt, 50.0);
}
#[test]
fn zero_size_is_unknown() {
    let p = analyse_placement(
        1,
        Matrix {
            a: 0.0,
            b: 0.0,
            c: 0.0,
            d: 0.0,
            e: 0.0,
            f: 0.0,
        },
        100,
        100,
    );
    assert!(p.effective_dpi.is_none());
}
