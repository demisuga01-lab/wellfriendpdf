use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, Stream, dictionary};
use std::fs;
use tempfile::tempdir;
use wellfriend_pdf_edit::{
    PdfBox, PdfEditError, PdfTextAlignment, ReflowTextRequest, SignatureEffect, inspect_pdf,
    reflow_text,
};

fn fixture_pdf(path: &std::path::Path, obstacle: bool, signed: bool) {
    let mut document = Document::with_version("1.7");
    let pages_id = document.new_object_id();
    let descriptor_id = document.add_object(dictionary! {
        "Type" => "FontDescriptor",
        "FontName" => "WellfriendFixtureSans",
        "Flags" => 32,
        "FontBBox" => vec![(-200).into(), (-220).into(), 1000.into(), 900.into()],
        "Ascent" => 800,
        "Descent" => -200,
        "CapHeight" => 700,
        "ItalicAngle" => 0,
        "StemV" => 80,
    });
    let widths = (0..=255)
        .map(|code| Object::Integer(if code == 32 { 250 } else { 500 }))
        .collect::<Vec<_>>();
    let font_id = document.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
        "FirstChar" => 0,
        "LastChar" => 255,
        "Widths" => widths,
        "FontDescriptor" => descriptor_id,
    });
    // Match the geometry-neutral text-state sequence emitted by pdf-lib:
    // outer save/restore, fill colour and leading before the sole text show,
    // followed by a terminal T* that cannot paint another run.
    let mut operations = vec![
        Operation::new("q", vec![]),
        Operation::new("BT", vec![]),
        Operation::new(
            "rg",
            vec![Object::Integer(0), Object::Integer(0), Object::Integer(0)],
        ),
        Operation::new(
            "Tf",
            vec![Object::Name(b"F1".to_vec()), Object::Integer(12)],
        ),
        Operation::new("TL", vec![Object::Integer(14)]),
        Operation::new(
            "Tm",
            vec![
                1.into(),
                0.into(),
                0.into(),
                1.into(),
                72.into(),
                700.into(),
            ],
        ),
        Operation::new("Tj", vec![Object::string_literal("Original paragraph")]),
        Operation::new("T*", vec![]),
        Operation::new("ET", vec![]),
        Operation::new("Q", vec![]),
    ];
    if obstacle {
        operations.extend([
            Operation::new("BT", vec![]),
            Operation::new(
                "Tf",
                vec![Object::Name(b"F1".to_vec()), Object::Integer(12)],
            ),
            Operation::new(
                "Tm",
                vec![
                    1.into(),
                    0.into(),
                    0.into(),
                    1.into(),
                    90.into(),
                    665.into(),
                ],
            ),
            Operation::new("Tj", vec![Object::string_literal("Neighbor")]),
            Operation::new("ET", vec![]),
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
        "Resources" => dictionary! { "Font" => dictionary! { "F1" => font_id } },
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
    document.compress();
    document.save(path).unwrap();
}

fn region() -> PdfBox {
    PdfBox {
        left: 72.0,
        bottom: 640.0,
        right: 300.0,
        top: 712.0,
    }
}

#[test]
fn bounded_reflow_rewrites_real_text_operators_with_pdf_font_metrics() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    let output = directory.path().join("output.pdf");
    fixture_pdf(&input, false, false);
    let inspection = inspect_pdf(&input).unwrap();
    let editable = &inspection.editable_text_runs[0];
    assert!(editable.reflow_eligible);
    assert_eq!(
        editable.font_metrics_source.as_deref(),
        Some("PDF_WIDTHS_FONT_DESCRIPTOR")
    );
    assert!(editable.bounding_box.is_some());

    let replacement =
        "A longer paragraph is wrapped into multiple measured lines inside one explicit region";
    let result = reflow_text(&ReflowTextRequest {
        input_path: input,
        output_path: output.clone(),
        selector: editable.selector.clone(),
        replacement_text: replacement.to_string(),
        region: region(),
        alignment: PdfTextAlignment::Left,
        line_spacing: 14.0,
        allow_signature_invalidation: false,
    })
    .unwrap();

    assert!(result.lines.len() > 1);
    assert!(result.validation.passed());
    assert!(result.validation.no_page_rasterization);
    assert_eq!(
        result.changed_objects,
        vec![editable.selector.content_stream_object]
    );
    assert_eq!(result.font_metrics_source, "PDF_WIDTHS_FONT_DESCRIPTOR");
    assert_eq!(result.signature_effect, SignatureEffect::None);
    let edited = Document::load(&output).unwrap();
    assert!(
        edited
            .get_page_images(edited.get_pages()[&1])
            .unwrap()
            .is_empty()
    );
    let stream = edited
        .get_object(editable.selector.content_stream_object.into())
        .unwrap()
        .as_stream()
        .unwrap();
    let content = Content::decode(&stream.decompressed_content().unwrap()).unwrap();
    assert_eq!(
        content
            .operations
            .iter()
            .filter(|operation| operation.operator == "Tj")
            .count(),
        result.lines.len()
    );
}

#[test]
fn reflow_refuses_neighbor_collisions_and_unbreakable_overflow() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    fixture_pdf(&input, true, false);
    let selector = inspect_pdf(&input).unwrap().editable_text_runs[0]
        .selector
        .clone();
    let collision = reflow_text(&ReflowTextRequest {
        input_path: input.clone(),
        output_path: directory.path().join("collision.pdf"),
        selector: selector.clone(),
        replacement_text: "A collision must be refused".to_string(),
        region: region(),
        alignment: PdfTextAlignment::Left,
        line_spacing: 14.0,
        allow_signature_invalidation: false,
    })
    .unwrap_err();
    assert!(matches!(collision, PdfEditError::ReflowUnsafe));

    let clean = directory.path().join("clean.pdf");
    fixture_pdf(&clean, false, false);
    let selector = inspect_pdf(&clean).unwrap().editable_text_runs[0]
        .selector
        .clone();
    let overflow = reflow_text(&ReflowTextRequest {
        input_path: clean,
        output_path: directory.path().join("overflow.pdf"),
        selector,
        replacement_text: "unbreakable_word_that_cannot_fit".to_string(),
        region: PdfBox {
            right: 100.0,
            ..region()
        },
        alignment: PdfTextAlignment::Left,
        line_spacing: 14.0,
        allow_signature_invalidation: false,
    })
    .unwrap_err();
    assert!(matches!(overflow, PdfEditError::ReflowUnsafe));
}

#[test]
fn signed_reflow_requires_explicit_invalidation_confirmation() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("signed.pdf");
    fixture_pdf(&input, false, true);
    let selector = inspect_pdf(&input).unwrap().editable_text_runs[0]
        .selector
        .clone();
    let output = directory.path().join("output.pdf");
    let error = reflow_text(&ReflowTextRequest {
        input_path: input.clone(),
        output_path: output.clone(),
        selector: selector.clone(),
        replacement_text: "Signed document replacement".to_string(),
        region: region(),
        alignment: PdfTextAlignment::Center,
        line_spacing: 14.0,
        allow_signature_invalidation: false,
    })
    .unwrap_err();
    assert!(matches!(error, PdfEditError::SignatureConfirmationRequired));
    assert!(!output.exists());
    let result = reflow_text(&ReflowTextRequest {
        input_path: input,
        output_path: output,
        selector,
        replacement_text: "Signed document replacement".to_string(),
        region: region(),
        alignment: PdfTextAlignment::Center,
        line_spacing: 14.0,
        allow_signature_invalidation: true,
    })
    .unwrap();
    assert_eq!(
        result.signature_effect,
        SignatureEffect::InvalidatesExistingSignatures
    );
}

#[test]
fn identical_reflow_requests_are_byte_deterministic() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    fixture_pdf(&input, false, false);
    let selector = inspect_pdf(&input).unwrap().editable_text_runs[0]
        .selector
        .clone();
    let run = |name: &str| {
        let output = directory.path().join(name);
        let result = reflow_text(&ReflowTextRequest {
            input_path: input.clone(),
            output_path: output.clone(),
            selector: selector.clone(),
            replacement_text: "Deterministic bounded paragraph reflow with exact PDF widths"
                .to_string(),
            region: region(),
            alignment: PdfTextAlignment::Right,
            line_spacing: 14.0,
            allow_signature_invalidation: false,
        })
        .unwrap();
        (output, result.output_sha256)
    };
    let first = run("first.pdf");
    let second = run("second.pdf");
    assert_eq!(first.1, second.1);
    assert_eq!(fs::read(first.0).unwrap(), fs::read(second.0).unwrap());
}
