//! Golden regression checks. These are ignored by default because the shared
//! benchmark run renders the 61 MB fixture; run with the command in BENCHMARK.md.

use std::{path::Path, sync::OnceLock};

use pdfdoctor::benchmark::BenchmarkReport;
use serde_json::Value;

static REPORT: OnceLock<BenchmarkReport> = OnceLock::new();
static MANIFEST: OnceLock<Value> = OnceLock::new();

fn report() -> &'static BenchmarkReport {
    REPORT.get_or_init(|| {
        pdfdoctor::benchmark::run_1080p(
            Path::new("tests/MT_AngelRaise_01.pdf"),
            Some(Path::new("vendor/pdfium/lib/libpdfium.dylib")),
        )
        .expect("golden benchmark must complete")
    })
}

fn manifest() -> &'static Value {
    MANIFEST.get_or_init(|| {
        serde_json::from_str(include_str!("benchmarks/MT_AngelRaise_1080p.json")).unwrap()
    })
}

#[test]
#[ignore = "expensive golden PDF render; run cargo test --release --test benchmark -- --ignored --test-threads=1"]
fn test_1080p_optimisation() {
    let r = report();
    let m = manifest();
    assert_eq!(r.input.sha256, m["source"]["sha256"].as_str().unwrap());
    assert_eq!(
        r.input.page_count,
        m["source"]["page_count"].as_u64().unwrap() as usize
    );
    let limits = &m["acceptable_output"];
    assert!(r.output.size_bytes >= limits["minimum_size_bytes"].as_u64().unwrap());
    assert!(r.output.size_bytes <= limits["maximum_size_bytes"].as_u64().unwrap());
    assert!(r.output.reduction_percent >= limits["minimum_reduction_percent"].as_f64().unwrap());
    assert!(r.validation.passed);
}

macro_rules! golden_check {
    ($name:ident, $body:expr) => {
        #[test]
        #[ignore = "covered by the shared expensive golden benchmark"]
        fn $name() {
            assert!(($body)(report(), manifest()));
        }
    };
}

golden_check!(test_page_geometry_preserved, |r: &BenchmarkReport, _| r
    .validation
    .page_geometry
    && r.validation.page_boxes
    && r.validation.page_rotation);
golden_check!(test_aspect_ratios_preserved, |r: &BenchmarkReport, _| r
    .validation
    .aspect_ratios);
golden_check!(test_text_preserved, |r: &BenchmarkReport, m: &Value| r
    .validation
    .selectable_text_present
    && r.analysis.text_operations_after
        >= m["acceptable_output"]["minimum_selectable_text_operations"]
            .as_u64()
            .unwrap() as usize);
golden_check!(test_vectors_preserved, |r: &BenchmarkReport, m: &Value| r
    .validation
    .native_vectors_present
    && r.analysis.vector_operations_after
        >= m["acceptable_output"]["minimum_native_vector_operations"]
            .as_u64()
            .unwrap() as usize);
golden_check!(test_raster_merge_order, |r: &BenchmarkReport, m: &Value| r
    .validation
    .rendered_pages
    && r.validation.maximum_mean_render_error
        <= m["acceptable_output"]["maximum_mean_render_error"]
            .as_f64()
            .unwrap());
golden_check!(test_output_validity, |r: &BenchmarkReport, _| r
    .validation
    .output_valid
    && r.validation.output_non_zero);
golden_check!(
    test_original_not_modified,
    |r: &BenchmarkReport, m: &Value| r.input.sha256 == m["source"]["sha256"].as_str().unwrap()
);
golden_check!(test_output_is_smaller, |r: &BenchmarkReport, _| r
    .validation
    .output_smaller);
