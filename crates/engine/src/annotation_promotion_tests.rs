//! Source regressions only. No tests/builds/PDF workloads were executed.
use super::*;
use crate::annotation_media_redaction::*;
use crate::authoring::{PageSize, PdfBuilder};
use crate::ContentEngine;

fn fixture() -> Vec<u8> {
    let mut b = PdfBuilder::new();
    b.add_page(PageSize::custom(200.0, 200.0));
    b.add_page(PageSize::custom(200.0, 200.0));
    let d = PdfDocument::open_bytes(b.to_bytes().unwrap()).unwrap();
    let p = d.get_page(1).unwrap();
    let r = d.reader();
    let next = r.object_ids().iter().map(|r| r.0).max().unwrap() + 1;
    let mut note = PdfDictionary::empty();
    note.insert("Type", PdfObject::Name("Annot".into()));
    note.insert("Subtype", PdfObject::Name("Text".into()));
    note.insert("NM", annotation_identity::text_string("same 注釈"));
    note.insert("Contents", annotation_identity::text_string("original"));
    note.insert(
        "Rect",
        PdfObject::Array(
            vec![10, 20, 30, 40]
                .into_iter()
                .map(PdfObject::Integer)
                .collect(),
        ),
    );
    note.insert("P", reference((p.object_number, p.generation_number)));
    note.insert(
        "WFOpaque",
        PdfObject::Array(vec![PdfObject::Null, PdfObject::Boolean(true)]),
    );
    let mut action = PdfDictionary::empty();
    action.insert("S", PdfObject::Name("URI".into()));
    action.insert(
        "URI",
        PdfObject::String(b"https://example.invalid/source".to_vec()),
    );
    note.insert("A", PdfObject::Dictionary(action));
    let mut ap = PdfDictionary::empty();
    ap.insert("N", reference((next + 1, 0)));
    note.insert("AP", PdfObject::Dictionary(ap));
    let mut form = PdfDictionary::empty();
    form.insert("Type", PdfObject::Name("XObject".into()));
    form.insert("Subtype", PdfObject::Name("Form".into()));
    form.insert(
        "BBox",
        PdfObject::Array(
            vec![0, 0, 20, 20]
                .into_iter()
                .map(PdfObject::Integer)
                .collect(),
        ),
    );
    form.insert("Resources", PdfObject::Dictionary(PdfDictionary::empty()));
    let mut indirect = note.clone();
    indirect.remove("NM");
    let (mut page, _) = annots(r, (p.object_number, p.generation_number)).unwrap();
    page.insert(
        "Annots",
        PdfObject::Array(vec![
            PdfObject::Dictionary(note.clone()),
            PdfObject::Dictionary(note),
            reference((next, 0)),
        ]),
    );
    write_incremental_update(
        r,
        vec![
            IncrementalObject {
                number: p.object_number,
                generation: p.generation_number,
                object: PdfObject::Dictionary(page),
            },
            IncrementalObject {
                number: next,
                generation: 0,
                object: PdfObject::Dictionary(indirect),
            },
            IncrementalObject {
                number: next + 1,
                generation: 0,
                object: PdfObject::Stream {
                    dict: form,
                    raw: b"0 0 m 20 20 l S\n".to_vec(),
                },
            },
        ],
    )
    .unwrap()
}
fn ids(input: &[u8]) -> IdentityIndex {
    annotation_identity::index(&PdfDocument::open_bytes(input.to_vec()).unwrap(), 100_000).unwrap()
}
fn dictionary(input: &[u8], slot: usize) -> PdfDictionary {
    let doc = PdfDocument::open_bytes(input.to_vec()).unwrap();
    let p = doc.get_page(1).unwrap();
    let (_, entries) = annots(doc.reader(), (p.object_number, p.generation_number)).unwrap();
    doc.reader()
        .resolve(entries[slot].clone())
        .unwrap()
        .as_dict()
        .unwrap()
        .clone()
}
fn change_direct(input: &[u8], slot: usize, change: impl FnOnce(&mut PdfDictionary)) -> Vec<u8> {
    let doc = PdfDocument::open_bytes(input.to_vec()).unwrap();
    let p = doc.get_page(1).unwrap();
    let (mut page, mut entries) =
        annots(doc.reader(), (p.object_number, p.generation_number)).unwrap();
    let mut dict = entries[slot].as_dict().unwrap().clone();
    change(&mut dict);
    entries[slot] = PdfObject::Dictionary(dict);
    page.insert("Annots", PdfObject::Array(entries));
    write_incremental_update(
        doc.reader(),
        vec![IncrementalObject {
            number: p.object_number,
            generation: p.generation_number,
            object: PdfObject::Dictionary(page),
        }],
    )
    .unwrap()
}

#[test]
fn equal_direct_dictionaries_are_selected_by_slot_and_only_selected_slot_is_promoted() {
    let input = fixture();
    let before = ids(&input);
    let id = before[&(1, 0)].id.clone();
    assert_ne!(id, before[&(1, 1)].id);
    let (output, report) =
        promote_annotation_sources_pdf(&input, &digest(&input), &[id.clone()]).unwrap();
    let after = ids(&output);
    assert_eq!(after.len(), 3);
    assert_eq!(after[&(1, 0)].id, id);
    assert!(after[&(1, 0)].reference.is_some());
    assert!(after[&(1, 1)].reference.is_none());
    assert_eq!(before[&(1, 2)].reference, after[&(1, 2)].reference);
    let mut expected = dictionary(&input, 0);
    expected.insert(
        annotation_identity::STABLE_ID,
        annotation_identity::text_string(&id),
    );
    assert_eq!(dictionary(&output, 0), expected);
    assert_eq!(dictionary(&output, 1), dictionary(&input, 1));
    assert_eq!(report.promoted_ids, vec![id.clone()]);
    assert!(report.source_order_verified && report.relationship_graph_verified);
    let (again, next) = promote_annotation_sources_pdf(&output, &digest(&output), &[id]).unwrap();
    assert_eq!(again, output);
    assert!(next.promoted_ids.is_empty());
}

#[test]
fn native_batch_binds_both_direct_and_anonymous_indirect_sources_before_revision_change() {
    let input = fixture();
    let before = ids(&input);
    let direct = before[&(1, 0)].id.clone();
    let indirect = before[&(1, 2)].id.clone();
    let changes = vec![
        AnnotationGeometryChange {
            annotation_id: direct.clone(),
            page: 1,
            rect: [30.0, 40.0, 60.0, 80.0],
        },
        AnnotationGeometryChange {
            annotation_id: indirect.clone(),
            page: 2,
            rect: [40.0, 50.0, 60.0, 70.0],
        },
    ];
    assert!(edit_annotation_geometries_pdf(&input, Some("stale"), &changes).is_err());
    let (output, report) =
        edit_annotation_geometries_pdf(&input, Some(&digest(&input)), &changes).unwrap();
    let after = ids(&output);
    assert_eq!(report.input_sha256, digest(&input));
    assert_eq!(report.output_sha256, digest(&output));
    assert_eq!(report.promoted_source_ids, vec![direct.clone()]);
    assert!(after.values().any(|i| i.id == indirect && i.page == 2));
    let selected = after.values().find(|i| i.id == direct).unwrap();
    assert!(selected.reference.is_some());
    let source = dictionary(&input, 0);
    let target = dictionary(&output, 0);
    assert_eq!(source.get("A"), target.get("A"));
    assert_eq!(source.get("AP"), target.get("AP"));
    assert_eq!(dictionary(&output, 1).get("AP"), target.get("AP"));
    assert_eq!(source.get("WFOpaque"), target.get("WFOpaque"));
    assert_eq!(source.get("NM"), target.get("NM"));
}

#[test]
fn xfdf_can_update_one_duplicate_direct_note_delete_another_and_create_a_reply_atomically() {
    let input = fixture();
    let before = ids(&input);
    let parent_id = before[&(1, 0)].id.clone();
    let removed_id = before[&(1, 1)].id.clone();
    let indirect_id = before[&(1, 2)].id.clone();
    let engine = ContentEngine::open_bytes(input.clone()).unwrap();
    let (xml, _) = export_annotation_xfdf(&engine).unwrap();
    let mut document = parse_annotation_xfdf(&xml).unwrap();
    document.annotations.retain(|r| r.id != removed_id);
    for record in &mut document.annotations {
        record.contents = Some("changed".into());
    }
    document.annotations.push(AnnotationXfdfRecord {
        id: "new-reply".into(),
        subtype: "Text".into(),
        page: 1,
        rect: Some([70.0, 20.0, 90.0, 40.0]),
        reply_to: Some(parent_id.clone()),
        reply_type: Some("R".into()),
        ..Default::default()
    });
    let (output, report) = import_annotation_xfdf_pdf(
        &input,
        write_annotation_xfdf(&document).as_bytes(),
        &AnnotationXfdfImportOptions {
            appearance_policy: AnnotationAppearancePolicy::PreserveValid,
            delete_policy: AnnotationDeletePolicy::ExplicitIds,
            delete_ids: vec![removed_id.clone()],
            ..Default::default()
        },
    )
    .unwrap();
    let doc = PdfDocument::open_bytes(output).unwrap();
    let after = annotation_identity::index(&doc, 100_000).unwrap();
    let graph = Graph::read(&doc, &after).unwrap();
    assert_eq!(graph.nodes.len(), 3);
    assert!(!graph.nodes.contains_key(&removed_id));
    assert!(graph.nodes.contains_key(&indirect_id));
    assert_eq!(
        graph.nodes["new-reply"].reply_to.as_deref(),
        Some(parent_id.as_str())
    );
    assert!(graph.nodes[&parent_id].reference.is_some());
    assert_eq!((report.created, report.updated, report.deleted), (1, 2, 1));
    assert_eq!(report.relationship_transaction.promoted_source_ids.len(), 2);
    assert!(report.relationship_transaction.output_graph_verified);
}

#[test]
fn staging_accepts_unrelated_revision_but_rejects_mutated_selected_occurrence() {
    let input = fixture();
    let selected = BTreeSet::from([ids(&input)[&(1, 0)].id.clone()]);
    let doc = PdfDocument::open_bytes(input.clone()).unwrap();
    let root = doc.reader().root_reference().unwrap();
    let mut catalog = doc.get_catalog().unwrap();
    catalog.insert("WFOpaque", PdfObject::Integer(42));
    let staged = write_incremental_update(
        doc.reader(),
        vec![IncrementalObject {
            number: root.0,
            generation: root.1,
            object: PdfObject::Dictionary(catalog),
        }],
    )
    .unwrap();
    let (output, _) = stage(&input, &staged, &selected, 100_000).unwrap();
    assert_eq!(
        PdfDocument::open_bytes(output)
            .unwrap()
            .get_catalog()
            .unwrap()
            .get("WFOpaque"),
        Some(&PdfObject::Integer(42))
    );
    let changed = change_direct(&input, 0, |d| {
        d.insert("Contents", annotation_identity::text_string("different"));
    });
    assert!(stage(&input, &changed, &selected, 100_000).is_err());
}

#[test]
fn inferred_popup_owner_and_indirect_neighbor_keep_their_ids_and_optional_parent() {
    let input = fixture();
    let doc = PdfDocument::open_bytes(input.clone()).unwrap();
    let next = doc.reader().object_ids().iter().map(|r| r.0).max().unwrap() + 1;
    let page = doc.get_page(1).unwrap();
    let (mut page_dict, mut entries) =
        annots(doc.reader(), (page.object_number, page.generation_number)).unwrap();
    let mut owner = entries[0].as_dict().unwrap().clone();
    owner.insert("Popup", reference((next, 0)));
    entries[0] = PdfObject::Dictionary(owner);
    let mut popup = dictionary(&input, 0);
    popup.remove("NM");
    popup.insert("Subtype", PdfObject::Name("Popup".into()));
    entries.push(reference((next, 0)));
    page_dict.insert("Annots", PdfObject::Array(entries));
    let input = write_incremental_update(
        doc.reader(),
        vec![
            IncrementalObject {
                number: page.object_number,
                generation: page.generation_number,
                object: PdfObject::Dictionary(page_dict),
            },
            IncrementalObject {
                number: next,
                generation: 0,
                object: PdfObject::Dictionary(popup),
            },
        ],
    )
    .unwrap();
    let before = ids(&input);
    let owner_id = before[&(1, 0)].id.clone();
    let popup_id = before[&(1, 3)].id.clone();
    let (output, report) =
        promote_annotation_sources_pdf(&input, &digest(&input), &[owner_id.clone()]).unwrap();
    assert_eq!(report.persisted_ids.len(), 2);
    let doc = PdfDocument::open_bytes(output.clone()).unwrap();
    let graph = Graph::read(&doc, &ids(&output)).unwrap();
    assert_eq!(
        graph.nodes[&popup_id].parent.as_deref(),
        Some(owner_id.as_str())
    );
    assert!(!graph.nodes[&popup_id].parent_explicit);
    assert_eq!(
        graph.nodes[&owner_id].popup.as_deref(),
        Some(popup_id.as_str())
    );
    assert!(dictionary(&output, 3).get("Parent").is_none());
}

#[test]
fn direct_widget_and_tagged_sources_are_not_orphaned_by_unmapped_owner_promotion() {
    for input in [
        change_direct(&fixture(), 0, |d| {
            d.insert("Subtype", PdfObject::Name("Widget".into()));
        }),
        change_direct(&fixture(), 0, |d| {
            d.insert("StructParent", PdfObject::Integer(7));
        }),
    ] {
        let id = ids(&input)[&(1, 0)].id.clone();
        let error = promote_annotation_sources_pdf(&input, &digest(&input), &[id]).unwrap_err();
        assert!(
            error.to_string().contains("owner mapping")
                || error.to_string().contains("StructTreeRoot")
        );
    }
}

#[test]
fn native_story_discovery_staging_and_cross_page_translation_accept_direct_occurrences() {
    let input = fixture();
    let source = crate::story_anchors::annotation_anchor_sources(&input).unwrap();
    assert_eq!(source.len(), 3);
    let id = ids(&input)[&(1, 0)].id.clone();
    let source = source.iter().find(|s| s.annotation_id == id).unwrap();
    let movement = crate::story_anchors::StoryAnnotationMove {
        annotation_id: id.clone(),
        source_page: 1,
        target_page: 2,
        old_rect: source.rect,
        new_rect: [50.0, 60.0, 70.0, 80.0],
        name_change: None,
    };
    let staged =
        crate::story_anchors::stage_identities(&input, &input, &[movement.clone()]).unwrap();
    let output = crate::story_anchors::apply_moves(&staged, &[movement]).unwrap();
    let after = crate::story_anchors::annotation_anchor_sources(&output).unwrap();
    let moved = after.iter().find(|s| s.annotation_id == id).unwrap();
    assert_eq!(moved.page, 2);
    assert_eq!(moved.rect, [50.0, 60.0, 70.0, 80.0]);
}

#[test]
fn source_promotion_routes_through_the_shared_document_subsystem() {
    use crate::document_subsystems::*;
    let input = fixture();
    let id = ids(&input)[&(1, 0)].id.clone();
    let action = DocumentSubsystemsAction::AnnotationPromoteSources {
        source_sha256: digest(&input),
        annotation_ids: vec![id.clone()],
    };
    let encoded = serde_json::to_value(&action).unwrap();
    assert_eq!(encoded["kind"], "annotation_promote_sources");
    let request = DocumentSubsystemsRequest {
        subsystem: DocumentSubsystemsSubsystem::AnnotationAppearance,
        action: Some(serde_json::from_value(encoded).unwrap()),
        reflow: None,
        approved: true,
        form_data: None,
        form_data_format: None,
        use_semantic_document_flow: false,
    };
    let (output, report) = apply_document_subsystems(&input, &request).unwrap();
    assert_eq!(report.operation, "annotation_exact_source_materialization");
    assert_eq!(ids(&output)[&(1, 0)].id, id);
    assert!(ids(&output)[&(1, 0)].reference.is_some());
    let (expected, _) = promote_annotation_sources_pdf(&input, &digest(&input), &[id]).unwrap();
    assert_eq!(output, expected);
}
