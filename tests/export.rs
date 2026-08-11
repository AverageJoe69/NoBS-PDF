use std::{collections::HashSet, fs, path::Path};

use jpeg_encoder::{ColorType, Encoder};
use lopdf::{
    content::{Content, Operation},
    dictionary, Document, Object, Stream,
};
use pdfdoctor::{
    exporter::{
        export_1080p, export_for_scale, export_original_resolution, ExportError, ExportOptions,
    },
    parser::InspectionError,
    validator::validate_export,
};
use tempfile::TempDir;

fn jpeg(width: u16, height: u16) -> Vec<u8> {
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 3);
    for y in 0..height {
        for x in 0..width {
            pixels.extend_from_slice(&[(x % 251) as u8, (y % 241) as u8, ((x + y) % 239) as u8]);
        }
    }
    let mut output = Vec::new();
    Encoder::new(&mut output, 92)
        .encode(&pixels, width, height, ColorType::Rgb)
        .unwrap();
    output
}

fn fixture(path: &Path) {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let image_id = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => 1200,
            "Height" => 800, "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8,
            "Filter" => "DCTDecode",
        },
        jpeg(1200, 800),
    ));
    let font_id = doc.add_object(
        dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" },
    );
    let annotation_id = doc.add_object(dictionary! { "Type" => "Annot", "Subtype" => "Link", "Rect" => vec![10.into(),10.into(),100.into(),30.into()] });
    let content = Content {
        operations: vec![
            Operation::new("q", vec![]),
            Operation::new(
                "cm",
                vec![
                    300.into(),
                    0.into(),
                    0.into(),
                    200.into(),
                    50.into(),
                    100.into(),
                ],
            ),
            Operation::new("Do", vec![Object::Name(b"Im0".to_vec())]),
            Operation::new("Q", vec![]),
            Operation::new("q", vec![]),
            Operation::new(
                "cm",
                vec![
                    300.into(),
                    0.into(),
                    0.into(),
                    200.into(),
                    500.into(),
                    100.into(),
                ],
            ),
            Operation::new("Do", vec![Object::Name(b"Im0".to_vec())]),
            Operation::new("Q", vec![]),
            Operation::new("m", vec![0.into(), 0.into()]),
            Operation::new("l", vec![100.into(), 100.into()]),
            Operation::new("S", vec![]),
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 12.into()]),
            Operation::new("Tj", vec![Object::string_literal("NoBS PDF")]),
            Operation::new("ET", vec![]),
        ],
    };
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id, "MediaBox" => vec![0.into(),0.into(),1920.into(),1080.into()],
        "CropBox" => vec![0.into(),0.into(),1920.into(),1080.into()], "Rotate" => 0, "Contents" => content_id,
        "Annots" => vec![annotation_id.into()], "Resources" => dictionary! { "XObject" => dictionary! { "Im0" => image_id }, "Font" => dictionary! { "F1" => font_id } }
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(
            dictionary! { "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1 },
        ),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    doc.save(path).unwrap();
}

#[test]
fn exports_resized_jpeg_and_preserves_document_structure() {
    let directory = TempDir::new().unwrap();
    let input = directory.path().join("input.pdf");
    let output = directory.path().join("output.pdf");
    fixture(&input);
    let original = fs::read(&input).unwrap();
    let before = pdfdoctor::inspect(&input).unwrap();
    let report = export_1080p(&input, &output, &ExportOptions { dry_run: false }).unwrap();
    let after = pdfdoctor::inspect(&output).unwrap();
    assert!(report.validation.as_ref().unwrap().passed);
    assert_eq!(report.images.modified, 1);
    assert_eq!(
        fs::read(&input).unwrap(),
        original,
        "source must never change"
    );
    assert_eq!(before.pages[0].width_pt, after.pages[0].width_pt);
    assert_eq!(before.pages[0].height_pt, after.pages[0].height_pt);
    assert_eq!(
        before.pages[0].object_counts.text_operations,
        after.pages[0].object_counts.text_operations
    );
    assert_eq!(
        before.pages[0].object_counts.vector_operations,
        after.pages[0].object_counts.vector_operations
    );
    assert_eq!(before.images[0].reference_count, 2);
    assert_eq!(after.images[0].reference_count, 2);
    assert!(after.images[0].pixel_width < before.images[0].pixel_width);
    assert_eq!(after.images[0].filters, vec!["DCTDecode"]);
    assert_eq!(
        before.images[0].placements[0].matrix,
        after.images[0].placements[0].matrix
    );
    let before_ratio = before.images[0].pixel_width as f64 / before.images[0].pixel_height as f64;
    let after_ratio = after.images[0].pixel_width as f64 / after.images[0].pixel_height as f64;
    assert!((before_ratio - after_ratio).abs() < 0.002);
}

#[test]
fn dry_run_writes_nothing_and_same_path_is_rejected() {
    let directory = TempDir::new().unwrap();
    let input = directory.path().join("input.pdf");
    let output = directory.path().join("dry.pdf");
    fixture(&input);
    let report = export_1080p(&input, &output, &ExportOptions { dry_run: true }).unwrap();
    assert!(report.dry_run);
    assert!(!output.exists());
    assert!(export_1080p(&input, &input, &ExportOptions { dry_run: false }).is_err());
}

#[test]
fn matches_jpeg_to_document_pixels_without_changing_placement_geometry() {
    let directory = TempDir::new().unwrap();
    let input = directory.path().join("input.pdf");
    let output = directory.path().join("original-resolution.pdf");
    fixture(&input);
    let before = pdfdoctor::inspect(&input).unwrap();

    let estimate =
        export_original_resolution(&input, &output, &ExportOptions { dry_run: true }, |_| {})
            .unwrap();
    assert_eq!(estimate.images.modified, 1);
    assert_eq!(estimate.image_results[0].target_pixels, Some([300, 200]));

    let report =
        export_original_resolution(&input, &output, &ExportOptions { dry_run: false }, |_| {})
            .unwrap();
    let after = pdfdoctor::inspect(&output).unwrap();
    assert!(report.validation.as_ref().unwrap().passed);
    assert_eq!(report.images.modified, 1);
    assert_eq!(after.images[0].pixel_width, 300);
    assert_eq!(after.images[0].pixel_height, 200);
    assert_eq!(
        before.images[0].placements[0].matrix,
        after.images[0].placements[0].matrix
    );
    assert_eq!(before.images[0].filters, after.images[0].filters);
    assert!(after.images[0].encoded_bytes < before.images[0].encoded_bytes);
}

#[test]
fn percentage_export_uses_detected_canvas_and_preserves_native_content() {
    let directory = TempDir::new().unwrap();
    let input = directory.path().join("input.pdf");
    fixture(&input);
    for (scale, expected) in [(100, [300, 200]), (50, [150, 100]), (10, [30, 20])] {
        let output = directory.path().join(format!("output-{scale}.pdf"));
        let before = pdfdoctor::inspect(&input).unwrap();
        let report = export_for_scale(
            &input,
            &output,
            &ExportOptions { dry_run: false },
            scale,
            |_| {},
        )
        .unwrap();
        let after = pdfdoctor::inspect(&output).unwrap();
        assert!(report.validation.as_ref().unwrap().passed);
        assert_eq!(report.document_target.scale_percent, Some(scale));
        assert_eq!(after.images[0].pixel_width, expected[0]);
        assert_eq!(after.images[0].pixel_height, expected[1]);
        assert_eq!(before.pages[0].width_pt, after.pages[0].width_pt);
        assert_eq!(before.pages[0].height_pt, after.pages[0].height_pt);
        assert_eq!(
            before.pages[0].object_counts.text_operations,
            after.pages[0].object_counts.text_operations
        );
        assert_eq!(
            before.pages[0].object_counts.vector_operations,
            after.pages[0].object_counts.vector_operations
        );
    }
}

#[test]
fn encrypted_pdf_is_rejected_before_export_and_writes_nothing() {
    let input = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/encrypted_password_nobs-test.pdf");
    let directory = TempDir::new().unwrap();
    let output = directory.path().join("must-not-exist.pdf");

    let app_error = pdfdoctor::app::inspect_pdf(&input).unwrap_err();
    assert_eq!(
        app_error.message,
        "Encrypted PDFs are not supported. Decrypt the document before optimising it."
    );

    let error = export_1080p(&input, &output, &ExportOptions { dry_run: false }).unwrap_err();
    assert!(matches!(
        error,
        ExportError::Inspection(InspectionError::EncryptedDocument)
    ));
    assert!(error
        .to_string()
        .contains("encrypted PDFs are not supported"));
    assert!(!output.exists());
}

#[test]
fn validation_rejects_zero_page_and_unparseable_outputs() {
    let directory = TempDir::new().unwrap();
    let input = directory.path().join("input.pdf");
    let empty_output = directory.path().join("empty.pdf");
    let invalid_output = directory.path().join("invalid.pdf");
    fixture(&input);

    let mut empty = Document::with_version("1.7");
    let pages = empty.add_object(
        dictionary! { "Type" => "Pages", "Kids" => Vec::<Object>::new(), "Count" => 0 },
    );
    let catalog = empty.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
    empty.trailer.set("Root", catalog);
    empty.save(&empty_output).unwrap();

    let before = pdfdoctor::inspect(&input).unwrap();
    let after = pdfdoctor::inspect(&empty_output).unwrap();
    let report = validate_export(&input, &empty_output, &before, &after, &HashSet::new()).unwrap();
    assert!(!report.passed);
    assert!(!report.output_has_pages);
    assert!(!report.page_count_preserved);
    assert!(report
        .errors
        .iter()
        .any(|error| error.contains("zero pages")));

    fs::write(&invalid_output, b"%PDF-1.7\nnot a valid PDF").unwrap();
    assert!(validate_export(&input, &invalid_output, &before, &before, &HashSet::new(),).is_err());
}
