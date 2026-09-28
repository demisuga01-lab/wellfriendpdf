//! Source regressions only. Not compiled or executed in the source-only phase.
use super::*;

fn base() -> LinkedStoryRequest {
    serde_json::from_value(serde_json::json!({"story_id":"s","input_sha256":"revision","fonts":[],
        "frames":[{"id":"frame/body","page":1,"logical_range":[0,0],"expected_text":"","rect":[0,0,300,600]}],
        "paragraphs":[{"id":"p","text":"alpha beta","preferred_font":"Helvetica","font_size":12,"line_height":16},
        {"id":"q","text":"unchanged","preferred_font":"Helvetica","font_size":12,"line_height":16},
        {"id":"r","text":"last","preferred_font":"Helvetica","font_size":12,"line_height":16}]})).unwrap()
}
fn branches(
    base: &LinkedStoryRequest,
    snapshots: Vec<LinkedStoryRequest>,
) -> StoryStructureMergeRequest {
    StoryStructureMergeRequest {
        base: base.clone(),
        branches: snapshots
            .into_iter()
            .enumerate()
            .map(|(i, proposed)| StoryStructureBranch {
                branch_id: format!("branch-{i}"),
                base_story_sha256: story_fingerprint(base).unwrap(),
                proposed,
            })
            .collect(),
    }
}
fn decisions(review: &StructureReview) -> StructureResolution {
    StructureResolution {
        expected_review_sha256: review.review_sha256.clone(),
        acknowledged_conflicts: review
            .conflicts
            .iter()
            .map(|c| c.conflict_id.clone())
            .collect(),
        paragraphs: review
            .candidate
            .paragraphs
            .iter()
            .chain(&review.unplaced_paragraphs)
            .cloned()
            .collect(),
        frame_geometry: review
            .candidate
            .frames
            .iter()
            .map(|f| ResolvedFrameGeometry {
                frame_id: f.id.clone(),
                rect: f.rect,
                exclusions: f.exclusions.clone(),
            })
            .collect(),
    }
}
fn text_conflict() -> StoryStructureMergeRequest {
    let base = base();
    let mut a = base.clone();
    let mut b = base.clone();
    a.paragraphs[0].text = "first beta".into();
    b.paragraphs[0].text = "second beta".into();
    a.paragraphs[1].text = "automatic change".into();
    branches(&base, vec![a, b])
}

#[test]
fn text_conflict_cannot_authorize_an_unrelated_previously_omitted_wrap_policy() {
    let request = text_conflict();
    let review = review_story_structure(&request).unwrap();
    let mut resolution = decisions(&review);
    resolution.paragraphs[0].text = "reviewed text".into();
    resolution.paragraphs[0].line_break.emergency =
        crate::fonts::line_break_policy::EmergencyWrap::PreserveWords;
    assert!(resolve_story_structure(&request, &resolution).is_err());
}

#[test]
fn competing_wrap_policies_expose_typed_alternatives_and_require_exact_review() {
    use crate::fonts::line_break_policy::{EmergencyWrap, LineBreakProfile};
    let base = base();
    let mut a = base.clone();
    let mut b = base.clone();
    a.paragraphs[0].line_break.profile = LineBreakProfile::JapaneseStrict;
    a.paragraphs[0].line_break.composition =
        crate::fonts::line_break_policy::LineComposition::Balanced;
    b.paragraphs[0].line_break.emergency = EmergencyWrap::PreserveWords;
    let request = branches(&base, vec![a, b]);
    let review = review_story_structure(&request).unwrap();
    assert_eq!(review.conflicts.len(), 1);
    assert!(
        matches!(&review.conflicts[0].target, StructureConflictTarget::ParagraphField { field, .. } if field == "line_break")
    );
    assert_eq!(review.conflicts[0].alternatives.len(), 3);
    let mut resolution = decisions(&review);
    resolution.paragraphs[0].line_break.profile = LineBreakProfile::JapaneseStrict;
    resolution.paragraphs[0].line_break.composition =
        crate::fonts::line_break_policy::LineComposition::Balanced;
    resolution.paragraphs[0].line_break.emergency = EmergencyWrap::PreserveWords;
    let result = resolve_story_structure(&request, &resolution).unwrap();
    assert_eq!(
        result.merged.paragraphs[0].line_break,
        resolution.paragraphs[0].line_break
    );
}

#[test]
fn explicit_text_resolution_preserves_automatic_changes_and_source_authority() {
    let request = text_conflict();
    let review = review_story_structure(&request).unwrap();
    assert_eq!(review.conflicts.len(), 1);
    let mut resolution = decisions(&review);
    resolution.paragraphs[0].text = "reviewed wording".into();
    let resolved = resolve_story_structure(&request, &resolution).unwrap();
    assert_eq!(resolved.merged.paragraphs[0].text, "reviewed wording");
    assert_eq!(resolved.merged.paragraphs[1].text, "automatic change");
    assert_eq!(
        authority(&resolved.merged).unwrap(),
        authority(&request.base).unwrap()
    );
    assert_eq!(
        json(&resolved.merged.frames).unwrap(),
        json(&request.base.frames).unwrap()
    );
    resolution.paragraphs[1].text = "overwritten unrelated value".into();
    assert!(resolve_story_structure(&request, &resolution).is_err());
}

#[test]
fn style_and_geometry_conflicts_are_scoped_and_typed_ids_can_contain_slashes() {
    let mut base = base();
    base.paragraphs[0].id = "p/text/with/slashes".into();
    let mut a = base.clone();
    let mut b = base.clone();
    a.paragraphs[0].font_size = 13.0;
    b.paragraphs[0].font_size = 14.0;
    a.frames[0].rect[2] = 310.0;
    b.frames[0].rect[2] = 320.0;
    let request = branches(&base, vec![a, b]);
    let review = review_story_structure(&request).unwrap();
    assert!(review.conflicts.iter().any(|c|matches!(&c.target,StructureConflictTarget::ParagraphField{paragraph_id,field} if paragraph_id=="p/text/with/slashes"&&field=="font_size")));
    let mut resolution = decisions(&review);
    resolution.paragraphs[0].font_size = 13.5;
    resolution.frame_geometry[0].rect[2] = 315.0;
    let resolved = resolve_story_structure(&request, &resolution).unwrap();
    assert_eq!(resolved.merged.paragraphs[0].font_size, 13.5);
    resolution.paragraphs[0].text = "not a text conflict".into();
    assert!(resolve_story_structure(&request, &resolution).is_err());
    resolution.paragraphs[0].text = base.paragraphs[0].text.clone();
    resolution.frame_geometry[0].frame_id = "other".into();
    assert!(resolve_story_structure(&request, &resolution).is_err());
}

#[test]
fn stale_missing_duplicate_and_unknown_conflict_acknowledgments_are_rejected() {
    let request = text_conflict();
    let review = review_story_structure(&request).unwrap();
    for kind in 0..4 {
        let mut resolution = decisions(&review);
        match kind {
            0 => resolution.expected_review_sha256 = "old".into(),
            1 => resolution.acknowledged_conflicts.clear(),
            2 => resolution
                .acknowledged_conflicts
                .push(resolution.acknowledged_conflicts[0].clone()),
            _ => resolution.acknowledged_conflicts.push("unreviewed".into()),
        }
        assert!(resolve_story_structure(&request, &resolution).is_err());
    }
    let mut changed = request.clone();
    changed.branches[0].proposed.paragraphs[0].text = "changed branch".into();
    assert!(resolve_story_structure(&changed, &decisions(&review)).is_err());
    let mut reordered = request.clone();
    reordered.branches.reverse();
    assert_eq!(
        review_story_structure(&reordered).unwrap().review_sha256,
        review.review_sha256
    );
}

#[test]
fn delete_edit_conflict_can_keep_or_delete_only_the_affected_paragraph() {
    let base = base();
    let mut a = base.clone();
    let mut b = base.clone();
    a.paragraphs.remove(0);
    b.paragraphs[0].text = "edited".into();
    let request = branches(&base, vec![a, b.clone()]);
    let review = review_story_structure(&request).unwrap();
    let mut resolution = decisions(&review);
    assert_eq!(resolution.paragraphs.len(), 2);
    assert_eq!(
        resolve_story_structure(&request, &resolution)
            .unwrap()
            .merged
            .paragraphs
            .len(),
        2
    );
    resolution.paragraphs.insert(0, b.paragraphs[0].clone());
    assert_eq!(
        resolve_story_structure(&request, &resolution)
            .unwrap()
            .merged
            .paragraphs[0]
            .text,
        "edited"
    );
    resolution.paragraphs.pop();
    assert!(resolve_story_structure(&request, &resolution).is_err());
}

#[test]
fn competing_insertions_can_be_combined_or_selected_without_dropping_other_insertions() {
    let base = base();
    let mut a = base.clone();
    let mut b = base.clone();
    let mut x = base.paragraphs[0].clone();
    x.id = "x".into();
    x.text = "left".into();
    let mut y = x.clone();
    y.id = "y".into();
    y.text = "right".into();
    let mut z = x.clone();
    z.id = "z".into();
    z.text = "independent".into();
    a.paragraphs.insert(1, x);
    a.paragraphs.push(z);
    b.paragraphs.insert(1, y);
    let request = branches(&base, vec![a, b]);
    let review = review_story_structure(&request).unwrap();
    assert_eq!(review.unplaced_paragraphs.len(), 1);
    let mut resolution = decisions(&review);
    resolution.paragraphs.retain(|p| p.id != "y");
    assert!(resolve_story_structure(&request, &resolution).is_ok());
    resolution.paragraphs.retain(|p| p.id != "z");
    assert!(resolve_story_structure(&request, &resolution).is_err());
}

#[test]
fn reordered_insertion_gap_requires_a_complete_reviewed_sequence() {
    let base = base();
    let mut a = base.clone();
    let mut b = base.clone();
    a.paragraphs.rotate_left(1);
    let mut z = base.paragraphs[0].clone();
    z.id = "z".into();
    b.paragraphs.insert(1, z);
    let request = branches(&base, vec![a, b]);
    let review = review_story_structure(&request).unwrap();
    assert!(review
        .conflicts
        .iter()
        .any(|c| c.target == StructureConflictTarget::ParagraphOrder));
    let mut resolution = decisions(&review);
    let z = resolution.paragraphs.pop().unwrap();
    assert_eq!(z.id, "z");
    resolution.paragraphs.insert(1, z);
    assert_eq!(
        resolve_story_structure(&request, &resolution)
            .unwrap()
            .merged
            .paragraphs
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        vec!["q", "z", "r", "p"]
    );
    resolution.paragraphs.retain(|p| p.id != "z");
    assert!(resolve_story_structure(&request, &resolution).is_err());
}

#[test]
fn typed_table_edits_are_not_silently_replaced_by_calculated_values() {
    let mut base = base();
    base.paragraphs.truncate(1);
    base.paragraphs[0].text = "approved".into();
    base.table_layout = Some(
        serde_json::from_value(
            serde_json::json!({"column_weights":[1],"rows":[{"id":"row"}],
        "cells":[{"id":"p","row":0,"column":0,"value":{"kind":"text","text":"approved"}}]}),
        )
        .unwrap(),
    );
    let mut changed = base.clone();
    changed.paragraphs[0].text = "proposal".into();
    let request = branches(&base, vec![changed]);
    assert!(merge_story_structure(&request).unwrap().merged.is_none());
    let review = review_story_structure(&request).unwrap();
    assert_eq!(review.candidate.paragraphs[0].text, "proposal");
    let mut resolution = decisions(&review);
    assert!(resolve_story_structure(&request, &resolution).is_err());
    resolution.paragraphs[0].text = "approved".into();
    assert!(resolve_story_structure(&request, &resolution).is_ok());
}

#[test]
fn source_or_policy_changes_are_rejected_before_creating_a_review() {
    for kind in 0..4 {
        let base = base();
        let mut proposed = base.clone();
        match kind {
            0 => proposed.signature_policy_override = true,
            1 => proposed.allow_font_substitution = true,
            2 => proposed.frames[0].page = 2,
            _ => proposed.frames[0].logical_range = [1, 2],
        }
        assert!(review_story_structure(&branches(&base, vec![proposed])).is_err());
    }
    let request = text_conflict();
    let review = review_story_structure(&request).unwrap();
    let mut json = serde_json::to_value(decisions(&review)).unwrap();
    json["signature_policy_override"] = Value::Bool(true);
    assert!(serde_json::from_value::<StructureResolution>(json).is_err());
}

#[test]
fn nonconflicting_order_unknown_identity_invalid_style_and_cancelled_work_are_rejected() {
    let request = text_conflict();
    let review = review_story_structure(&request).unwrap();
    for kind in 0..4 {
        let mut resolution = decisions(&review);
        match kind {
            0 => resolution.paragraphs.swap(1, 2),
            1 => resolution.paragraphs[0].id = "unknown".into(),
            2 => resolution.paragraphs[0].font_size = -1.0,
            _ => resolution.frame_geometry[0].rect[0] = f64::NAN,
        }
        assert!(resolve_story_structure(&request, &resolution).is_err());
    }
    let cancelled = crate::CancelToken::new();
    cancelled.cancel();
    assert!(cancelled
        .scope(|| resolve_story_structure(&request, &decisions(&review)))
        .is_err());
}

#[test]
fn no_conflict_resolution_preserves_the_entire_automatic_candidate() {
    let base = base();
    let mut a = base.clone();
    a.paragraphs[1].text = "automatic".into();
    let request = branches(&base, vec![a]);
    let review = review_story_structure(&request).unwrap();
    assert!(review.conflicts.is_empty());
    let resolved = resolve_story_structure(&request, &decisions(&review)).unwrap();
    assert_eq!(
        json(&resolved.merged).unwrap(),
        json(&review.candidate).unwrap()
    );
}

#[test]
fn retained_annotation_requires_its_paragraph_without_granting_style_changes() {
    let mut base = base();
    base.annotation_anchors.push(
        serde_json::from_value(serde_json::json!({
            "annotation_id":"page/1/annotation/0", "paragraph_id":"p",
            "geometry_sha256":"bound-geometry", "offset":[0,0]
        }))
        .unwrap(),
    );
    let mut changed = base.clone();
    changed.paragraphs.remove(0);
    let request = branches(&base, vec![changed]);
    let review = review_story_structure(&request).unwrap();
    assert!(review.conflicts.iter().any(|c| c.target
        == StructureConflictTarget::RetainedParagraph {
            paragraph_id: "p".into()
        }));
    let mut resolution = decisions(&review);
    assert!(resolve_story_structure(&request, &resolution).is_err());
    resolution.paragraphs.insert(0, base.paragraphs[0].clone());
    let result = resolve_story_structure(&request, &resolution).unwrap();
    assert_eq!(
        json(&result.merged.annotation_anchors).unwrap(),
        json(&base.annotation_anchors).unwrap()
    );
    resolution.paragraphs[0].font_size = 20.0;
    assert!(resolve_story_structure(&request, &resolution).is_err());
}

#[test]
fn restoring_deleted_paragraph_restores_original_tag_owner_not_provisional_ordinal() {
    use crate::tagged_structure::story::{StoryTagging, TagReference};
    let owner = |object| TagReference {
        object,
        generation: 0,
        key: None,
    };
    let mut base = base();
    base.source_tags = Some(StoryTagging {
        parent: owner(1),
        selected: vec![owner(2), owner(3), owner(4)],
        insert_at: Some(0),
        paragraph_sources: BTreeMap::new(),
        new_roles: BTreeMap::new(),
        semantic_text: BTreeMap::new(),
        figures: BTreeMap::new(),
    });
    let mut deleted = base.clone();
    deleted.paragraphs.remove(0);
    let mut edited = base.clone();
    edited.paragraphs[0].text = "restored edit".into();
    let request = branches(&base, vec![deleted, edited.clone()]);
    let review = review_story_structure(&request).unwrap();
    assert!(!review
        .candidate
        .source_tags
        .as_ref()
        .unwrap()
        .paragraph_sources
        .contains_key("p"));
    let mut resolution = decisions(&review);
    resolution.paragraphs.push(edited.paragraphs[0].clone());
    let result = resolve_story_structure(&request, &resolution).unwrap();
    let tags = result.merged.source_tags.unwrap();
    assert_eq!(tags.paragraph_sources["p"], Some(owner(2)));
    assert_eq!(tags.paragraph_sources["q"], Some(owner(3)));
    assert_eq!(tags.paragraph_sources["r"], Some(owner(4)));
    assert_eq!(tags.selected, vec![owner(2), owner(3), owner(4)]);
}
