use std::{fs, panic::AssertUnwindSafe};

fn assert_inspection_does_not_panic(name: &str, bytes: &[u8]) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(format!("{name}.pdf"));
    fs::write(&path, bytes).unwrap();
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| pdfdoctor::inspect(&path)));
    assert!(result.is_ok(), "inspection panicked for corpus case {name}");
}

fn pdf_with_extra_object(object: &[u8]) -> Vec<u8> {
    let mut pdf = b"%PDF-1.7\n\
1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n\
2 0 obj\n<< /Type /Pages /Kids [] /Count 0 >>\nendobj\n\
3 0 obj\n"
        .to_vec();
    pdf.extend_from_slice(object);
    pdf.extend_from_slice(b"\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF\n");
    pdf
}

#[test]
fn benign_malformed_corpus_never_panics() {
    let cases: &[(&str, &[u8])] = &[
        ("header_only", b"%PDF-1.7\n"),
        ("truncated_object", b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog"),
        (
            "invalid_reference",
            b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog /Pages 999999 0 R >>\nendobj\n%%EOF\n",
        ),
        (
            "malformed_stream_length",
            b"%PDF-1.7\n1 0 obj\n<< /Length 999999999 >>\nstream\nshort\nendstream\nendobj\n%%EOF\n",
        ),
        (
            "invalid_xref",
            b"%PDF-1.7\n1 0 obj\nnull\nendobj\nxref\n0 2\nnot-an-xref\ntrailer\n<< /Root 1 0 R >>\n%%EOF\n",
        ),
        (
            "encrypted_without_dictionary",
            b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog >>\nendobj\ntrailer\n<< /Root 1 0 R /Encrypt 99 0 R >>\n%%EOF\n",
        ),
    ];

    for (name, bytes) in cases {
        assert_inspection_does_not_panic(name, bytes);
    }

    assert_inspection_does_not_panic(
        "corrupt_font_stream",
        &pdf_with_extra_object(
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Broken /Length 4 >>\nstream\nzzzz\nendstream",
        ),
    );
    assert_inspection_does_not_panic(
        "corrupt_image_stream",
        &pdf_with_extra_object(
            b"<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length 4 >>\nstream\nzzzz\nendstream",
        ),
    );
    assert_inspection_does_not_panic(
        "unusual_filter",
        &pdf_with_extra_object(
            b"<< /Length 4 /Filter [/ASCII85Decode /FlateDecode] >>\nstream\n!!!!\nendstream",
        ),
    );
}

#[test]
fn deeply_nested_direct_object_is_rejected_without_stack_overflow() {
    // This is intentionally only modestly beyond lopdf 0.42's limit of 100,
    // exercising the security boundary without constructing an exploit-sized payload.
    let depth = 128;
    let mut nested = Vec::with_capacity(depth * 2 + 1);
    nested.extend(std::iter::repeat_n(b'[', depth));
    nested.push(b'0');
    nested.extend(std::iter::repeat_n(b']', depth));
    let pdf = pdf_with_extra_object(&nested);

    let parsed = std::panic::catch_unwind(AssertUnwindSafe(|| lopdf::Document::load_mem(&pdf)));
    assert!(parsed.is_ok(), "deeply nested object caused a panic");
    assert!(parsed.unwrap().is_err(), "object beyond the nesting limit was accepted");
}
