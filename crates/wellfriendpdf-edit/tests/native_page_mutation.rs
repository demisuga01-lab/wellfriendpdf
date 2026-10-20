use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, Stream, dictionary};
use std::fs;
use tempfile::tempdir;
use wellfriend_pdf_edit::{
    MutatePagesRequest, PageIdentitySelector, PageMutation, PageMutationKind, PdfBox, PdfEditError,
    SignatureEffect, mutate_pages,
};

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

fn page_fixture(path: &std::path::Path, labels: &[&str], signed: bool) {
    let mut document = Document::with_version("1.7");
    let pages_id = document.new_object_id();
    let font_id = document.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let page_ids = labels
        .iter()
        .map(|label| {
            let content_id = document.add_object(text_stream(label));
            document.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
                "Resources" => dictionary! {
                    "Font" => dictionary! { "F1" => font_id },
                },
                "Contents" => content_id,
            })
        })
        .collect::<Vec<_>>();
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => page_ids.iter().copied().map(Object::Reference).collect::<Vec<_>>(),
            "Count" => labels.len() as i64,
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

fn selector(document: &Document, page_number: u32) -> PageIdentitySelector {
    PageIdentitySelector {
        page_number,
        page_object: (*document.get_pages().get(&page_number).unwrap()).into(),
    }
}

#[test]
fn delete_removes_only_the_selected_page_from_the_real_page_tree() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    let output = directory.path().join("output.pdf");
    page_fixture(&input, &["One", "Two", "Three"], false);
    let source = Document::load(&input).unwrap();
    let source_pages = source.get_pages().values().copied().collect::<Vec<_>>();

    let result = mutate_pages(&MutatePagesRequest {
        input_path: input,
        output_path: output.clone(),
        mutation: PageMutation::Delete {
            selector: selector(&source, 2),
        },
        allow_signature_invalidation: false,
    })
    .unwrap();

    assert_eq!(result.mutation, PageMutationKind::Delete);
    assert!(result.validation.passed());
    assert_eq!(result.source_page_count, 3);
    assert_eq!(result.output_page_count, 2);
    assert_eq!(result.removed_page_objects, vec![source_pages[1].into()]);
    let edited = Document::load(output).unwrap();
    assert_eq!(
        edited.get_pages().values().copied().collect::<Vec<_>>(),
        vec![source_pages[0], source_pages[2]]
    );
    assert_eq!(edited.extract_text(&[1, 2]).unwrap(), "One\nThree\n");
}

#[test]
fn duplicate_deep_clones_page_content_and_resources_without_rasterizing() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    let output = directory.path().join("output.pdf");
    page_fixture(&input, &["Original", "Second"], false);
    let source = Document::load(&input).unwrap();
    let source_page = *source.get_pages().get(&1).unwrap();
    let source_contents = source.get_page_contents(source_page);

    let result = mutate_pages(&MutatePagesRequest {
        input_path: input,
        output_path: output.clone(),
        mutation: PageMutation::Duplicate {
            selector: selector(&source, 1),
            insert_before_page_number: 2,
        },
        allow_signature_invalidation: false,
    })
    .unwrap();

    assert_eq!(result.mutation, PageMutationKind::Duplicate);
    assert!(result.validation.passed());
    assert!(result.validation.no_page_rasterization);
    let edited = Document::load(output).unwrap();
    let pages = edited.get_pages();
    let duplicate_page = *pages.get(&2).unwrap();
    assert_ne!(duplicate_page, source_page);
    assert_ne!(edited.get_page_contents(duplicate_page), source_contents);
    assert_eq!(
        edited.extract_text(&[1, 2, 3]).unwrap(),
        "Original\nOriginal\nSecond\n"
    );
    assert!(edited.get_page_images(duplicate_page).unwrap().is_empty());
}

#[test]
fn blank_insertion_and_external_import_create_real_page_objects() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    let blank_output = directory.path().join("blank.pdf");
    let import = directory.path().join("import.pdf");
    let imported_output = directory.path().join("imported.pdf");
    page_fixture(&input, &["Existing"], false);
    page_fixture(&import, &["Imported A", "Imported B"], false);

    let blank = mutate_pages(&MutatePagesRequest {
        input_path: input,
        output_path: blank_output.clone(),
        mutation: PageMutation::InsertBlank {
            insert_before_page_number: 1,
            media_box: PdfBox {
                left: 0.0,
                bottom: 0.0,
                right: 595.0,
                top: 842.0,
            },
        },
        allow_signature_invalidation: false,
    })
    .unwrap();
    assert_eq!(blank.mutation, PageMutationKind::InsertBlank);
    assert!(blank.validation.passed());
    let blank_pdf = Document::load(&blank_output).unwrap();
    assert_eq!(blank_pdf.get_pages().len(), 2);
    assert_eq!(blank_pdf.extract_text(&[2]).unwrap(), "Existing\n");

    let import_source = Document::load(&import).unwrap();
    let imported = mutate_pages(&MutatePagesRequest {
        input_path: blank_output,
        output_path: imported_output.clone(),
        mutation: PageMutation::Import {
            import_path: import,
            source_pages: vec![selector(&import_source, 2), selector(&import_source, 1)],
            insert_before_page_number: 2,
        },
        allow_signature_invalidation: false,
    })
    .unwrap();
    assert_eq!(imported.mutation, PageMutationKind::Import);
    assert!(imported.import_input_sha256.is_some());
    assert!(imported.validation.passed());
    let edited = Document::load(imported_output).unwrap();
    assert_eq!(edited.get_pages().len(), 4);
    assert_eq!(
        edited.extract_text(&[2, 3, 4]).unwrap(),
        "Imported B\nImported A\nExisting\n"
    );
}

#[test]
fn stale_dependent_and_unconfirmed_signed_mutations_fail_closed_while_confirmed_edits_are_deterministic()
 {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    page_fixture(&input, &["One", "Two"], false);
    let source = Document::load(&input).unwrap();
    let mut stale = selector(&source, 1);
    stale.page_object.number += 999;
    let stale_error = mutate_pages(&MutatePagesRequest {
        input_path: input.clone(),
        output_path: directory.path().join("stale.pdf"),
        mutation: PageMutation::Delete { selector: stale },
        allow_signature_invalidation: false,
    })
    .unwrap_err();
    assert!(matches!(stale_error, PdfEditError::StalePageMutation));

    let mut dependent = Document::load(&input).unwrap();
    let target_page = *dependent.get_pages().get(&1).unwrap();
    let destination_id = dependent.add_object(dictionary! {
        "Type" => "Action",
        "D" => vec![Object::Reference(target_page), Object::Name(b"Fit".to_vec())],
    });
    let catalog = dependent.catalog_mut().unwrap();
    catalog.set("OpenAction", destination_id);
    let dependent_path = directory.path().join("dependent.pdf");
    dependent.save(&dependent_path).unwrap();
    let dependent_source = Document::load(&dependent_path).unwrap();
    let dependency_error = mutate_pages(&MutatePagesRequest {
        input_path: dependent_path,
        output_path: directory.path().join("dependent-output.pdf"),
        mutation: PageMutation::Delete {
            selector: selector(&dependent_source, 1),
        },
        allow_signature_invalidation: false,
    })
    .unwrap_err();
    assert!(matches!(
        dependency_error,
        PdfEditError::PageDependencyUnsupported
    ));

    let signed = directory.path().join("signed.pdf");
    page_fixture(&signed, &["One", "Two"], true);
    let signed_source = Document::load(&signed).unwrap();
    let signature_error = mutate_pages(&MutatePagesRequest {
        input_path: signed.clone(),
        output_path: directory.path().join("signed-denied.pdf"),
        mutation: PageMutation::Duplicate {
            selector: selector(&signed_source, 1),
            insert_before_page_number: 2,
        },
        allow_signature_invalidation: false,
    })
    .unwrap_err();
    assert!(matches!(
        signature_error,
        PdfEditError::SignatureConfirmationRequired
    ));

    let output_a = directory.path().join("a.pdf");
    let output_b = directory.path().join("b.pdf");
    let request = |output_path| MutatePagesRequest {
        input_path: signed.clone(),
        output_path,
        mutation: PageMutation::Duplicate {
            selector: selector(&signed_source, 1),
            insert_before_page_number: 2,
        },
        allow_signature_invalidation: true,
    };
    let a = mutate_pages(&request(output_a.clone())).unwrap();
    let b = mutate_pages(&request(output_b.clone())).unwrap();
    assert_eq!(
        a.signature_effect,
        SignatureEffect::InvalidatesExistingSignatures
    );
    assert_eq!(a.output_sha256, b.output_sha256);
    assert_eq!(fs::read(output_a).unwrap(), fs::read(output_b).unwrap());
}
