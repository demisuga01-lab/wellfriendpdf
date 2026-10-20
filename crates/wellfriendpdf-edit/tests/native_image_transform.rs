use jpeg_encoder::{ColorType, Encoder};
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, Stream, dictionary};
use tempfile::tempdir;
use wellfriend_pdf_edit::{
    PdfEditError, PdfMatrix, SignatureEffect, TransformImageRequest, inspect_pdf, transform_image,
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

fn image_fixture(path: &std::path::Path, signed: bool) {
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
    let content_id = document.add_object(Stream::new(
        Dictionary::new(),
        Content {
            operations: vec![
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
                Operation::new("Do", vec![Object::Name(b"Photo".to_vec())]),
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
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        "Resources" => dictionary! { "XObject" => dictionary! { "Photo" => image_id } },
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

fn number(value: &Object) -> f64 {
    match value {
        Object::Integer(value) => *value as f64,
        Object::Real(value) => f64::from(*value),
        _ => panic!("matrix operand must remain numeric"),
    }
}

#[test]
fn transforming_an_image_resizes_and_rotates_its_real_matrix_without_rasterizing() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    let output = directory.path().join("output.pdf");
    image_fixture(&input, false);
    let placement = inspect_pdf(&input)
        .unwrap()
        .editable_image_placements
        .remove(0);
    let destination_matrix = PdfMatrix {
        a: 0.0,
        b: 180.0,
        c: -60.0,
        d: 0.0,
        e: 144.0,
        f: 500.0,
    };

    let result = transform_image(&TransformImageRequest {
        input_path: input.clone(),
        output_path: output.clone(),
        selector: placement.selector.clone(),
        destination_matrix,
        allow_signature_invalidation: false,
    })
    .unwrap();

    assert_eq!(result.operation, "TRANSFORM_IMAGE");
    assert_eq!(result.capability, "PDF_IMAGE_TRANSFORM");
    assert_eq!(result.source_matrix, placement.matrix);
    assert_eq!(result.result_matrix, destination_matrix);
    assert!(result.validation.passed());
    assert!(result.validation.requested_transform_preserved);
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
    let actual = matrix.operands.iter().map(number).collect::<Vec<_>>();
    assert_eq!(actual, vec![0.0, 180.0, -60.0, 0.0, 144.0, 500.0]);
    assert_eq!(
        content.operations[placement.selector.draw_operation_index].operator,
        "Do"
    );
}

#[test]
fn unchanged_or_degenerate_image_transforms_fail_closed() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    image_fixture(&input, false);
    let placement = inspect_pdf(&input)
        .unwrap()
        .editable_image_placements
        .remove(0);

    for (name, destination_matrix) in [
        ("unchanged", placement.matrix),
        (
            "degenerate",
            PdfMatrix {
                a: 0.0,
                b: 0.0,
                c: 0.0,
                d: 0.0,
                e: 72.0,
                f: 620.0,
            },
        ),
    ] {
        let result = transform_image(&TransformImageRequest {
            input_path: input.clone(),
            output_path: directory.path().join(format!("{name}.pdf")),
            selector: placement.selector.clone(),
            destination_matrix,
            allow_signature_invalidation: false,
        });
        assert!(matches!(result, Err(PdfEditError::InvalidImagePlacement)));
    }
}

#[test]
fn signed_pdf_image_transform_requires_explicit_invalidation_confirmation() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("signed.pdf");
    image_fixture(&input, true);
    let placement = inspect_pdf(&input)
        .unwrap()
        .editable_image_placements
        .remove(0);
    let destination_matrix = PdfMatrix {
        a: 150.0,
        d: 120.0,
        ..placement.matrix
    };
    let blocked = transform_image(&TransformImageRequest {
        input_path: input.clone(),
        output_path: directory.path().join("blocked.pdf"),
        selector: placement.selector.clone(),
        destination_matrix,
        allow_signature_invalidation: false,
    });
    assert!(matches!(
        blocked,
        Err(PdfEditError::SignatureConfirmationRequired)
    ));

    let result = transform_image(&TransformImageRequest {
        input_path: input,
        output_path: directory.path().join("confirmed.pdf"),
        selector: placement.selector,
        destination_matrix,
        allow_signature_invalidation: true,
    })
    .unwrap();
    assert_eq!(
        result.signature_effect,
        SignatureEffect::InvalidatesExistingSignatures
    );
}
