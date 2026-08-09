use std::{fs, path::Path};

use jpeg_encoder::{ColorType, Encoder};
use lopdf::{
    content::{Content, Operation},
    dictionary, Document, Object, Stream,
};
use tempfile::TempDir;

fn jpeg(width: u16, height: u16, colour: [u8; 3]) -> Vec<u8> {
    let mut out = Vec::new();
    let pixels = colour.repeat(width as usize * height as usize);
    Encoder::new(&mut out, 95)
        .encode(&pixels, width, height, ColorType::Rgb)
        .unwrap();
    out
}
fn fixture(path: &Path, interleaved: bool) {
    let mut doc = Document::with_version("1.7");
    let pages = doc.new_object_id();
    let a=doc.add_object(Stream::new(dictionary!{"Type"=>"XObject","Subtype"=>"Image","Width"=>1200,"Height"=>800,"ColorSpace"=>"DeviceRGB","BitsPerComponent"=>8,"Filter"=>"DCTDecode"},jpeg(1200,800,[200,20,20])));
    let b=doc.add_object(Stream::new(dictionary!{"Type"=>"XObject","Subtype"=>"Image","Width"=>300,"Height"=>200,"ColorSpace"=>"DeviceRGB","BitsPerComponent"=>8,"Filter"=>"DCTDecode"},jpeg(300,200,[20,20,200])));
    let font =
        doc.add_object(dictionary! {"Type"=>"Font","Subtype"=>"Type1","BaseFont"=>"Helvetica"});
    let mut ops = vec![
        Operation::new("q", vec![]),
        Operation::new(
            "cm",
            vec![
                600.into(),
                0.into(),
                0.into(),
                400.into(),
                100.into(),
                100.into(),
            ],
        ),
        Operation::new("Do", vec![Object::Name(b"A".to_vec())]),
        Operation::new("Q", vec![]),
    ];
    if interleaved {
        ops.extend([
            Operation::new("m", vec![0.into(), 0.into()]),
            Operation::new("l", vec![10.into(), 10.into()]),
            Operation::new("S", vec![]),
        ]);
    }
    ops.extend([
        Operation::new("q", vec![]),
        Operation::new(
            "cm",
            vec![
                150.into(),
                0.into(),
                0.into(),
                100.into(),
                250.into(),
                200.into(),
            ],
        ),
        Operation::new("Do", vec![Object::Name(b"B".to_vec())]),
        Operation::new("Q", vec![]),
        Operation::new("m", vec![0.into(), 0.into()]),
        Operation::new("l", vec![100.into(), 100.into()]),
        Operation::new("S", vec![]),
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 12.into()]),
        Operation::new("Tj", vec![Object::string_literal("selectable")]),
        Operation::new("ET", vec![]),
    ]);
    let content = doc.add_object(Stream::new(
        dictionary! {},
        Content { operations: ops }.encode().unwrap(),
    ));
    let page=doc.add_object(dictionary!{"Type"=>"Page","Parent"=>pages,"MediaBox"=>vec![0.into(),0.into(),1920.into(),1080.into()],"Contents"=>content,"Resources"=>dictionary!{"XObject"=>dictionary!{"A"=>a,"B"=>b},"Font"=>dictionary!{"F1"=>font}}});
    doc.objects.insert(
        pages,
        Object::Dictionary(dictionary! {"Type"=>"Pages","Kids"=>vec![page.into()],"Count"=>1}),
    );
    let catalog = doc.add_object(dictionary! {"Type"=>"Catalog","Pages"=>pages});
    doc.trailer.set("Root", catalog);
    doc.save(path).unwrap();
}

#[test]
fn merges_ordered_rasters_and_preserves_text_vectors_and_page() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("in.pdf");
    let output = dir.path().join("out.pdf");
    fixture(&input, false);
    let source = fs::read(&input).unwrap();
    let before = pdfdoctor::inspect(&input).unwrap();
    let report = pdfdoctor::raster_merge::merge_1080p(&input, &output, false).unwrap();
    let after = pdfdoctor::inspect(&output).unwrap();
    assert_eq!(report.pages_merged, 1);
    assert_eq!(after.pages[0].object_counts.image_placements, 1);
    assert_eq!(
        before.pages[0].object_counts.text_operations,
        after.pages[0].object_counts.text_operations
    );
    assert_eq!(
        before.pages[0].object_counts.vector_operations,
        after.pages[0].object_counts.vector_operations
    );
    assert_eq!(
        (before.pages[0].width_pt, before.pages[0].height_pt),
        (after.pages[0].width_pt, after.pages[0].height_pt)
    );
    assert_eq!(source, fs::read(&input).unwrap());
}

#[test]
fn interleaved_content_is_skipped_and_dry_run_writes_nothing() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("in.pdf");
    let output = dir.path().join("out.pdf");
    fixture(&input, true);
    let report = pdfdoctor::raster_merge::merge_1080p(&input, &output, true).unwrap();
    assert_eq!(report.pages_merged, 0);
    assert_eq!(
        report.pages[0].skipped_reason.as_deref(),
        Some("unsafe_content_order")
    );
    assert!(!output.exists());
}
