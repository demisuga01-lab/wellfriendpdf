use lopdf::{Dictionary, Document, Object, Stream, dictionary};
use std::collections::BTreeMap;
use tempfile::tempdir;
use wellfriend_pdf_edit::{PdfEditError, SignatureEffect, native_rotation::rotate_pages};

fn fixture(path: &std::path::Path, count: usize, signed: bool) {
    let mut pdf = Document::with_version("1.7");
    let pages_id = pdf.new_object_id();
    let contents = pdf.add_object(Stream::new(
        Dictionary::new(),
        b"q 17 31 57 91 re S Q".to_vec(),
    ));
    let mut pages = Vec::new();
    for _ in 0..count {
        pages.push(Object::Reference(pdf.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id, "Contents" => contents,
        })));
    }
    pdf.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages", "Kids" => pages, "Count" => count as i64,
            "MediaBox" => vec![0.into(),0.into(),144.into(),216.into()],
            "CropBox" => vec![3.into(),5.into(),140.into(),211.into()],
            "Rotate" => 90, "Resources" => Dictionary::new(),
        }),
    );
    let root = pdf.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    pdf.trailer.set("Root", root);
    let info = pdf.add_object(dictionary! { "Title" => Object::string_literal("synthetic title"), "Producer" => Object::string_literal("native rotation fixture") });
    pdf.trailer.set("Info", info);
    if signed {
        pdf.add_object(dictionary! { "Type" => "Sig", "ByteRange" => vec![0.into(),10.into(),20.into(),30.into()], "Contents" => Object::String(vec![0;64],lopdf::StringFormat::Hexadecimal) });
    }
    pdf.compress();
    pdf.save(path).unwrap();
}

#[test]
fn bulk_rotation_preserves_inherited_boxes_source_content_and_unselected_pages() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    let output = directory.path().join("output.pdf");
    fixture(&input, 3, false);
    let source_bytes = std::fs::read(&input).unwrap();
    let before = Document::load(&input).unwrap();
    let result = rotate_pages(
        &input,
        &output,
        &BTreeMap::from([(1, 180), (3, 270)]),
        false,
    )
    .unwrap();
    let after = Document::load(&output).unwrap();
    assert_eq!(result.page_count, 3);
    assert_eq!(result.rotated_pages, 2);
    assert_eq!(result.signature_effect, SignatureEffect::None);
    assert!(result.page_content_preserved && result.non_target_objects_preserved);
    assert_eq!(source_bytes, std::fs::read(&input).unwrap());
    assert_eq!(before.get_pages(), after.get_pages());
    for (&number, &id) in &before.get_pages() {
        assert_eq!(before.get_page_contents(id), after.get_page_contents(id));
        let page = after.get_dictionary(id).unwrap();
        if number == 2 {
            assert!(!page.has(b"Rotate"));
            assert_eq!(
                before.get_object(id).unwrap(),
                after.get_object(id).unwrap()
            );
        } else {
            assert_eq!(
                page.get(b"Rotate").unwrap().as_i64().unwrap(),
                if number == 1 { 270 } else { 0 }
            );
        }
        assert!(!page.has(b"MediaBox") && !page.has(b"CropBox"));
    }
}

#[test]
fn signed_rotation_requires_explicit_core_policy_and_reports_full_rewrite() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    let output = directory.path().join("output.pdf");
    fixture(&input, 1, true);
    let rotations = BTreeMap::from([(1, 90)]);
    assert!(matches!(
        rotate_pages(&input, &output, &rotations, false),
        Err(PdfEditError::SignatureConfirmationRequired)
    ));
    assert!(!output.exists());
    let result = rotate_pages(&input, &output, &rotations, true).unwrap();
    assert_eq!(
        result.signature_effect,
        SignatureEffect::InvalidatesExistingSignatures
    );
}

#[test]
fn invalid_angles_pages_existing_outputs_and_page_bombs_produce_no_artifact() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    let output = directory.path().join("output.pdf");
    fixture(&input, 1, false);
    for rotations in [
        BTreeMap::new(),
        BTreeMap::from([(1, 45)]),
        BTreeMap::from([(2, 90)]),
    ] {
        assert!(rotate_pages(&input, &output, &rotations, false).is_err());
        assert!(!output.exists());
    }
    std::fs::write(&output, b"original output").unwrap();
    assert!(rotate_pages(&input, &output, &BTreeMap::from([(1, 90)]), false).is_err());
    assert_eq!(std::fs::read(&output).unwrap(), b"original output");
    let bomb = directory.path().join("pages.pdf");
    let result = directory.path().join("pages-out.pdf");
    fixture(&bomb, 1001, false);
    assert!(matches!(
        rotate_pages(&bomb, &result, &BTreeMap::from([(1, 90)]), false),
        Err(PdfEditError::PageLimitExceeded)
    ));
    assert!(!result.exists());
}
