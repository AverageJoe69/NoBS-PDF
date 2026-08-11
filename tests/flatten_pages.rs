use std::path::Path;

use jpeg_encoder::{ColorType, Encoder};
use lopdf::{
    content::{Content, Operation},
    dictionary, Document, Object, Stream,
};
use tempfile::TempDir;

fn pdfium_library() -> std::path::PathBuf {
    let relative = if cfg!(target_os = "windows") {
        "vendor/pdfium/bin/pdfium.dll"
    } else {
        "vendor/pdfium/lib/libpdfium.dylib"
    };
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn fixture(path: &Path) {
    let mut doc = Document::with_version("1.7");
    // Deliberately unreachable legacy payload: flattening must prune obsolete
    // resources instead of carrying historical document bloat forward.
    doc.add_object(Stream::new(dictionary! {}, vec![0x5a; 100_000]));
    let pages = doc.new_object_id();
    let font =
        doc.add_object(dictionary! {"Type"=>"Font","Subtype"=>"Type1","BaseFont"=>"Helvetica"});
    let mut jpeg = Vec::new();
    Encoder::new(&mut jpeg, 90)
        .encode(&[220, 220, 220], 1, 1, ColorType::Rgb)
        .unwrap();
    let image = doc.add_object(Stream::new(dictionary!{"Type"=>"XObject","Subtype"=>"Image","Width"=>1,"Height"=>1,"ColorSpace"=>"DeviceRGB","BitsPerComponent"=>8,"Filter"=>"DCTDecode"},jpeg));
    let annotation=doc.add_object(dictionary!{"Type"=>"Annot","Subtype"=>"Link","Rect"=>vec![10.into(),10.into(),100.into(),30.into()]});
    let content = Content {
        operations: vec![
            Operation::new("m", vec![0.into(), 0.into()]),
            Operation::new("l", vec![200.into(), 100.into()]),
            Operation::new("S", vec![]),
            Operation::new("q", vec![]),
            Operation::new(
                "cm",
                vec![
                    1920.into(),
                    0.into(),
                    0.into(),
                    1080.into(),
                    0.into(),
                    0.into(),
                ],
            ),
            Operation::new("Do", vec![Object::Name(b"BG".to_vec())]),
            Operation::new("Q", vec![]),
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 24.into()]),
            Operation::new(
                "Tm",
                vec![
                    1.into(),
                    0.into(),
                    0.into(),
                    1.into(),
                    100.into(),
                    500.into(),
                ],
            ),
            Operation::new("Tj", vec![Object::string_literal("Flatten me")]),
            Operation::new("ET", vec![]),
        ],
    };
    let stream = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
    let page=doc.add_object(dictionary!{"Type"=>"Page","Parent"=>pages,"MediaBox"=>vec![0.into(),0.into(),1920.into(),1080.into()],"CropBox"=>vec![0.into(),0.into(),1920.into(),1080.into()],"Contents"=>stream,"Annots"=>vec![annotation.into()],"Resources"=>dictionary!{"Font"=>dictionary!{"F1"=>font},"XObject"=>dictionary!{"BG"=>image}}});
    doc.objects.insert(
        pages,
        Object::Dictionary(dictionary! {"Type"=>"Pages","Kids"=>vec![page.into()],"Count"=>1}),
    );
    let catalog = doc.add_object(dictionary! {"Type"=>"Catalog","Pages"=>pages});
    doc.trailer.set("Root", catalog);
    doc.save(path).unwrap();
}

#[test]
fn dry_run_writes_nothing() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("in.pdf");
    let output = dir.path().join("out.pdf");
    fixture(&input);
    let report = pdfdoctor::flatten_pages::flatten_1080p(&input, &output, true, None).unwrap();
    assert!(report.dry_run);
    assert!(!output.exists());
}

#[test]
fn full_page_export_flattens_content_and_validates() {
    let library = pdfium_library();
    if !library.exists() {
        return;
    }
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("in.pdf");
    let output = dir.path().join("out.pdf");
    fixture(&input);
    let before = std::fs::read(&input).unwrap();
    let report =
        pdfdoctor::flatten_pages::flatten_1080p(&input, &output, false, Some(&library)).unwrap();
    let inspected = pdfdoctor::inspect(&output).unwrap();
    assert!(report.validation.unwrap().passed);
    assert_eq!(inspected.pages.len(), 1);
    assert_eq!(inspected.pages[0].object_counts.image_placements, 1);
    assert_eq!(inspected.pages[0].object_counts.text_operations, 0);
    assert_eq!(inspected.pages[0].object_counts.vector_operations, 0);
    assert_eq!(std::fs::read(input).unwrap(), before);
}

#[test]
fn graphics_flatten_preserves_selectable_text() {
    let library = pdfium_library();
    if !library.exists() {
        return;
    }
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("in.pdf");
    let output = dir.path().join("out.pdf");
    fixture(&input);
    let before = pdfdoctor::inspect(&input).unwrap();
    let report = pdfdoctor::flatten_pages::flatten_pages_preserve_text(
        &input,
        &output,
        false,
        Some(&library),
        1920,
        "1080p",
    )
    .unwrap();
    let after = pdfdoctor::inspect(&output).unwrap();
    assert_eq!(report.mode, "graphics_raster_text_preserved");
    assert!(report.validation.unwrap().passed);
    assert_eq!(after.pages[0].object_counts.image_placements, 1);
    assert_eq!(
        after.pages[0].object_counts.text_operations,
        before.pages[0].object_counts.text_operations
    );
    assert_eq!(after.pages[0].object_counts.vector_operations, 0);

    let output_document = Document::load(&output).unwrap();
    let page_id = *output_document.get_pages().values().next().unwrap();
    let page = output_document.get_dictionary(page_id).unwrap();
    let resources = match page.get(b"Resources").unwrap() {
        Object::Dictionary(resources) => resources,
        Object::Reference(id) => output_document.get_dictionary(*id).unwrap(),
        _ => panic!("unexpected Resources object"),
    };
    let xobjects = match resources.get(b"XObject").unwrap() {
        Object::Dictionary(xobjects) => xobjects,
        Object::Reference(id) => output_document.get_dictionary(*id).unwrap(),
        _ => panic!("unexpected XObject object"),
    };
    assert_eq!(xobjects.len(), 1);
    assert!(xobjects.has(b"NoBSFlattenedPage"));
}

#[test]
fn test_validation_failure_blocks_output() {
    let library = pdfium_library();
    if !library.exists() {
        return;
    }
    let dir = TempDir::new().unwrap();
    let padded = dir.path().join("padded.pdf");
    let input = dir.path().join("small.pdf");
    let output = dir.path().join("out.pdf");
    fixture(&padded);
    let mut document = Document::load(&padded).unwrap();
    document.prune_objects();
    document.save(&input).unwrap();
    let error = pdfdoctor::flatten_pages::flatten_1080p(&input, &output, false, Some(&library))
        .unwrap_err();
    assert!(error.to_string().contains("larger than the source"));
    assert!(!output.exists());
}

#[test]
fn desktop_scale_path_uses_foreground_text_hybrid_when_safe() {
    let library = pdfium_library();
    if !library.exists() {
        return;
    }
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("in.pdf");
    let output = dir.path().join("out.pdf");
    fixture(&input);
    let result = pdfdoctor::app::optimise_pdf_scale_with_options(
        &input,
        100,
        &output,
        &pdfdoctor::app::CancellationToken::default(),
        |_| {},
    )
    .unwrap();
    assert_eq!(result.mode, "scale_100");
    assert_eq!(result.scale_percent, Some(100));
    assert!(result.validation_passed);
    assert!(result.text_preserved);
    assert!(!result.vectors_preserved);
    assert!(output.exists());
}
