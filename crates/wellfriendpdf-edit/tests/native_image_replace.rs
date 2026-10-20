use jpeg_encoder::{ColorType, Encoder};
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, Stream, dictionary};
use std::fs;
use tempfile::tempdir;
use wellfriend_pdf_edit::{
    PdfEditError, ReplaceImageRequest, SignatureEffect, inspect_pdf, replace_image,
};

fn jpeg(width: u16, height: u16, rgb: [u8; 3]) -> Vec<u8> {
    let pixels = [rgb[0], rgb[1], rgb[2]]
        .into_iter()
        .cycle()
        .take(usize::from(width) * usize::from(height) * 3)
        .collect::<Vec<_>>();
    let mut encoded = Vec::new();
    Encoder::new(&mut encoded, 92)
        .encode(&pixels, width, height, ColorType::Rgb)
        .unwrap();
    encoded
}

fn image_fixture(path: &std::path::Path, signed: bool, masked: bool) {
    let mut document = Document::with_version("1.7");
    let pages_id = document.new_object_id();
    let mut image_dictionary = dictionary! {
        "Type" => "XObject",
        "Subtype" => "Image",
        "Width" => 4,
        "Height" => 3,
        "ColorSpace" => "DeviceRGB",
        "BitsPerComponent" => 8,
        "Filter" => "DCTDecode",
    };
    if masked {
        image_dictionary.set("ImageMask", true);
    }
    let image_id = document.add_object(Stream::new(image_dictionary, jpeg(4, 3, [220, 20, 30])));
    let content = Content {
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
    };
    let content_id = document.add_object(Stream::new(Dictionary::new(), content.encode().unwrap()));
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
    document.save(path).unwrap();
}

#[test]
fn image_replacement_changes_the_real_xobject_and_preserves_page_placement() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    let replacement = directory.path().join("replacement.jpg");
    let output = directory.path().join("output.pdf");
    image_fixture(&input, false, false);
    fs::write(&replacement, jpeg(7, 5, [10, 80, 230])).unwrap();

    let inspection = inspect_pdf(&input).unwrap();
    assert_eq!(inspection.editable_images.len(), 1);
    assert_eq!(inspection.unsupported_images, 0);
    let editable = &inspection.editable_images[0];
    assert_eq!(editable.width, 4);
    assert_eq!(editable.height, 3);
    assert_eq!(editable.usages[0].page_number, 1);
    assert_eq!(editable.usages[0].resource_name, "Photo");

    let result = replace_image(&ReplaceImageRequest {
        input_path: input.clone(),
        output_path: output.clone(),
        replacement_image_path: replacement,
        selector: editable.selector.clone(),
        allow_signature_invalidation: false,
    })
    .unwrap();

    assert!(result.validation.passed());
    assert!(result.validation.no_page_rasterization);
    assert_eq!(result.capability, "PDF_IMAGE_REPLACE");
    assert_eq!(result.changed_objects, vec![editable.selector.image_object]);
    assert_eq!(result.replacement_width, 7);
    assert_eq!(result.replacement_height, 5);
    assert_eq!(result.replacement_color_space, "DeviceRGB");
    assert_eq!(result.affected_usages, editable.usages);
    assert_eq!(result.signature_effect, SignatureEffect::None);

    let source_document = Document::load(&input).unwrap();
    let edited_document = Document::load(&output).unwrap();
    let page_id = edited_document.get_pages()[&1];
    assert_eq!(
        edited_document.get_page_contents(page_id),
        source_document.get_page_contents(page_id)
    );
    let image_id: lopdf::ObjectId = editable.selector.image_object.into();
    let image = edited_document
        .get_object(image_id)
        .unwrap()
        .as_stream()
        .unwrap();
    assert_eq!(image.dict.get(b"Width").unwrap().as_i64().unwrap(), 7);
    assert_eq!(image.dict.get(b"Height").unwrap().as_i64().unwrap(), 5);
    assert_eq!(
        image.dict.get(b"Filter").unwrap().as_name().unwrap(),
        b"DCTDecode"
    );
    assert_eq!(edited_document.get_page_images(page_id).unwrap().len(), 1);
}

#[test]
fn image_replacement_refuses_stale_masked_and_invalid_inputs() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    let replacement = directory.path().join("replacement.jpg");
    image_fixture(&input, false, false);
    fs::write(&replacement, jpeg(3, 3, [10, 20, 30])).unwrap();
    let mut selector = inspect_pdf(&input).unwrap().editable_images[0]
        .selector
        .clone();
    selector.expected_width += 1;
    let stale = replace_image(&ReplaceImageRequest {
        input_path: input,
        output_path: directory.path().join("stale.pdf"),
        replacement_image_path: replacement,
        selector,
        allow_signature_invalidation: false,
    })
    .unwrap_err();
    assert!(matches!(stale, PdfEditError::StaleImage));

    let masked = directory.path().join("masked.pdf");
    image_fixture(&masked, false, true);
    let masked_inspection = inspect_pdf(&masked).unwrap();
    assert!(masked_inspection.editable_images.is_empty());
    assert_eq!(masked_inspection.unsupported_images, 1);

    let valid = directory.path().join("valid.pdf");
    image_fixture(&valid, false, false);
    let invalid = directory.path().join("invalid.jpg");
    fs::write(&invalid, b"not-a-jpeg").unwrap();
    let invalid_error = replace_image(&ReplaceImageRequest {
        input_path: valid.clone(),
        output_path: directory.path().join("invalid-output.pdf"),
        replacement_image_path: invalid,
        selector: inspect_pdf(&valid).unwrap().editable_images[0]
            .selector
            .clone(),
        allow_signature_invalidation: false,
    })
    .unwrap_err();
    assert!(matches!(
        invalid_error,
        PdfEditError::ReplacementImageInvalid
    ));
}

#[test]
fn signed_image_replacement_requires_confirmation_and_is_deterministic() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("signed.pdf");
    let replacement = directory.path().join("replacement.jpg");
    image_fixture(&input, true, false);
    fs::write(&replacement, jpeg(8, 6, [80, 90, 100])).unwrap();
    let selector = inspect_pdf(&input).unwrap().editable_images[0]
        .selector
        .clone();
    let refused = directory.path().join("refused.pdf");
    let error = replace_image(&ReplaceImageRequest {
        input_path: input.clone(),
        output_path: refused.clone(),
        replacement_image_path: replacement.clone(),
        selector: selector.clone(),
        allow_signature_invalidation: false,
    })
    .unwrap_err();
    assert!(matches!(error, PdfEditError::SignatureConfirmationRequired));
    assert!(!refused.exists());

    let output_a = directory.path().join("a.pdf");
    let output_b = directory.path().join("b.pdf");
    let first = replace_image(&ReplaceImageRequest {
        input_path: input.clone(),
        output_path: output_a.clone(),
        replacement_image_path: replacement.clone(),
        selector: selector.clone(),
        allow_signature_invalidation: true,
    })
    .unwrap();
    let second = replace_image(&ReplaceImageRequest {
        input_path: input,
        output_path: output_b.clone(),
        replacement_image_path: replacement,
        selector,
        allow_signature_invalidation: true,
    })
    .unwrap();
    assert_eq!(
        first.signature_effect,
        SignatureEffect::InvalidatesExistingSignatures
    );
    assert_eq!(first.output_sha256, second.output_sha256);
    assert_eq!(fs::read(output_a).unwrap(), fs::read(output_b).unwrap());
}
