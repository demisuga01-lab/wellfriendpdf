//! Regression source only; these PDFs/tests have not been executed.
use super::*;
use crate::tagged_structure::stream_clones::{
    StructureActualTextUpdate, TaggedCloneOptions, TaggedClonePolicy,
};

fn staged(input: &[u8], objects: Vec<(u32, PdfObject)>) -> Vec<u8> {
    let engine = ContentEngine::open_bytes(input.to_vec()).unwrap();
    write_incremental_update(
        engine.document().reader(),
        objects
            .into_iter()
            .map(|(number, object)| IncrementalObject {
                number,
                generation: 0,
                object,
            })
            .collect(),
    )
    .unwrap()
}

pub(super) fn tagged_fixture(nested: bool, shared_state: bool) -> Vec<u8> {
    let input = fixture(nested);
    let mut catalog = object(&input, 1).as_dict().unwrap().clone();
    catalog.insert("StructTreeRoot", r(12));
    let mut page = object(&input, 3).as_dict().unwrap().clone();
    page.insert("Annots", PdfObject::Array(vec![r(7)]));
    let mut ap = object(&input, 8).as_dict().unwrap().clone();
    ap.insert(
        "R",
        r(if shared_state {
            if nested {
                10
            } else {
                5
            }
        } else {
            11
        }),
    );
    let mut mcr = d(&[
        ("Type", n("MCR")),
        ("Pg", r(3)),
        ("Stm", r(5)),
        ("MCID", PdfObject::Integer(0)),
    ]);
    if !nested {
        mcr.insert("StmOwn", r(7));
    }
    let input = staged(&input, vec![
        (1, PdfObject::Dictionary(catalog)), (3, PdfObject::Dictionary(page)), (8, PdfObject::Dictionary(ap)),
        (5, form(b"/Span << /MCID 0 /ActualText (ABC) >> BDC BT /F1 12 Tf 1 0 0 1 5 20 Tm (ABC) Tj ET EMC", false)),
        (12, PdfObject::Dictionary(d(&[("Type", n("StructTreeRoot")), ("K", r(13))]))),
        (13, PdfObject::Dictionary(d(&[("Type", n("StructElem")), ("S", n("Span")), ("P", r(12)), ("Pg", r(3)),
            ("ID", PdfObject::String(b"stable-logical-owner".to_vec())), ("K", PdfObject::Dictionary(mcr))]))),
    ]);
    crate::tagged_structure::rebuild_parent_tree(&input, "en")
        .unwrap()
        .0
}

fn edit(input: &[u8], nested: bool, policy: TaggedClonePolicy) -> AppearanceTextEditRequest {
    let mut result = request(selected(input, nested), "XYZ");
    result.tagged_clone.policy = policy;
    result
}

fn carriers(input: &[u8], owner: u32) -> Vec<PdfDictionary> {
    let object = object(input, owner);
    match object.as_dict().unwrap().get("K").unwrap() {
        PdfObject::Dictionary(dict) => vec![dict.clone()],
        PdfObject::Array(items) => items
            .iter()
            .map(|item| item.as_dict().unwrap().clone())
            .collect(),
        other => panic!("unexpected content carriers: {other:?}"),
    }
}

#[test]
fn exclusive_tagged_clone_rebinds_mcr_preserving_owner_id_and_original_stream() {
    let input = tagged_fixture(false, false);
    let (output, report) = edit_appearance_text(
        &input,
        &edit(&input, false, TaggedClonePolicy::MoveExclusiveNamespaces),
        None,
    )
    .unwrap();
    assert_eq!(report.direct_text_after, "XYZ");
    assert!(report.tagged_ownership.as_ref().unwrap().ownership_verified);
    assert!(
        !report
            .tagged_ownership
            .as_ref()
            .unwrap()
            .conformance_certified
    );
    assert!(output.starts_with(&input));
    let kids = carriers(&output, 13);
    assert_eq!(kids.len(), 1);
    assert_eq!(
        kids[0].get_reference("Stm"),
        Some(report.target_after.appearance_stream)
    );
    assert_eq!(kids[0].get_reference("StmOwn"), Some((7, 0)));
    assert_eq!(kids[0].get_integer("MCID"), Some(0));
    for id in [3, 4, 5, 6, 8, 9, 10, 11] {
        assert_eq!(object(&input, id), object(&output, id));
    }
    for key in ["S", "P", "Pg", "ID"] {
        assert_eq!(
            object(&input, 13).as_dict().unwrap().get(key),
            object(&output, 13).as_dict().unwrap().get(key)
        );
    }
    crate::tagged_structure::validate_parent_tree(&output).unwrap();
}

#[test]
fn tagged_clone_requires_explicit_policy_and_shared_state_order_decision() {
    let input = tagged_fixture(false, true);
    assert!(edit_appearance_text(
        &input,
        &edit(&input, false, TaggedClonePolicy::Reject),
        None
    )
    .unwrap_err()
    .to_string()
    .contains("stream-owned structure"));
    assert!(edit_appearance_text(
        &input,
        &edit(&input, false, TaggedClonePolicy::MoveExclusiveNamespaces),
        None
    )
    .unwrap_err()
    .to_string()
    .contains("source namespace remains live"));
}

#[test]
fn shared_normal_rollover_clone_splits_carriers_and_allocates_distinct_parent_keys() {
    let input = tagged_fixture(false, true);
    let (output, report) = edit_appearance_text(
        &input,
        &edit(
            &input,
            false,
            TaggedClonePolicy::SplitSharedNamespacesAfterSource,
        ),
        None,
    )
    .unwrap();
    let kids = carriers(&output, 13);
    assert_eq!(
        kids.iter()
            .map(|kid| kid.get_reference("Stm").unwrap())
            .collect::<Vec<_>>(),
        vec![(5, 0), report.target_after.appearance_stream]
    );
    let old = object(&output, 5);
    let new = object(&output, report.target_after.appearance_stream.0);
    assert_ne!(
        old.as_stream().unwrap().0.get_integer("StructParents"),
        new.as_stream().unwrap().0.get_integer("StructParents")
    );
    assert_eq!(object(&input, 5), old);
    let annotation = object(&output, 7);
    let ap = annotation
        .as_dict()
        .unwrap()
        .get("AP")
        .unwrap()
        .as_dict()
        .unwrap();
    assert_eq!(ap.get("R"), Some(&r(5)));
    assert_eq!(ap.get("WFKeep"), Some(&n("unchanged")));
    crate::tagged_structure::validate_parent_tree(&output).unwrap();
}

#[test]
fn nested_repeated_form_splits_leaf_namespace_without_fabricating_stmown() {
    let input = tagged_fixture(true, false);
    let (output, report) = edit_appearance_text(
        &input,
        &edit(
            &input,
            true,
            TaggedClonePolicy::SplitSharedNamespacesAfterSource,
        ),
        None,
    )
    .unwrap();
    let kids = carriers(&output, 13);
    let leaf = report.target_after.invocation_path.last().unwrap();
    assert_eq!(kids.len(), 2);
    assert_eq!(kids[0].get_reference("Stm"), Some((5, 0)));
    assert_eq!(
        kids[1].get_reference("Stm"),
        Some((leaf.form_object, leaf.form_generation))
    );
    assert!(!kids[1].contains_key("StmOwn"));
    let texts = analyze_appearance_text(&output, 1)
        .unwrap()
        .occurrences
        .into_iter()
        .filter(|item| !item.target.invocation_path.is_empty())
        .map(|item| item.text.logical_text)
        .collect::<Vec<_>>();
    assert_eq!(texts, vec!["XYZ", "ABC"]);
    assert_eq!(object(&input, 5), object(&output, 5));
    assert_eq!(object(&input, 10), object(&output, 10));
    crate::tagged_structure::validate_parent_tree(&output).unwrap();
}

#[test]
fn logical_actual_text_requires_exact_decision_and_is_updated_atomically() {
    let input = tagged_fixture(false, false);
    let mut owner = object(&input, 13).as_dict().unwrap().clone();
    owner.insert("ActualText", crate::annotation_identity::text_string("ABC"));
    let input = modify(&input, 13, PdfObject::Dictionary(owner));
    let mut request = edit(&input, false, TaggedClonePolicy::MoveExclusiveNamespaces);
    assert!(edit_appearance_text(&input, &request, None)
        .unwrap_err()
        .to_string()
        .contains("exact old/new"));
    request
        .tagged_clone
        .actual_text_updates
        .push(StructureActualTextUpdate {
            element: (13, 0),
            expected_text: "stale".into(),
            replacement_text: "XYZ".into(),
        });
    assert!(edit_appearance_text(&input, &request, None)
        .unwrap_err()
        .to_string()
        .contains("compare-and-swap"));
    request.tagged_clone.actual_text_updates[0].expected_text = "ABC".into();
    let (output, report) = edit_appearance_text(&input, &request, None).unwrap();
    assert_eq!(report.direct_text_after, "XYZ");
    assert_eq!(
        object(&output, 13).as_dict().unwrap().get("ActualText"),
        Some(&crate::annotation_identity::text_string("XYZ"))
    );
}

#[test]
fn structural_ancestor_actual_text_participates_in_the_same_compare_and_swap() {
    let input = tagged_fixture(false, false);
    let engine = ContentEngine::open_bytes(input.clone()).unwrap();
    let ancestor = engine
        .document()
        .reader()
        .object_ids()
        .into_iter()
        .map(|id| id.0)
        .max()
        .unwrap()
        + 1;
    let mut root = object(&input, 12).as_dict().unwrap().clone();
    root.insert("K", r(ancestor));
    let mut owner = object(&input, 13).as_dict().unwrap().clone();
    owner.insert("P", r(ancestor));
    let input = staged(
        &input,
        vec![
            (12, PdfObject::Dictionary(root)),
            (13, PdfObject::Dictionary(owner)),
            (
                ancestor,
                PdfObject::Dictionary(d(&[
                    ("Type", n("StructElem")),
                    ("S", n("Sect")),
                    ("P", r(12)),
                    ("K", r(13)),
                    (
                        "ActualText",
                        crate::annotation_identity::text_string("whole ABC"),
                    ),
                ])),
            ),
        ],
    );
    let input = crate::tagged_structure::rebuild_parent_tree(&input, "en")
        .unwrap()
        .0;
    let mut request = edit(&input, false, TaggedClonePolicy::MoveExclusiveNamespaces);
    assert!(edit_appearance_text(&input, &request, None).is_err());
    request
        .tagged_clone
        .actual_text_updates
        .push(StructureActualTextUpdate {
            element: (ancestor, 0),
            expected_text: "whole ABC".into(),
            replacement_text: "whole XYZ".into(),
        });
    let (output, _) = edit_appearance_text(&input, &request, None).unwrap();
    assert_eq!(
        object(&output, ancestor)
            .as_dict()
            .unwrap()
            .get("ActualText"),
        Some(&crate::annotation_identity::text_string("whole XYZ"))
    );
    crate::tagged_structure::validate_parent_tree(&output).unwrap();
}

#[test]
fn unrelated_duplicate_and_nonexistent_actual_text_updates_are_not_ignored() {
    let input = tagged_fixture(false, false);
    for ids in [vec![13], vec![7], vec![13, 13]] {
        let mut request = edit(&input, false, TaggedClonePolicy::MoveExclusiveNamespaces);
        request.tagged_clone.actual_text_updates = ids
            .into_iter()
            .map(|id| StructureActualTextUpdate {
                element: (id, 0),
                expected_text: "ABC".into(),
                replacement_text: "XYZ".into(),
            })
            .collect();
        assert!(edit_appearance_text(&input, &request, None).is_err());
    }
}

#[test]
fn repeated_tagged_edit_reuses_logical_owner_without_accumulating_dead_carriers() {
    let input = tagged_fixture(false, true);
    let (first, report) = edit_appearance_text(
        &input,
        &edit(
            &input,
            false,
            TaggedClonePolicy::SplitSharedNamespacesAfterSource,
        ),
        None,
    )
    .unwrap();
    let mut request = request(report.target_after, "DEF");
    request.tagged_clone.policy = TaggedClonePolicy::MoveExclusiveNamespaces;
    let (second, report) = edit_appearance_text(&first, &request, None).unwrap();
    assert_eq!(report.direct_text_after, "DEF");
    let kids = carriers(&second, 13);
    assert_eq!(kids.len(), 2);
    assert_eq!(kids[0].get_reference("Stm"), Some((5, 0)));
    assert_eq!(
        kids[1].get_reference("Stm"),
        Some(report.target_after.appearance_stream)
    );
    crate::tagged_structure::validate_parent_tree(&second).unwrap();
}

#[test]
fn annotation_objr_actual_text_is_checked_even_without_marked_appearance_text() {
    let input = tagged_fixture(false, false);
    let mut owner = object(&input, 13).as_dict().unwrap().clone();
    owner.insert(
        "K",
        PdfObject::Dictionary(d(&[("Type", n("OBJR")), ("Obj", r(7)), ("Pg", r(3))])),
    );
    owner.insert("ActualText", crate::annotation_identity::text_string("ABC"));
    let input = staged(
        &input,
        vec![
            (13, PdfObject::Dictionary(owner)),
            (5, form(b"BT /F1 12 Tf (ABC) Tj ET", false)),
        ],
    );
    let input = crate::tagged_structure::rebuild_parent_tree(&input, "en")
        .unwrap()
        .0;
    assert!(edit_appearance_text(
        &input,
        &edit(&input, false, TaggedClonePolicy::Reject),
        None
    )
    .unwrap_err()
    .to_string()
    .contains("whole-annotation OBJR ownership"));
    let mut request = edit(&input, false, TaggedClonePolicy::MoveExclusiveNamespaces);
    assert!(edit_appearance_text(&input, &request, None).is_err());
    request
        .tagged_clone
        .actual_text_updates
        .push(StructureActualTextUpdate {
            element: (13, 0),
            expected_text: "ABC".into(),
            replacement_text: "XYZ".into(),
        });
    let (output, _) = edit_appearance_text(&input, &request, None).unwrap();
    assert_eq!(carriers(&output, 13)[0].get_reference("Obj"), Some((7, 0)));
    assert_eq!(
        object(&output, 13).as_dict().unwrap().get("ActualText"),
        Some(&crate::annotation_identity::text_string("XYZ"))
    );
    crate::tagged_structure::validate_parent_tree(&output).unwrap();
}

#[test]
fn tagged_appearance_json_and_missing_option_default_reach_the_same_writer() {
    let input = tagged_fixture(false, false);
    let request = edit(&input, false, TaggedClonePolicy::MoveExclusiveNamespaces);
    let json = serde_json::to_string(&request).unwrap();
    let (output, report) =
        crate::sdk::advanced_editing_appearance_text_edit_json(&input, &json, None, None).unwrap();
    let report: serde_json::Value = serde_json::from_str(&report).unwrap();
    assert_eq!(
        report["report"]["tagged_ownership"]["ownership_verified"],
        true
    );
    crate::tagged_structure::validate_parent_tree(&output).unwrap();
    let mut json = serde_json::to_value(&request).unwrap();
    json.as_object_mut().unwrap().remove("tagged_clone");
    let request: AppearanceTextEditRequest = serde_json::from_value(json).unwrap();
    assert_eq!(request.tagged_clone.policy, TaggedClonePolicy::Reject);
}

#[test]
fn low_level_clone_migration_rejects_existing_ids_and_observes_cancellation() {
    let input = tagged_fixture(false, false);
    let options = TaggedCloneOptions {
        policy: TaggedClonePolicy::MoveExclusiveNamespaces,
        ..Default::default()
    };
    assert!(crate::tagged_structure::stream_clones::finish(
        &input,
        input.clone(),
        (7, 0),
        (3, 0),
        &BTreeMap::from([((5, 0), (11, 0))]),
        &options
    )
    .unwrap_err()
    .to_string()
    .contains("fresh"));
    let token = crate::cancel::CancelToken::new();
    token.cancel();
    assert!(matches!(
        token.scope(|| crate::tagged_structure::stream_clones::finish(
            &input,
            input.clone(),
            (7, 0),
            (3, 0),
            &BTreeMap::new(),
            &options
        )),
        Err(WellfriendError::Cancelled(_))
    ));
}

#[test]
fn tagged_nested_forms_without_resources_use_page_properties_not_parent_properties() {
    let input = tagged_fixture(true, false);
    let mut page = object(&input, 3).as_dict().unwrap().clone();
    let mut resources = page.get("Resources").unwrap().as_dict().unwrap().clone();
    resources.insert(
        "Properties",
        PdfObject::Dictionary(d(&[(
            "Mark",
            PdfObject::Dictionary(d(&[("MCID", PdfObject::Integer(0))])),
        )])),
    );
    page.insert("Resources", PdfObject::Dictionary(resources));
    let mut leaf = form(b"/Span /Mark BDC BT /F1 12 Tf (ABC) Tj ET EMC", false);
    if let PdfObject::Stream { dict, .. } = &mut leaf {
        dict.remove("Resources");
    }
    let mut root = object(&input, 10);
    if let PdfObject::Stream { dict, .. } = &mut root {
        let mut resources = dict.get("Resources").unwrap().as_dict().unwrap().clone();
        resources.insert(
            "Properties",
            PdfObject::Dictionary(d(&[(
                "Mark",
                PdfObject::Dictionary(d(&[("MCID", PdfObject::Integer(17))])),
            )])),
        );
        dict.insert("Resources", PdfObject::Dictionary(resources));
    }
    let input = staged(
        &input,
        vec![(3, PdfObject::Dictionary(page)), (5, leaf), (10, root)],
    );
    let input = crate::tagged_structure::rebuild_parent_tree(&input, "en")
        .unwrap()
        .0;
    let report = crate::tagged_structure::validate_parent_tree(&input).unwrap();
    assert_eq!(report.marked_content_items, 1);
}

#[test]
fn alternate_soft_mask_reference_prevents_false_exclusive_namespace_migration() {
    let input = tagged_fixture(false, false);
    let mut page = object(&input, 3).as_dict().unwrap().clone();
    let mut resources = page.get("Resources").unwrap().as_dict().unwrap().clone();
    resources.insert(
        "ExtGState",
        PdfObject::Dictionary(d(&[(
            "MaskState",
            PdfObject::Dictionary(d(&[(
                "SMask",
                PdfObject::Dictionary(d(&[("S", n("Alpha")), ("G", r(5))])),
            )])),
        )])),
    );
    page.insert("Resources", PdfObject::Dictionary(resources));
    let input = modify(&input, 3, PdfObject::Dictionary(page));
    // Even an unused soft-mask resource conservatively keeps its old carrier:
    // this writer does not claim execution-level liveness for soft masks.
    assert!(edit_appearance_text(
        &input,
        &edit(&input, false, TaggedClonePolicy::MoveExclusiveNamespaces),
        None
    )
    .unwrap_err()
    .to_string()
    .contains("source namespace remains live"));
    // The old explicit annotation StmOwn cannot be retained after the old
    // stream is only referenced as a mask. Splitting is not approval to guess
    // another owner or silently discard an explicit ownership assertion.
    assert!(edit_appearance_text(
        &input,
        &edit(
            &input,
            false,
            TaggedClonePolicy::SplitSharedNamespacesAfterSource
        ),
        None
    )
    .is_err());
    let mut owner = object(&input, 13).as_dict().unwrap().clone();
    let mut carrier = owner.get("K").unwrap().as_dict().unwrap().clone();
    carrier.remove("StmOwn");
    owner.insert("K", PdfObject::Dictionary(carrier));
    let input = modify(&input, 13, PdfObject::Dictionary(owner));
    let (output, report) = edit_appearance_text(
        &input,
        &edit(
            &input,
            false,
            TaggedClonePolicy::SplitSharedNamespacesAfterSource,
        ),
        None,
    )
    .unwrap();
    assert_eq!(carriers(&output, 13).len(), 2);
    assert_eq!(report.direct_text_after, "XYZ");
    assert_eq!(object(&input, 5), object(&output, 5));
    crate::tagged_structure::validate_parent_tree(&output).unwrap();
}

#[test]
fn clone_mcid_set_changes_are_rejected_before_logical_ownership_is_rebound() {
    let input = tagged_fixture(false, false);
    let engine = ContentEngine::open_bytes(input.clone()).unwrap();
    let clone = engine
        .document()
        .reader()
        .object_ids()
        .into_iter()
        .map(|id| id.0)
        .max()
        .unwrap()
        + 1;
    let mut owner = object(&input, 7).as_dict().unwrap().clone();
    owner.insert(
        "AP",
        PdfObject::Dictionary(d(&[("N", PdfObject::Dictionary(d(&[("Yes", r(clone))])))])),
    );
    let candidate = staged(
        &input,
        vec![
            (7, PdfObject::Dictionary(owner)),
            (
                clone,
                form(
                    b"/Span << /MCID 7 >> BDC BT /F1 12 Tf (XYZ) Tj ET EMC",
                    false,
                ),
            ),
        ],
    );
    let options = TaggedCloneOptions {
        policy: TaggedClonePolicy::MoveExclusiveNamespaces,
        ..Default::default()
    };
    assert!(crate::tagged_structure::stream_clones::finish(
        &input,
        candidate,
        (7, 0),
        (3, 0),
        &BTreeMap::from([((5, 0), (clone, 0))]),
        &options
    )
    .unwrap_err()
    .to_string()
    .contains("changed the source MCID set"));
}
