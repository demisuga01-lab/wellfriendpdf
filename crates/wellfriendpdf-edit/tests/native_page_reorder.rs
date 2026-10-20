use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, Stream, dictionary};
use std::fs;
use tempfile::tempdir;
use wellfriend_pdf_edit::{PdfEditError, ReorderPagesRequest, SignatureEffect, reorder_pages};

fn text_stream(text: &str) -> Stream {
    Stream::new(
        Dictionary::new(),
        Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new(
                    "Tf",
                    vec![Object::Name(b"F1".to_vec()), Object::Integer(18)],
                ),
                Operation::new(
                    "Tm",
                    vec![
                        1.into(),
                        0.into(),
                        0.into(),
                        1.into(),
                        72.into(),
                        720.into(),
                    ],
                ),
                Operation::new("Tj", vec![Object::string_literal(text)]),
                Operation::new("ET", vec![]),
            ],
        }
        .encode()
        .unwrap(),
    )
}

fn nested_page_tree_fixture(path: &std::path::Path, signed: bool) {
    let mut document = Document::with_version("1.7");
    let root_pages_id = document.new_object_id();
    let branch_pages_id = document.new_object_id();
    let font_id = document.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let content_ids = ["Page one", "Page two", "Page three"]
        .iter()
        .map(|text| document.add_object(text_stream(text)))
        .collect::<Vec<_>>();
    let page_ids = content_ids
        .iter()
        .map(|content_id| {
            document.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => branch_pages_id,
                "Contents" => *content_id,
            })
        })
        .collect::<Vec<_>>();
    document.objects.insert(
        branch_pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Parent" => root_pages_id,
            "Kids" => page_ids.iter().copied().map(Object::Reference).collect::<Vec<_>>(),
            "Count" => 3,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Resources" => dictionary! {
                "Font" => dictionary! { "F1" => font_id },
            },
        }),
    );
    document.objects.insert(
        root_pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![Object::Reference(branch_pages_id)],
            "Count" => 3,
        }),
    );
    let catalog_id = document.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => root_pages_id,
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
fn page_reorder_rewrites_the_real_page_tree_without_rasterizing() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    let output = directory.path().join("output.pdf");
    nested_page_tree_fixture(&input, false);
    let source = Document::load(&input).unwrap();
    let source_page_ids = source.get_pages().values().copied().collect::<Vec<_>>();
    let source_contents = source_page_ids
        .iter()
        .map(|page_id| source.get_page_contents(*page_id))
        .collect::<Vec<_>>();

    let result = reorder_pages(&ReorderPagesRequest {
        input_path: input,
        output_path: output.clone(),
        page_order: vec![3, 1, 2],
        allow_signature_invalidation: false,
    })
    .unwrap();

    assert!(result.validation.passed());
    assert!(result.validation.no_page_rasterization);
    assert_eq!(result.capability, "PDF_PAGE_EDIT");
    assert_eq!(result.requested_source_page_numbers, vec![3, 1, 2]);
    assert_eq!(result.source_object_count, result.output_object_count);
    assert_eq!(result.signature_effect, SignatureEffect::None);

    let edited = Document::load(&output).unwrap();
    let edited_page_ids = edited.get_pages().values().copied().collect::<Vec<_>>();
    assert_eq!(
        edited_page_ids,
        vec![source_page_ids[2], source_page_ids[0], source_page_ids[1]]
    );
    assert_eq!(
        edited.extract_text_with_limit(&[1], 1024 * 1024).unwrap(),
        "Page three\n"
    );
    assert_eq!(
        edited.extract_text_with_limit(&[2], 1024 * 1024).unwrap(),
        "Page one\n"
    );
    for (source_page_id, content_ids) in source_page_ids.iter().zip(source_contents) {
        assert_eq!(edited.get_page_contents(*source_page_id), content_ids);
        let page = edited.get_dictionary(*source_page_id).unwrap();
        assert!(page.has(b"MediaBox"));
        assert!(page.has(b"Resources"));
    }
    assert!(
        edited_page_ids
            .iter()
            .all(|page_id| edited.get_page_images(*page_id).unwrap().is_empty())
    );
}

#[test]
fn invalid_orders_and_signed_page_reorders_fail_closed() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    nested_page_tree_fixture(&input, false);
    for (index, page_order) in [vec![1, 2, 3], vec![1, 1, 3], vec![1, 2], vec![1, 2, 4]]
        .into_iter()
        .enumerate()
    {
        let error = reorder_pages(&ReorderPagesRequest {
            input_path: input.clone(),
            output_path: directory.path().join(format!("invalid-{index}.pdf")),
            page_order,
            allow_signature_invalidation: false,
        })
        .unwrap_err();
        assert!(matches!(error, PdfEditError::InvalidPageOrder));
    }

    let signed = directory.path().join("signed.pdf");
    let signed_output = directory.path().join("signed-output.pdf");
    nested_page_tree_fixture(&signed, true);
    let error = reorder_pages(&ReorderPagesRequest {
        input_path: signed.clone(),
        output_path: signed_output.clone(),
        page_order: vec![3, 2, 1],
        allow_signature_invalidation: false,
    })
    .unwrap_err();
    assert!(matches!(error, PdfEditError::SignatureConfirmationRequired));
    assert!(!signed_output.exists());
    let result = reorder_pages(&ReorderPagesRequest {
        input_path: signed,
        output_path: signed_output,
        page_order: vec![3, 2, 1],
        allow_signature_invalidation: true,
    })
    .unwrap();
    assert_eq!(
        result.signature_effect,
        SignatureEffect::InvalidatesExistingSignatures
    );
}

#[test]
fn identical_page_reorder_commands_are_byte_deterministic() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    let output_a = directory.path().join("a.pdf");
    let output_b = directory.path().join("b.pdf");
    nested_page_tree_fixture(&input, false);
    let result_a = reorder_pages(&ReorderPagesRequest {
        input_path: input.clone(),
        output_path: output_a.clone(),
        page_order: vec![2, 3, 1],
        allow_signature_invalidation: false,
    })
    .unwrap();
    let result_b = reorder_pages(&ReorderPagesRequest {
        input_path: input,
        output_path: output_b.clone(),
        page_order: vec![2, 3, 1],
        allow_signature_invalidation: false,
    })
    .unwrap();
    assert_eq!(result_a.output_sha256, result_b.output_sha256);
    assert_eq!(fs::read(output_a).unwrap(), fs::read(output_b).unwrap());
}
