//! Unexecuted regression source. No PDF/runtime/CRDT qualification is implied.
use super::*;
fn base(text: &str) -> LinkedStoryRequest {
    serde_json::from_value(serde_json::json!({"story_id":"history","input_sha256":"revision","frames":[],"fonts":[],
        "paragraphs":[{"id":"p","text":text,"preferred_font":"Helvetica","font_size":12.0,"line_height":14.0}]})).unwrap()
}
fn edited(
    base: &LinkedStoryRequest,
    history: &StoryTextHistory,
    actor: &str,
    range: [usize; 2],
    replacement: &str,
) -> StoryHistoryResult {
    let current = merge_histories(base, std::slice::from_ref(history)).unwrap();
    let expected = current.merged.as_ref().unwrap().paragraphs[0]
        .text
        .get(range[0]..range[1])
        .unwrap()
        .to_owned();
    edit_history(
        base,
        history,
        &StoryHistoryEdit {
            expected_history_sha256: current.history_sha256,
            actor: actor.into(),
            paragraph_id: "p".into(),
            range,
            expected_text: expected,
            replacement: replacement.into(),
        },
    )
    .unwrap()
}
fn text(result: &StoryHistoryResult) -> &str {
    &result.merged.as_ref().unwrap().paragraphs[0].text
}

fn styled(
    base: &LinkedStoryRequest,
    history: &StoryTextHistory,
    actor: &str,
    expected: StoryParagraphStylePatch,
    replacement: StoryParagraphStylePatch,
) -> StoryHistoryResult {
    let current = merge_histories(base, std::slice::from_ref(history)).unwrap();
    edit_paragraph_style(
        base,
        history,
        &StoryHistoryStyleEdit {
            expected_history_sha256: current.history_sha256,
            expected_style_conflicts_sha256: current.style_conflicts_sha256,
            actor: actor.into(),
            paragraph_id: "p".into(),
            expected,
            replacement,
        },
    )
    .unwrap()
}

fn structured(
    base: &LinkedStoryRequest,
    history: &StoryTextHistory,
    actor: &str,
    paragraph_id: &str,
    expected_absent: bool,
    expected: StoryParagraphStructurePatch,
    replacement: StoryParagraphStructurePatch,
) -> StoryHistoryResult {
    let current = merge_histories(base, std::slice::from_ref(history)).unwrap();
    edit_paragraph_structure(
        base,
        history,
        &StoryHistoryStructureEdit {
            expected_history_sha256: current.history_sha256,
            expected_structure_conflicts_sha256: current.structure_conflicts_sha256,
            actor: actor.into(),
            paragraph_id: paragraph_id.into(),
            expected_absent,
            expected,
            replacement,
        },
    )
    .unwrap()
}

fn inline_styled(
    base: &LinkedStoryRequest,
    history: &StoryTextHistory,
    actor: &str,
    range: [usize; 2],
    replacement: StoryInlineStylePatch,
) -> StoryHistoryResult {
    let current = merge_histories(base, std::slice::from_ref(history)).unwrap();
    let expected = current.merged.as_ref().unwrap().paragraphs[0]
        .text
        .get(range[0]..range[1])
        .unwrap()
        .to_owned();
    edit_inline_style(
        base,
        history,
        &StoryHistoryInlineStyleEdit {
            expected_history_sha256: current.history_sha256,
            expected_inline_conflicts_sha256: current.inline_conflicts_sha256,
            actor: actor.into(),
            paragraph_id: "p".into(),
            range,
            expected_text: expected,
            replacement,
        },
    )
    .unwrap()
}

#[test]
fn inline_style_targets_stable_atoms_and_text_insertions_inherit_observed_style() {
    let base = base("AB");
    let empty = new_history(&base).unwrap();
    let red = inline_styled(
        &base,
        &empty,
        "a",
        [0, 1],
        StoryInlineStylePatch {
            rgb: Some([1.0, 0.0, 0.0]),
            ..Default::default()
        },
    );
    assert_eq!(red.history.schema_version, 5);
    assert_eq!(red.inline_style_runs.len(), 1);
    assert_eq!(red.inline_style_runs[0].logical_range, [0, 1]);

    let inserted = edited(&base, &red.history, "b", [1, 1], "X");
    assert_eq!(text(&inserted), "AXB");
    assert_eq!(inserted.history.schema_version, 5);
    assert_eq!(inserted.inline_style_runs.len(), 1);
    assert_eq!(inserted.inline_style_runs[0].logical_range, [0, 2]);
    assert_eq!(
        inserted.inline_style_runs[0].style.rgb,
        Some([1.0, 0.0, 0.0])
    );

    let cleared = inline_styled(
        &base,
        &inserted.history,
        "a",
        [0, 2],
        StoryInlineStylePatch {
            clear: BTreeSet::from([StoryInlineStyleField::Rgb]),
            ..Default::default()
        },
    );
    assert!(cleared.inline_style_runs.is_empty());
}

#[test]
fn materialized_inline_baseline_round_trips_through_schema_three_seed() {
    let mut base = base("AB");
    base.paragraphs[0].inline_styles = vec![crate::linked_stories::StoryInlineStyleSpan {
        logical_range: [0, 1],
        preferred_font: None,
        font_size: Some(15.0),
        rgb: Some([0.25, 0.5, 0.75]),
        shaping: None,
    }];
    let empty = new_history(&base).unwrap();
    let projected = merge_histories(&base, std::slice::from_ref(&empty)).unwrap();
    assert_eq!(projected.inline_style_runs.len(), 1);
    assert_eq!(
        projected.merged.as_ref().unwrap().paragraphs[0].inline_styles,
        base.paragraphs[0].inline_styles
    );
    let seed = seed_from_base(&base).unwrap();
    assert_eq!(seed.schema_version, 3);
    let resumed = merge_seed(&seed, &base, std::slice::from_ref(&empty)).unwrap();
    assert_eq!(
        resumed.merged.as_ref().unwrap().paragraphs[0].inline_styles,
        base.paragraphs[0].inline_styles
    );

    let mut invalid = base.clone();
    invalid.paragraphs[0]
        .inline_styles
        .push(crate::linked_stories::StoryInlineStyleSpan {
            logical_range: [0, 2],
            preferred_font: Some("Helvetica".into()),
            font_size: None,
            rgb: None,
            shaping: None,
        });
    assert!(new_history(&invalid).is_err());
}

#[test]
fn nondefault_tab_stops_require_schema_four_seed_without_bumping_default_stories() {
    let mut tabbed = base("A\t12.5");
    tabbed.paragraphs[0].tab_stops = crate::fonts::tab_stops::TabStops {
        stops: vec![crate::fonts::tab_stops::TabStop {
            position: 96.0,
            alignment: crate::fonts::tab_stops::TabAlignment::Decimal,
            decimal: '.',
            decimal_token: None,
            leader: crate::fonts::tab_stops::TabLeader::None,
            bar: false,
        }],
        default_interval: 36.0,
    };
    let history = new_history(&tabbed).unwrap();
    let seed = seed_from_base(&tabbed).unwrap();
    assert_eq!(seed.schema_version, 4);
    let resumed = merge_seed(&seed, &tabbed, std::slice::from_ref(&history))
        .unwrap()
        .merged
        .unwrap();
    assert_eq!(
        &resumed.paragraphs[0].tab_stops,
        &tabbed.paragraphs[0].tab_stops
    );

    let mut mislabeled = seed;
    mislabeled.schema_version = 3;
    assert!(merge_seed(&mislabeled, &tabbed, &[history]).is_err());
    assert_eq!(seed_from_base(&base("plain")).unwrap().schema_version, 3);
}

#[test]
fn decorated_tab_stops_require_schema_five_seed() {
    let mut tabbed = base("A\tB");
    tabbed.paragraphs[0].tab_stops = crate::fonts::tab_stops::TabStops {
        stops: vec![crate::fonts::tab_stops::TabStop {
            position: 96.0,
            alignment: crate::fonts::tab_stops::TabAlignment::Left,
            decimal: '.',
            decimal_token: None,
            leader: crate::fonts::tab_stops::TabLeader::Solid,
            bar: true,
        }],
        default_interval: 36.0,
    };
    let history = new_history(&tabbed).unwrap();
    let seed = seed_from_base(&tabbed).unwrap();
    assert_eq!(seed.schema_version, 5);
    let resumed = merge_seed(&seed, &tabbed, std::slice::from_ref(&history))
        .unwrap()
        .merged
        .unwrap();
    assert_eq!(
        resumed.paragraphs[0].tab_stops,
        tabbed.paragraphs[0].tab_stops
    );

    let mut mislabeled = seed;
    mislabeled.schema_version = 4;
    assert!(merge_seed(&mislabeled, &tabbed, &[history]).is_err());
}

#[test]
fn multi_character_decimal_tabs_require_schema_six_seed() {
    let mut tabbed = base("A\t12::50");
    tabbed.paragraphs[0].tab_stops = crate::fonts::tab_stops::TabStops {
        stops: vec![crate::fonts::tab_stops::TabStop {
            position: 96.0,
            alignment: crate::fonts::tab_stops::TabAlignment::Decimal,
            decimal: '.',
            decimal_token: Some("::".into()),
            leader: crate::fonts::tab_stops::TabLeader::None,
            bar: false,
        }],
        default_interval: 36.0,
    };
    let history = new_history(&tabbed).unwrap();
    let seed = seed_from_base(&tabbed).unwrap();
    assert_eq!(seed.schema_version, 6);
    assert!(merge_seed(&seed, &tabbed, std::slice::from_ref(&history)).is_ok());

    let mut mislabeled = seed;
    mislabeled.schema_version = 5;
    assert!(merge_seed(&mislabeled, &tabbed, &[history]).is_err());
}

#[test]
fn concurrent_inline_values_conflict_per_atom_and_exact_resolution_is_undoable() {
    let base = base("AB");
    let empty = new_history(&base).unwrap();
    let red = inline_styled(
        &base,
        &empty,
        "a",
        [0, 2],
        StoryInlineStylePatch {
            rgb: Some([1.0, 0.0, 0.0]),
            ..Default::default()
        },
    );
    let blue = inline_styled(
        &base,
        &empty,
        "b",
        [0, 2],
        StoryInlineStylePatch {
            rgb: Some([0.0, 0.0, 1.0]),
            ..Default::default()
        },
    );
    let conflict = merge_histories(&base, &[red.history, blue.history]).unwrap();
    assert!(conflict.merged.is_none());
    assert_eq!(conflict.inline_conflicts.len(), 2);
    assert!(conflict
        .inline_conflicts
        .iter()
        .all(|item| item.field == StoryInlineStyleField::Rgb));

    let resolved = resolve_inline_style_conflicts(
        &base,
        &conflict.history,
        &StoryHistoryInlineStyleResolution {
            expected_history_sha256: conflict.history_sha256,
            expected_inline_conflicts_sha256: conflict.inline_conflicts_sha256,
            actor: "reviewer".into(),
            paragraph_id: "p".into(),
            targets: vec![StoryAtomRange {
                operation: None,
                start: 0,
                end: 2,
            }],
            replacement: StoryInlineStylePatch {
                rgb: Some([0.0, 1.0, 0.0]),
                ..Default::default()
            },
        },
    )
    .unwrap();
    assert!(resolved.inline_conflicts.is_empty());
    assert_eq!(resolved.inline_style_runs.len(), 1);
    assert_eq!(resolved.inline_style_runs[0].logical_range, [0, 2]);
    assert_eq!(
        resolved.inline_style_runs[0].style.rgb,
        Some([0.0, 1.0, 0.0])
    );

    let undone = controlled(&base, &resolved.history, "reviewer", 1, false);
    assert!(undone.merged.is_none());
    assert_eq!(undone.inline_conflicts.len(), 2);
}

#[test]
fn concurrent_equal_inline_values_converge_without_a_false_conflict() {
    let base = base("AB");
    let empty = new_history(&base).unwrap();
    let patch = StoryInlineStylePatch {
        font_size: Some(14.0),
        ..Default::default()
    };
    let first = inline_styled(&base, &empty, "a", [0, 2], patch.clone());
    let second = inline_styled(&base, &empty, "b", [0, 2], patch);
    let forward = merge_histories(&base, &[first.history.clone(), second.history.clone()]).unwrap();
    let reverse = merge_histories(&base, &[second.history, first.history]).unwrap();
    assert!(forward.inline_conflicts.is_empty());
    assert_eq!(forward.history, reverse.history);
    assert_eq!(forward.inline_style_runs.len(), 1);
    assert_eq!(forward.inline_style_runs[0].style.font_size, Some(14.0));
}

#[test]
fn inline_style_delta_waits_for_the_inserted_atom_origin_and_schema_is_enforced() {
    let base = base("AB");
    let empty = new_history(&base).unwrap();
    let inserted = edited(&base, &empty, "a", [1, 1], "X");
    let styled = inline_styled(
        &base,
        &inserted.history,
        "a",
        [1, 2],
        StoryInlineStylePatch {
            font_size: Some(16.0),
            ..Default::default()
        },
    );
    let delta = export_delta(&base, &styled.history, &BTreeMap::from([("a".into(), 1)])).unwrap();
    let pending = merge_histories(&base, &[delta]).unwrap();
    assert!(pending.merged.is_none());
    assert_eq!(
        pending.missing_dependencies,
        vec![StoryOperationId {
            actor: "a".into(),
            sequence: 1,
        }]
    );

    let mut wrong_schema = styled.history.clone();
    wrong_schema.schema_version = 4;
    assert!(merge_histories(&base, &[wrong_schema]).is_err());
    let mut mixed = styled.history.clone();
    mixed.operations[1].inserted = "forbidden".into();
    assert!(merge_histories(&base, &[mixed]).is_err());
}

#[test]
fn deleted_atoms_keep_inline_conflicts_latent_until_selective_undo_restores_them() {
    let base = base("AB");
    let empty = new_history(&base).unwrap();
    let deleted = edited(&base, &empty, "deleter", [0, 1], "");
    let red = inline_styled(
        &base,
        &empty,
        "red",
        [0, 1],
        StoryInlineStylePatch {
            rgb: Some([1.0, 0.0, 0.0]),
            ..Default::default()
        },
    );
    let blue = inline_styled(
        &base,
        &empty,
        "blue",
        [0, 1],
        StoryInlineStylePatch {
            rgb: Some([0.0, 0.0, 1.0]),
            ..Default::default()
        },
    );
    let hidden = merge_histories(&base, &[deleted.history, red.history, blue.history]).unwrap();
    assert_eq!(text(&hidden), "B");
    assert!(hidden.inline_conflicts.is_empty());

    let restored = controlled(&base, &hidden.history, "deleter", 1, false);
    assert!(restored.merged.is_none());
    assert_eq!(restored.inline_conflicts.len(), 1);
    assert_eq!(restored.inline_conflicts[0].target.offset, 0);
}

fn three_paragraphs() -> LinkedStoryRequest {
    let mut request = base("A");
    for (id, text) in [("q", "B"), ("r", "C")] {
        let mut paragraph = request.paragraphs[0].clone();
        paragraph.id = id.into();
        paragraph.text = text.into();
        request.paragraphs.push(paragraph);
    }
    request
}

#[test]
fn paragraph_insert_move_delete_and_selective_activity_use_stable_ids() {
    let base = three_paragraphs();
    let empty = new_history(&base).unwrap();
    let moved = structured(
        &base,
        &empty,
        "a",
        "q",
        false,
        StoryParagraphStructurePatch {
            position: Some(StoryParagraphPosition {
                after: Some("p".into()),
            }),
            ..Default::default()
        },
        StoryParagraphStructurePatch {
            position: Some(StoryParagraphPosition { after: None }),
            ..Default::default()
        },
    );
    assert_eq!(
        moved
            .merged
            .as_ref()
            .unwrap()
            .paragraphs
            .iter()
            .map(|paragraph| paragraph.id.as_str())
            .collect::<Vec<_>>(),
        vec!["q", "p", "r"]
    );

    let mut inserted_paragraph = base.paragraphs[0].clone();
    inserted_paragraph.id = "new".into();
    inserted_paragraph.text = "new text".into();
    let inserted = structured(
        &base,
        &moved.history,
        "a",
        "new",
        true,
        StoryParagraphStructurePatch::default(),
        StoryParagraphStructurePatch {
            present: Some(true),
            position: Some(StoryParagraphPosition {
                after: Some("p".into()),
            }),
            inserted_paragraph: Some(inserted_paragraph),
        },
    );
    assert_eq!(inserted.history.schema_version, 4);
    assert_eq!(
        inserted
            .merged
            .as_ref()
            .unwrap()
            .paragraphs
            .iter()
            .map(|paragraph| paragraph.id.as_str())
            .collect::<Vec<_>>(),
        vec!["q", "p", "new", "r"]
    );
    let current = merge_histories(&base, std::slice::from_ref(&inserted.history)).unwrap();
    let edited_insert = edit_history(
        &base,
        &inserted.history,
        &StoryHistoryEdit {
            expected_history_sha256: current.history_sha256,
            actor: "b".into(),
            paragraph_id: "new".into(),
            range: [0, 3],
            expected_text: "new".into(),
            replacement: "fresh".into(),
        },
    )
    .unwrap();
    assert_eq!(
        edited_insert
            .merged
            .as_ref()
            .unwrap()
            .paragraphs
            .iter()
            .find(|paragraph| paragraph.id == "new")
            .unwrap()
            .text,
        "fresh text"
    );
    let hidden = controlled(&base, &edited_insert.history, "a", 2, false);
    assert!(!hidden
        .merged
        .as_ref()
        .unwrap()
        .paragraphs
        .iter()
        .any(|paragraph| paragraph.id == "new"));
    let restored = controlled(&base, &hidden.history, "a", 2, true);
    assert_eq!(
        restored
            .merged
            .as_ref()
            .unwrap()
            .paragraphs
            .iter()
            .find(|paragraph| paragraph.id == "new")
            .unwrap()
            .text,
        "fresh text"
    );
}

#[test]
fn out_of_order_inserted_paragraph_delta_is_retained_without_a_partial_projection() {
    let base = base("A");
    let empty = new_history(&base).unwrap();
    let mut paragraph = base.paragraphs[0].clone();
    paragraph.id = "new".into();
    paragraph.text = "B".into();
    let inserted = structured(
        &base,
        &empty,
        "a",
        "new",
        true,
        StoryParagraphStructurePatch::default(),
        StoryParagraphStructurePatch {
            present: Some(true),
            position: Some(StoryParagraphPosition {
                after: Some("p".into()),
            }),
            inserted_paragraph: Some(paragraph),
        },
    );
    let current = merge_histories(&base, std::slice::from_ref(&inserted.history)).unwrap();
    let edited = edit_history(
        &base,
        &inserted.history,
        &StoryHistoryEdit {
            expected_history_sha256: current.history_sha256,
            actor: "a".into(),
            paragraph_id: "new".into(),
            range: [0, 1],
            expected_text: "B".into(),
            replacement: "C".into(),
        },
    )
    .unwrap();
    let delta = export_delta(&base, &edited.history, &BTreeMap::from([("a".into(), 1)])).unwrap();
    assert_eq!(delta.operations.len(), 1);
    let pending = merge_histories(&base, &[delta]).unwrap();
    assert!(pending.merged.is_none());
    assert_eq!(
        pending.missing_dependencies,
        vec![StoryOperationId {
            actor: "a".into(),
            sequence: 1,
        }]
    );
}

#[test]
fn concurrent_paragraph_delete_and_content_edit_require_explicit_keep_or_delete() {
    let base = base("AB");
    let empty = new_history(&base).unwrap();
    let deleted = structured(
        &base,
        &empty,
        "a",
        "p",
        false,
        StoryParagraphStructurePatch {
            present: Some(true),
            ..Default::default()
        },
        StoryParagraphStructurePatch {
            present: Some(false),
            ..Default::default()
        },
    );
    let content = edited(&base, &empty, "b", [1, 1], "X");
    let conflict = merge_histories(&base, &[deleted.history, content.history]).unwrap();
    assert!(conflict.merged.is_none());
    assert_eq!(conflict.structure_conflicts.len(), 1);
    assert_eq!(conflict.structure_conflicts[0].field, "present");

    let kept = edit_paragraph_structure(
        &base,
        &conflict.history,
        &StoryHistoryStructureEdit {
            expected_history_sha256: conflict.history_sha256.clone(),
            expected_structure_conflicts_sha256: conflict.structure_conflicts_sha256.clone(),
            actor: "reviewer".into(),
            paragraph_id: "p".into(),
            expected_absent: false,
            expected: StoryParagraphStructurePatch::default(),
            replacement: StoryParagraphStructurePatch {
                present: Some(true),
                ..Default::default()
            },
        },
    )
    .unwrap();
    assert!(kept.structure_conflicts.is_empty());
    assert_eq!(text(&kept), "AXB");

    let deleted = edit_paragraph_structure(
        &base,
        &conflict.history,
        &StoryHistoryStructureEdit {
            expected_history_sha256: conflict.history_sha256.clone(),
            expected_structure_conflicts_sha256: conflict.structure_conflicts_sha256.clone(),
            actor: "deleter".into(),
            paragraph_id: "p".into(),
            expected_absent: false,
            expected: StoryParagraphStructurePatch::default(),
            replacement: StoryParagraphStructurePatch {
                present: Some(false),
                ..Default::default()
            },
        },
    )
    .unwrap();
    assert!(deleted.structure_conflicts.is_empty());
    assert!(deleted.merged.as_ref().unwrap().paragraphs.is_empty());

    let resolution_undone = controlled(&base, &deleted.history, "deleter", 1, false);
    assert!(resolution_undone.merged.is_none());
    assert_eq!(resolution_undone.structure_conflicts.len(), 1);
    assert_eq!(resolution_undone.structure_conflicts[0].field, "present");
}

#[test]
fn concurrent_paragraph_positions_converge_or_report_candidates() {
    let base = three_paragraphs();
    let empty = new_history(&base).unwrap();
    let move_to_start = |actor: &str| {
        structured(
            &base,
            &empty,
            actor,
            "q",
            false,
            StoryParagraphStructurePatch {
                position: Some(StoryParagraphPosition {
                    after: Some("p".into()),
                }),
                ..Default::default()
            },
            StoryParagraphStructurePatch {
                position: Some(StoryParagraphPosition { after: None }),
                ..Default::default()
            },
        )
    };
    let a = move_to_start("a");
    let b = move_to_start("b");
    let equal = merge_histories(&base, &[a.history.clone(), b.history]).unwrap();
    assert!(equal.structure_conflicts.is_empty());
    assert_eq!(equal.merged.as_ref().unwrap().paragraphs[0].id, "q");

    let different = structured(
        &base,
        &empty,
        "c",
        "q",
        false,
        StoryParagraphStructurePatch {
            position: Some(StoryParagraphPosition {
                after: Some("p".into()),
            }),
            ..Default::default()
        },
        StoryParagraphStructurePatch {
            position: Some(StoryParagraphPosition {
                after: Some("r".into()),
            }),
            ..Default::default()
        },
    );
    let conflict = merge_histories(&base, &[a.history, different.history]).unwrap();
    assert!(conflict.merged.is_none());
    assert_eq!(conflict.structure_conflicts[0].field, "position");
    assert_eq!(conflict.structure_conflicts[0].candidates.len(), 2);
}

#[test]
fn structure_schema_payload_cycles_and_stale_conflict_hashes_fail_closed() {
    let base = three_paragraphs();
    let empty = new_history(&base).unwrap();
    let first = structured(
        &base,
        &empty,
        "a",
        "p",
        false,
        StoryParagraphStructurePatch {
            position: Some(StoryParagraphPosition { after: None }),
            ..Default::default()
        },
        StoryParagraphStructurePatch {
            position: Some(StoryParagraphPosition {
                after: Some("q".into()),
            }),
            ..Default::default()
        },
    );
    let mut wrong_schema = first.history.clone();
    wrong_schema.schema_version = 3;
    assert!(merge_histories(&base, &[wrong_schema]).is_err());
    let mut mixed = first.history.clone();
    mixed.operations[0].inserted = "forbidden".into();
    assert!(merge_histories(&base, &[mixed]).is_err());

    let cycled = structured(
        &base,
        &first.history,
        "b",
        "q",
        false,
        StoryParagraphStructurePatch {
            position: Some(StoryParagraphPosition { after: None }),
            ..Default::default()
        },
        StoryParagraphStructurePatch {
            position: Some(StoryParagraphPosition {
                after: Some("p".into()),
            }),
            ..Default::default()
        },
    );
    assert!(cycled.merged.is_none());
    assert!(cycled
        .structure_conflicts
        .iter()
        .any(|conflict| conflict.paragraph_id == "p" && conflict.field == "position"));
    let stale = StoryHistoryStructureEdit {
        expected_history_sha256: cycled.history_sha256.clone(),
        expected_structure_conflicts_sha256: "0".repeat(64),
        actor: "reviewer".into(),
        paragraph_id: "p".into(),
        expected_absent: false,
        expected: StoryParagraphStructurePatch::default(),
        replacement: StoryParagraphStructurePatch {
            position: Some(StoryParagraphPosition { after: None }),
            ..Default::default()
        },
    };
    assert!(edit_paragraph_structure(&base, &cycled.history, &stale).is_err());

    let resolved = edit_paragraph_structure(
        &base,
        &cycled.history,
        &StoryHistoryStructureEdit {
            expected_history_sha256: cycled.history_sha256,
            expected_structure_conflicts_sha256: cycled.structure_conflicts_sha256,
            actor: "reviewer".into(),
            paragraph_id: "p".into(),
            expected_absent: false,
            expected: StoryParagraphStructurePatch::default(),
            replacement: StoryParagraphStructurePatch {
                position: Some(StoryParagraphPosition { after: None }),
                ..Default::default()
            },
        },
    )
    .unwrap();
    assert!(resolved.structure_conflicts.is_empty());
    assert_eq!(
        resolved
            .merged
            .as_ref()
            .unwrap()
            .paragraphs
            .iter()
            .map(|paragraph| paragraph.id.as_str())
            .collect::<Vec<_>>(),
        vec!["p", "q", "r"]
    );
}

#[test]
fn paragraph_style_registers_merge_disjoint_writes_and_require_explicit_conflict_resolution() {
    let base = base("AB");
    let empty = new_history(&base).unwrap();
    let size = styled(
        &base,
        &empty,
        "a",
        StoryParagraphStylePatch {
            font_size: Some(12.0),
            ..Default::default()
        },
        StoryParagraphStylePatch {
            font_size: Some(15.0),
            ..Default::default()
        },
    );
    let colour = styled(
        &base,
        &empty,
        "b",
        StoryParagraphStylePatch {
            rgb: Some([0.0, 0.0, 0.0]),
            ..Default::default()
        },
        StoryParagraphStylePatch {
            rgb: Some([0.2, 0.3, 0.4]),
            ..Default::default()
        },
    );
    let combined = merge_histories(&base, &[size.history.clone(), colour.history]).unwrap();
    let paragraph = &combined.merged.as_ref().unwrap().paragraphs[0];
    assert_eq!(paragraph.font_size, 15.0);
    assert_eq!(paragraph.rgb, [0.2, 0.3, 0.4]);
    assert!(combined.style_conflicts.is_empty());

    let competing = styled(
        &base,
        &empty,
        "c",
        StoryParagraphStylePatch {
            font_size: Some(12.0),
            ..Default::default()
        },
        StoryParagraphStylePatch {
            font_size: Some(20.0),
            ..Default::default()
        },
    );
    let conflict = merge_histories(&base, &[size.history, competing.history]).unwrap();
    assert!(conflict.merged.is_none());
    assert_eq!(conflict.style_conflicts.len(), 1);
    assert_eq!(conflict.style_conflicts[0].field, "font_size");
    assert_eq!(conflict.style_conflicts[0].candidates.len(), 2);

    let resolved = edit_paragraph_style(
        &base,
        &conflict.history,
        &StoryHistoryStyleEdit {
            expected_history_sha256: conflict.history_sha256,
            expected_style_conflicts_sha256: conflict.style_conflicts_sha256,
            actor: "reviewer".into(),
            paragraph_id: "p".into(),
            expected: StoryParagraphStylePatch::default(),
            replacement: StoryParagraphStylePatch {
                font_size: Some(18.0),
                ..Default::default()
            },
        },
    )
    .unwrap();
    assert_eq!(resolved.history.schema_version, 3);
    assert_eq!(
        resolved.merged.as_ref().unwrap().paragraphs[0].font_size,
        18.0
    );
    assert!(resolved.style_conflicts.is_empty());
    let undone = controlled(&base, &resolved.history, "reviewer", 1, false);
    assert!(undone.merged.is_none());
    assert_eq!(undone.style_conflicts.len(), 1);
}

#[test]
fn equal_concurrent_style_writes_converge_without_a_conflict() {
    let base = base("AB");
    let empty = new_history(&base).unwrap();
    let a = styled(
        &base,
        &empty,
        "a",
        StoryParagraphStylePatch {
            font_size: Some(12.0),
            ..Default::default()
        },
        StoryParagraphStylePatch {
            font_size: Some(16.0),
            ..Default::default()
        },
    );
    let b = styled(
        &base,
        &empty,
        "b",
        StoryParagraphStylePatch {
            font_size: Some(12.0),
            ..Default::default()
        },
        StoryParagraphStylePatch {
            font_size: Some(16.0),
            ..Default::default()
        },
    );
    let ab = merge_histories(&base, &[a.history.clone(), b.history.clone()]).unwrap();
    let ba = merge_histories(&base, &[b.history, a.history]).unwrap();
    assert!(ab.style_conflicts.is_empty());
    assert_eq!(ab.history_sha256, ba.history_sha256);
    assert_eq!(ab.merged.as_ref().unwrap().paragraphs[0].font_size, 16.0);
}

#[test]
fn paragraph_style_schema_payload_and_conflict_cas_fail_closed() {
    let base = base("AB");
    let empty = new_history(&base).unwrap();
    let first = styled(
        &base,
        &empty,
        "a",
        StoryParagraphStylePatch {
            font_size: Some(12.0),
            ..Default::default()
        },
        StoryParagraphStylePatch {
            font_size: Some(15.0),
            ..Default::default()
        },
    );
    let second = styled(
        &base,
        &empty,
        "b",
        StoryParagraphStylePatch {
            font_size: Some(12.0),
            ..Default::default()
        },
        StoryParagraphStylePatch {
            font_size: Some(20.0),
            ..Default::default()
        },
    );

    let mut wrong_schema = first.history.clone();
    wrong_schema.schema_version = 2;
    assert!(merge_histories(&base, &[wrong_schema]).is_err());
    let mut mixed_payload = first.history.clone();
    mixed_payload.operations[0].inserted = "not-style".into();
    assert!(merge_histories(&base, &[mixed_payload]).is_err());

    let conflict = merge_histories(&base, &[first.history, second.history]).unwrap();
    let mut resolution = StoryHistoryStyleEdit {
        expected_history_sha256: conflict.history_sha256.clone(),
        expected_style_conflicts_sha256: conflict.style_conflicts_sha256.clone(),
        actor: "reviewer".into(),
        paragraph_id: "p".into(),
        expected: StoryParagraphStylePatch::default(),
        replacement: StoryParagraphStylePatch {
            font_size: Some(18.0),
            ..Default::default()
        },
    };
    resolution.expected_style_conflicts_sha256 = "0".repeat(64);
    assert!(edit_paragraph_style(&base, &conflict.history, &resolution).is_err());
    resolution.expected_style_conflicts_sha256 = conflict.style_conflicts_sha256.clone();
    resolution.replacement = StoryParagraphStylePatch {
        rgb: Some([0.1, 0.2, 0.3]),
        ..Default::default()
    };
    assert!(edit_paragraph_style(&base, &conflict.history, &resolution).is_err());
}

#[test]
fn legacy_seed_refuses_style_and_structure_merge_edit_and_activity() {
    let base = base("AB");
    let empty = new_history(&base).unwrap();
    let styled = styled(
        &base,
        &empty,
        "a",
        StoryParagraphStylePatch {
            font_size: Some(12.0),
            ..Default::default()
        },
        StoryParagraphStylePatch {
            font_size: Some(15.0),
            ..Default::default()
        },
    );
    let seed = StoryHistorySeed {
        schema_version: 1,
        story_id: base.story_id.clone(),
        base_revision_sha256: empty.base_revision_sha256.clone(),
        base_story_sha256: empty.base_story_sha256.clone(),
        paragraphs: vec![StorySeedParagraph {
            id: "p".into(),
            text: "AB".into(),
            style: None,
            inline_styles: Vec::new(),
        }],
    };
    assert!(merge_seed(&seed, &base, std::slice::from_ref(&styled.history)).is_err());
    let current = merge_histories(&base, std::slice::from_ref(&styled.history)).unwrap();
    assert!(edit_style_seed(
        &seed,
        &base,
        &styled.history,
        &StoryHistoryStyleEdit {
            expected_history_sha256: current.history_sha256.clone(),
            expected_style_conflicts_sha256: current.style_conflicts_sha256,
            actor: "a".into(),
            paragraph_id: "p".into(),
            expected: StoryParagraphStylePatch {
                font_size: Some(15.0),
                ..Default::default()
            },
            replacement: StoryParagraphStylePatch {
                font_size: Some(16.0),
                ..Default::default()
            },
        }
    )
    .is_err());
    assert!(set_active_seed(
        &seed,
        &base,
        &styled.history,
        &StoryHistorySetActive {
            expected_history_sha256: current.history_sha256,
            actor: "a".into(),
            target: StoryOperationId {
                actor: "a".into(),
                sequence: 1,
            },
            expected_active: true,
            active: false,
        }
    )
    .is_err());

    let structure = structured(
        &base,
        &empty,
        "a",
        "p",
        false,
        StoryParagraphStructurePatch {
            present: Some(true),
            ..Default::default()
        },
        StoryParagraphStructurePatch {
            present: Some(false),
            ..Default::default()
        },
    );
    assert!(merge_seed(&seed, &base, std::slice::from_ref(&structure.history)).is_err());
    let structure_current =
        merge_histories(&base, std::slice::from_ref(&structure.history)).unwrap();
    assert!(edit_structure_seed(
        &seed,
        &base,
        &structure.history,
        &StoryHistoryStructureEdit {
            expected_history_sha256: structure_current.history_sha256.clone(),
            expected_structure_conflicts_sha256: structure_current.structure_conflicts_sha256,
            actor: "a".into(),
            paragraph_id: "p".into(),
            expected_absent: false,
            expected: StoryParagraphStructurePatch {
                present: Some(false),
                ..Default::default()
            },
            replacement: StoryParagraphStructurePatch {
                present: Some(true),
                ..Default::default()
            },
        }
    )
    .is_err());
    assert!(set_active_seed(
        &seed,
        &base,
        &structure.history,
        &StoryHistorySetActive {
            expected_history_sha256: structure_current.history_sha256,
            actor: "a".into(),
            target: StoryOperationId {
                actor: "a".into(),
                sequence: 1,
            },
            expected_active: true,
            active: false,
        }
    )
    .is_err());
}

#[test]
fn concurrent_runs_converge_commutatively_idempotently_and_without_character_interleaving() {
    let base = base("AB");
    let empty = new_history(&base).unwrap();
    let a = edited(&base, &empty, "a", [1, 1], "one");
    let b = edited(&base, &empty, "b", [1, 1], "two");
    let ab = merge_histories(&base, &[a.history.clone(), b.history.clone()]).unwrap();
    let ba = merge_histories(&base, &[b.history.clone(), a.history.clone()]).unwrap();
    assert_eq!(text(&ab), "AtwooneB");
    assert_eq!(ab.history_sha256, ba.history_sha256);
    let twice = merge_histories(
        &base,
        &[ab.history.clone(), a.history, b.history, ab.history.clone()],
    )
    .unwrap();
    assert_eq!(twice.history_sha256, ab.history_sha256);
    assert_eq!(text(&twice), text(&ab));
    assert_eq!(
        ab.merged.as_ref().unwrap().paragraphs[0].font_size,
        base.paragraphs[0].font_size
    );
}

#[test]
fn observed_deletion_retains_unseen_concurrent_insertions_and_tombstone_anchors() {
    let base = base("AB");
    let empty = new_history(&base).unwrap();
    let removed = edited(&base, &empty, "a", [0, 1], "");
    let inserted = edited(&base, &empty, "b", [1, 1], "X");
    let result = merge_histories(&base, &[removed.history, inserted.history]).unwrap();
    assert_eq!(text(&result), "XB");
    assert_eq!(result.tombstone_count, 1);
    let next = edited(&base, &result.history, "b", [1, 1], "Y");
    assert_eq!(text(&next), "XYB");
}

#[test]
fn concurrent_replacements_preserve_both_proposals_instead_of_a_last_writer_winner() {
    let base = base("AB");
    let empty = new_history(&base).unwrap();
    let a = edited(&base, &empty, "a", [0, 1], "X");
    let b = edited(&base, &empty, "b", [0, 1], "Y");
    let result = merge_histories(&base, &[a.history, b.history]).unwrap();
    assert_eq!(text(&result), "YXB");
    assert_eq!(result.tombstone_count, 1);
    // Explicit later review resolves wording with a new operation; tombstones
    // never resurrect an old operation or rewrite its identity.
    assert_eq!(
        text(&edited(
            &base,
            &result.history,
            "reviewer",
            [0, 2],
            "chosen"
        )),
        "chosenB"
    );
}

#[test]
fn out_of_order_delivery_retains_operations_and_delta_rejoin_closes_dependencies() {
    let base = base("AB");
    let empty = new_history(&base).unwrap();
    let a1 = edited(&base, &empty, "a", [1, 1], "x");
    let a2 = edited(&base, &a1.history, "a", [2, 2], "y");
    let delta = export_delta(&base, &a2.history, &a1.frontier).unwrap();
    assert_eq!(delta.operations.len(), 1);
    let waiting = merge_histories(&base, &[delta.clone()]).unwrap();
    assert!(waiting.merged.is_none());
    assert_eq!(waiting.history.operations.len(), 1);
    assert!(waiting.frontier.is_empty());
    assert_eq!(
        waiting.missing_dependencies,
        vec![StoryOperationId {
            actor: "a".into(),
            sequence: 1
        }]
    );
    let joined = merge_histories(&base, &[waiting.history, a1.history]).unwrap();
    assert_eq!(text(&joined), "AxyB");
    assert_eq!(joined.history_sha256, a2.history_sha256);
    let persisted: StoryTextHistory =
        serde_json::from_slice(&serde_json::to_vec(&joined.history).unwrap()).unwrap();
    assert_eq!(
        merge_histories(&base, &[persisted, delta])
            .unwrap()
            .history_sha256,
        joined.history_sha256
    );
}

#[test]
fn nested_delivery_associativity_and_branch_permutations_have_one_history_and_projection() {
    let base = base("AB");
    let empty = new_history(&base).unwrap();
    let histories = [("a", "one"), ("b", "two"), ("c", "three")]
        .map(|(actor, text)| edited(&base, &empty, actor, [1, 1], text).history);
    let joined = merge_histories(&base, &histories).unwrap();
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let first = merge_histories(
            &base,
            &[histories[order[0]].clone(), histories[order[1]].clone()],
        )
        .unwrap();
        let combined =
            merge_histories(&base, &[histories[order[2]].clone(), first.history]).unwrap();
        assert_eq!(combined.history_sha256, joined.history_sha256);
        assert_eq!(text(&combined), "AthreetwooneB");
    }
}

#[test]
fn actor_equivocation_stale_preimages_and_clock_inflation_are_rejected() {
    let base = base("AB");
    let empty = new_history(&base).unwrap();
    let a = edited(&base, &empty, "a", [1, 1], "x");
    let mut forged = a.history.clone();
    forged.operations[0].inserted = "different".into();
    assert!(merge_histories(&base, &[a.history.clone(), forged]).is_err());
    let mut inflated = a.history.clone();
    inflated.operations[0].lamport = MAX_CLOCK;
    assert!(merge_histories(&base, &[inflated]).is_err());
    let edit = StoryHistoryEdit {
        actor: "a".into(),
        paragraph_id: "p".into(),
        range: [1, 1],
        expected_text: String::new(),
        replacement: "z".into(),
        expected_history_sha256: "stale".into(),
    };
    assert!(edit_history(&base, &a.history, &edit).is_err());
    let mut wrong = base.clone();
    wrong.paragraphs[0].font_size = 13.0;
    assert!(merge_histories(&wrong, &[a.history]).is_err());
}

#[test]
fn causality_closure_cross_paragraph_and_out_of_bounds_atoms_cannot_enter_a_projection() {
    let mut base = base("AB");
    let mut second = base.paragraphs[0].clone();
    second.id = "q".into();
    base.paragraphs.push(second);
    let empty = new_history(&base).unwrap();
    let a = edited(&base, &empty, "a", [1, 1], "x");
    let b = edited(&base, &a.history, "b", [2, 2], "y");
    let c = edited(&base, &b.history, "c", [3, 3], "z");
    let mut forged = c.history.clone();
    forged
        .operations
        .iter_mut()
        .find(|op| op.id.actor == "c")
        .unwrap()
        .context
        .remove("a");
    assert!(merge_histories(&base, &[forged]).is_err());
    let mut forged = b.history.clone();
    forged
        .operations
        .iter_mut()
        .find(|op| op.id.actor == "b")
        .unwrap()
        .paragraph_id = "q".into();
    assert!(merge_histories(&base, &[forged]).is_err());
    let mut forged = b.history;
    forged
        .operations
        .iter_mut()
        .find(|op| op.id.actor == "b")
        .unwrap()
        .after
        .as_mut()
        .unwrap()
        .offset = 99;
    assert!(merge_histories(&base, &[forged]).is_err());
}

#[test]
fn unicode_scalar_atoms_keep_grapheme_safe_local_edits_and_exact_utf8_preimages() {
    let base = base("a😀e\u{301}z");
    let empty = new_history(&base).unwrap();
    let start = merge_histories(&base, &[empty.clone()]).unwrap();
    let bad = StoryHistoryEdit {
        expected_history_sha256: start.history_sha256,
        actor: "a".into(),
        paragraph_id: "p".into(),
        range: [5, 6],
        expected_text: "e".into(),
        replacement: "Q".into(),
    };
    assert!(edit_history(&base, &empty, &bad).is_err());
    let good = edited(&base, &empty, "a", [5, 8], "Q");
    assert_eq!(text(&good), "a😀Qz");
    let supplementary = edited(&base, &good.history, "a", [1, 5], "🇮🇳");
    assert_eq!(text(&supplementary), "a🇮🇳Qz");
    assert_eq!(supplementary.history.operations[1].removed[0].start, 1);
    assert_eq!(supplementary.history.operations[1].removed[0].end, 2);
}

#[test]
fn no_op_and_cancelled_history_work_do_not_change_the_source_or_append_operations() {
    let base = base("AB");
    let empty = new_history(&base).unwrap();
    let before = serde_json::to_vec(&base).unwrap();
    assert!(edited(&base, &empty, "a", [0, 1], "A")
        .history
        .operations
        .is_empty());
    let cancelled = crate::CancelToken::new();
    cancelled.cancel();
    assert!(cancelled
        .scope(|| merge_histories(&base, &[empty]))
        .is_err());
    assert_eq!(serde_json::to_vec(&base).unwrap(), before);
}

#[test]
fn duplicate_json_actor_context_keys_do_not_silently_choose_a_winner() {
    let duplicate = r#"{"id":{"actor":"b","sequence":1},"lamport":2,"context":{"a":1,"a":1},"paragraph_id":"p","after":null,"removed":[],"inserted":"X"}"#;
    assert!(serde_json::from_str::<StoryTextOperation>(duplicate).is_err());
}

fn controlled(
    base: &LinkedStoryRequest,
    history: &StoryTextHistory,
    actor: &str,
    sequence: u64,
    active: bool,
) -> StoryHistoryResult {
    let current = merge_histories(base, std::slice::from_ref(history)).unwrap();
    let target = StoryOperationId {
        actor: actor.into(),
        sequence,
    };
    set_operation_active(
        base,
        history,
        &StoryHistorySetActive {
            expected_history_sha256: current.history_sha256,
            actor: actor.into(),
            expected_active: !current.inactive_operations.contains(&target),
            target,
            active,
        },
    )
    .unwrap()
}

#[test]
fn selective_undo_redo_reuses_original_atoms_instead_of_reinserting_copies() {
    let base = base("ABCD");
    let original = edited(&base, &new_history(&base).unwrap(), "a", [1, 3], "xy");
    let undo = controlled(&base, &original.history, "a", 1, false);
    assert_eq!(text(&undo), "ABCD");
    assert_eq!(undo.history.schema_version, 2);
    assert_eq!(undo.atom_count, original.atom_count);
    assert_eq!(undo.suppressed_atom_count, 2);
    assert_eq!(undo.tombstone_count, 0);
    assert_eq!(undo.history.operations[0], original.history.operations[0]);
    let redo = controlled(&base, &undo.history, "a", 1, true);
    assert_eq!(text(&redo), text(&original));
    assert_eq!(redo.atom_count, original.atom_count);
    assert!(redo.inactive_operations.is_empty());
    assert_eq!(redo.history.operations.len(), 3);
    assert_eq!(redo.frontier.get("a"), Some(&3));
}

#[test]
fn grouped_undo_redo_is_atomic_canonical_and_reuses_original_atoms() {
    let base = base("ABCD");
    let first = edited(&base, &new_history(&base).unwrap(), "a", [0, 1], "X");
    let second = edited(&base, &first.history, "a", [3, 4], "Y");
    assert_eq!(text(&second), "XBCY");
    let targets = vec![
        StoryOperationId {
            actor: "a".into(),
            sequence: 2,
        },
        StoryOperationId {
            actor: "a".into(),
            sequence: 1,
        },
    ];
    let undone = set_operations_active(
        &base,
        &second.history,
        &StoryHistorySetManyActive {
            expected_history_sha256: second.history_sha256,
            actor: "a".into(),
            targets: targets.clone(),
            expected_active: true,
            active: false,
        },
    )
    .unwrap();
    assert_eq!(text(&undone), "ABCD");
    assert_eq!(undone.history.operations.len(), 4);
    assert_eq!(undone.frontier.get("a"), Some(&4));
    assert_eq!(
        undone.inactive_operations,
        vec![
            StoryOperationId {
                actor: "a".into(),
                sequence: 1,
            },
            StoryOperationId {
                actor: "a".into(),
                sequence: 2,
            },
        ]
    );
    let redone = set_operations_active(
        &base,
        &undone.history,
        &StoryHistorySetManyActive {
            expected_history_sha256: undone.history_sha256,
            actor: "a".into(),
            targets,
            expected_active: false,
            active: true,
        },
    )
    .unwrap();
    assert_eq!(text(&redone), "XBCY");
    assert_eq!(redone.atom_count, second.atom_count);
    assert!(redone.inactive_operations.is_empty());
    assert_eq!(redone.frontier.get("a"), Some(&6));
}

#[test]
fn grouped_activity_rejects_mixed_replicas_duplicates_and_partial_preimages() {
    let base = base("ABCD");
    let empty = new_history(&base).unwrap();
    let a = edited(&base, &empty, "a", [0, 1], "X");
    let a2 = edited(&base, &a.history, "a", [3, 4], "Y");
    let b = edited(&base, &empty, "b", [1, 2], "Z");
    let joined = merge_histories(&base, &[a2.history.clone(), b.history]).unwrap();
    let one = StoryOperationId {
        actor: "a".into(),
        sequence: 1,
    };
    let two = StoryOperationId {
        actor: "a".into(),
        sequence: 2,
    };
    let mut change = StoryHistorySetManyActive {
        expected_history_sha256: joined.history_sha256.clone(),
        actor: "a".into(),
        targets: vec![one.clone(), one.clone()],
        expected_active: true,
        active: false,
    };
    assert!(set_operations_active(&base, &joined.history, &change).is_err());
    change.targets = vec![
        one.clone(),
        StoryOperationId {
            actor: "b".into(),
            sequence: 1,
        },
    ];
    assert!(set_operations_active(&base, &joined.history, &change).is_err());

    let partly_undone = controlled(&base, &joined.history, "a", 1, false);
    change.expected_history_sha256 = partly_undone.history_sha256;
    change.targets = vec![one, two];
    assert!(set_operations_active(&base, &partly_undone.history, &change).is_err());
}

#[test]
fn undo_keeps_remote_descendants_of_a_hidden_insertion_and_redo_keeps_remote_deletions() {
    let base = base("AB");
    let original = edited(&base, &new_history(&base).unwrap(), "a", [1, 1], "X");
    let remote = edited(&base, &original.history, "b", [2, 2], "Y");
    let undo = controlled(&base, &remote.history, "a", 1, false);
    assert_eq!(text(&undo), "AYB");
    assert_eq!(
        text(&controlled(&base, &undo.history, "a", 1, true)),
        "AXYB"
    );
    let remote_deleted = edited(&base, &remote.history, "b", [1, 2], "");
    let undo = controlled(&base, &remote_deleted.history, "a", 1, false);
    assert_eq!(text(&undo), "AYB");
    let redo = controlled(&base, &undo.history, "a", 1, true);
    assert_eq!(text(&redo), "AYB");
    assert_eq!(redo.tombstone_count, 1);
}

#[test]
fn overlapping_deletions_have_independent_activity_and_cannot_resurrect_remote_deleted_atoms() {
    let base = base("ABC");
    let empty = new_history(&base).unwrap();
    let a = edited(&base, &empty, "a", [1, 2], "");
    let b = edited(&base, &empty, "b", [1, 2], "");
    let both = merge_histories(&base, &[a.history, b.history]).unwrap();
    let undo_a = controlled(&base, &both.history, "a", 1, false);
    assert_eq!(text(&undo_a), "AC");
    let undo_b = controlled(&base, &undo_a.history, "b", 1, false);
    assert_eq!(text(&undo_b), "ABC");
    assert_eq!(
        text(&controlled(&base, &undo_b.history, "a", 1, true)),
        "AC"
    );
}

#[test]
fn independent_replacements_keep_the_other_authors_words_during_selective_undo() {
    let base = base("ABC");
    let empty = new_history(&base).unwrap();
    let a = edited(&base, &empty, "a", [1, 2], "X");
    let b = edited(&base, &empty, "b", [1, 2], "Y");
    let joined = merge_histories(&base, &[a.history, b.history]).unwrap();
    assert_eq!(text(&joined), "AYXC");
    let undo_a = controlled(&base, &joined.history, "a", 1, false);
    assert_eq!(text(&undo_a), "AYC");
    let undo_b = controlled(&base, &undo_a.history, "b", 1, false);
    assert_eq!(text(&undo_b), "ABC");
    assert_eq!(
        text(&controlled(&base, &undo_b.history, "a", 1, true)),
        "AXC"
    );
}

#[test]
fn undoing_a_delete_does_not_reactivate_an_undone_insertion() {
    let base = base("AB");
    let inserted = edited(&base, &new_history(&base).unwrap(), "a", [1, 1], "X");
    let deleted = edited(&base, &inserted.history, "b", [1, 2], "");
    let hidden = controlled(&base, &deleted.history, "a", 1, false);
    let restored_deletion = controlled(&base, &hidden.history, "b", 1, false);
    assert_eq!(text(&restored_deletion), "AB");
    assert_eq!(restored_deletion.suppressed_atom_count, 1);
    assert_eq!(
        text(&controlled(&base, &restored_deletion.history, "a", 1, true)),
        "AXB"
    );
}

#[test]
fn controls_share_actor_sequences_and_new_edits_keep_restored_unicode_atom_identity() {
    let base = base("a\u{1f600}e\u{301}z");
    let changed = edited(&base, &new_history(&base).unwrap(), "a", [5, 8], "Q");
    let undo = controlled(&base, &changed.history, "a", 1, false);
    assert_eq!(text(&undo), base.paragraphs[0].text);
    let next = edited(&base, &undo.history, "a", [5, 8], "R");
    assert_eq!(next.frontier.get("a"), Some(&3));
    assert_eq!(text(&next), "a\u{1f600}Rz");
    assert_eq!(next.history.operations[2].removed[0].operation, None);
    let without_later = controlled(&base, &next.history, "a", 3, false);
    assert_eq!(text(&without_later), base.paragraphs[0].text);
}

#[test]
fn control_deltas_wait_for_dependencies_and_converge_in_any_delivery_order() {
    let base = base("AB");
    let empty = new_history(&base).unwrap();
    let a = edited(&base, &empty, "a", [1, 1], "X");
    let undo = controlled(&base, &a.history, "a", 1, false);
    let b = edited(&base, &empty, "b", [1, 1], "Y");
    let delta = export_delta(&base, &undo.history, &a.frontier).unwrap();
    let partial = merge_histories(&base, &[delta.clone()]).unwrap();
    assert!(partial.merged.is_none());
    assert_eq!(partial.history.operations.len(), 1);
    assert_eq!(
        partial.missing_dependencies,
        vec![StoryOperationId {
            actor: "a".into(),
            sequence: 1
        }]
    );
    let branches = [a.history, delta, b.history];
    let expected = merge_histories(&base, &branches).unwrap();
    assert_eq!(text(&expected), "AYB");
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let first = merge_histories(
            &base,
            &[branches[order[0]].clone(), branches[order[1]].clone()],
        )
        .unwrap();
        let final_result = merge_histories(
            &base,
            &[
                first.history,
                branches[order[2]].clone(),
                branches[order[0]].clone(),
            ],
        )
        .unwrap();
        assert_eq!(final_result.history_sha256, expected.history_sha256);
        assert_eq!(text(&final_result), text(&expected));
        assert_eq!(
            final_result.inactive_operations,
            expected.inactive_operations
        );
    }
    let persisted: StoryTextHistory =
        serde_json::from_slice(&serde_json::to_vec(&expected.history).unwrap()).unwrap();
    assert_eq!(text(&merge_histories(&base, &[persisted]).unwrap()), "AYB");
}

#[test]
fn control_schema_ownership_payload_and_target_validation_cannot_be_bypassed() {
    let base = base("AB");
    let empty = new_history(&base).unwrap();
    let a = edited(&base, &empty, "a", [1, 1], "X");
    let undo = controlled(&base, &a.history, "a", 1, false);
    for kind in 0..6 {
        let mut bad = undo.history.clone();
        match kind {
            0 => bad.schema_version = 1,
            1 => bad.operations[1].inserted = "hidden payload".into(),
            2 => bad.operations[1].visibility.as_mut().unwrap().target.actor = "b".into(),
            3 => {
                bad.operations[1]
                    .visibility
                    .as_mut()
                    .unwrap()
                    .target
                    .sequence = 2
            }
            4 => bad.operations[1].context.clear(),
            _ => bad.operations[1].paragraph_id = "absent".into(),
        }
        assert!(merge_histories(&base, &[bad]).is_err());
    }
    let mut targeting_control = controlled(&base, &undo.history, "a", 1, true).history;
    targeting_control.operations[2]
        .visibility
        .as_mut()
        .unwrap()
        .target
        .sequence = 2;
    assert!(merge_histories(&base, &[targeting_control]).is_err());
    let mut malformed_disabled = undo.history.clone();
    malformed_disabled.operations[0]
        .after
        .as_mut()
        .unwrap()
        .offset = 99;
    assert!(merge_histories(&base, &[malformed_disabled]).is_err());
    let mut reused = undo.history.clone();
    reused.operations[1].visibility.as_mut().unwrap().active = true;
    assert!(merge_histories(&base, &[undo.history, reused]).is_err());
}

#[test]
fn activity_cas_noops_and_cancellation_leave_the_existing_history_unchanged() {
    let base = base("AB");
    let a = edited(&base, &new_history(&base).unwrap(), "a", [1, 1], "X");
    let before = serde_json::to_vec(&a.history).unwrap();
    let mut change = StoryHistorySetActive {
        expected_history_sha256: a.history_sha256.clone(),
        actor: "a".into(),
        target: StoryOperationId {
            actor: "a".into(),
            sequence: 1,
        },
        expected_active: true,
        active: true,
    };
    assert_eq!(
        set_operation_active(&base, &a.history, &change)
            .unwrap()
            .history,
        a.history
    );
    change.expected_active = false;
    assert!(set_operation_active(&base, &a.history, &change).is_err());
    change.expected_active = true;
    change.active = false;
    let undone = set_operation_active(&base, &a.history, &change).unwrap();
    assert!(set_operation_active(&base, &undone.history, &change).is_err());
    change.actor = "different".into();
    assert!(set_operation_active(&base, &a.history, &change).is_err());
    change.actor = "a".into();
    let cancelled = crate::CancelToken::new();
    cancelled.cancel();
    assert!(cancelled
        .scope(|| set_operation_active(&base, &a.history, &change))
        .is_err());
    assert_eq!(serde_json::to_vec(&a.history).unwrap(), before);
}

#[test]
fn schema_one_text_history_keeps_its_canonical_representation_until_a_control_is_added() {
    let base = base("AB");
    let original = edited(&base, &new_history(&base).unwrap(), "a", [1, 1], "X");
    let serialized = serde_json::to_string(&original.history).unwrap();
    assert!(!serialized.contains("visibility"));
    assert_eq!(original.history.schema_version, 1);
    let mut json = serde_json::to_value(&original.history).unwrap();
    json["schema_version"] = serde_json::json!(2);
    let delta: StoryTextHistory = serde_json::from_value(json).unwrap();
    assert_eq!(
        merge_histories(&base, &[delta]).unwrap().history_sha256,
        original.history_sha256
    );
    let undo = controlled(&base, &original.history, "a", 1, false);
    assert_eq!(undo.history.schema_version, 2);
    assert!(serde_json::to_string(&undo.history)
        .unwrap()
        .contains("visibility"));
}
