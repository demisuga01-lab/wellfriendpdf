use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, Stream, dictionary};
use tempfile::tempdir;
use wellfriend_pdf_edit::{
    EditVectorPathRequest, PdfEditError, PdfMatrix, PdfRgbColor, SignatureEffect,
    VectorPathMutation, edit_vector_path, inspect_pdf,
};

fn rectangle_group(x: i64, y: i64, red: f64, green: f64, blue: f64) -> Vec<Operation> {
    vec![
        Operation::new("q", vec![]),
        Operation::new("rg", vec![red.into(), green.into(), blue.into()]),
        Operation::new("RG", vec![0.into(), 0.into(), 0.into()]),
        Operation::new("w", vec![2.into()]),
        Operation::new("re", vec![x.into(), y.into(), 120.into(), 60.into()]),
        Operation::new("B", vec![]),
        Operation::new("Q", vec![]),
    ]
}

fn vector_fixture(path: &std::path::Path, signed: bool, unsafe_group: bool) {
    let mut document = Document::with_version("1.7");
    let pages_id = document.new_object_id();
    let mut operations = rectangle_group(72, 620, 0.8, 0.1, 0.2);
    operations.extend(rectangle_group(260, 620, 0.1, 0.6, 0.3));
    if unsafe_group {
        operations.extend([
            Operation::new("q", vec![]),
            Operation::new("re", vec![72.into(), 500.into(), 90.into(), 30.into()]),
            Operation::new("W", vec![]),
            Operation::new("n", vec![]),
            Operation::new("Q", vec![]),
        ]);
    }
    let content_id = document.add_object(Stream::new(
        Dictionary::new(),
        Content { operations }.encode().unwrap(),
    ));
    let page_id = document.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        "Resources" => Dictionary::new(),
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

fn assert_color(actual: Option<PdfRgbColor>, expected: PdfRgbColor) {
    let actual = actual.expect("the path color must be present");
    assert!((actual.red - expected.red).abs() < 0.000_001);
    assert!((actual.green - expected.green).abs() < 0.000_001);
    assert!((actual.blue - expected.blue).abs() < 0.000_001);
}

#[test]
fn isolated_vector_path_can_transform_recolor_and_delete_without_rasterization() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    let transformed = directory.path().join("transformed.pdf");
    let recolored = directory.path().join("recolored.pdf");
    let deleted = directory.path().join("deleted.pdf");
    vector_fixture(&input, false, false);
    let inspection = inspect_pdf(&input).unwrap();
    assert_eq!(inspection.editable_vector_paths.len(), 2);
    let first = inspection.editable_vector_paths[0].clone();
    let destination = PdfMatrix {
        a: 1.5,
        b: 0.0,
        c: 0.0,
        d: 0.75,
        e: 18.0,
        f: -12.0,
    };
    let transformed_result = edit_vector_path(&EditVectorPathRequest {
        input_path: input,
        output_path: transformed.clone(),
        selector: first.selector,
        mutation: VectorPathMutation::Transform {
            destination_matrix: destination,
        },
        allow_signature_invalidation: false,
    })
    .unwrap();
    assert_eq!(transformed_result.operation, "EDIT_VECTOR_PATH");
    assert_eq!(transformed_result.capability, "PDF_VECTOR_EDIT");
    assert!(transformed_result.validation.passed());
    assert!(transformed_result.validation.no_page_rasterization);
    let transformed_path = inspect_pdf(&transformed)
        .unwrap()
        .editable_vector_paths
        .remove(0);
    assert_eq!(transformed_path.transform, destination);

    let purple = PdfRgbColor {
        red: 0.35,
        green: 0.1,
        blue: 0.8,
    };
    let green = PdfRgbColor {
        red: 0.0,
        green: 0.7,
        blue: 0.25,
    };
    let recolored_result = edit_vector_path(&EditVectorPathRequest {
        input_path: transformed,
        output_path: recolored.clone(),
        selector: transformed_path.selector,
        mutation: VectorPathMutation::Recolor {
            destination_fill_color: Some(purple),
            destination_stroke_color: Some(green),
        },
        allow_signature_invalidation: false,
    })
    .unwrap();
    assert!(recolored_result.validation.passed());
    assert_color(recolored_result.result_fill_color, purple);
    assert_color(recolored_result.result_stroke_color, green);
    let recolored_path = inspect_pdf(&recolored)
        .unwrap()
        .editable_vector_paths
        .remove(0);
    assert_color(recolored_path.fill_color, purple);
    assert_color(recolored_path.stroke_color, green);

    let deleted_result = edit_vector_path(&EditVectorPathRequest {
        input_path: recolored,
        output_path: deleted.clone(),
        selector: recolored_path.selector,
        mutation: VectorPathMutation::Delete,
        allow_signature_invalidation: false,
    })
    .unwrap();
    assert!(deleted_result.validation.passed());
    assert_eq!(deleted_result.result_bounds, None);
    let remaining = inspect_pdf(&deleted).unwrap();
    assert_eq!(remaining.editable_vector_paths.len(), 1);
    assert!(remaining.editable_vector_paths[0].bounds.left >= 260.0);
}

#[test]
fn clipping_and_nested_or_stale_path_groups_fail_closed() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    vector_fixture(&input, false, true);
    let inspection = inspect_pdf(&input).unwrap();
    assert_eq!(inspection.editable_vector_paths.len(), 2);
    assert_eq!(inspection.unsupported_vector_path_groups, 1);
    let mut stale = inspection.editable_vector_paths[0].selector.clone();
    stale.expected_operations_sha256 = "0".repeat(64);
    assert!(matches!(
        edit_vector_path(&EditVectorPathRequest {
            input_path: input,
            output_path: directory.path().join("stale.pdf"),
            selector: stale,
            mutation: VectorPathMutation::Delete,
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::StaleVectorPath)
    ));
}

#[test]
fn unchanged_invalid_and_signed_vector_edits_require_explicit_authority() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("signed.pdf");
    vector_fixture(&input, true, false);
    let path = inspect_pdf(&input).unwrap().editable_vector_paths.remove(0);
    assert!(matches!(
        edit_vector_path(&EditVectorPathRequest {
            input_path: input.clone(),
            output_path: directory.path().join("unchanged.pdf"),
            selector: path.selector.clone(),
            mutation: VectorPathMutation::Transform {
                destination_matrix: path.transform,
            },
            allow_signature_invalidation: true,
        }),
        Err(PdfEditError::InvalidVectorEdit)
    ));
    assert!(matches!(
        edit_vector_path(&EditVectorPathRequest {
            input_path: input.clone(),
            output_path: directory.path().join("confirmation.pdf"),
            selector: path.selector.clone(),
            mutation: VectorPathMutation::Delete,
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::SignatureConfirmationRequired)
    ));
    let result = edit_vector_path(&EditVectorPathRequest {
        input_path: input,
        output_path: directory.path().join("confirmed.pdf"),
        selector: path.selector,
        mutation: VectorPathMutation::Delete,
        allow_signature_invalidation: true,
    })
    .unwrap();
    assert_eq!(
        result.signature_effect,
        SignatureEffect::InvalidatesExistingSignatures
    );
}
