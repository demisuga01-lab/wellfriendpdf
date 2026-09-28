//! Source-only regressions; not compiled, rendered or executed in this change.
use super::group_tests::{fixture, mutate, request};
use super::*;
use crate::linked_stories::{
    apply_linked_story, load_linked_stories, preview_linked_story, LinkedStorySession,
};
use crate::writer::{write_incremental_update, IncrementalObject};
use crate::CancelToken;

fn reference(id: (u32, u16)) -> PdfObject {
    PdfObject::Reference {
        number: id.0,
        generation: id.1,
    }
}
fn duplicate(input: &[u8], page: usize, name: &str, private: Option<&str>) -> Vec<u8> {
    let e = ContentEngine::open_bytes(input.to_vec()).unwrap();
    let reader = e.document().reader();
    let p = e.document().get_page(page).unwrap();
    let (mut dict, mut annots) = page_annots(&e, &p).unwrap();
    let n = reader.object_ids().iter().map(|r| r.0).max().unwrap() + 1;
    let mut annotation = PdfDictionary::empty();
    annotation.insert("Type", PdfObject::Name("Annot".into()));
    annotation.insert("Subtype", PdfObject::Name("Text".into()));
    annotation.insert("NM", identity::text_string(name));
    annotation.insert("P", reference((p.object_number, p.generation_number)));
    annotation.insert("Rect", array([100.0, 20.0, 120.0, 40.0]));
    annotation.insert(
        "Contents",
        PdfObject::String(b"unrelated resident".to_vec()),
    );
    if let Some(id) = private {
        annotation.insert("WFStoryAnnotationID", identity::text_string(id));
    }
    annots.push(reference((n, 0)));
    dict.insert("Annots", PdfObject::Array(annots));
    write_incremental_update(
        reader,
        vec![
            IncrementalObject {
                number: n,
                generation: 0,
                object: PdfObject::Dictionary(annotation),
            },
            IncrementalObject {
                number: p.object_number,
                generation: p.generation_number,
                object: PdfObject::Dictionary(dict),
            },
        ],
    )
    .unwrap()
}

#[test]
fn page_local_duplicate_names_have_distinct_source_ids_and_unselected_objects_stay_unchanged() {
    let input = duplicate(&fixture(), 2, "root", None);
    let sources = annotation_anchor_sources(&input).unwrap();
    let named = sources
        .iter()
        .filter(|s| s.name.as_deref() == Some("root"))
        .collect::<Vec<_>>();
    assert_eq!(named.len(), 2);
    assert_ne!(named[0].annotation_id, named[1].annotation_id);
    let req = request(&input, "Short");
    let selected = req.annotation_anchors[0].annotation_id.clone();
    let original = ContentEngine::open_bytes(input.clone()).unwrap();
    let original_entries = inventory(&original).unwrap();
    let resident = original_entries
        .values()
        .find(|e| e.source.page == 2)
        .unwrap();
    let preview = preview_linked_story(&input, &req).unwrap();
    assert!(preview.anchor_moves.iter().all(|m| m.name_change.is_none()));
    let (output, _) = apply_linked_story(&input, &req).unwrap();
    let final_engine = ContentEngine::open_bytes(output.clone()).unwrap();
    assert_eq!(
        final_engine
            .document()
            .reader()
            .get_object(resident.reference.unwrap().0, resident.reference.unwrap().1)
            .unwrap()
            .as_dict(),
        Some(&resident.dict)
    );
    let final_entries = inventory(&final_engine).unwrap();
    assert_eq!(
        final_entries[&selected].source.name.as_deref(),
        Some("root")
    );
    let saved = load_linked_stories(&output).unwrap().remove(0).request;
    assert_eq!(saved.annotation_anchors[0].annotation_id, selected);
    let (again, _) = apply_linked_story(&output, &saved).unwrap();
    assert_eq!(
        load_linked_stories(&again).unwrap()[0]
            .request
            .annotation_anchors[0]
            .annotation_id,
        selected
    );
}

#[test]
fn name_collision_requires_explicit_receipt_bound_approval_and_preserves_the_resident() {
    let input = duplicate(&fixture(), 2, "root", None);
    let mut req = request(&input, "Short");
    req.frames[0].page = 2;
    assert!(preview_linked_story(&input, &req).is_err());
    req.annotation_anchors[0].rename_conflicting_names = true;
    let selected = req.annotation_anchors[0].annotation_id.clone();
    let mut session = LinkedStorySession::open(input.clone()).unwrap();
    let preview = session.preview(&req, &CancelToken::none()).unwrap();
    let renamed = preview
        .anchor_moves
        .iter()
        .filter(|m| m.name_change.is_some())
        .collect::<Vec<_>>();
    assert_eq!(renamed.len(), 1);
    assert_eq!(renamed[0].annotation_id, selected);
    let change = renamed[0].name_change.as_ref().unwrap().clone();
    assert_eq!(change.previous, "root");
    let receipt = session.preview_receipt().unwrap();
    let mut stale = req.clone();
    stale.annotation_anchors[0].rename_conflicting_names = false;
    assert!(session
        .checkpoint_approved(&stale, &receipt, &CancelToken::none())
        .is_err());
    assert_eq!(session.bytes(), input);
    // The internal writer also rejects a changed plan, not merely the public receipt.
    let staged = stage_identities(&input, &input, &preview.anchor_moves).unwrap();
    let mut forged = preview.anchor_moves.clone();
    forged
        .iter_mut()
        .find(|m| m.name_change.is_some())
        .unwrap()
        .name_change
        .as_mut()
        .unwrap()
        .replacement = "forged".into();
    assert!(apply_moves(&staged, &forged).is_err());
    session
        .checkpoint_approved(&req, &receipt, &CancelToken::none())
        .unwrap();
    let output = session.bytes().to_vec();
    let e = ContentEngine::open_bytes(output.clone()).unwrap();
    let entries = inventory(&e).unwrap();
    assert_eq!(
        entries[&selected].source.name.as_deref(),
        Some(change.replacement.as_str())
    );
    let names = entries
        .values()
        .filter(|e| e.source.page == 2)
        .filter_map(|e| e.source.name.as_ref())
        .collect::<Vec<_>>();
    assert_eq!(
        names.len(),
        names.iter().copied().collect::<BTreeSet<_>>().len()
    );
    assert!(entries
        .values()
        .any(|e| e.source.name.as_deref() == Some("root") && e.source.group.is_none()));
    let saved = load_linked_stories(&output).unwrap().remove(0).request;
    assert_eq!(saved.annotation_anchors[0].annotation_id, selected);
    assert!(preview_linked_story(&output, &saved)
        .unwrap()
        .anchor_moves
        .iter()
        .all(|m| m.name_change.is_none()));
    assert!(session.undo().unwrap());
    assert_eq!(session.bytes(), input);
    assert!(session.redo().unwrap());
    assert_eq!(session.bytes(), output);
}

#[test]
fn unicode_and_indirect_names_are_persisted_as_valid_pdf_text_strings() {
    let unicode = "注釈 α 😀";
    let mut input = mutate(&fixture(), "root", |d| {
        d.insert("NM", identity::text_string(unicode));
    });
    let e = ContentEngine::open_bytes(input.clone()).unwrap();
    let entries = inventory(&e).unwrap();
    let root = &entries[unicode];
    let reader = e.document().reader();
    let n = reader.object_ids().iter().map(|r| r.0).max().unwrap() + 1;
    let mut dict = root.dict.clone();
    dict.insert("NM", reference((n, 0)));
    input = write_incremental_update(
        reader,
        vec![
            IncrementalObject {
                number: n,
                generation: 0,
                object: identity::text_string(unicode),
            },
            IncrementalObject {
                number: root.reference.unwrap().0,
                generation: root.reference.unwrap().1,
                object: PdfObject::Dictionary(dict),
            },
        ],
    )
    .unwrap();
    let sources = annotation_anchor_sources(&input).unwrap();
    assert!(sources.iter().any(|s| s.annotation_id == unicode));
    let moves = sources
        .into_iter()
        .map(|s| StoryAnnotationMove {
            annotation_id: s.annotation_id,
            source_page: s.page,
            target_page: s.page,
            old_rect: s.rect,
            new_rect: s.rect,
            name_change: None,
        })
        .collect::<Vec<_>>();
    let staged = stage_identities(&input, &input, &moves).unwrap();
    let output = apply_moves(&staged, &moves).unwrap();
    let e = ContentEngine::open_bytes(output).unwrap();
    let entries = inventory(&e).unwrap();
    assert_eq!(entries[unicode].source.name.as_deref(), Some(unicode));
    assert_eq!(
        entries[unicode].dict.get("WFStoryAnnotationID"),
        Some(&identity::text_string(unicode))
    );
}

#[test]
fn raw_name_collision_cannot_steal_a_persisted_identity_and_duplicate_persisted_ids_reject() {
    let input = fixture();
    let req = request(&input, "Short");
    let (saved, _) = apply_linked_story(&input, &req).unwrap();
    let added = duplicate(&saved, 2, "root", None);
    let loaded = load_linked_stories(&added).unwrap();
    assert_eq!(
        loaded[0].request.annotation_anchors[0].annotation_id,
        "root"
    );
    let sources = annotation_anchor_sources(&added).unwrap();
    let resident = sources.iter().find(|s| s.page == 2).unwrap();
    assert_ne!(resident.annotation_id, "root");
    let conflict = duplicate(&saved, 2, "different-name", Some("root"));
    assert!(annotation_anchor_sources(&conflict).is_err());
    assert!(load_linked_stories(&conflict).is_err());
}

#[test]
fn generated_page_namespace_mapping_and_arrival_order_match_the_canonical_writer() {
    let input = duplicate(&fixture(), 2, "root", None);
    let engine = ContentEngine::open_bytes(input.clone()).unwrap();
    let entries = inventory(&engine).unwrap();
    let root = entries
        .values()
        .find(|e| e.source.page == 1 && e.source.name.as_deref() == Some("root"))
        .unwrap();
    let ids = root
        .source
        .group
        .as_ref()
        .unwrap()
        .members
        .iter()
        .map(|m| m.annotation_id.clone())
        .collect::<BTreeSet<_>>();
    let mut moves = entries
        .values()
        .filter(|e| ids.contains(&e.source.annotation_id))
        .map(|e| StoryAnnotationMove {
            annotation_id: e.source.annotation_id.clone(),
            source_page: 1,
            target_page: 2,
            old_rect: e.source.rect,
            new_rect: e.source.rect,
            name_change: None,
        })
        .collect::<Vec<_>>();
    // Insert a new page after source page 1: original page 2 is now page 3.
    identity::plan_names(&entries, &mut moves, Some((1, 1)), &[]).unwrap();
    assert!(moves.iter().all(|m| m.name_change.is_none()));
    for m in &mut moves {
        m.target_page = 3;
    }
    identity::plan_names(&entries, &mut moves, Some((1, 1)), &[]).unwrap();
    assert_eq!(moves.iter().filter(|m| m.name_change.is_some()).count(), 1);
    let staged = stage_identities(&input, &input, &moves).unwrap();
    let staged = ContentEngine::open_bytes(staged).unwrap();
    let mut blank = crate::authoring::PdfBuilder::new();
    blank.add_page(crate::authoring::PageSize::custom(300.0, 300.0));
    let blank = ContentEngine::open_bytes(blank.to_bytes().unwrap()).unwrap();
    let inserted = crate::writer::insert_authored_pages_preserving_catalog(
        staged.document(),
        &[(blank.document(), None)],
        2,
    )
    .unwrap();
    let output = apply_moves(&inserted, &moves).unwrap();
    let e = ContentEngine::open_bytes(output).unwrap();
    let now = inventory(&e).unwrap();
    assert_eq!(now[&root.source.annotation_id].source.page, 3);
    assert_ne!(
        now[&root.source.annotation_id].source.name.as_deref(),
        Some("root")
    );
}

#[test]
fn generated_repair_names_do_not_steal_a_later_arrivals_existing_name() {
    let original = fixture();
    let (saved, _) = apply_linked_story(&original, &request(&original, "Short")).unwrap();
    let input = duplicate(&saved, 2, "root", None);
    let base = format!("WFStory-{:x}", Sha256::digest(b"root"));
    let input = duplicate(&input, 1, &base, None);
    let engine = ContentEngine::open_bytes(input.clone()).unwrap();
    let entries = inventory(&engine).unwrap();
    let mut moves = entries
        .values()
        .filter(|e| e.source.page == 1)
        .map(|e| StoryAnnotationMove {
            annotation_id: e.source.annotation_id.clone(),
            source_page: 1,
            target_page: 2,
            old_rect: e.source.rect,
            new_rect: e.source.rect,
            name_change: None,
        })
        .collect::<Vec<_>>();
    identity::plan_names(&entries, &mut moves, None, &[]).unwrap();
    let root = moves.iter().find(|m| m.annotation_id == "root").unwrap();
    assert_eq!(
        root.name_change.as_ref().unwrap().replacement,
        format!("{base}-1")
    );
    assert!(moves
        .iter()
        .find(|m| m.annotation_id == base)
        .unwrap()
        .name_change
        .is_none());
    let staged = stage_identities(&input, &input, &moves).unwrap();
    let output = apply_moves(&staged, &moves).unwrap();
    let engine = ContentEngine::open_bytes(output).unwrap();
    let entries = inventory(&engine).unwrap();
    assert_eq!(entries[&base].source.name.as_deref(), Some(base.as_str()));
}
