use jpeg_encoder::{ColorType, Encoder};
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, Stream, dictionary};
use tempfile::tempdir;
use wellfriend_pdf_edit::{
    CropImageRequest, PdfEditError, PdfMatrix, PdfNormalizedCrop, SetImageOpacityRequest,
    SignatureEffect, TransformImageRequest, crop_image, inspect_pdf, set_image_opacity,
    transform_image,
};

fn jpeg() -> Vec<u8> {
    let pixels = [18_u8, 96, 188]
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

fn image_fixture(path: &std::path::Path, signed: bool, foreign_opacity: bool) {
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
    let foreign_state_id = foreign_opacity.then(|| {
        document.add_object(dictionary! {
            "Type" => "ExtGState",
            "ca" => 0.5,
            "CA" => 0.5,
        })
    });
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
    if foreign_opacity {
        operations.push(Operation::new(
            "gs",
            vec![Object::Name(b"ForeignAlpha".to_vec())],
        ));
    }
    operations.extend([
        Operation::new("Do", vec![Object::Name(b"Photo".to_vec())]),
        Operation::new("Q", vec![]),
    ]);
    let content_id = document.add_object(Stream::new(
        Dictionary::new(),
        Content { operations }.encode().unwrap(),
    ));
    let mut resources = dictionary! { "XObject" => dictionary! { "Photo" => image_id } };
    if let Some(foreign_state_id) = foreign_state_id {
        resources.set(
            "ExtGState",
            dictionary! { "ForeignAlpha" => foreign_state_id },
        );
    }
    let page_id = document.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        "Resources" => resources,
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
fn opacity_uses_a_real_owned_ext_g_state_and_remains_crop_and_transform_editable() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    let translucent = directory.path().join("translucent.pdf");
    let cropped = directory.path().join("cropped.pdf");
    let transformed = directory.path().join("transformed.pdf");
    image_fixture(&input, false, false);
    let placement = inspect_pdf(&input)
        .unwrap()
        .editable_image_placements
        .remove(0);

    let result = set_image_opacity(&SetImageOpacityRequest {
        input_path: input,
        output_path: translucent.clone(),
        selector: placement.selector,
        destination_opacity: 0.42,
        allow_signature_invalidation: false,
    })
    .unwrap();
    assert_eq!(result.operation, "SET_IMAGE_OPACITY");
    assert_eq!(result.capability, "PDF_IMAGE_OPACITY");
    assert_eq!(result.source_opacity, 1.0);
    assert!((result.result_opacity - 0.42).abs() < 0.000_001);
    assert!(result.validation.passed());
    assert!(result.validation.owned_ext_g_state_valid);
    assert!(result.validation.source_image_object_preserved);
    assert!(result.validation.no_page_rasterization);

    let opacity_placement = inspect_pdf(&translucent)
        .unwrap()
        .editable_image_placements
        .remove(0);
    assert!((opacity_placement.opacity - 0.42).abs() < 0.000_001);
    let opacity = opacity_placement
        .selector
        .expected_opacity
        .as_ref()
        .expect("owned opacity state must be stale-bound");
    assert!(opacity.resource_name.starts_with("WFImgAlpha"));
    assert_eq!(opacity_placement.selector.draw_operation_index, 3);

    let crop = PdfNormalizedCrop {
        left: 0.1,
        bottom: 0.2,
        right: 0.85,
        top: 0.9,
    };
    crop_image(&CropImageRequest {
        input_path: translucent,
        output_path: cropped.clone(),
        selector: opacity_placement.selector,
        destination_crop: crop,
        allow_signature_invalidation: false,
    })
    .unwrap();
    let cropped_placement = inspect_pdf(&cropped)
        .unwrap()
        .editable_image_placements
        .remove(0);
    assert_crop(cropped_placement.crop, crop);
    assert!((cropped_placement.opacity - 0.42).abs() < 0.000_001);
    assert_eq!(cropped_placement.selector.draw_operation_index, 6);

    let matrix = PdfMatrix {
        a: 150.0,
        b: 0.0,
        c: 0.0,
        d: 110.0,
        e: 90.0,
        f: 500.0,
    };
    transform_image(&TransformImageRequest {
        input_path: cropped,
        output_path: transformed.clone(),
        selector: cropped_placement.selector,
        destination_matrix: matrix,
        allow_signature_invalidation: false,
    })
    .unwrap();
    let transformed_placement = inspect_pdf(&transformed)
        .unwrap()
        .editable_image_placements
        .remove(0);
    assert_eq!(transformed_placement.matrix, matrix);
    assert_crop(transformed_placement.crop, crop);
    assert!((transformed_placement.opacity - 0.42).abs() < 0.000_001);
}

#[test]
fn reset_removes_the_owned_ext_g_state_and_foreign_graphics_state_is_read_only() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    let translucent = directory.path().join("translucent.pdf");
    let reset = directory.path().join("reset.pdf");
    image_fixture(&input, false, false);
    let placement = inspect_pdf(&input)
        .unwrap()
        .editable_image_placements
        .remove(0);
    set_image_opacity(&SetImageOpacityRequest {
        input_path: input,
        output_path: translucent.clone(),
        selector: placement.selector,
        destination_opacity: 0.6,
        allow_signature_invalidation: false,
    })
    .unwrap();
    let opacity_placement = inspect_pdf(&translucent)
        .unwrap()
        .editable_image_placements
        .remove(0);
    let opacity_object = opacity_placement
        .selector
        .expected_opacity
        .as_ref()
        .unwrap()
        .ext_g_state_object;
    let reset_result = set_image_opacity(&SetImageOpacityRequest {
        input_path: translucent,
        output_path: reset.clone(),
        selector: opacity_placement.selector,
        destination_opacity: 1.0,
        allow_signature_invalidation: false,
    })
    .unwrap();
    assert_eq!(reset_result.result_opacity_state, None);
    assert!(reset_result.validation.owned_ext_g_state_valid);
    let reset_document = Document::load(&reset).unwrap();
    assert!(
        !reset_document
            .objects
            .contains_key(&lopdf::ObjectId::from(opacity_object))
    );
    let reset_placement = inspect_pdf(&reset)
        .unwrap()
        .editable_image_placements
        .remove(0);
    assert_eq!(reset_placement.opacity, 1.0);
    assert_eq!(reset_placement.selector.expected_opacity, None);
    assert_eq!(reset_placement.selector.draw_operation_index, 2);

    let foreign = directory.path().join("foreign.pdf");
    image_fixture(&foreign, false, true);
    let inspection = inspect_pdf(&foreign).unwrap();
    assert!(inspection.editable_image_placements.is_empty());
    assert_eq!(inspection.unsupported_image_placements, 1);
}

#[test]
fn unchanged_or_invalid_opacity_fails_closed_and_signed_edits_require_confirmation() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("signed.pdf");
    image_fixture(&input, true, false);
    let placement = inspect_pdf(&input)
        .unwrap()
        .editable_image_placements
        .remove(0);
    for destination_opacity in [1.0, -0.1, 1.1, f64::NAN] {
        assert!(matches!(
            set_image_opacity(&SetImageOpacityRequest {
                input_path: input.clone(),
                output_path: directory
                    .path()
                    .join(format!("blocked-{destination_opacity:?}.pdf")),
                selector: placement.selector.clone(),
                destination_opacity,
                allow_signature_invalidation: false,
            }),
            Err(PdfEditError::InvalidImageOpacity)
        ));
    }
    assert!(matches!(
        set_image_opacity(&SetImageOpacityRequest {
            input_path: input.clone(),
            output_path: directory.path().join("confirmation-required.pdf"),
            selector: placement.selector.clone(),
            destination_opacity: 0.7,
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::SignatureConfirmationRequired)
    ));
    let result = set_image_opacity(&SetImageOpacityRequest {
        input_path: input,
        output_path: directory.path().join("confirmed.pdf"),
        selector: placement.selector,
        destination_opacity: 0.7,
        allow_signature_invalidation: true,
    })
    .unwrap();
    assert_eq!(
        result.signature_effect,
        SignatureEffect::InvalidatesExistingSignatures
    );
}
