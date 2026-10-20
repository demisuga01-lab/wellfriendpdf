use lopdf::{Dictionary, Document, Object, Stream, dictionary};
use tempfile::tempdir;
use wellfriend_pdf_edit::{
    EditLinkRequest, LinkMutation, PdfBox, PdfEditError, PdfLinkTarget, SignatureEffect, edit_link,
    inspect_pdf,
};

fn link_fixture(path: &std::path::Path, signed: bool) {
    let mut document = Document::with_version("1.7");
    let pages_id = document.new_object_id();
    let first_content = document.add_object(Stream::new(Dictionary::new(), Vec::new()));
    let second_content = document.add_object(Stream::new(Dictionary::new(), Vec::new()));
    let first_page = document.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => first_content,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        "Resources" => Dictionary::new(),
    });
    let second_page = document.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => second_content,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        "Resources" => Dictionary::new(),
    });
    let mut action = Dictionary::new();
    action.set("S", Object::Name(b"URI".to_vec()));
    action.set("URI", Object::string_literal("https://wellfriend.ai/start"));
    let annotation = document.add_object(dictionary! {
        "Type" => "Annot",
        "Subtype" => "Link",
        "Rect" => vec![72.into(), 680.into(), 250.into(), 710.into()],
        "Border" => vec![0.into(), 0.into(), 0.into()],
        "P" => first_page,
        "A" => action,
    });
    let annotations = document.add_object(Object::Array(vec![annotation.into()]));
    document
        .get_dictionary_mut(first_page)
        .unwrap()
        .set("Annots", annotations);
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![first_page.into(), second_page.into()],
            "Count" => 2,
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

fn rectangle(left: f64, bottom: f64, right: f64, top: f64) -> PdfBox {
    PdfBox {
        left,
        bottom,
        right,
        top,
    }
}

#[test]
fn link_annotations_are_created_updated_and_deleted_as_pdf_objects() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    let created = directory.path().join("created.pdf");
    let updated = directory.path().join("updated.pdf");
    let deleted = directory.path().join("deleted.pdf");
    link_fixture(&input, false);

    let source = inspect_pdf(&input).unwrap();
    assert_eq!(source.editable_links.len(), 1);
    assert_eq!(source.editable_link_pages.len(), 2);
    assert_eq!(source.object_graph.links, 1);
    let new_rectangle = rectangle(80.0, 620.0, 320.0, 655.0);
    let created_result = edit_link(&EditLinkRequest {
        input_path: input,
        output_path: created.clone(),
        mutation: LinkMutation::Create {
            page: source.editable_link_pages[0].selector.clone(),
            rectangle: new_rectangle,
            target: PdfLinkTarget::ExternalUrl {
                url: "https://wellfriend.ai/docs?q=pdf#links".to_string(),
            },
        },
        allow_signature_invalidation: false,
    })
    .unwrap();
    assert_eq!(created_result.operation, "EDIT_LINK");
    assert_eq!(created_result.capability, "PDF_LINK_EDIT");
    assert!(created_result.validation.passed());
    assert!(created_result.validation.no_page_rasterization);
    let created_inspection = inspect_pdf(&created).unwrap();
    assert_eq!(created_inspection.editable_links.len(), 2);
    assert_eq!(created_inspection.object_graph.links, 2);
    let created_link = created_inspection
        .editable_links
        .iter()
        .find(|link| link.rectangle == new_rectangle)
        .unwrap()
        .clone();

    let updated_rectangle = rectangle(96.0, 560.0, 360.0, 600.0);
    let updated_result = edit_link(&EditLinkRequest {
        input_path: created,
        output_path: updated.clone(),
        mutation: LinkMutation::Update {
            selector: created_link.selector,
            rectangle: updated_rectangle,
            target: PdfLinkTarget::InternalPage { page_number: 2 },
        },
        allow_signature_invalidation: false,
    })
    .unwrap();
    assert!(updated_result.validation.passed());
    assert_eq!(updated_result.result_rectangle, Some(updated_rectangle));
    assert_eq!(
        updated_result.result_target,
        Some(PdfLinkTarget::InternalPage { page_number: 2 })
    );
    let updated_inspection = inspect_pdf(&updated).unwrap();
    let updated_link = updated_inspection
        .editable_links
        .iter()
        .find(|link| link.rectangle == updated_rectangle)
        .unwrap()
        .clone();

    let deleted_result = edit_link(&EditLinkRequest {
        input_path: updated,
        output_path: deleted.clone(),
        mutation: LinkMutation::Delete {
            selector: updated_link.selector,
        },
        allow_signature_invalidation: false,
    })
    .unwrap();
    assert!(deleted_result.validation.passed());
    assert_eq!(deleted_result.result_rectangle, None);
    assert_eq!(deleted_result.result_target, None);
    let deleted_inspection = inspect_pdf(&deleted).unwrap();
    assert_eq!(deleted_inspection.editable_links.len(), 1);
    assert_eq!(deleted_inspection.object_graph.links, 1);
    assert_eq!(deleted_inspection.object_graph.images, 0);
}

#[test]
fn external_link_protocols_and_stale_collections_fail_closed() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    link_fixture(&input, false);
    let inspection = inspect_pdf(&input).unwrap();
    let page = inspection.editable_link_pages[0].selector.clone();
    for (index, url) in [
        "javascript:alert(1)",
        "data:text/html,unsafe",
        "file:///etc/passwd",
        "https://user:secret@example.com/private",
    ]
    .iter()
    .enumerate()
    {
        assert!(matches!(
            edit_link(&EditLinkRequest {
                input_path: input.clone(),
                output_path: directory.path().join(format!("forbidden-{index}.pdf")),
                mutation: LinkMutation::Create {
                    page: page.clone(),
                    rectangle: rectangle(72.0, 600.0, 240.0, 630.0),
                    target: PdfLinkTarget::ExternalUrl {
                        url: (*url).to_string(),
                    },
                },
                allow_signature_invalidation: false,
            }),
            Err(PdfEditError::InvalidLinkProtocol)
        ));
    }

    let mut stale_page = page;
    stale_page.expected_annotations_sha256 = "0".repeat(64);
    assert!(matches!(
        edit_link(&EditLinkRequest {
            input_path: input.clone(),
            output_path: directory.path().join("stale-page.pdf"),
            mutation: LinkMutation::Create {
                page: stale_page,
                rectangle: rectangle(72.0, 600.0, 240.0, 630.0),
                target: PdfLinkTarget::ExternalUrl {
                    url: "mailto:security@wellfriend.test".to_string(),
                },
            },
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::StaleLink)
    ));

    let mut stale_link = inspection.editable_links[0].selector.clone();
    stale_link.expected_annotation_sha256 = "f".repeat(64);
    assert!(matches!(
        edit_link(&EditLinkRequest {
            input_path: input,
            output_path: directory.path().join("stale-link.pdf"),
            mutation: LinkMutation::Delete {
                selector: stale_link,
            },
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::StaleLink)
    ));
}

#[test]
fn signed_link_edits_require_explicit_invalidation_confirmation() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("signed.pdf");
    link_fixture(&input, true);
    let link = inspect_pdf(&input).unwrap().editable_links.remove(0);
    assert!(matches!(
        edit_link(&EditLinkRequest {
            input_path: input.clone(),
            output_path: directory.path().join("unconfirmed.pdf"),
            mutation: LinkMutation::Delete {
                selector: link.selector.clone(),
            },
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::SignatureConfirmationRequired)
    ));
    let confirmed = edit_link(&EditLinkRequest {
        input_path: input,
        output_path: directory.path().join("confirmed.pdf"),
        mutation: LinkMutation::Delete {
            selector: link.selector,
        },
        allow_signature_invalidation: true,
    })
    .unwrap();
    assert_eq!(
        confirmed.signature_effect,
        SignatureEffect::InvalidatesExistingSignatures
    );
}
