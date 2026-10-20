use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, Stream, dictionary};
use std::fs;
use tempfile::tempdir;
use wellfriend_pdf_edit::{
    EditPageGeometryRequest, PdfBox, PdfEditError, SignatureEffect, edit_page_geometry, inspect_pdf,
};

fn page_geometry_fixture(path: &std::path::Path, signed: bool) {
    let mut document = Document::with_version("1.7");
    let pages_id = document.new_object_id();
    let content_id = document.add_object(Stream::new(
        Dictionary::new(),
        Content {
            operations: vec![
                Operation::new("q", vec![]),
                Operation::new("re", vec![72.into(), 72.into(), 100.into(), 40.into()]),
                Operation::new("S", vec![]),
                Operation::new("Q", vec![]),
            ],
        }
        .encode()
        .unwrap(),
    ));
    let page_id = document.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
        "Resources" => Dictionary::new(),
    });
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![Object::Reference(page_id)],
            "Count" => 1,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "CropBox" => vec![18.into(), 24.into(), 594.into(), 768.into()],
            "Rotate" => 0,
            "Resources" => Dictionary::new(),
        }),
    );
    let catalog_id = document.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    document.trailer.set("Root", catalog_id);
    if signed {
        document.add_object(dictionary! {
            "Type" => "Sig",
            "ByteRange" => vec![0.into(), 10.into(), 20.into(), 30.into()],
            "Contents" => Object::String(vec![0; 64], lopdf::StringFormat::Hexadecimal),
        });
    }
    document.compress();
    document.save(path).unwrap();
}

#[test]
fn page_geometry_edits_the_real_page_dictionary_without_rasterizing() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    let output = directory.path().join("output.pdf");
    page_geometry_fixture(&input, false);
    let inspection = inspect_pdf(&input).unwrap();
    assert_eq!(inspection.editable_pages.len(), 1);
    assert_eq!(inspection.unsupported_page_geometries, 0);
    let page = inspection.editable_pages[0].clone();
    let source = Document::load(&input).unwrap();
    let source_page_id = source.get_pages()[&1];
    let source_contents = source.get_page_contents(source_page_id);

    let result = edit_page_geometry(&EditPageGeometryRequest {
        input_path: input,
        output_path: output.clone(),
        selector: page.selector,
        media_box: PdfBox {
            left: 0.0,
            bottom: 0.0,
            right: 640.0,
            top: 820.0,
        },
        crop_box: PdfBox {
            left: 20.0,
            bottom: 30.0,
            right: 620.0,
            top: 790.0,
        },
        rotation: 90,
        allow_signature_invalidation: false,
    })
    .unwrap();

    assert!(result.validation.passed());
    assert!(result.validation.no_page_rasterization);
    assert_eq!(result.capability, "PDF_PAGE_GEOMETRY_EDIT");
    assert_eq!(result.page_object.number, source_page_id.0);
    assert_eq!(result.result_rotation, 90);
    assert_eq!(result.source_object_count, result.output_object_count);
    assert_eq!(result.signature_effect, SignatureEffect::None);

    let edited = Document::load(&output).unwrap();
    let edited_page_id = edited.get_pages()[&1];
    assert_eq!(edited_page_id, source_page_id);
    assert_eq!(edited.get_page_contents(edited_page_id), source_contents);
    let edited_page = edited.get_dictionary(edited_page_id).unwrap();
    assert!(edited_page.has(b"MediaBox"));
    assert!(edited_page.has(b"CropBox"));
    assert_eq!(edited_page.get(b"Rotate").unwrap().as_i64().unwrap(), 90);
    assert!(edited.get_page_images(edited_page_id).unwrap().is_empty());
}

#[test]
fn page_geometry_refuses_stale_invalid_and_signed_edits() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    page_geometry_fixture(&input, false);
    let page = inspect_pdf(&input).unwrap().editable_pages[0].clone();
    let mut stale = page.selector.clone();
    stale.expected_rotation = 90;
    let error = edit_page_geometry(&EditPageGeometryRequest {
        input_path: input.clone(),
        output_path: directory.path().join("stale.pdf"),
        selector: stale,
        media_box: page.media_box,
        crop_box: page.crop_box,
        rotation: 90,
        allow_signature_invalidation: false,
    })
    .unwrap_err();
    assert!(matches!(error, PdfEditError::StalePageGeometry));

    for (index, (crop_box, rotation)) in [
        (
            PdfBox {
                left: -1.0,
                bottom: 0.0,
                right: 612.0,
                top: 792.0,
            },
            90,
        ),
        (
            PdfBox {
                left: 10.0,
                bottom: 10.0,
                right: 10.5,
                top: 700.0,
            },
            90,
        ),
        (page.crop_box, 45),
    ]
    .into_iter()
    .enumerate()
    {
        let error = edit_page_geometry(&EditPageGeometryRequest {
            input_path: input.clone(),
            output_path: directory.path().join(format!("invalid-{index}.pdf")),
            selector: page.selector.clone(),
            media_box: page.media_box,
            crop_box,
            rotation,
            allow_signature_invalidation: false,
        })
        .unwrap_err();
        assert!(matches!(error, PdfEditError::InvalidPageGeometry));
    }

    let signed = directory.path().join("signed.pdf");
    let signed_output = directory.path().join("signed-output.pdf");
    page_geometry_fixture(&signed, true);
    let signed_page = inspect_pdf(&signed).unwrap().editable_pages[0].clone();
    let request = EditPageGeometryRequest {
        input_path: signed,
        output_path: signed_output.clone(),
        selector: signed_page.selector,
        media_box: signed_page.media_box,
        crop_box: signed_page.crop_box,
        rotation: 180,
        allow_signature_invalidation: false,
    };
    let error = edit_page_geometry(&request).unwrap_err();
    assert!(matches!(error, PdfEditError::SignatureConfirmationRequired));
    assert!(!signed_output.exists());
    let result = edit_page_geometry(&EditPageGeometryRequest {
        allow_signature_invalidation: true,
        ..request
    })
    .unwrap();
    assert_eq!(
        result.signature_effect,
        SignatureEffect::InvalidatesExistingSignatures
    );
}

#[test]
fn identical_page_geometry_commands_are_byte_deterministic() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    let output_a = directory.path().join("a.pdf");
    let output_b = directory.path().join("b.pdf");
    page_geometry_fixture(&input, false);
    let page = inspect_pdf(&input).unwrap().editable_pages[0].clone();
    let request = |output_path| EditPageGeometryRequest {
        input_path: input.clone(),
        output_path,
        selector: page.selector.clone(),
        media_box: page.media_box,
        crop_box: page.crop_box,
        rotation: 270,
        allow_signature_invalidation: false,
    };
    let result_a = edit_page_geometry(&request(output_a.clone())).unwrap();
    let result_b = edit_page_geometry(&request(output_b.clone())).unwrap();
    assert_eq!(result_a.output_sha256, result_b.output_sha256);
    assert_eq!(fs::read(output_a).unwrap(), fs::read(output_b).unwrap());
}
