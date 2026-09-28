//! Regression source only: not compiled or executed in this source-only phase.
use super::*;
use crate::authoring::{FontFace, PageSize, PdfBuilder, TextStyle};
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
pub(super) fn fixture() -> Vec<u8> {
    let mut builder = PdfBuilder::new();
    for _ in 0..2 {
        builder
            .add_page(PageSize::custom(300.0, 300.0))
            .draw_text(
                "OLD",
                10.0,
                120.0,
                &TextStyle::new(FontFace::BuiltinUnicode, 12.0),
            )
            .unwrap();
    }
    let engine = ContentEngine::open_bytes(builder.to_bytes().unwrap()).unwrap();
    let reader = engine.document().reader();
    let page = engine.document().get_page(1).unwrap();
    let (mut page_dict, _) = page_annots(&engine, &page).unwrap();
    let n = reader.object_ids().iter().map(|r| r.0).max().unwrap() + 1;
    let mut dictionaries = Vec::new();
    for (subtype, id, rect) in [
        ("Text", Some("root"), [20.0, 20.0, 40.0, 40.0]),
        ("Popup", None, [45.0, 20.0, 75.0, 40.0]),
        ("Text", None, [25.0, 45.0, 40.0, 55.0]),
        ("Square", Some("subordinate"), [45.0, 45.0, 60.0, 55.0]),
    ] {
        let mut d = PdfDictionary::empty();
        d.insert("Type", PdfObject::Name("Annot".into()));
        d.insert("Subtype", PdfObject::Name(subtype.into()));
        if let Some(id) = id {
            d.insert("NM", PdfObject::String(id.as_bytes().to_vec()));
        }
        d.insert("Rect", array(rect));
        d.insert("P", reference((page.object_number, page.generation_number)));
        d.insert(
            "Contents",
            PdfObject::String(b"retain this comment".to_vec()),
        );
        dictionaries.push(d);
    }
    dictionaries[0].insert("Popup", reference((n + 1, 0)));
    dictionaries[1].insert("Parent", reference((n, 0)));
    dictionaries[1].insert("Open", PdfObject::Boolean(true));
    dictionaries[2].insert("IRT", reference((n, 0)));
    dictionaries[2].insert("RT", PdfObject::Name("R".into()));
    dictionaries[2].insert("State", PdfObject::String(b"Accepted".to_vec()));
    dictionaries[2].insert("StateModel", PdfObject::String(b"Review".to_vec()));
    dictionaries[3].insert("IRT", reference((n, 0)));
    dictionaries[3].insert("RT", PdfObject::Name("Group".into()));
    let mut updates = dictionaries
        .into_iter()
        .enumerate()
        .map(|(i, d)| IncrementalObject {
            number: n + i as u32,
            generation: 0,
            object: PdfObject::Dictionary(d),
        })
        .collect::<Vec<_>>();
    page_dict.insert(
        "Annots",
        PdfObject::Array((0..4).map(|i| reference((n + i, 0))).collect()),
    );
    updates.push(IncrementalObject {
        number: page.object_number,
        generation: page.generation_number,
        object: PdfObject::Dictionary(page_dict),
    });
    write_incremental_update(reader, updates).unwrap()
}
pub(super) fn request(input: &[u8], text: &str) -> LinkedStoryRequest {
    let root = annotation_anchor_sources(input)
        .unwrap()
        .into_iter()
        .find(|a| a.name.as_deref() == Some("root") && a.page == 1)
        .unwrap();
    serde_json::from_value(serde_json::json!({
        "story_id":"annotation-group-story","input_sha256":format!("{:x}",Sha256::digest(input)),
        "frames":[{"id":"first","page":1,"logical_range":[0,3],"expected_text":"OLD","rect":[10,80,290,150],"exclusions":[]}],
        "paragraphs":[
            {"id":"body","text":text,"preferred_font":"Helvetica","font_size":12,"line_height":14},
            {"id":"tail","text":"TAIL","preferred_font":"Helvetica","font_size":12,"line_height":14}
        ],
        "fonts":[{"lookup_name":"Helvetica","bytes":crate::render::get_fallback_font("Helvetica").unwrap()}],
        "annotation_anchors":[{"annotation_id":root.annotation_id,"paragraph_id":"tail","geometry_sha256":root.geometry_sha256,"group":root.group,"offset":[0,0]}],
        "allow_font_substitution":false,"allow_page_creation":true,"max_new_pages":64,"prune_empty_pages":true
    })).unwrap()
}
fn translated(input: &[u8], target: usize) -> Vec<StoryAnnotationMove> {
    annotation_anchor_sources(input)
        .unwrap()
        .into_iter()
        .map(|a| StoryAnnotationMove {
            annotation_id: a.annotation_id,
            source_page: a.page,
            name_change: None,
            target_page: target,
            old_rect: a.rect,
            new_rect: [
                a.rect[0] + 10.0,
                a.rect[1] + 20.0,
                a.rect[2] + 10.0,
                a.rect[3] + 20.0,
            ],
        })
        .collect()
}
pub(super) fn mutate(input: &[u8], id: &str, work: impl FnOnce(&mut PdfDictionary)) -> Vec<u8> {
    let e = ContentEngine::open_bytes(input.to_vec()).unwrap();
    let entries = inventory(&e).unwrap();
    let entry = &entries[id];
    let mut d = entry.dict.clone();
    work(&mut d);
    write_incremental_update(
        e.document().reader(),
        vec![IncrementalObject {
            number: entry.reference.unwrap().0,
            generation: entry.reference.unwrap().1,
            object: PdfObject::Dictionary(d),
        }],
    )
    .unwrap()
}

#[test]
fn anonymous_popups_replies_and_group_members_move_without_changing_relationships() {
    let input = fixture();
    let moves = translated(&input, 2);
    let staged = stage_identities(&input, &input, &moves).unwrap();
    let before_engine = ContentEngine::open_bytes(staged.clone()).unwrap();
    let before = inventory(&before_engine).unwrap();
    assert_eq!(before.len(), 4);
    assert_eq!(
        before["root"].source.group.as_ref().unwrap().members.len(),
        4
    );
    let output = apply_moves(&staged, &moves).unwrap();
    let engine = ContentEngine::open_bytes(output.clone()).unwrap();
    let after = inventory(&engine).unwrap();
    for (id, old) in &before {
        let new = &after[id];
        assert_eq!(new.source.page, 2);
        assert_eq!(new.source.rect[0], old.source.rect[0] + 10.0);
        assert_eq!(new.source.rect[1], old.source.rect[1] + 20.0);
        let mut expected = old.dict.clone();
        let target = engine.document().get_page(2).unwrap();
        expected.insert(
            "P",
            reference((target.object_number, target.generation_number)),
        );
        expected.insert("Rect", array(new.source.rect));
        assert_eq!(new.dict, expected); // includes IRT/RT/Parent/Popup/State/Open/Contents
        assert_eq!(
            new.source.group.as_ref().unwrap().topology_sha256,
            old.source.group.as_ref().unwrap().topology_sha256
        );
    }
    assert!(
        page_annots(&engine, &engine.document().get_page(1).unwrap())
            .unwrap()
            .1
            .is_empty()
    );
    assert_eq!(
        page_annots(&engine, &engine.document().get_page(2).unwrap())
            .unwrap()
            .1
            .len(),
        4
    );
    assert!(output.starts_with(&staged));
    let mut before_order = before.values().collect::<Vec<_>>();
    before_order.sort_by_key(|e| e.annotation_order);
    let mut after_order = after.values().collect::<Vec<_>>();
    after_order.sort_by_key(|e| e.annotation_order);
    assert_eq!(
        before_order
            .iter()
            .map(|e| &e.source.annotation_id)
            .collect::<Vec<_>>(),
        after_order
            .iter()
            .map(|e| &e.source.annotation_id)
            .collect::<Vec<_>>()
    );
    // Canonical insertion renumbers object graphs. Private anonymous identity
    // strings and topology receipts must survive without being rediscovered.
    let mut blank = PdfBuilder::new();
    blank.add_page(PageSize::custom(300.0, 300.0));
    let blank = ContentEngine::open_bytes(blank.to_bytes().unwrap()).unwrap();
    let inserted = crate::writer::insert_authored_pages_preserving_catalog(
        engine.document(),
        &[(blank.document(), None)],
        1,
    )
    .unwrap();
    let sources = annotation_anchor_sources(&inserted).unwrap();
    for current in sources {
        assert!(before.contains_key(&current.annotation_id));
        assert_eq!(current.page, 3);
        assert_eq!(
            current.group.as_ref().unwrap().topology_sha256,
            before[&current.annotation_id]
                .source
                .group
                .as_ref()
                .unwrap()
                .topology_sha256
        );
    }
}

#[test]
fn group_requires_explicit_complete_approval_and_one_shared_destination() {
    let input = fixture();
    let mut req = request(&input, "Short");
    let approved = req.annotation_anchors[0].group.clone();
    req.annotation_anchors[0].group = None;
    assert!(preview_linked_story(&input, &req).is_err());
    req.annotation_anchors[0].group = approved;
    let omitted = req.annotation_anchors[0]
        .group
        .as_mut()
        .unwrap()
        .members
        .pop()
        .unwrap();
    assert!(preview_linked_story(&input, &req).is_err());
    req.annotation_anchors[0]
        .group
        .as_mut()
        .unwrap()
        .members
        .push(omitted);
    assert_eq!(
        preview_linked_story(&input, &req)
            .unwrap()
            .anchor_moves
            .len(),
        4
    );
    req.annotation_anchors
        .push(req.annotation_anchors[0].clone());
    assert!(preview_linked_story(&input, &req).is_err());
    let mut moves = translated(&input, 2);
    let staged = stage_identities(&input, &input, &moves).unwrap();
    assert!(apply_moves(&staged, &moves[..3]).is_err());
    moves[0].target_page = 1;
    assert!(apply_moves(&staged, &moves).is_err());
    moves[0].target_page = 2;
    moves[0].new_rect[0] += 1.0;
    moves[0].new_rect[2] += 1.0;
    assert!(apply_moves(&staged, &moves).is_err());
}

#[test]
fn growth_contraction_pruning_save_reopen_and_history_preserve_entire_group() {
    let input = fixture();
    let req = request(&input, &"Long source paragraph text. ".repeat(65));
    let ids = annotation_anchor_sources(&input)
        .unwrap()
        .into_iter()
        .map(|a| a.annotation_id)
        .collect::<BTreeSet<_>>();
    let mut session = LinkedStorySession::open(input.clone()).unwrap();
    let preview = session.preview(&req, &CancelToken::none()).unwrap();
    assert!(preview.generated_pages > 1);
    assert_eq!(preview.anchor_moves.len(), 4);
    let receipt = session.preview_receipt().unwrap();
    let cancelled = CancelToken::new();
    cancelled.cancel();
    assert!(session
        .checkpoint_approved(&req, &receipt, &cancelled)
        .is_err());
    assert_eq!(session.bytes(), input);
    session
        .checkpoint_approved(&req, &receipt, &CancelToken::none())
        .unwrap();
    let grown = session.bytes().to_vec();
    let mut saved = load_linked_stories(&grown).unwrap().remove(0).request;
    assert_eq!(
        saved.annotation_anchors[0]
            .group
            .as_ref()
            .unwrap()
            .members
            .iter()
            .map(|m| m.annotation_id.clone())
            .collect::<BTreeSet<_>>(),
        ids
    );
    assert!(saved.annotation_anchors[0]
        .group
        .as_ref()
        .unwrap()
        .members
        .iter()
        .all(|m| m.page > 1));
    saved.paragraphs[0].text = "Short".into();
    let preview = session.preview(&saved, &CancelToken::none()).unwrap();
    assert!(!preview.page_pruning.removed_pages.is_empty());
    assert!(preview.anchor_moves.iter().all(|m| m.target_page == 1));
    let receipt = session.preview_receipt().unwrap();
    session
        .checkpoint_approved(&saved, &receipt, &CancelToken::none())
        .unwrap();
    let short = session.bytes().to_vec();
    let e = ContentEngine::open_bytes(short.clone()).unwrap();
    assert_eq!(e.page_count().unwrap(), 2);
    assert!(e.get_page_text(2).unwrap().contains("OLD"));
    assert!(inventory(&e).unwrap().values().all(|a| a.source.page == 1));
    let saved = load_linked_stories(&short).unwrap().remove(0).request;
    let (again, _) = apply_linked_story(&short, &saved).unwrap();
    assert_eq!(
        load_linked_stories(&again).unwrap()[0]
            .request
            .annotation_anchors[0]
            .group,
        saved.annotation_anchors[0].group
    );
    assert!(session.undo().unwrap());
    assert_eq!(session.bytes(), grown);
    assert!(session.redo().unwrap());
    assert_eq!(session.bytes(), short);
}

#[test]
fn saved_group_rejects_external_topology_or_member_geometry_changes() {
    let input = fixture();
    let req = request(&input, "Short");
    let (saved, _) = apply_linked_story(&input, &req).unwrap();
    let source = ContentEngine::open_bytes(saved.clone()).unwrap();
    let mut blank = PdfBuilder::new();
    blank.add_page(PageSize::custom(300.0, 300.0));
    let blank = ContentEngine::open_bytes(blank.to_bytes().unwrap()).unwrap();
    let shifted = crate::writer::insert_authored_pages_preserving_catalog(
        source.document(),
        &[(blank.document(), None)],
        1,
    )
    .unwrap();
    let rebound = load_linked_stories(&shifted).unwrap().remove(0).request;
    assert_eq!(rebound.frames[0].page, 2);
    assert!(rebound.annotation_anchors[0]
        .group
        .as_ref()
        .unwrap()
        .members
        .iter()
        .all(|m| m.page == 2));
    preview_linked_story(&shifted, &rebound).unwrap();
    let changed = mutate(&saved, "subordinate", |d| {
        d.insert("RT", PdfObject::Name("R".into()));
    });
    assert!(load_linked_stories(&changed).is_err());
    let changed = mutate(&saved, "subordinate", |d| {
        d.insert("Rect", array([20.0, 20.0, 35.0, 30.0]));
    });
    assert!(load_linked_stories(&changed).is_err());
    let mut another = request(&saved, "Other story");
    another.story_id = "other-story".into();
    another.frames[0].page = 2;
    assert!(preview_linked_story(&saved, &another).is_err());
    let engine = ContentEngine::open_bytes(saved.clone()).unwrap();
    let page = engine.document().get_page(1).unwrap();
    let (mut dict, mut annots) = page_annots(&engine, &page).unwrap();
    annots.reverse();
    dict.insert("Annots", PdfObject::Array(annots));
    let reordered = write_incremental_update(
        engine.document().reader(),
        vec![IncrementalObject {
            number: page.object_number,
            generation: page.generation_number,
            object: PdfObject::Dictionary(dict),
        }],
    )
    .unwrap();
    assert!(load_linked_stories(&reordered).is_err());
}

#[test]
fn cycles_dangling_relations_and_split_ownership_are_rejected_but_optional_parent_is_valid() {
    let input = fixture();
    let e = ContentEngine::open_bytes(input.clone()).unwrap();
    let entries = inventory(&e).unwrap();
    let popup = entries
        .values()
        .find(|e| e.source.subtype == "Popup")
        .unwrap();
    let optional = mutate(&input, &popup.source.annotation_id, |d| {
        d.remove("Parent");
    });
    assert_eq!(annotation_anchor_sources(&optional).unwrap().len(), 4);
    let subordinate = entries["subordinate"].reference.unwrap();
    let reply = mutate(&input, "subordinate", |d| {
        d.insert("RT", PdfObject::Name("R".into()));
    });
    let cycle = mutate(&reply, "root", |d| {
        d.insert("IRT", reference(subordinate));
    });
    assert!(annotation_anchor_sources(&cycle).is_err());
    let dangling = mutate(&input, "root", |d| {
        d.insert("Popup", reference((999999, 0)));
    });
    assert!(annotation_anchor_sources(&dangling).is_err());
    let wrong_parent = mutate(&input, &popup.source.annotation_id, |d| {
        d.insert("Parent", reference(subordinate));
    });
    assert!(annotation_anchor_sources(&wrong_parent).is_err());
}
