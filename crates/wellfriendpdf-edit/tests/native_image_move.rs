use jpeg_encoder::{ColorType, Encoder};
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, Stream, dictionary};
use tempfile::tempdir;
use wellfriend_pdf_edit::{
    MoveImageRequest, PdfEditError, SignatureEffect, inspect_pdf, move_image,
};

fn jpeg() -> Vec<u8> {
    let pixels = [30_u8, 120, 210]
        .into_iter()
        .cycle()
        .take(4 * 3 * 3)
        .collect::<Vec<_>>();
    let mut encoded = Vec::new();
    Encoder::new(&mut encoded, 90)
        .encode(&pixels, 4, 3, ColorType::Rgb)
        .unwrap();
    encoded
}

fn image_fixture(path: &std::path::Path, signed: bool, isolated: bool) {
    let mut document = Document::with_version("1.7");
    let pages_id = document.new_object_id();
    let image_id = document.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => 4,
            "Height" => 3,
            "ColorSpace" => "DeviceRGB",
            "BitsPerComponent" => 8,
            "Filter" => "DCTDecode",
        },
        jpeg(),
    ));
    let mut operations = vec![
        Operation::new("q", vec![]),
        Operation::new(
            "cm",
            vec![
                120.into(),
                0.into(),
                0.into(),
                90.into(),
                72.into(),
                620.into(),
            ],
        ),
    ];
    if !isolated {
        operations.push(Operation::new("w", vec![1.into()]));
    }
    operations.push(Operation::new("Do", vec![Object::Name(b"Photo".to_vec())]));
    operations.push(Operation::new("Q", vec![]));
    let content_id = document.add_object(Stream::new(
        Dictionary::new(),
        Content { operations }.encode().unwrap(),
    ));
    let page_id = document.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        "Resources" => dictionary! {
            "XObject" => dictionary! { "Photo" => image_id },
        },
    });
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        }),
    );
    let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    document.trailer.set("Root", catalog_id);
    if signed {
        document.add_object(dictionary! {
            "Type" => "Sig",
            "ByteRange" => vec![0.into(), 10.into(), 20.into(), 30.into()],
            "Contents" => Object::String(vec![0; 64], lopdf::StringFormat::Hexadecimal),
        });
    }
    document.save(path).unwrap();
}

#[test]
fn moving_an_image_changes_its_real_content_matrix_without_rasterizing_the_page() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    let output = directory.path().join("output.pdf");
    image_fixture(&input, false, true);

    let inspection = inspect_pdf(&input).unwrap();
    assert_eq!(inspection.editable_image_placements.len(), 1);
    assert_eq!(inspection.unsupported_image_placements, 0);
    let placement = &inspection.editable_image_placements[0];
    assert_eq!(placement.matrix.e, 72.0);
    assert_eq!(placement.matrix.f, 620.0);

    let result = move_image(&MoveImageRequest {
        input_path: input.clone(),
        output_path: output.clone(),
        selector: placement.selector.clone(),
        destination_x: 144.0,
        destination_y: 500.0,
        allow_signature_invalidation: false,
    })
    .unwrap();

    assert_eq!(result.operation, "MOVE_IMAGE");
    assert_eq!(result.capability, "PDF_OBJECT_MOVE");
    assert_eq!(result.result_matrix.e, 144.0);
    assert_eq!(result.result_matrix.f, 500.0);
    assert_eq!(result.result_matrix.a, result.source_matrix.a);
    assert_eq!(result.result_matrix.d, result.source_matrix.d);
    assert!(result.validation.passed());
    assert!(result.validation.source_image_object_preserved);
    assert!(result.validation.no_page_rasterization);
    assert_eq!(
        result.changed_objects,
        vec![placement.selector.content_stream_object]
    );

    let source = Document::load(&input).unwrap();
    let edited = Document::load(&output).unwrap();
    let image_id: lopdf::ObjectId = placement.selector.expected_image_object.into();
    assert_eq!(
        source.get_object(image_id).unwrap(),
        edited.get_object(image_id).unwrap()
    );
    let content_id: lopdf::ObjectId = placement.selector.content_stream_object.into();
    let stream = edited.get_object(content_id).unwrap().as_stream().unwrap();
    let content = Content::decode(&stream.decompressed_content().unwrap()).unwrap();
    let matrix = &content.operations[placement.selector.matrix_operation_index];
    assert_eq!(matrix.operator, "cm");
    assert_eq!(matrix.operands[4].as_i64().unwrap(), 144);
    assert_eq!(matrix.operands[5].as_i64().unwrap(), 500);
    assert_eq!(
        content.operations[placement.selector.draw_operation_index].operator,
        "Do"
    );
}

#[test]
fn ambiguous_placement_is_read_only_and_stale_or_unchanged_moves_fail_closed() {
    let directory = tempdir().unwrap();
    let unsafe_input = directory.path().join("unsafe.pdf");
    image_fixture(&unsafe_input, false, false);
    let unsafe_inspection = inspect_pdf(&unsafe_input).unwrap();
    assert!(unsafe_inspection.editable_image_placements.is_empty());
    assert_eq!(unsafe_inspection.unsupported_image_placements, 1);

    let input = directory.path().join("input.pdf");
    image_fixture(&input, false, true);
    let placement = inspect_pdf(&input)
        .unwrap()
        .editable_image_placements
        .remove(0);
    let unchanged = move_image(&MoveImageRequest {
        input_path: input.clone(),
        output_path: directory.path().join("unchanged.pdf"),
        selector: placement.selector.clone(),
        destination_x: placement.matrix.e,
        destination_y: placement.matrix.f,
        allow_signature_invalidation: false,
    });
    assert!(matches!(
        unchanged,
        Err(PdfEditError::InvalidImagePlacement)
    ));

    let mut stale_selector = placement.selector;
    stale_selector.expected_matrix.e += 1.0;
    let stale = move_image(&MoveImageRequest {
        input_path: input,
        output_path: directory.path().join("stale.pdf"),
        selector: stale_selector,
        destination_x: 100.0,
        destination_y: 500.0,
        allow_signature_invalidation: false,
    });
    assert!(matches!(stale, Err(PdfEditError::StaleImagePlacement)));
}

#[test]
fn signed_pdf_image_move_requires_explicit_invalidation_confirmation() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("signed.pdf");
    image_fixture(&input, true, true);
    let placement = inspect_pdf(&input)
        .unwrap()
        .editable_image_placements
        .remove(0);
    let blocked = move_image(&MoveImageRequest {
        input_path: input.clone(),
        output_path: directory.path().join("blocked.pdf"),
        selector: placement.selector.clone(),
        destination_x: 100.0,
        destination_y: 500.0,
        allow_signature_invalidation: false,
    });
    assert!(matches!(
        blocked,
        Err(PdfEditError::SignatureConfirmationRequired)
    ));

    let result = move_image(&MoveImageRequest {
        input_path: input,
        output_path: directory.path().join("confirmed.pdf"),
        selector: placement.selector,
        destination_x: 100.0,
        destination_y: 500.0,
        allow_signature_invalidation: true,
    })
    .unwrap();
    assert_eq!(
        result.signature_effect,
        SignatureEffect::InvalidatesExistingSignatures
    );
}
