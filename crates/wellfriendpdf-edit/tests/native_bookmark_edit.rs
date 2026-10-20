use lopdf::{Dictionary, Document, Object, Stream, dictionary};
use tempfile::tempdir;
use wellfriend_pdf_edit::{
    BookmarkMutation, EditBookmarkRequest, PdfEditError, PdfLinkTarget, SignatureEffect,
    edit_bookmark, inspect_pdf,
};

fn bookmark_fixture(path: &std::path::Path, with_outlines: bool, signed: bool) {
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
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![first_page.into(), second_page.into()],
            "Count" => 2,
        }),
    );
    let catalog_id = document.new_object_id();
    if with_outlines {
        let root_id = document.new_object_id();
        let first_id = document.new_object_id();
        let second_id = document.new_object_id();
        let child_id = document.new_object_id();
        document.objects.insert(
            root_id,
            Object::Dictionary(dictionary! {
                "Type" => "Outlines",
                "First" => first_id,
                "Last" => second_id,
                "Count" => 3,
            }),
        );
        document.objects.insert(
            first_id,
            Object::Dictionary(dictionary! {
                "Title" => Object::string_literal("First"),
                "Parent" => root_id,
                "Next" => second_id,
                "First" => child_id,
                "Last" => child_id,
                "Count" => 1,
                "Dest" => vec![first_page.into(), Object::Name(b"Fit".to_vec())],
            }),
        );
        document.objects.insert(
            second_id,
            Object::Dictionary(dictionary! {
                "Title" => Object::string_literal("Second"),
                "Parent" => root_id,
                "Prev" => first_id,
                "Dest" => vec![second_page.into(), Object::Name(b"Fit".to_vec())],
            }),
        );
        document.objects.insert(
            child_id,
            Object::Dictionary(dictionary! {
                "Title" => Object::string_literal("Child"),
                "Parent" => first_id,
                "Dest" => vec![second_page.into(), Object::Name(b"Fit".to_vec())],
            }),
        );
        document.objects.insert(
            catalog_id,
            Object::Dictionary(dictionary! {
                "Type" => "Catalog",
                "Pages" => pages_id,
                "Outlines" => root_id,
            }),
        );
    } else {
        document.objects.insert(
            catalog_id,
            Object::Dictionary(dictionary! { "Type" => "Catalog", "Pages" => pages_id }),
        );
    }
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
fn bookmark_objects_are_created_renamed_retargeted_moved_and_deleted() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    let created = directory.path().join("created.pdf");
    let updated = directory.path().join("updated.pdf");
    let moved = directory.path().join("moved.pdf");
    let deleted = directory.path().join("deleted.pdf");
    bookmark_fixture(&input, true, false);

    let source = inspect_pdf(&input).unwrap();
    assert_eq!(source.editable_bookmarks.len(), 3);
    assert_eq!(
        source
            .editable_bookmark_tree
            .as_ref()
            .unwrap()
            .bookmark_count,
        3
    );
    assert_eq!(source.unsupported_bookmarks, 0);
    let created_result = edit_bookmark(&EditBookmarkRequest {
        input_path: input,
        output_path: created.clone(),
        mutation: BookmarkMutation::Create {
            tree: source.editable_bookmark_tree.unwrap().selector,
            parent: None,
            sibling_index: 1,
            title: "Inserted".to_string(),
            target: PdfLinkTarget::InternalPage { page_number: 2 },
        },
        allow_signature_invalidation: false,
    })
    .unwrap();
    assert_eq!(created_result.operation, "EDIT_BOOKMARK");
    assert_eq!(created_result.capability, "PDF_BOOKMARK_EDIT");
    assert!(created_result.validation.passed());
    assert!(created_result.validation.no_page_rasterization);
    let created_inspection = inspect_pdf(&created).unwrap();
    assert_eq!(created_inspection.editable_bookmarks.len(), 4);
    let inserted = created_inspection
        .editable_bookmarks
        .iter()
        .find(|bookmark| bookmark.title == "Inserted")
        .unwrap()
        .clone();

    let updated_result = edit_bookmark(&EditBookmarkRequest {
        input_path: created,
        output_path: updated.clone(),
        mutation: BookmarkMutation::Update {
            selector: inserted.selector,
            title: "Résumé Ω".to_string(),
            target: PdfLinkTarget::InternalPage { page_number: 1 },
        },
        allow_signature_invalidation: false,
    })
    .unwrap();
    assert_eq!(updated_result.result_title.as_deref(), Some("Résumé Ω"));
    assert_eq!(
        updated_result.result_target,
        Some(PdfLinkTarget::InternalPage { page_number: 1 })
    );
    assert!(updated_result.validation.passed());

    let updated_inspection = inspect_pdf(&updated).unwrap();
    let child = updated_inspection
        .editable_bookmarks
        .iter()
        .find(|bookmark| bookmark.title == "Child")
        .unwrap()
        .clone();
    let second = updated_inspection
        .editable_bookmarks
        .iter()
        .find(|bookmark| bookmark.title == "Second")
        .unwrap()
        .clone();
    let moved_result = edit_bookmark(&EditBookmarkRequest {
        input_path: updated,
        output_path: moved.clone(),
        mutation: BookmarkMutation::Move {
            selector: child.selector,
            destination_parent: Some(second.selector),
            destination_sibling_index: 0,
        },
        allow_signature_invalidation: false,
    })
    .unwrap();
    assert!(moved_result.validation.passed());
    assert_eq!(moved_result.result_sibling_index, Some(0));
    let moved_inspection = inspect_pdf(&moved).unwrap();
    let moved_child = moved_inspection
        .editable_bookmarks
        .iter()
        .find(|bookmark| bookmark.title == "Child")
        .unwrap();
    assert_eq!(moved_child.depth, 1);
    let second = moved_inspection
        .editable_bookmarks
        .iter()
        .find(|bookmark| bookmark.title == "Second")
        .unwrap()
        .clone();

    let deleted_result = edit_bookmark(&EditBookmarkRequest {
        input_path: moved,
        output_path: deleted.clone(),
        mutation: BookmarkMutation::Delete {
            selector: second.selector,
        },
        allow_signature_invalidation: false,
    })
    .unwrap();
    assert!(deleted_result.validation.passed());
    assert_eq!(deleted_result.removed_subtree_count, 2);
    let deleted_inspection = inspect_pdf(&deleted).unwrap();
    assert_eq!(deleted_inspection.editable_bookmarks.len(), 2);
    assert!(
        deleted_inspection
            .editable_bookmarks
            .iter()
            .all(|bookmark| bookmark.title != "Child" && bookmark.title != "Second")
    );
}

#[test]
fn missing_outline_tree_can_be_created_but_stale_and_recursive_moves_fail_closed() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    let created = directory.path().join("created.pdf");
    bookmark_fixture(&input, false, false);
    let inspection = inspect_pdf(&input).unwrap();
    assert!(inspection.editable_bookmarks.is_empty());
    let tree = inspection.editable_bookmark_tree.unwrap().selector;
    assert!(tree.outlines_object.is_none());
    edit_bookmark(&EditBookmarkRequest {
        input_path: input,
        output_path: created.clone(),
        mutation: BookmarkMutation::Create {
            tree,
            parent: None,
            sibling_index: 0,
            title: "Start".to_string(),
            target: PdfLinkTarget::InternalPage { page_number: 1 },
        },
        allow_signature_invalidation: false,
    })
    .unwrap();
    let created_inspection = inspect_pdf(&created).unwrap();
    assert_eq!(created_inspection.editable_bookmarks.len(), 1);
    let bookmark = created_inspection.editable_bookmarks[0].clone();

    let mut stale = bookmark.selector.clone();
    stale.tree.expected_tree_sha256 = "0".repeat(64);
    assert!(matches!(
        edit_bookmark(&EditBookmarkRequest {
            input_path: created.clone(),
            output_path: directory.path().join("stale.pdf"),
            mutation: BookmarkMutation::Delete { selector: stale },
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::StaleBookmark)
    ));
    assert!(matches!(
        edit_bookmark(&EditBookmarkRequest {
            input_path: created,
            output_path: directory.path().join("recursive.pdf"),
            mutation: BookmarkMutation::Move {
                selector: bookmark.selector.clone(),
                destination_parent: Some(bookmark.selector),
                destination_sibling_index: 0,
            },
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::InvalidBookmarkEdit)
    ));
}

#[test]
fn signed_bookmark_edits_require_explicit_invalidation_confirmation() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("signed.pdf");
    bookmark_fixture(&input, true, true);
    let bookmark = inspect_pdf(&input).unwrap().editable_bookmarks.remove(0);
    assert!(matches!(
        edit_bookmark(&EditBookmarkRequest {
            input_path: input.clone(),
            output_path: directory.path().join("unconfirmed.pdf"),
            mutation: BookmarkMutation::Update {
                selector: bookmark.selector.clone(),
                title: "Renamed".to_string(),
                target: PdfLinkTarget::InternalPage { page_number: 1 },
            },
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::SignatureConfirmationRequired)
    ));
    let confirmed = edit_bookmark(&EditBookmarkRequest {
        input_path: input,
        output_path: directory.path().join("confirmed.pdf"),
        mutation: BookmarkMutation::Update {
            selector: bookmark.selector,
            title: "Renamed".to_string(),
            target: PdfLinkTarget::InternalPage { page_number: 1 },
        },
        allow_signature_invalidation: true,
    })
    .unwrap();
    assert_eq!(
        confirmed.signature_effect,
        SignatureEffect::InvalidatesExistingSignatures
    );
}
