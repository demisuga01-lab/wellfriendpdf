//! Revision-bound three-way merging of logical story structure. No PDF object
//! bytes, source ownership, fonts or mutation permissions are merged implicitly.
use crate::linked_stories::{LinkedStoryRequest, StoryParagraph};
use crate::story_merge::{
    merge_story_branches, story_fingerprint, StoryBranch, StoryMergeRequest, StoryTextPatch,
};
use crate::{Result, WellfriendError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryStructureBranch {
    pub branch_id: String,
    pub base_story_sha256: String,
    /// Complete proposed snapshot, not a sparse update. Missing paragraphs mean
    /// deletion. Frames and source identities may not be added/deleted/rebound.
    pub proposed: LinkedStoryRequest,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryStructureMergeRequest {
    pub base: LinkedStoryRequest,
    pub branches: Vec<StoryStructureBranch>,
}
#[derive(Debug, Clone, Serialize)]
pub struct StoryStructureConflict {
    pub path: String,
    pub branch_ids: Vec<String>,
    pub reason: String,
    pub target: StructureConflictTarget,
}
/// Typed addresses, not slash-delimited paths: paragraph IDs may contain '/'.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StructureConflictTarget {
    FrameField { frame_id: String, field: String },
    ParagraphField { paragraph_id: String, field: String },
    Paragraph { paragraph_id: String },
    ParagraphOrder,
    Insertions { paragraph_ids: Vec<String> },
    TableProjection { paragraph_ids: Vec<String> },
    RetainedParagraph { paragraph_id: String },
}
#[derive(Debug, Clone, Serialize)]
pub struct StoryStructureMergeResult {
    pub merged: Option<LinkedStoryRequest>,
    pub conflicts: Vec<StoryStructureConflict>,
    pub base_story_sha256: String,
}
fn fail(s: &str) -> WellfriendError {
    WellfriendError::invalid_input(s)
}
fn json(value: &impl Serialize) -> Result<Value> {
    serde_json::to_value(value).map_err(|e| fail(&e.to_string()))
}
/// Optional default fields stay absent from persisted old-story hashes, but
/// must participate in field-by-field merge and resolution authorization.
fn paragraph_json(paragraph: &StoryParagraph) -> Result<Value> {
    paragraph.line_break.validate()?;
    let mut value = json(paragraph)?;
    value["line_break"] = json(&paragraph.line_break)?;
    value["page_break_before"] = json(&paragraph.page_break_before)?;
    Ok(value)
}
fn authority(request: &LinkedStoryRequest) -> Result<String> {
    crate::linked_stories::value_hash(&(
        &request.story_id,
        &request.input_sha256,
        &request.fonts,
        &request.annotation_anchors,
        &request.figures,
        &request.figure_removals,
        &request.figure_detachments,
        &request.source_tags,
        &request.table_layout,
        request.allow_font_substitution,
        request.allow_page_creation,
        request.prune_empty_pages,
        request.max_new_pages,
        request.mode,
        request.writing_mode,
        request.signature_policy_override,
    ))
}
fn conflict(
    out: &mut Vec<StoryStructureConflict>,
    path: &str,
    branches: Vec<String>,
    reason: &str,
    target: StructureConflictTarget,
) {
    out.push(StoryStructureConflict {
        path: path.into(),
        branch_ids: branches,
        reason: reason.into(),
        target,
    });
}
fn choose(
    base: &Value,
    values: Vec<(&str, Value)>,
    path: &str,
    out: &mut Vec<StoryStructureConflict>,
    target: StructureConflictTarget,
) -> Value {
    let changed = values
        .into_iter()
        .filter(|(_, v)| v != base)
        .collect::<Vec<_>>();
    if let Some((_, first)) = changed.first() {
        if changed.iter().any(|(_, v)| v != first) {
            conflict(
                out,
                path,
                changed.iter().map(|(id, _)| id.to_string()).collect(),
                "competing values require explicit resolution",
                target,
            );
            base.clone()
        } else {
            first.clone()
        }
    } else {
        base.clone()
    }
}
fn paragraph_map(request: &LinkedStoryRequest) -> Result<BTreeMap<String, StoryParagraph>> {
    let mut map = BTreeMap::new();
    for p in &request.paragraphs {
        if p.id.is_empty() || p.id.len() > 1024 || map.insert(p.id.clone(), p.clone()).is_some() {
            return Err(fail(
                "empty/duplicate paragraph identity in structure branch",
            ));
        }
    }
    Ok(map)
}

/// One minimal contiguous replacement on grapheme boundaries. Multiple hunks
/// within a snapshot are conservatively one hunk; use StoryTextPatch for finer
/// parallel edits. We never infer deletions from matching visible PDF strings.
fn text_patch(base: &str, proposed: &str, paragraph: &str, branch: &str) -> StoryTextPatch {
    let a = base.graphemes(true).collect::<Vec<_>>();
    let b = proposed.graphemes(true).collect::<Vec<_>>();
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let start = a[..prefix].iter().map(|s| s.len()).sum();
    let end = base.len() - a[a.len() - suffix..].iter().map(|s| s.len()).sum::<usize>();
    StoryTextPatch {
        operation_id: branch.into(),
        paragraph_id: paragraph.into(),
        range: [start, end],
        expected_text: base[start..end].into(),
        replacement: b[prefix..b.len() - suffix].concat(),
    }
}

pub fn merge_story_structure(
    request: &StoryStructureMergeRequest,
) -> Result<StoryStructureMergeResult> {
    let draft = merge_draft(request)?;
    Ok(StoryStructureMergeResult {
        merged: draft.conflicts.is_empty().then_some(draft.candidate),
        conflicts: draft.conflicts,
        base_story_sha256: draft.fingerprint,
    })
}

struct StructureDraft {
    candidate: LinkedStoryRequest,
    values: BTreeMap<String, StoryParagraph>,
    conflicts: Vec<StoryStructureConflict>,
    fingerprint: String,
}

fn merge_draft(request: &StoryStructureMergeRequest) -> Result<StructureDraft> {
    if request.branches.len() > 64 || request.base.paragraphs.len() > 100_000 {
        return Err(fail("structural merge size budget exceeded"));
    }
    let count = request
        .branches
        .iter()
        .try_fold(request.base.paragraphs.len(), |n, branch| {
            n.checked_add(branch.proposed.paragraphs.len())
        })
        .ok_or_else(|| fail("structural paragraph count overflow"))?;
    if count > 200_000 || request.base.frames.len() > 4096 {
        return Err(fail(
            "aggregate structural merge paragraph/frame budget exceeded",
        ));
    }
    let mut font_bytes = 0usize;
    for story in std::iter::once(&request.base).chain(request.branches.iter().map(|b| &b.proposed))
    {
        for font in &story.fonts {
            font_bytes = font_bytes.saturating_add(font.bytes.len());
            if font_bytes > 256 * 1024 * 1024 {
                return Err(fail("aggregate structural font asset budget exceeded"));
            }
        }
    }
    let mut frame_ids = BTreeSet::new();
    for frame in &request.base.frames {
        if frame.id.is_empty() || frame.id.len() > 1024 || !frame_ids.insert(&frame.id) {
            return Err(fail("invalid/duplicate base frame ID"));
        }
    }
    let fingerprint = story_fingerprint(&request.base)?;
    let base_authority = authority(&request.base)?;
    let mut branches = request.branches.iter().collect::<Vec<_>>();
    branches.sort_by(|a, b| a.branch_id.cmp(&b.branch_id));
    let mut ids = BTreeSet::new();
    let base = paragraph_map(&request.base)?;
    let mut maps = Vec::new();
    let mut budget = request
        .base
        .paragraphs
        .iter()
        .map(|p| p.text.len())
        .sum::<usize>();
    if budget > 16_000_000 {
        return Err(fail("structural merge text budget exceeded"));
    }
    for branch in &branches {
        crate::cancel::check_current_cancel("structural story merge")?;
        if branch.branch_id.is_empty()
            || branch.branch_id.len() > 1024
            || !ids.insert(&branch.branch_id)
            || branch.base_story_sha256 != fingerprint
        {
            return Err(fail("duplicate branch or stale structural merge base"));
        }
        budget = budget.saturating_add(
            branch
                .proposed
                .paragraphs
                .iter()
                .map(|p| p.text.len())
                .sum::<usize>(),
        );
        if budget > 16_000_000 || branch.proposed.paragraphs.len() > 100_000 {
            return Err(fail("structural merge text/paragraph budget exceeded"));
        }
        // Compare all non-editable request fields, including authority, fonts,
        // frame ordering and source bindings. Only geometry is a mergeable view.
        if branch.proposed.frames.len() != request.base.frames.len() {
            return Err(fail("structural merge cannot add/delete source frames"));
        }
        for (proposed, original) in branch.proposed.frames.iter().zip(&request.base.frames) {
            let mut frame = proposed.clone();
            frame.rect = original.rect;
            frame.exclusions = original.exclusions.clone();
            if json(&frame)? != json(original)? {
                return Err(fail("branch changed frame order or source bindings"));
            }
        }
        if authority(&branch.proposed)? != base_authority {
            return Err(fail("branch changed source ownership, font assets or mutation authority; replan explicitly"));
        }
        maps.push(paragraph_map(&branch.proposed)?);
    }
    let mut conflicts = Vec::new();
    let reordered = branches
        .iter()
        .zip(&maps)
        .map(|(branch, map)| {
            let before = request
                .base
                .paragraphs
                .iter()
                .filter(|p| map.contains_key(&p.id))
                .map(|p| &p.id)
                .collect::<Vec<_>>();
            let after = branch
                .proposed
                .paragraphs
                .iter()
                .filter(|p| base.contains_key(&p.id))
                .map(|p| &p.id)
                .collect::<Vec<_>>();
            before != after
        })
        .collect::<Vec<_>>();
    let mut merged = request.base.clone();
    for (index, frame) in merged.frames.iter_mut().enumerate() {
        for field in ["rect", "exclusions"] {
            let original = if field == "rect" {
                json(&request.base.frames[index].rect)?
            } else {
                json(&request.base.frames[index].exclusions)?
            };
            let values = branches
                .iter()
                .map(|b| {
                    Ok((
                        b.branch_id.as_str(),
                        if field == "rect" {
                            json(&b.proposed.frames[index].rect)?
                        } else {
                            json(&b.proposed.frames[index].exclusions)?
                        },
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            let selected = choose(
                &original,
                values,
                &format!("frames/{}/{field}", frame.id),
                &mut conflicts,
                StructureConflictTarget::FrameField {
                    frame_id: frame.id.clone(),
                    field: field.into(),
                },
            );
            if field == "rect" {
                frame.rect = serde_json::from_value(selected).map_err(|e| fail(&e.to_string()))?;
            } else {
                frame.exclusions =
                    serde_json::from_value(selected).map_err(|e| fail(&e.to_string()))?;
            }
        }
    }
    let mut result = BTreeMap::new();
    for (id, original) in &base {
        crate::cancel::check_current_cancel("structural paragraph merge")?;
        let deleted = maps.iter().any(|map| !map.contains_key(id));
        if deleted {
            if maps
                .iter()
                .zip(&reordered)
                .any(|(map, moved)| *moved && map.contains_key(id))
            {
                conflict(&mut conflicts,&format!("paragraphs/{id}/order"),branches.iter().map(|b|b.branch_id.clone()).collect(),
                    "paragraph deletion conflicts with a branch that reorders surviving source paragraphs", StructureConflictTarget::Paragraph { paragraph_id: id.clone() });
            }
            if maps.iter().any(|map| {
                map.get(id)
                    .is_some_and(|p| json(p).ok() != json(original).ok())
            }) {
                conflict(
                    &mut conflicts,
                    &format!("paragraphs/{id}"),
                    branches.iter().map(|b| b.branch_id.clone()).collect(),
                    "paragraph deletion conflicts with a text/style edit",
                    StructureConflictTarget::Paragraph {
                        paragraph_id: id.clone(),
                    },
                );
            }
            continue;
        }
        let mut value = paragraph_json(original)?;
        let fields = value
            .as_object()
            .ok_or_else(|| fail("paragraph schema is not an object"))?
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        let branch_values = maps
            .iter()
            .map(|map| paragraph_json(&map[id]))
            .collect::<Result<Vec<_>>>()?;
        for field in fields
            .iter()
            .filter(|f| f.as_str() != "text" && f.as_str() != "id")
        {
            let values = branch_values
                .iter()
                .zip(&branches)
                .map(|(paragraph, b)| (b.branch_id.as_str(), paragraph[field].clone()))
                .collect::<Vec<_>>();
            value[field] = choose(
                &value[field],
                values,
                &format!("paragraphs/{id}/{field}"),
                &mut conflicts,
                StructureConflictTarget::ParagraphField {
                    paragraph_id: id.clone(),
                    field: field.clone(),
                },
            );
        }
        // This private text-only merge does not need source frames/font bytes.
        // Avoid cloning the full story/large font pool once per paragraph.
        let local = LinkedStoryRequest {
            writing_mode: request.base.writing_mode,
            story_id: request.base.story_id.clone(),
            input_sha256: request.base.input_sha256.clone(),
            frames: Vec::new(),
            paragraphs: vec![original.clone()],
            fonts: Vec::new(),
            annotation_anchors: Vec::new(),
            figures: Vec::new(),
            figure_removals: Vec::new(),
            figure_detachments: Vec::new(),
            source_tags: None,
            table_layout: None,
            allow_font_substitution: false,
            allow_page_creation: false,
            prune_empty_pages: false,
            max_new_pages: 0,
            mode: request.base.mode,
            signature_policy_override: false,
        };
        let local_hash = story_fingerprint(&local)?;
        let text_branches = maps
            .iter()
            .zip(&branches)
            .filter(|(map, _)| map[id].text != original.text)
            .map(|(map, b)| StoryBranch {
                branch_id: b.branch_id.clone(),
                base_revision_sha256: local.input_sha256.clone(),
                base_story_sha256: local_hash.clone(),
                patches: vec![text_patch(&original.text, &map[id].text, id, &b.branch_id)],
            })
            .collect();
        let text = merge_story_branches(&StoryMergeRequest {
            base: local,
            branches: text_branches,
        })?;
        if let Some(text) = text.merged {
            value["text"] = Value::String(text.paragraphs[0].text.clone());
        } else {
            conflict(
                &mut conflicts,
                &format!("paragraphs/{id}/text"),
                text.conflicts
                    .into_iter()
                    .flat_map(|c| c.operation_ids)
                    .collect(),
                "overlapping text edits",
                StructureConflictTarget::ParagraphField {
                    paragraph_id: id.clone(),
                    field: "text".into(),
                },
            );
        }
        result.insert(
            id.clone(),
            serde_json::from_value::<StoryParagraph>(value).map_err(|e| fail(&e.to_string()))?,
        );
    }
    let surviving = result.keys().cloned().collect::<BTreeSet<_>>();
    let base_order = request
        .base
        .paragraphs
        .iter()
        .filter(|p| surviving.contains(&p.id))
        .map(|p| p.id.clone())
        .collect::<Vec<_>>();
    let order_values = branches
        .iter()
        .map(|b| {
            Ok((
                b.branch_id.as_str(),
                json(
                    &b.proposed
                        .paragraphs
                        .iter()
                        .filter(|p| surviving.contains(&p.id))
                        .map(|p| p.id.clone())
                        .collect::<Vec<_>>(),
                )?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let order = choose(
        &json(&base_order)?,
        order_values,
        "paragraph_order",
        &mut conflicts,
        StructureConflictTarget::ParagraphOrder,
    );
    let order: Vec<String> = serde_json::from_value(order).map_err(|e| fail(&e.to_string()))?;
    // New paragraph blocks are attached to an exact surviving-neighbour gap.
    // Competing blocks in the same gap require review, not an arbitrary sort.
    type Gap = (Option<String>, Option<String>);
    let mut gaps: BTreeMap<Gap, (Vec<String>, String)> = BTreeMap::new();
    for (branch, map) in branches.iter().zip(&maps) {
        for (id, paragraph) in map.iter().filter(|(id, _)| !base.contains_key(*id)) {
            if let Some(old) = result.get(id) {
                if json(old)? != json(paragraph)? {
                    conflict(
                        &mut conflicts,
                        &format!("paragraphs/{id}"),
                        vec![branch.branch_id.clone()],
                        "new paragraph identity reused with different content",
                        StructureConflictTarget::Paragraph {
                            paragraph_id: id.clone(),
                        },
                    );
                }
            } else {
                result.insert(id.clone(), paragraph.clone());
            }
        }
        let sequence = branch
            .proposed
            .paragraphs
            .iter()
            .filter(|p| surviving.contains(&p.id) || !base.contains_key(&p.id))
            .collect::<Vec<_>>();
        let mut position = 0;
        let mut before = None;
        while position < sequence.len() {
            if surviving.contains(&sequence[position].id) {
                before = Some(sequence[position].id.clone());
                position += 1;
                continue;
            }
            let start = position;
            while position < sequence.len() && !surviving.contains(&sequence[position].id) {
                position += 1;
            }
            let after = sequence.get(position).map(|p| p.id.clone());
            let block = sequence[start..position]
                .iter()
                .map(|p| p.id.clone())
                .collect::<Vec<_>>();
            let gap = (before.clone(), after);
            if let Some((previous, other)) = gaps.get(&gap) {
                if previous != &block {
                    conflict(
                        &mut conflicts,
                        "paragraph_insertion",
                        vec![other.clone(), branch.branch_id.clone()],
                        "different insertion blocks target the same logical gap",
                        StructureConflictTarget::Insertions {
                            paragraph_ids: previous
                                .iter()
                                .chain(&block)
                                .cloned()
                                .collect::<BTreeSet<_>>()
                                .into_iter()
                                .collect(),
                        },
                    );
                }
            } else {
                gaps.insert(gap, (block, branch.branch_id.clone()));
            }
        }
    }
    let mut assembled = Vec::new();
    let mut seen = BTreeSet::new();
    for index in 0..=order.len() {
        let gap = (
            index.checked_sub(1).map(|i| order[i].clone()),
            order.get(index).cloned(),
        );
        if let Some((block, _)) = gaps.remove(&gap) {
            assembled.extend(block);
        }
        if let Some(id) = order.get(index) {
            assembled.push(id.clone());
        }
    }
    if !gaps.is_empty() {
        conflict(
            &mut conflicts,
            "paragraph_order",
            Vec::new(),
            "paragraph move invalidated an insertion gap",
            StructureConflictTarget::ParagraphOrder,
        );
    }
    if assembled.iter().any(|id| !seen.insert(id.clone())) {
        conflict(
            &mut conflicts,
            "paragraph_order",
            Vec::new(),
            "new paragraph placed in multiple gaps",
            StructureConflictTarget::ParagraphOrder,
        );
    }
    // A conflicted private draft is for review only. Expose each identity once;
    // values missing from this provisional order are returned as unplaced.
    seen.clear();
    merged.paragraphs = assembled
        .iter()
        .filter(|id| seen.insert((*id).clone()))
        .map(|id| {
            result
                .get(id)
                .cloned()
                .ok_or_else(|| fail("merged paragraph missing"))
        })
        .collect::<Result<Vec<_>>>()?;
    rebind_derived_tags(&request.base, &mut merged)?;
    collect_dependencies(&mut merged, &branches, &mut conflicts)?;
    Ok(StructureDraft {
        candidate: merged,
        values: result,
        conflicts,
        fingerprint,
    })
}

fn rebind_derived_tags(base: &LinkedStoryRequest, merged: &mut LinkedStoryRequest) -> Result<()> {
    merged.source_tags = base.source_tags.clone();
    if let Some(tags) = &mut merged.source_tags {
        // Ownership stays bound to the original source selection, not paragraph
        // positions in a branch. Materialize ordinal defaults against the base
        // before applying approved insertion/deletion/reordering operations.
        let mut bindings = BTreeMap::new();
        for (position, paragraph) in base.paragraphs.iter().enumerate() {
            let binding = match tags.paragraph_sources.get(&paragraph.id) {
                Some(value) => value.clone(),
                None if tags.selected.len() == base.paragraphs.len() => {
                    Some(tags.selected[position].clone())
                }
                None if tags.selected.is_empty() => None,
                None => {
                    return Err(fail(
                        "ambiguous base paragraph tag binding; replan explicitly",
                    ))
                }
            };
            bindings.insert(paragraph.id.clone(), binding);
        }
        tags.paragraph_sources = merged
            .paragraphs
            .iter()
            .map(|paragraph| {
                (
                    paragraph.id.clone(),
                    bindings.remove(&paragraph.id).unwrap_or(None),
                )
            })
            .collect();
        tags.new_roles
            .retain(|id, _| tags.paragraph_sources.contains_key(id));
        tags.semantic_text
            .retain(|id, _| tags.paragraph_sources.contains_key(id));
        // New paragraphs use the engine's new-P default. Retargeting a source
        // element or choosing another new role remains a separately approved
        // draft change, not something inferred from matching words.
    }
    Ok(())
}

fn collect_dependencies(
    merged: &mut LinkedStoryRequest,
    branches: &[&StoryStructureBranch],
    conflicts: &mut Vec<StoryStructureConflict>,
) -> Result<()> {
    if let Some(table_layout) = &merged.table_layout {
        if let Err(error) = crate::linked_stories::tables::validate_topology(merged) {
            conflict(
                conflicts,
                "table_layout",
                branches.iter().map(|b| b.branch_id.clone()).collect(),
                &format!(
                    "merged paragraphs require explicit table-topology reconciliation: {error}"
                ),
                StructureConflictTarget::TableProjection {
                    paragraph_ids: table_layout
                        .cells
                        .iter()
                        .flat_map(|cell| cell.block_ids().map(str::to_owned))
                        .collect(),
                },
            );
        }
    }
    let paragraphs = merged
        .paragraphs
        .iter()
        .map(|p| p.id.as_str())
        .collect::<BTreeSet<_>>();
    for figure in &merged.figures {
        if !paragraphs.contains(figure.caption_paragraph.as_str()) {
            conflict(conflicts,&format!("figures/{}/caption",figure.id),
                branches.iter().map(|b|b.branch_id.clone()).collect(),
                "caption deletion would orphan a retained image; restore the paragraph or replan Figure deletion/reassociation", StructureConflictTarget::RetainedParagraph { paragraph_id: figure.caption_paragraph.clone() });
        }
    }
    for anchor in &merged.annotation_anchors {
        if !paragraphs.contains(anchor.paragraph_id.as_str()) {
            conflict(conflicts, &format!("annotations/{}/paragraph", anchor.annotation_id), branches.iter().map(|b|b.branch_id.clone()).collect(),
                "paragraph deletion would orphan a retained annotation anchor; restore it or replan the anchor", StructureConflictTarget::RetainedParagraph { paragraph_id: anchor.paragraph_id.clone() });
        }
    }
    Ok(())
}

#[path = "story_structure_resolution.rs"]
pub mod resolution;

/// Pure merge followed by the existing revision/approval plan, not a bypass of
/// signature, source ownership, font or output-contract policy.
pub fn plan_merged_story_structure(
    input: &[u8],
    request: &StoryStructureMergeRequest,
    policy: crate::universal_editing::UniversalEditPolicyV2,
) -> Result<crate::universal_editing::UniversalEditPlanV2> {
    let story = merge_story_structure(request)?
        .merged
        .ok_or_else(|| fail("unresolved structural story conflicts"))?;
    crate::universal_editing::plan_universal_edit_v2(
        input,
        &crate::universal_editing::UniversalEditRequestV2 {
            operation: crate::universal_editing::UniversalEditOperationV2::LinkedStory {
                request: story,
            },
            policy,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn base() -> LinkedStoryRequest {
        serde_json::from_value(serde_json::json!({"story_id":"s","input_sha256":"revision","frames":[],"fonts":[],
            "paragraphs":[{"id":"p","text":"alpha beta","preferred_font":"Helvetica","font_size":12.0,"line_height":14.0}]})).unwrap()
    }
    fn branch(
        base: &LinkedStoryRequest,
        id: &str,
        proposed: LinkedStoryRequest,
    ) -> StoryStructureBranch {
        StoryStructureBranch {
            branch_id: id.into(),
            base_story_sha256: story_fingerprint(base).unwrap(),
            proposed,
        }
    }
    #[test]
    fn text_and_style_merge_but_delete_edit_conflicts() {
        let base = base();
        let mut left = base.clone();
        let mut right = base.clone();
        left.paragraphs[0].text = "ALPHA beta".into();
        right.paragraphs[0].font_size = 13.0;
        let mut request = StoryStructureMergeRequest {
            base: base.clone(),
            branches: vec![branch(&base, "left", left), branch(&base, "right", right)],
        };
        let first = merge_story_structure(&request).unwrap().merged.unwrap();
        assert_eq!(first.paragraphs[0].text, "ALPHA beta");
        assert_eq!(first.paragraphs[0].font_size, 13.0);
        request.branches.reverse();
        assert_eq!(
            json(&first).unwrap(),
            json(&merge_story_structure(&request).unwrap().merged.unwrap()).unwrap()
        );
        request.branches[0].proposed.paragraphs.clear();
        assert!(merge_story_structure(&request).unwrap().merged.is_none());
    }

    #[test]
    fn wrap_policy_merges_from_an_omitted_default_and_detects_competing_changes() {
        use crate::fonts::line_break_policy::{EmergencyWrap, LineBreakProfile};
        let base = base();
        let mut text = base.clone();
        text.paragraphs[0].text = "changed text".into();
        let mut style = base.clone();
        style.paragraphs[0].line_break.profile = LineBreakProfile::JapaneseStrict;
        let mut request = StoryStructureMergeRequest {
            base: base.clone(),
            branches: vec![branch(&base, "text", text), branch(&base, "style", style)],
        };
        let merged = merge_story_structure(&request).unwrap().merged.unwrap();
        assert_eq!(merged.paragraphs[0].text, "changed text");
        assert_eq!(
            merged.paragraphs[0].line_break.profile,
            LineBreakProfile::JapaneseStrict
        );
        request.branches[0].proposed.paragraphs[0]
            .line_break
            .emergency = EmergencyWrap::PreserveWords;
        let result = merge_story_structure(&request).unwrap();
        assert!(result.merged.is_none());
        assert!(result.conflicts.iter().any(|c| matches!(&c.target, StructureConflictTarget::ParagraphField { field, .. } if field == "line_break")));
    }
    #[test]
    fn page_policy_merges_from_an_omitted_default_and_detects_competing_parity() {
        use crate::linked_stories::StoryPageBreakBefore;
        let base = base();
        let mut text = base.clone();
        text.paragraphs[0].text = "changed text".into();
        let mut page = base.clone();
        page.paragraphs[0].page_break_before = StoryPageBreakBefore::NextPage;
        let mut request = StoryStructureMergeRequest {
            base: base.clone(),
            branches: vec![branch(&base, "text", text), branch(&base, "page", page)],
        };
        let merged = merge_story_structure(&request).unwrap().merged.unwrap();
        assert_eq!(merged.paragraphs[0].text, "changed text");
        assert_eq!(
            merged.paragraphs[0].page_break_before,
            StoryPageBreakBefore::NextPage
        );
        request.branches[0].proposed.paragraphs[0].page_break_before =
            StoryPageBreakBefore::NextOddPage;
        let result = merge_story_structure(&request).unwrap();
        assert!(result.merged.is_none());
        assert!(result.conflicts.iter().any(|conflict| matches!(
            &conflict.target,
            StructureConflictTarget::ParagraphField { field, .. }
                if field == "page_break_before"
        )));
    }
    #[test]
    fn source_authority_cannot_be_merged() {
        let base = base();
        let mut proposed = base.clone();
        proposed.signature_policy_override = true;
        assert!(merge_story_structure(&StoryStructureMergeRequest {
            base: base.clone(),
            branches: vec![branch(&base, "b", proposed)]
        })
        .is_err());
    }

    #[test]
    fn tag_defaults_follow_paragraph_identity_through_structural_merge() {
        use crate::tagged_structure::story::{StoryTagging, TagReference};
        let mut base = base();
        let mut second = base.paragraphs[0].clone();
        second.id = "q".into();
        base.paragraphs.push(second);
        let owner = |object| TagReference {
            object,
            generation: 0,
            key: None,
        };
        base.source_tags = Some(StoryTagging {
            parent: owner(1),
            selected: vec![owner(2), owner(3)],
            insert_at: Some(0),
            paragraph_sources: BTreeMap::new(),
            new_roles: BTreeMap::new(),
            semantic_text: BTreeMap::new(),
            figures: BTreeMap::new(),
        });
        let mut proposed = base.clone();
        proposed.paragraphs.reverse();
        let mut new = proposed.paragraphs[0].clone();
        new.id = "new".into();
        proposed.paragraphs.push(new);
        let merged = merge_story_structure(&StoryStructureMergeRequest {
            base: base.clone(),
            branches: vec![branch(&base, "move-and-insert", proposed)],
        })
        .unwrap()
        .merged
        .unwrap();
        let tags = merged.source_tags.unwrap();
        assert_eq!(tags.paragraph_sources["q"], Some(owner(3)));
        assert_eq!(tags.paragraph_sources["p"], Some(owner(2)));
        assert_eq!(tags.paragraph_sources["new"], None);
        assert_eq!(tags.selected, vec![owner(2), owner(3)]);

        let mut proposed = base.clone();
        proposed.paragraphs.remove(0);
        let merged = merge_story_structure(&StoryStructureMergeRequest {
            base: base.clone(),
            branches: vec![branch(&base, "delete", proposed)],
        })
        .unwrap()
        .merged
        .unwrap();
        let tags = merged.source_tags.unwrap();
        assert!(!tags.paragraph_sources.contains_key("p"));
        assert_eq!(tags.paragraph_sources["q"], Some(owner(3)));
    }

    #[test]
    fn disjoint_insertions_merge_and_same_gap_is_a_conflict() {
        let mut base = base();
        let mut second = base.paragraphs[0].clone();
        second.id = "q".into();
        base.paragraphs.push(second);
        let mut left = base.clone();
        let mut right = base.clone();
        let mut a = base.paragraphs[0].clone();
        a.id = "a".into();
        let mut b = a.clone();
        b.id = "b".into();
        left.paragraphs.insert(0, a);
        right.paragraphs.push(b.clone());
        let mut request = StoryStructureMergeRequest {
            base: base.clone(),
            branches: vec![branch(&base, "left", left), branch(&base, "right", right)],
        };
        assert_eq!(
            merge_story_structure(&request)
                .unwrap()
                .merged
                .unwrap()
                .paragraphs
                .iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "p", "q", "b"]
        );
        request.branches[1].proposed.paragraphs = base.paragraphs.clone();
        request.branches[1].proposed.paragraphs.insert(0, b);
        assert!(merge_story_structure(&request).unwrap().merged.is_none());
    }
}
