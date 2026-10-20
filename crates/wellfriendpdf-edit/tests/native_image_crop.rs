use jpeg_encoder::{ColorType, Encoder};
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, Stream, dictionary};
use tempfile::tempdir;
use wellfriend_pdf_edit::{
    CropImageRequest, PdfEditError, PdfMatrix, PdfNormalizedCrop, SignatureEffect,
    TransformImageRequest, crop_image, inspect_pdf, transform_image,
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

fn assert_crop(actual: Option<PdfNormalizedCrop>, expected: PdfNormalizedCrop) {
    let actual = actual.expect("the normalized crop must be present");
    for (actual, expected) in [
        (actual.left, expected.left),
        (actual.bottom, expected.bottom),
        (actual.right, expected.right),
        (actual.top, expected.top),
    ] {
        assert!((actual - expected).abs() <= 0.000_001);
    }
}

#[test]
fn cropping_inserts_a_real_clip_path_and_remains_transformable_and_resettable() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    let cropped = directory.path().join("cropped.pdf");
    let transformed = directory.path().join("transformed.pdf");
    let reset = directory.path().join("reset.pdf");
    image_fixture(&input, false);
    let placement = inspect_pdf(&input)
        .unwrap()
        .editable_image_placements
        .remove(0);
    let requested_crop = PdfNormalizedCrop {
        left: 0.1,
        bottom: 0.2,
        right: 0.8,
        top: 0.9,
    };

    let result = crop_image(&CropImageRequest {
        input_path: input.clone(),
        output_path: cropped.clone(),
        selector: placement.selector.clone(),
        destination_crop: requested_crop,
        allow_signature_invalidation: false,
    })
    .unwrap();

    assert_eq!(result.operation, "CROP_IMAGE");
    assert_eq!(result.capability, "PDF_IMAGE_CROP");
    assert_eq!(result.source_crop, None);
    assert_eq!(result.result_crop, Some(requested_crop));
    assert!(result.validation.passed());
    assert!(result.validation.requested_crop_preserved);
    assert!(result.validation.source_image_object_preserved);
    assert!(result.validation.no_page_rasterization);
    assert_eq!(
        result.changed_objects,
        vec![placement.selector.content_stream_object]
    );

    let source = Document::load(&input).unwrap();
    let edited = Document::load(&cropped).unwrap();
    let image_id: lopdf::ObjectId = placement.selector.expected_image_object.into();
    assert_eq!(
        source.get_object(image_id).unwrap(),
        edited.get_object(image_id).unwrap()
    );
    let cropped_placement = inspect_pdf(&cropped)
        .unwrap()
        .editable_image_placements
        .remove(0);
    assert_crop(cropped_placement.crop, requested_crop);
    assert_crop(cropped_placement.selector.expected_crop, requested_crop);
    assert_eq!(cropped_placement.selector.draw_operation_index, 5);

    let transformed_matrix = PdfMatrix {
        a: 150.0,
        b: 0.0,
        c: 0.0,
        d: 110.0,
        e: 90.0,
        f: 500.0,
    };
    transform_image(&TransformImageRequest {
        input_path: cropped.clone(),
        output_path: transformed.clone(),
        selector: cropped_placement.selector,
        destination_matrix: transformed_matrix,
        allow_signature_invalidation: false,
    })
    .unwrap();
    let transformed_placement = inspect_pdf(&transformed)
        .unwrap()
        .editable_image_placements
        .remove(0);
    assert_crop(transformed_placement.crop, requested_crop);
    assert_eq!(transformed_placement.matrix, transformed_matrix);

    let reset_result = crop_image(&CropImageRequest {
        input_path: transformed,
        output_path: reset.clone(),
        selector: transformed_placement.selector,
        destination_crop: PdfNormalizedCrop {
            left: 0.0,
            bottom: 0.0,
            right: 1.0,
            top: 1.0,
        },
        allow_signature_invalidation: false,
    })
    .unwrap();
    assert_eq!(reset_result.result_crop, None);
    let reset_placement = inspect_pdf(&reset)
        .unwrap()
        .editable_image_placements
        .remove(0);
    assert_eq!(reset_placement.crop, None);
    assert_eq!(reset_placement.selector.draw_operation_index, 2);
}

#[test]
fn image_crop_rejects_an_unchanged_full_or_existing_rectangle() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    let cropped = directory.path().join("cropped.pdf");
    image_fixture(&input, false);
    let placement = inspect_pdf(&input)
        .unwrap()
        .editable_image_placements
        .remove(0);
    let full = PdfNormalizedCrop {
        left: 0.0,
        bottom: 0.0,
        right: 1.0,
        top: 1.0,
    };
    assert!(matches!(
        crop_image(&CropImageRequest {
            input_path: input.clone(),
            output_path: directory.path().join("full.pdf"),
            selector: placement.selector.clone(),
            destination_crop: full,
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::InvalidImageCrop)
    ));
    let crop = PdfNormalizedCrop {
        left: 0.1,
        bottom: 0.1,
        right: 0.9,
        top: 0.9,
    };
    crop_image(&CropImageRequest {
        input_path: input,
        output_path: cropped.clone(),
        selector: placement.selector,
        destination_crop: crop,
        allow_signature_invalidation: false,
    })
    .unwrap();
    let cropped_placement = inspect_pdf(&cropped)
        .unwrap()
        .editable_image_placements
        .remove(0);
    assert!(matches!(
        crop_image(&CropImageRequest {
            input_path: cropped,
            output_path: directory.path().join("unchanged.pdf"),
            selector: cropped_placement.selector,
            destination_crop: crop,
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::InvalidImageCrop)
    ));
}

#[test]
fn signed_pdf_image_crop_requires_explicit_invalidation_confirmation() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("signed.pdf");
    image_fixture(&input, true);
    let placement = inspect_pdf(&input)
        .unwrap()
        .editable_image_placements
        .remove(0);
    let crop = PdfNormalizedCrop {
        left: 0.15,
        bottom: 0.2,
        right: 0.85,
        top: 0.8,
    };
    let blocked = crop_image(&CropImageRequest {
        input_path: input.clone(),
        output_path: directory.path().join("blocked.pdf"),
        selector: placement.selector.clone(),
        destination_crop: crop,
        allow_signature_invalidation: false,
    });
    assert!(matches!(
        blocked,
        Err(PdfEditError::SignatureConfirmationRequired)
    ));

    let result = crop_image(&CropImageRequest {
        input_path: input,
        output_path: directory.path().join("confirmed.pdf"),
        selector: placement.selector,
        destination_crop: crop,
        allow_signature_invalidation: true,
    })
    .unwrap();
    assert_eq!(
        result.signature_effect,
        SignatureEffect::InvalidatesExistingSignatures
    );
}
