use lopdf::{
    content::{Content, Operation},
    dictionary, Document, Object, Stream,
};
use tempfile::NamedTempFile;

fn fixture() -> NamedTempFile {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let image_data = vec![7_u8; 64];
    let image1 = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => 10_000,
            "Height" => 10_000, "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8,
        },
        image_data.clone(),
    ));
    let image2 = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => 10_000,
            "Height" => 10_000, "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8,
        },
        image_data,
    ));
    let content = Content {
        operations: vec![
            Operation::new("q", vec![]),
            Operation::new(
                "cm",
                vec![
                    141.732.into(),
                    0.into(),
                    0.into(),
                    141.732.into(),
                    10.into(),
                    20.into(),
                ],
            ),
            Operation::new("Do", vec![Object::Name(b"Im1".to_vec())]),
            Operation::new("Q", vec![]),
            Operation::new("q", vec![]),
            Operation::new(
                "cm",
                vec![
                    141.732.into(),
                    0.into(),
                    0.into(),
                    141.732.into(),
                    200.into(),
                    20.into(),
                ],
            ),
            Operation::new("Do", vec![Object::Name(b"Im1".to_vec())]),
            Operation::new("Q", vec![]),
            Operation::new("q", vec![]),
            Operation::new(
                "cm",
                vec![
                    141.732.into(),
                    0.into(),
                    0.into(),
                    141.732.into(),
                    10.into(),
                    200.into(),
                ],
            ),
            Operation::new("Do", vec![Object::Name(b"Im2".to_vec())]),
            Operation::new("Q", vec![]),
        ],
    };
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id, "MediaBox" => vec![0.into(),0.into(),595.276.into(),841.89.into()], "Rotate" => 90,
        "Contents" => content_id, "Resources" => dictionary! { "XObject" => dictionary! { "Im1" => image1, "Im2" => image2 } }
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(
            dictionary! { "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1 },
        ),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    let mut file = NamedTempFile::new().unwrap();
    doc.save_to(&mut file).unwrap();
    file
}

#[test]
fn inspects_reuse_duplicates_rotation_and_dpi() {
    let file = fixture();
    let report = pdfdoctor::inspect(file.path()).unwrap();
    assert_eq!(report.pages[0].rotation_degrees, 90);
    assert_eq!(report.pages[0].object_counts.image_placements, 3);
    assert_eq!(report.images.len(), 2);
    let reused = report
        .images
        .iter()
        .find(|i| i.reference_count == 2)
        .unwrap();
    assert!(reused.same_object_reused);
    assert!((reused.placements[0].effective_dpi.unwrap() - 5080.0).abs() < 0.1);
    assert_eq!(
        report
            .images
            .iter()
            .filter(|i| i.identical_data_duplicate)
            .count(),
        1
    );
}
