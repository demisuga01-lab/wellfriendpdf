//! Regression source only; no builds/tests/PDF workloads authorized in this phase.
use super::*;
use crate::annotation_relationships::Graph;
use crate::authoring::{PageSize, PdfBuilder};
use crate::writer::{write_incremental_update, IncrementalObject};

fn blank() -> Vec<u8> {
    let mut b = PdfBuilder::new();
    b.add_page(PageSize::custom(200.0, 200.0));
    b.add_page(PageSize::custom(200.0, 200.0));
    b.to_bytes().unwrap()
}
fn record(id: &str, subtype: &str) -> AnnotationXfdfRecord {
    AnnotationXfdfRecord {
        id: id.into(),
        subtype: subtype.into(),
        page: 1,
        rect: Some([10.0, 20.0, 30.0, 40.0]),
        contents: Some(format!("contents-{id}")),
        ..Default::default()
    }
}
fn import(
    input: &[u8],
    records: Vec<AnnotationXfdfRecord>,
    deletes: &[&str],
) -> Result<(Vec<u8>, AnnotationXfdfImportReport)> {
    let document = AnnotationXfdfDocument {
        source_sha256: Some(resource_digest(input)),
        annotations: records,
        ..Default::default()
    };
    import_annotation_xfdf_pdf(
        input,
        write_annotation_xfdf(&document).as_bytes(),
        &AnnotationXfdfImportOptions {
            appearance_policy: AnnotationAppearancePolicy::PreserveValid,
            delete_policy: AnnotationDeletePolicy::ExplicitIds,
            delete_ids: deletes.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        },
    )
}
fn records(input: &[u8]) -> Vec<AnnotationXfdfRecord> {
    annotation_xfdf_document(&PdfDocument::open_bytes(input.to_vec()).unwrap())
        .unwrap()
        .annotations
}
fn graph(input: &[u8]) -> Graph {
    let doc = PdfDocument::open_bytes(input.to_vec()).unwrap();
    Graph::read(
        &doc,
        &crate::annotation_identity::index(&doc, MAX_ANNOTATIONS).unwrap(),
    )
    .unwrap()
}
fn fixture() -> Vec<u8> {
    let a = record("A", "Text");
    let b = record("B", "Text");
    let mut popup = record("P", "Popup");
    popup.popup_for = Some("A".into());
    let mut reply = record("R", "Text");
    reply.reply_to = Some("A".into());
    let mut nested = record("S", "Text");
    nested.reply_to = Some("R".into());
    import(&blank(), vec![nested, popup, b, reply, a], &[])
        .unwrap()
        .0
}
fn change(input: &[u8], id: &str, edit: impl FnOnce(&mut PdfDictionary)) -> Vec<u8> {
    let before = graph(input);
    let reference = before.nodes[id].reference.unwrap();
    let doc = PdfDocument::open_bytes(input.to_vec()).unwrap();
    let reader = doc.reader();
    let mut dict = reader
        .get_object(reference.0, reference.1)
        .unwrap()
        .as_dict()
        .unwrap()
        .clone();
    edit(&mut dict);
    write_incremental_update(
        reader,
        vec![IncrementalObject {
            number: reference.0,
            generation: reference.1,
            object: PdfObject::Dictionary(dict),
        }],
    )
    .unwrap()
}
fn dictionary(input: &[u8], id: &str) -> PdfDictionary {
    let g = graph(input);
    let reference = g.nodes[id].reference.unwrap();
    let doc = PdfDocument::open_bytes(input.to_vec()).unwrap();
    doc.reader()
        .get_object(reference.0, reference.1)
        .unwrap()
        .as_dict()
        .unwrap()
        .clone()
}

#[test]
fn popup_creation_and_reparenting_update_both_owners_without_sanitizing_implicit_updates() {
    let input = fixture();
    let g = graph(&input);
    assert_eq!(g.nodes["A"].popup.as_deref(), Some("P"));
    assert_eq!(g.nodes["P"].parent.as_deref(), Some("A"));
    assert_eq!(g.nodes["S"].reply_to.as_deref(), Some("R"));
    let input = change(&input, "B", |d| {
        let mut action = PdfDictionary::empty();
        action.insert("S", PdfObject::Name("URI".into()));
        action.insert("URI", pdf_text_string("https://example.invalid/preserved"));
        d.insert("A", PdfObject::Dictionary(action));
        d.insert("VendorOpaque", pdf_text_string("unchanged"));
    });
    let old_b = dictionary(&input, "B");
    let old_a = dictionary(&input, "A");
    let mut popup = records(&input).into_iter().find(|r| r.id == "P").unwrap();
    popup.popup_for = Some("B".into());
    let (output, report) = import(&input, vec![popup], &[]).unwrap();
    assert!(report.relationship_transaction.output_graph_verified);
    assert!(report
        .relationship_transaction
        .reciprocal_only_updates
        .contains(&"A".into()));
    assert!(report
        .relationship_transaction
        .reciprocal_only_updates
        .contains(&"B".into()));
    let g = graph(&output);
    assert!(g.nodes["A"].popup.is_none());
    assert_eq!(g.nodes["B"].popup.as_deref(), Some("P"));
    for key in ["A", "VendorOpaque", "Contents"] {
        assert_eq!(dictionary(&output, "B").get(key), old_b.get(key));
    }
    assert_eq!(
        dictionary(&output, "A").get("Contents"),
        old_a.get("Contents")
    );
    let (again, report) = import(&output, records(&output), &[]).unwrap();
    assert_eq!(graph(&again).nodes["P"].parent.as_deref(), Some("B"));
    assert!(report.relationship_transaction.output_graph_verified);
}

#[test]
fn deleting_popup_clears_backlink_and_owner_deletion_needs_explicit_dependents() {
    let input = fixture();
    assert!(import(&input, Vec::new(), &["A"]).is_err());
    let (output, report) = import(&input, Vec::new(), &["P"]).unwrap();
    let g = graph(&output);
    assert!(!g.nodes.contains_key("P"));
    assert!(g.nodes["A"].popup.is_none());
    assert!(report
        .relationship_transaction
        .changes
        .iter()
        .any(|c| c.annotation_id == "A" && c.key == "Popup" && c.replacement.is_none()));
    assert!(import(&output, Vec::new(), &["A", "R"]).is_err());
    let (output, report) = import(&output, Vec::new(), &["A", "R", "S"]).unwrap();
    assert_eq!(report.deleted, 3);
    assert_eq!(
        graph(&output).nodes.keys().cloned().collect::<Vec<_>>(),
        vec!["B".to_string()]
    );
}

#[test]
fn reparent_and_delete_are_atomic_and_invalid_reply_graphs_reject() {
    let input = fixture();
    let mut reply = records(&input).into_iter().find(|r| r.id == "R").unwrap();
    reply.reply_to = Some("B".into());
    let (output, _) = import(&input, vec![reply], &["A", "P"]).unwrap();
    let g = graph(&output);
    assert_eq!(g.nodes["R"].reply_to.as_deref(), Some("B"));
    assert_eq!(g.nodes["S"].reply_to.as_deref(), Some("R"));
    let mut reply = records(&input).into_iter().find(|r| r.id == "R").unwrap();
    for target in ["R", "S", "missing"] {
        reply.reply_to = Some(target.into());
        assert!(import(&input, vec![reply.clone()], &[]).is_err());
    }
    let mut nested = records(&input).into_iter().find(|r| r.id == "S").unwrap();
    nested.reply_type = Some("Group".into());
    assert!(import(&input, vec![nested.clone()], &[]).is_err());
    nested.reply_to = Some("A".into());
    import(&input, vec![nested.clone()], &[]).unwrap();
    nested.reply_to = None;
    assert!(import(&input, vec![nested], &[]).is_err());
    let mut extra = record("Q", "Popup");
    extra.popup_for = Some("A".into());
    assert!(import(&input, vec![extra], &[]).is_err());
    let mut wrong = record("A", "Link");
    assert!(import(&input, vec![wrong.clone()], &[]).is_err());
    wrong.id = "new-link".into();
    wrong.reply_to = Some("A".into());
    assert!(import(&input, vec![wrong], &[]).is_err());
}

#[test]
fn cross_page_import_requires_closed_relationships_and_preserves_source_order() {
    let input = fixture();
    let mut moving = records(&input)
        .into_iter()
        .filter(|r| r.id != "B")
        .collect::<Vec<_>>();
    for record in &mut moving {
        record.page = 2;
    }
    assert!(import(
        &input,
        vec![moving.iter().find(|r| r.id == "A").unwrap().clone()],
        &[]
    )
    .is_err());
    moving.reverse();
    let before = graph(&input);
    let (output, report) = import(&input, moving, &[]).unwrap();
    assert!(report.relationship_transaction.output_graph_verified);
    let after = graph(&output);
    assert_eq!(after.nodes["B"].page, 1);
    let order = |g: &Graph, page: usize| {
        let mut nodes = g
            .nodes
            .iter()
            .filter(|(id, n)| id.as_str() != "B" && n.page == page)
            .collect::<Vec<_>>();
        nodes.sort_by_key(|(_, n)| n.order);
        nodes
            .into_iter()
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(order(&before, 1), order(&after, 2));
    let source = crate::story_anchors::annotation_anchor_sources(&output).unwrap();
    assert_eq!(
        source
            .iter()
            .find(|s| s.annotation_id == "A")
            .unwrap()
            .group
            .as_ref()
            .unwrap()
            .members
            .len(),
        4
    );
}

#[test]
fn optional_and_absent_popup_parent_are_valid_without_inventing_an_owner() {
    let input = change(&fixture(), "P", |d| {
        d.remove("Parent");
    });
    let original = graph(&input);
    assert_eq!(original.nodes["P"].parent.as_deref(), Some("A"));
    assert!(!original.nodes["P"].parent_explicit);
    let (output, _) = import(&input, records(&input), &[]).unwrap();
    assert!(!graph(&output).nodes["P"].parent_explicit);
    let (output, _) = import(&output, vec![record("Loose", "Popup")], &[]).unwrap();
    assert!(graph(&output).nodes["Loose"].parent.is_none());
    let (output, _) =
        move_resize_annotation_pdf(&output, "Loose", 2, [20.0, 30.0, 40.0, 50.0]).unwrap();
    let g = graph(&output);
    assert_eq!(g.nodes["Loose"].page, 2);
    assert!(g.nodes["Loose"].parent.is_none());
}

#[test]
fn duplicate_import_records_are_not_order_dependent_topology_decisions() {
    let input = fixture();
    let mut first = record("R", "Text");
    first.reply_to = Some("A".into());
    let mut second = first.clone();
    second.reply_to = Some("B".into());
    assert!(import(&input, vec![first, second], &[]).is_err());
}

#[test]
fn safe_merge_preserves_omitted_relations_and_explicit_clear_and_detach_are_serialized() {
    let input = fixture();
    let (output, _) = import(&input, vec![record("R", "Text")], &[]).unwrap();
    assert_eq!(graph(&output).nodes["R"].reply_to.as_deref(), Some("A"));
    let mut clear = record("R", "Text");
    clear.clear_reply = true;
    let mut detach = record("P", "Popup");
    detach.detach_popup = true;
    let (output, report) = import(&input, vec![clear.clone(), detach], &[]).unwrap();
    let g = graph(&output);
    assert!(g.nodes["R"].reply_to.is_none());
    assert!(g.nodes["P"].parent.is_none());
    assert!(g.nodes["A"].popup.is_none());
    assert_eq!(g.nodes["S"].reply_to.as_deref(), Some("R"));
    assert!(report
        .relationship_transaction
        .changes
        .iter()
        .any(|c| c.annotation_id == "P" && c.key == "Parent" && c.replacement.is_none()));
    clear.reply_to = Some("A".into());
    assert!(import(&input, vec![clear], &[]).is_err());
    let xmlns = WELLFRIENDPDF_XFDF_NAMESPACE;
    let xfdf=format!("<xfdf xmlns='{XFDF_NAMESPACE}' xmlns:edit='{xmlns}'><annots><text name='R' page='0' rect='10,20,30,40' edit:clear-reply='true'/></annots></xfdf>");
    assert!(parse_annotation_xfdf(xfdf.as_bytes()).unwrap().annotations[0].clear_reply);
    assert!(parse_annotation_xfdf(
        xfdf.replace("clear-reply='true'", "clear-reply='perhaps'")
            .as_bytes()
    )
    .is_err());
}

#[test]
fn widget_parent_is_not_popup_ownership_and_xfdf_cannot_orphan_the_field_tree() {
    let input = change(&fixture(), "B", |d| {
        d.insert("Subtype", PdfObject::Name("Widget".into()));
        d.insert("FT", PdfObject::Name("Tx".into()));
        d.insert("T", pdf_text_string("field"));
        d.insert("V", pdf_text_string("retain"));
    });
    let doc = PdfDocument::open_bytes(input.clone()).unwrap();
    let widget = graph(&input).nodes["B"].reference.unwrap();
    let mut form = PdfDictionary::empty();
    form.insert(
        "Fields",
        PdfObject::Array(vec![PdfObject::Reference {
            number: widget.0,
            generation: widget.1,
        }]),
    );
    let mut catalog = doc.get_catalog().unwrap();
    catalog.insert("AcroForm", PdfObject::Dictionary(form));
    let root = doc.reader().root_reference().unwrap();
    let input = write_incremental_update(
        doc.reader(),
        vec![IncrementalObject {
            number: root.0,
            generation: root.1,
            object: PdfObject::Dictionary(catalog),
        }],
    )
    .unwrap();
    assert!(graph(&input).nodes["B"].parent.is_none());
    assert!(import(&input, Vec::new(), &["B"]).is_err());
    // A scalar widget update stays within its original annotation/field object.
    let mut update = records(&input).into_iter().find(|r| r.id == "B").unwrap();
    update.contents = Some("new tooltip".into());
    let (output, _) = import(&input, vec![update], &[]).unwrap();
    assert_eq!(
        dictionary(&output, "B").get("V"),
        Some(&pdf_text_string("retain"))
    );
    assert_eq!(dictionary(&output, "B").get_name("FT"), Some("Tx"));
}

#[test]
fn large_reply_chain_uses_bounded_global_cycle_analysis() {
    let mut records = Vec::new();
    for i in 0..400 {
        let mut row = record(&format!("n{i:04}"), "Text");
        if i > 0 {
            row.reply_to = Some(format!("n{:04}", i - 1));
        }
        records.push(row);
    }
    let (output, report) = import(&blank(), records, &[]).unwrap();
    assert!(report.relationship_transaction.output_graph_verified);
    assert_eq!(graph(&output).nodes.len(), 400);
    let mut cycle = record("n0000", "Text");
    cycle.reply_to = Some("n0399".into());
    assert!(import(&output, vec![cycle], &[]).is_err());
}
