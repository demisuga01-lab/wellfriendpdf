//! Deterministic offline merge of revision-bound logical text patches.
//! This is not a PDF-byte CRDT: overlaps and non-text conflicts need review.
use crate::linked_stories::{LinkedStoryPreview, LinkedStoryRequest, LinkedStorySession};
use crate::{Result, WellfriendError};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryTextPatch {
    /// Globally unique client operation ID. Repeated identical operations are
    /// idempotent; reusing an ID with different bytes is rejected.
    pub operation_id: String,
    pub paragraph_id: String,
    /// UTF-8 offsets in the common base paragraph, on grapheme boundaries.
    pub range: [usize; 2],
    pub expected_text: String,
    pub replacement: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryBranch {
    pub branch_id: String,
    pub base_revision_sha256: String,
    /// Binds paragraph content, styles, geometry and ownership, not just PDF bytes.
    pub base_story_sha256: String,
    pub patches: Vec<StoryTextPatch>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryMergeRequest {
    pub base: LinkedStoryRequest,
    pub branches: Vec<StoryBranch>,
}
#[derive(Debug, Clone, Serialize)]
pub struct StoryMergeConflict {
    pub paragraph_id: String,
    pub operation_ids: Vec<String>,
    pub reason: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct StoryMergeResult {
    pub merged: Option<LinkedStoryRequest>,
    pub conflicts: Vec<StoryMergeConflict>,
    pub accepted_operation_ids: Vec<String>,
    pub base_story_sha256: String,
}
pub fn story_fingerprint(request: &LinkedStoryRequest) -> Result<String> {
    crate::linked_stories::value_hash(request)
}

/// Merge is independent of branch arrival order. Same-position insertions are
/// deliberate conflicts, not arbitrary concatenation or lost-update winners.
pub fn merge_story_branches(request: &StoryMergeRequest) -> Result<StoryMergeResult> {
    let fail = |s: &str| WellfriendError::invalid_input(s);
    if request.branches.len() > 256 {
        return Err(fail("story merge branch limit exceeded"));
    }
    let fingerprint = story_fingerprint(&request.base)?;
    let mut paragraphs = BTreeMap::new();
    for p in &request.base.paragraphs {
        if paragraphs.insert(p.id.as_str(), p).is_some() {
            return Err(fail("duplicate base paragraph ID"));
        }
    }
    let mut ids = BTreeMap::new();
    let mut branches = BTreeSet::new();
    let mut patches: BTreeMap<&str, Vec<&StoryTextPatch>> = BTreeMap::new();
    let mut total = 0usize;
    for branch in &request.branches {
        if branch.branch_id.is_empty() || !branches.insert(&branch.branch_id) {
            return Err(fail("duplicate/empty branch ID"));
        }
        if branch.base_revision_sha256 != request.base.input_sha256
            || branch.base_story_sha256 != fingerprint
        {
            return Err(fail(
                "story branch is not based on this exact PDF and story revision",
            ));
        }
        for patch in &branch.patches {
            crate::cancel::check_current_cancel("logical story merge")?;
            total = total
                .saturating_add(patch.expected_text.len())
                .saturating_add(patch.replacement.len());
            if ids.len() >= 100_000 || total > 16_000_000 || patch.operation_id.is_empty() {
                return Err(fail("story merge operation budget/identity invalid"));
            }
            let encoded = serde_json::to_vec(patch).map_err(|e| fail(&e.to_string()))?;
            if let Some(previous) = ids.get(&patch.operation_id) {
                if previous != &encoded {
                    return Err(fail("operation ID reused with different content"));
                }
                continue;
            }
            ids.insert(patch.operation_id.clone(), encoded);
            let paragraph = paragraphs
                .get(patch.paragraph_id.as_str())
                .ok_or_else(|| fail("patch paragraph absent from base"))?;
            if paragraph.text.get(patch.range[0]..patch.range[1])
                != Some(patch.expected_text.as_str())
            {
                return Err(fail("patch preimage mismatch"));
            }
            patches.entry(&patch.paragraph_id).or_default().push(patch);
        }
    }
    let mut merged = request.base.clone();
    let mut conflicts = Vec::new();
    for p in &mut merged.paragraphs {
        let Some(edits) = patches.get_mut(p.id.as_str()) else {
            continue;
        };
        let boundaries: BTreeSet<_> = p
            .text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .chain(std::iter::once(p.text.len()))
            .collect();
        for edit in edits.iter() {
            if !boundaries.contains(&edit.range[0]) || !boundaries.contains(&edit.range[1]) {
                return Err(fail("patch splits a grapheme cluster"));
            }
        }
        edits.sort_by_key(|p| (p.range[0], p.range[1], p.operation_id.as_str()));
        // Identical semantic replacements from distinct clients converge once.
        edits.dedup_by(|a, b| a.range == b.range && a.replacement == b.replacement);
        let mut farthest: Option<&StoryTextPatch> = None;
        for &edit in edits.iter() {
            if let Some(previous) = farthest {
                let overlap = edit.range[0] < previous.range[1]
                    || (edit.range[0] == previous.range[0])
                    || (edit.range[0] == previous.range[1] && edit.range[0] == edit.range[1]);
                if overlap {
                    conflicts.push(StoryMergeConflict { paragraph_id: p.id.clone(),
                        operation_ids: vec![previous.operation_id.clone(), edit.operation_id.clone()],
                        reason: "overlapping edits or ambiguous insertion boundary; explicit resolution required".into() });
                }
                if edit.range[1] > previous.range[1] {
                    farthest = Some(edit);
                }
            } else {
                farthest = Some(edit);
            }
        }
        // Apply only to a private candidate. Conflicted candidates never leave
        // this function, and no source byte ranges are merged.
        if conflicts.iter().any(|c| c.paragraph_id == p.id) {
            continue;
        }
        for edit in edits.iter().rev() {
            p.text
                .replace_range(edit.range[0]..edit.range[1], &edit.replacement);
        }
        if p.text.len() > 4_000_000 {
            return Err(fail("merged paragraph exceeds text budget"));
        }
    }
    let accepted_operation_ids = if conflicts.is_empty() {
        ids.into_keys().collect()
    } else {
        Vec::new()
    };
    Ok(StoryMergeResult {
        merged: conflicts.is_empty().then_some(merged),
        conflicts,
        accepted_operation_ids,
        base_story_sha256: fingerprint,
    })
}

/// Plan through the same universal source editor and approval protocol; this
/// helper does not grant mutation, signature or font-substitution authority.
pub fn plan_merged_story(
    input: &[u8],
    request: &StoryMergeRequest,
    policy: crate::universal_editing::UniversalEditPolicyV2,
) -> Result<crate::universal_editing::UniversalEditPlanV2> {
    let result = merge_story_branches(request)?;
    let story = result
        .merged
        .ok_or_else(|| WellfriendError::invalid_input("unresolved story merge conflicts"))?;
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

impl LinkedStorySession {
    pub fn merge_checkpoint(
        &mut self,
        request: &StoryMergeRequest,
        cancel: &crate::cancel::CancelToken,
    ) -> Result<LinkedStoryPreview> {
        cancel.check("story merge checkpoint")?;
        if self.revision_sha256() != request.base.input_sha256 {
            return Err(WellfriendError::invalid_input(
                "merge base differs from current session",
            ));
        }
        let scope =
            crate::cancel::CancelToken::linked_pair(cancel, &crate::cancel::current_cancel_token());
        let result = scope.scope(|| merge_story_branches(request))?;
        let story = result.merged.ok_or_else(|| {
            WellfriendError::invalid_input("unresolved story merge conflicts; session unchanged")
        })?;
        self.checkpoint(&story, cancel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn base() -> LinkedStoryRequest {
        serde_json::from_value(serde_json::json!({"story_id":"s", "input_sha256":"revision", "frames":[], "fonts":[],
            "paragraphs":[{"id":"p", "text":"alpha beta", "preferred_font":"Helvetica", "font_size":12.0, "line_height":14.0}]})).unwrap()
    }
    #[test]
    fn independent_branches_merge_commutatively_and_conflicts_have_no_candidate() {
        let base = base();
        let fingerprint = story_fingerprint(&base).unwrap();
        let branch = |id: &str, range, old: &str, new: &str| StoryBranch {
            branch_id: id.into(),
            base_revision_sha256: "revision".into(),
            base_story_sha256: fingerprint.clone(),
            patches: vec![StoryTextPatch {
                operation_id: id.into(),
                paragraph_id: "p".into(),
                range,
                expected_text: old.into(),
                replacement: new.into(),
            }],
        };
        let mut request = StoryMergeRequest {
            base,
            branches: vec![
                branch("a", [0, 5], "alpha", "A"),
                branch("b", [6, 10], "beta", "B"),
            ],
        };
        let result = merge_story_branches(&request).unwrap().merged.unwrap();
        assert_eq!(result.paragraphs[0].text, "A B");
        request.branches.reverse();
        assert_eq!(
            merge_story_branches(&request)
                .unwrap()
                .merged
                .unwrap()
                .paragraphs[0]
                .text,
            "A B"
        );
        request.branches.push(branch("c", [0, 5], "alpha", "C"));
        let conflict = merge_story_branches(&request).unwrap();
        assert!(conflict.merged.is_none());
        assert!(!conflict.conflicts.is_empty());
    }
}
