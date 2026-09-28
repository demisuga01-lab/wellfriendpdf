//! Reviewed structural conflict resolution. Construct a draft from constrained
//! logical fields; never accept a caller-supplied source/permission authority.
use super::*;
use crate::linked_stories::value_hash;

#[derive(Debug, Clone, Serialize)]
pub struct StructureConflictAlternative {
    /// None is the base; Some is a branch ID. Values are data, never patches.
    pub branch_id: Option<String>,
    pub value: Value,
}
#[derive(Debug, Clone, Serialize)]
pub struct ReviewedStructureConflict {
    pub conflict_id: String,
    pub target: StructureConflictTarget,
    pub path: String,
    pub reason: String,
    pub branch_ids: Vec<String>,
    pub alternatives: Vec<StructureConflictAlternative>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedFrameGeometry {
    pub frame_id: String,
    pub rect: [f64; 4],
    pub exclusions: Vec<[f64; 4]>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureResolution {
    pub expected_review_sha256: String,
    pub acknowledged_conflicts: Vec<String>,
    /// Complete desired logical sequence, not source IDs or JSON patch paths.
    pub paragraphs: Vec<StoryParagraph>,
    /// One entry per existing frame in its existing order.
    pub frame_geometry: Vec<ResolvedFrameGeometry>,
}
#[derive(Debug, Clone, Serialize)]
pub struct StructureReview {
    pub schema_version: u32,
    pub input_sha256: String,
    pub base_story_sha256: String,
    pub review_sha256: String,
    /// Provisional values only. Conflicted candidates are NOT approved drafts.
    pub candidate: LinkedStoryRequest,
    pub unplaced_paragraphs: Vec<StoryParagraph>,
    pub conflicts: Vec<ReviewedStructureConflict>,
    pub limits: Vec<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct StructureResolutionResult {
    pub merged: LinkedStoryRequest,
    pub review_sha256: String,
    pub resolution_sha256: String,
    pub resolved_conflict_ids: Vec<String>,
    pub limits: Vec<String>,
}

fn limits() -> Vec<String> {
    vec![
        "resolved logical draft is not native layout approval; preview and checkpoint are still required".into(),
        "source bindings, font assets, tables, anchors, image ownership and permissions are inherited, not merged or granted".into(),
        "structural saves may detach a stored causal text epoch; reconcile or explicitly start a new epoch".into(),
    ]
}
struct ReviewSource<'a> {
    story: &'a LinkedStoryRequest,
    paragraphs: BTreeMap<&'a str, (usize, &'a StoryParagraph)>,
    frames: BTreeMap<&'a str, &'a crate::linked_stories::StoryFrame>,
}
impl<'a> ReviewSource<'a> {
    fn new(story: &'a LinkedStoryRequest) -> Self {
        Self {
            story,
            paragraphs: story
                .paragraphs
                .iter()
                .enumerate()
                .map(|(i, p)| (p.id.as_str(), (i, p)))
                .collect(),
            frames: story.frames.iter().map(|f| (f.id.as_str(), f)).collect(),
        }
    }
}
fn paragraph_field(p: &StoryParagraph, field: &str) -> Result<Value> {
    // Do not clone a multi-megabyte text string while inspecting a scalar style.
    match field {
        "text" => json(&p.text),
        "preferred_font" => json(&p.preferred_font),
        "font_size" => json(&p.font_size),
        "line_height" => json(&p.line_height),
        "rgb" => json(&p.rgb),
        "rtl" => json(&p.rtl),
        "keep_with_next" => json(&p.keep_with_next),
        "keep_together" => json(&p.keep_together),
        "break_before" => json(&p.break_before),
        "page_break_before" => json(&p.page_break_before),
        "orphans" => json(&p.orphans),
        "widows" => json(&p.widows),
        "space_before" => json(&p.space_before),
        "space_after" => json(&p.space_after),
        "shaping" => json(&p.shaping),
        "line_break" => json(&p.line_break),
        "tab_stops" => json(&p.tab_stops),
        _ => Err(fail("unknown structural paragraph field")),
    }
}
fn alternative(source: &ReviewSource<'_>, target: &StructureConflictTarget) -> Result<Value> {
    Ok(match target {
        StructureConflictTarget::FrameField { frame_id, field } => {
            match source.frames.get(frame_id.as_str()) {
                None => Value::Null,
                Some(frame) => match field.as_str() {
                    "rect" => json(&frame.rect)?,
                    "exclusions" => json(&frame.exclusions)?,
                    _ => return Err(fail("unknown frame field")),
                },
            }
        }
        StructureConflictTarget::ParagraphField {
            paragraph_id,
            field,
        } => source
            .paragraphs
            .get(paragraph_id.as_str())
            .map(|(_, p)| paragraph_field(p, field))
            .transpose()?
            .unwrap_or(Value::Null),
        StructureConflictTarget::Paragraph { paragraph_id }
        | StructureConflictTarget::RetainedParagraph { paragraph_id } => source
            .paragraphs
            .get(paragraph_id.as_str())
            .map(|(_, p)| json(p))
            .transpose()?
            .unwrap_or(Value::Null),
        StructureConflictTarget::ParagraphOrder => json(
            &source
                .story
                .paragraphs
                .iter()
                .map(|p| &p.id)
                .collect::<Vec<_>>(),
        )?,
        StructureConflictTarget::Insertions { paragraph_ids }
        | StructureConflictTarget::TableProjection { paragraph_ids } => {
            let ids = paragraph_ids
                .iter()
                .map(String::as_str)
                .collect::<BTreeSet<_>>();
            let mut selected = ids
                .into_iter()
                .filter_map(|id| source.paragraphs.get(id))
                .collect::<Vec<_>>();
            selected.sort_by_key(|(position, _)| *position);
            json(&selected.into_iter().map(|(_, p)| p).collect::<Vec<_>>())?
        }
    })
}

fn review_from(
    request: &StoryStructureMergeRequest,
    draft: &StructureDraft,
) -> Result<StructureReview> {
    let mut branches = request.branches.iter().collect::<Vec<_>>();
    branches.sort_by(|a, b| a.branch_id.cmp(&b.branch_id));
    let input_hash = value_hash(&(&request.base, &branches))?;
    let sources = std::iter::once((None, ReviewSource::new(&request.base)))
        .chain(branches.iter().map(|branch| {
            (
                Some(branch.branch_id.clone()),
                ReviewSource::new(&branch.proposed),
            )
        }))
        .collect::<Vec<_>>();
    let mut conflicts = BTreeMap::new();
    let mut alternative_bytes = 0usize;
    for item in &draft.conflicts {
        crate::cancel::check_current_cancel("structural conflict review")?;
        let mut branch_ids = item.branch_ids.clone();
        branch_ids.sort();
        branch_ids.dedup();
        let conflict_id = value_hash(&(&input_hash, &item.target, &item.reason, &branch_ids))?;
        if conflicts.contains_key(&conflict_id) {
            continue;
        }
        let mut alternatives = Vec::new();
        for (branch_id, source) in &sources {
            let value = alternative(source, &item.target)?;
            let alternative = StructureConflictAlternative {
                branch_id: branch_id.clone(),
                value,
            };
            alternative_bytes = alternative_bytes.saturating_add(
                serde_json::to_vec(&alternative)
                    .map_err(|e| fail(&e.to_string()))?
                    .len(),
            );
            if alternative_bytes > 16 * 1024 * 1024 {
                return Err(fail(
                    "conflict alternatives exceed the 16 MiB review budget",
                ));
            }
            alternatives.push(alternative);
        }
        conflicts.insert(
            conflict_id.clone(),
            ReviewedStructureConflict {
                conflict_id,
                target: item.target.clone(),
                path: item.path.clone(),
                reason: item.reason.clone(),
                branch_ids,
                alternatives,
            },
        );
        if conflicts.len() > 4096 {
            return Err(fail("structural conflict count budget exceeded"));
        }
    }
    let placed = draft
        .candidate
        .paragraphs
        .iter()
        .map(|p| p.id.as_str())
        .collect::<BTreeSet<_>>();
    let unplaced_paragraphs = draft
        .values
        .values()
        .filter(|p| !placed.contains(p.id.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let conflicts = conflicts.into_values().collect::<Vec<_>>();
    let review_sha256 = value_hash(&(
        1u32,
        &input_hash,
        &draft.candidate,
        &unplaced_paragraphs,
        &conflicts,
    ))?;
    Ok(StructureReview {
        schema_version: 1,
        input_sha256: request.base.input_sha256.clone(),
        base_story_sha256: draft.fingerprint.clone(),
        review_sha256,
        candidate: draft.candidate.clone(),
        unplaced_paragraphs,
        conflicts,
        limits: limits(),
    })
}
pub fn review_story_structure(request: &StoryStructureMergeRequest) -> Result<StructureReview> {
    review_from(request, &merge_draft(request)?)
}

#[derive(Default)]
struct Scope {
    whole: BTreeSet<String>,
    optional: BTreeSet<String>,
    restored: BTreeSet<String>,
    paragraph_fields: BTreeMap<String, BTreeSet<String>>,
    frame_fields: BTreeMap<String, BTreeSet<String>>,
    reorder: bool,
}
fn scope_for(request: &StoryStructureMergeRequest, draft: &StructureDraft) -> Scope {
    let mut scope = Scope::default();
    for conflict in &draft.conflicts {
        match &conflict.target {
            StructureConflictTarget::FrameField { frame_id, field } => {
                scope
                    .frame_fields
                    .entry(frame_id.clone())
                    .or_default()
                    .insert(field.clone());
            }
            StructureConflictTarget::ParagraphField {
                paragraph_id,
                field,
            } => {
                scope
                    .paragraph_fields
                    .entry(paragraph_id.clone())
                    .or_default()
                    .insert(field.clone());
            }
            StructureConflictTarget::Paragraph { paragraph_id } => {
                scope.whole.insert(paragraph_id.clone());
                scope.optional.insert(paragraph_id.clone());
            }
            StructureConflictTarget::ParagraphOrder => scope.reorder = true,
            StructureConflictTarget::Insertions { paragraph_ids } => {
                scope.whole.extend(paragraph_ids.iter().cloned());
                scope.optional.extend(paragraph_ids.iter().cloned());
            }
            StructureConflictTarget::RetainedParagraph { paragraph_id } => {
                scope.restored.insert(paragraph_id.clone());
            }
            StructureConflictTarget::TableProjection { paragraph_ids } => {
                let owned = paragraph_ids.iter().cloned().collect::<BTreeSet<_>>();
                scope.restored.extend(owned.iter().cloned());
                for id in &owned {
                    scope
                        .paragraph_fields
                        .entry(id.clone())
                        .or_default()
                        .insert("text".into());
                }
                // Fixed table topology cannot retain unowned new blocks. Discard
                // only those additions explicitly covered by this conflict.
                let base_ids = request
                    .base
                    .paragraphs
                    .iter()
                    .map(|p| p.id.as_str())
                    .collect::<BTreeSet<_>>();
                for id in draft
                    .values
                    .keys()
                    .filter(|id| !owned.contains(*id) && !base_ids.contains(id.as_str()))
                {
                    scope.optional.insert(id.clone());
                }
            }
        }
    }
    scope
}

pub fn resolve_story_structure(
    request: &StoryStructureMergeRequest,
    resolution: &StructureResolution,
) -> Result<StructureResolutionResult> {
    crate::cancel::check_current_cancel("structural resolution")?;
    if resolution.paragraphs.len() > 100_000
        || resolution.frame_geometry.len() > 4096
        || resolution.acknowledged_conflicts.len() > 4096
    {
        return Err(fail("structural resolution count budget exceeded"));
    }
    let draft = merge_draft(request)?;
    let review = review_from(request, &draft)?;
    if resolution.expected_review_sha256 != review.review_sha256 {
        return Err(fail(
            "structural resolution review is stale or belongs to another merge",
        ));
    }
    let expected = review
        .conflicts
        .iter()
        .map(|c| c.conflict_id.as_str())
        .collect::<BTreeSet<_>>();
    let approved = resolution
        .acknowledged_conflicts
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if approved.len() != resolution.acknowledged_conflicts.len() || approved != expected {
        return Err(fail(
            "acknowledge every exact conflict once; no unknown conflict IDs",
        ));
    }
    let scope = scope_for(request, &draft);
    let base = paragraph_map(&request.base)?;
    let mut proposed = BTreeMap::new();
    let mut text_bytes = 0usize;
    for paragraph in &resolution.paragraphs {
        crate::cancel::check_current_cancel("structural paragraph resolution")?;
        text_bytes = text_bytes.saturating_add(paragraph.text.len());
        if text_bytes > 4_000_000
            || paragraph.id.is_empty()
            || paragraph.id.len() > 1024
            || proposed.insert(paragraph.id.clone(), paragraph).is_some()
        {
            return Err(fail(
                "invalid/duplicate resolved paragraph or text budget exceeded",
            ));
        }
        let original = draft
            .values
            .get(&paragraph.id)
            .or_else(|| {
                base.get(&paragraph.id).filter(|_| {
                    scope.optional.contains(&paragraph.id) || scope.restored.contains(&paragraph.id)
                })
            })
            .ok_or_else(|| {
                fail("resolution introduced an unknown or unambiguously deleted paragraph")
            })?;
        if !scope.whole.contains(&paragraph.id) {
            let before = paragraph_json(original)?;
            let after = paragraph_json(paragraph)?;
            for (field, value) in before
                .as_object()
                .ok_or_else(|| fail("paragraph schema mismatch"))?
            {
                if after.get(field) != Some(value)
                    && !scope
                        .paragraph_fields
                        .get(&paragraph.id)
                        .is_some_and(|allowed| allowed.contains(field))
                {
                    return Err(fail(
                        "resolution overwrote a non-conflicting paragraph field",
                    ));
                }
            }
        }
    }
    for id in draft
        .values
        .keys()
        .filter(|id| !scope.optional.contains(*id))
        .chain(scope.restored.iter())
    {
        if !proposed.contains_key(id) {
            return Err(fail(
                "resolution dropped an automatic or dependency-owned paragraph",
            ));
        }
    }
    if !scope.reorder {
        let placed = draft
            .candidate
            .paragraphs
            .iter()
            .map(|p| p.id.as_str())
            .collect::<BTreeSet<_>>();
        let protected = |id: &String| {
            !scope.optional.contains(id)
                && !(scope.restored.contains(id) && !placed.contains(id.as_str()))
        };
        let before = draft
            .candidate
            .paragraphs
            .iter()
            .filter(|p| protected(&p.id))
            .map(|p| &p.id)
            .collect::<Vec<_>>();
        let after = resolution
            .paragraphs
            .iter()
            .filter(|p| protected(&p.id))
            .map(|p| &p.id)
            .collect::<Vec<_>>();
        if before != after {
            return Err(fail("resolution reordered non-conflicting paragraphs"));
        }
    }
    if resolution.frame_geometry.len() != draft.candidate.frames.len() {
        return Err(fail("resolution must retain every source frame"));
    }
    let mut merged = draft.candidate;
    for (frame, desired) in merged.frames.iter_mut().zip(&resolution.frame_geometry) {
        if frame.id != desired.frame_id {
            return Err(fail("resolution changed frame identity/order"));
        }
        let allowed = scope.frame_fields.get(&frame.id);
        if frame.rect != desired.rect && !allowed.is_some_and(|fields| fields.contains("rect"))
            || frame.exclusions != desired.exclusions
                && !allowed.is_some_and(|fields| fields.contains("exclusions"))
        {
            return Err(fail("resolution changed non-conflicting frame geometry"));
        }
        let valid_rect =
            |r: &[f64; 4]| r.iter().all(|v| v.is_finite()) && r[0] < r[2] && r[1] < r[3];
        if !valid_rect(&desired.rect)
            || desired.exclusions.len() > 4096
            || desired.exclusions.iter().any(|r| !valid_rect(r))
        {
            return Err(fail("invalid resolved frame geometry"));
        }
        frame.rect = desired.rect;
        frame.exclusions = desired.exclusions.clone();
    }
    merged.paragraphs = resolution.paragraphs.clone();
    rebind_derived_tags(&request.base, &mut merged)?;
    crate::linked_stories::validate_paragraphs(&merged)?;
    let mut remaining = Vec::new();
    collect_dependencies(
        &mut merged,
        &request.branches.iter().collect::<Vec<_>>(),
        &mut remaining,
    )?;
    if !remaining.is_empty() {
        return Err(fail(
            "resolution leaves an unresolved retained-object dependency",
        ));
    }
    let resolved_conflict_ids = expected.into_iter().map(str::to_owned).collect::<Vec<_>>();
    let resolution_sha256 = value_hash(&(&review.review_sha256, &merged, &resolved_conflict_ids))?;
    crate::cancel::check_current_cancel("structural resolution publication")?;
    Ok(StructureResolutionResult {
        merged,
        review_sha256: review.review_sha256,
        resolution_sha256,
        resolved_conflict_ids,
        limits: limits(),
    })
}

#[cfg(test)]
#[path = "story_structure_resolution_tests.rs"]
mod tests;
