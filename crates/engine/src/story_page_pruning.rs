//! Opt-in contraction: only this story's explicitly owned continuation pages
//! can be removed. The preview and final transaction use the same exact plan.
use super::*;
use crate::content::operation::Operand;
use crate::writer::{page_pruning as writer, write_incremental_update, IncrementalObject};
use crate::{PdfDictionary, PdfObject};
type Ref = (u32, u16);
const MARKER: &str = "WFStoryContinuation";

#[cfg(test)]
#[path = "story_page_pruning_tests.rs"]
mod tests;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StoryPagePruning {
    /// Input page numbers. Frame and anchor destinations in the final preview
    /// are already mapped to the output document; source bindings are not.
    pub removed_pages: Vec<usize>,
    pub retained_pages: Vec<RetainedContinuation>,
    pub output_page_count: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetainedContinuation {
    pub page: usize,
    pub reason: String,
}

fn marker(dict: &PdfDictionary, story: &str) -> Option<String> {
    let d = dict.get(MARKER)?.as_dict()?;
    if d.get_integer("Version") != Some(1)
        || d.get("Story")?.as_string()? != hash(story.as_bytes()).as_bytes()
    {
        return None;
    }
    std::str::from_utf8(d.get("Frame")?.as_string()?)
        .ok()
        .map(str::to_owned)
}

fn page_dictionary(engine: &ContentEngine, page: usize) -> Result<PdfDictionary> {
    let p = engine.document().get_page(page)?;
    engine
        .document()
        .reader()
        .get_object(p.object_number, p.generation_number)?
        .as_dict()
        .cloned()
        .ok_or_else(|| fail("invalid continuation page"))
}

/// Reject anything not provably non-painting outside exact, already validated
/// source owner scopes. This is not whitespace extraction or a raster heuristic.
fn content_clear(
    engine: &ContentEngine,
    page: usize,
    owners: &BTreeSet<String>,
    images: &BTreeSet<String>,
    mask: bool,
) -> Result<bool> {
    let p = engine.document().get_page(page)?;
    let reader = engine.document().reader();
    let mut scopes = Vec::<bool>::new();
    let mut ignored = 0usize;
    let mut clear = true;
    let mut budget = 0usize;
    fn property<'a>(op: &'a crate::content::ContentOperation, key: &str) -> Option<&'a Operand> {
        op.operands
            .get(1)?
            .as_dictionary()?
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }
    fn nonempty(value: &Operand) -> bool {
        match value {
            Operand::String(s) => !s.is_empty(),
            Operand::Array(a) => a.iter().any(nonempty),
            _ => false,
        }
    }
    for id in p.contents {
        let stream = reader.get_object(id.0, id.1)?;
        let decoded = crate::filters::decode_stream_lossless_with_limits(
            &stream,
            reader,
            &crate::filters::DecodeLimits {
                max_decoded_bytes_per_stream: 64 * 1024 * 1024,
                ..Default::default()
            },
        )?;
        budget = budget.saturating_add(decoded.data.len());
        if budget > 128 * 1024 * 1024
            || decoded.status != crate::filters::StreamDecodeStatus::Complete
        {
            return Err(fail("continuation vacancy decode/budget failure"));
        }
        crate::image_fragments::operations(&decoded.data, |_, _, op, _| {
            let name = op.operator.as_str();
            if matches!(name, "BDC" | "BMC") {
                if scopes.len() >= 4096 {
                    return Err(fail("continuation scope budget exceeded"));
                }
                let frame = property(op, "WFStoryFrame")
                    .and_then(Operand::as_bytes)
                    .and_then(|b| std::str::from_utf8(b).ok());
                let image = (op.name(0) == Some("WFImageFragment"))
                    .then(|| property(op, "Key"))
                    .flatten()
                    .and_then(Operand::as_bytes)
                    .and_then(|b| std::str::from_utf8(b).ok());
                let owned = frame.is_some_and(|k| owners.contains(k))
                    || image.is_some_and(|k| images.contains(k));
                if (frame.is_some() || image.is_some()) && !owned {
                    clear = false;
                }
                if (!owned || !mask) && ignored == 0 {
                    if let Some(text) = property(op, "ActualText").and_then(Operand::as_bytes) {
                        if !text.is_empty() && text != [0xfe, 0xff] {
                            clear = false;
                        }
                    }
                    if name == "BDC" && op.operands.get(1).and_then(Operand::as_name).is_some() {
                        clear = false;
                    }
                }
                scopes.push(owned && mask);
                ignored += usize::from(owned && mask);
                return Ok(());
            }
            if name == "EMC" {
                ignored -= usize::from(
                    scopes
                        .pop()
                        .ok_or_else(|| fail("unbalanced continuation scope"))?,
                );
                return Ok(());
            }
            if ignored > 0 {
                return Ok(());
            }
            match name {
                "Tj" | "TJ" | "'" | "\"" => {
                    if op.operands.iter().any(nonempty) {
                        clear = false;
                    }
                }
                // Explicitly non-painting operators only. Unknown extensions,
                // inline images, Do, sh, all path paint and marked points retain
                // the page even when they happen to render invisibly.
                "q" | "Q" | "cm" | "w" | "J" | "j" | "M" | "d" | "ri" | "i" | "gs" | "m" | "l"
                | "c" | "v" | "y" | "h" | "re" | "n" | "W" | "W*" | "BT" | "ET" | "Tc" | "Tw"
                | "Tz" | "TL" | "Tf" | "Tr" | "Ts" | "Td" | "TD" | "Tm" | "T*" | "CS" | "cs"
                | "SC" | "sc" | "SCN" | "scn" | "G" | "g" | "RG" | "rg" | "K" | "k" => {}
                _ => clear = false,
            }
            Ok(())
        })?;
    }
    if !scopes.is_empty() {
        return Err(fail("unterminated continuation scope"));
    }
    Ok(clear)
}

fn page_features(dict: &PdfDictionary) -> bool {
    // Entries not written by the continuation writer may represent meaningful
    // page-level state even with no visible paint (actions, articles, metadata).
    dict.iter().any(|(k, _)| {
        !matches!(
            k.as_str(),
            "Type"
                | "Parent"
                | "MediaBox"
                | "CropBox"
                | "BleedBox"
                | "TrimBox"
                | "ArtBox"
                | "Rotate"
                | "UserUnit"
                | "Resources"
                | "Contents"
                | "Annots"
                | "StructParents"
                | "Tabs"
                | "WFStoryContinuation"
        )
    })
}

pub(super) fn plan(
    input: &[u8],
    request: &LinkedStoryRequest,
    preview: &LinkedStoryPreview,
) -> Result<StoryPagePruning> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let count = engine.page_count()?;
    let mut plan = StoryPagePruning {
        output_page_count: count + preview.generated_pages,
        ..Default::default()
    };
    if !request.prune_empty_pages {
        return Ok(plan);
    }
    let mut candidates = BTreeMap::<Ref, usize>::new();
    let mut departures = writer::Departures::default();
    departures.tags = crate::tagged_structure::story::migration_source_nodes(input, request)?;
    departures.annotations = crate::story_anchors::departing_objects(input, &preview.anchor_moves)?;
    let pages = request
        .frames
        .iter()
        .map(|f| f.page)
        .collect::<BTreeSet<_>>();
    // An explicit empty frame break is not unused overflow. Preserve both
    // sides of each requested break, including intentionally trailing blanks.
    let mut break_pages = BTreeSet::new();
    for transition in &preview.page_breaks {
        break_pages.extend(transition.from_page..=transition.to_page);
        if matches!(
            transition.policy,
            StoryPageBreakBefore::NextOddPage | StoryPageBreakBefore::NextEvenPage
        ) {
            // Removing any earlier owned continuation toggles the one-based
            // physical parity of this destination. Retain those pages until a
            // future transaction can repaginate and issue a new receipt.
            break_pages.extend(1..=transition.to_page);
        }
    }
    for (index, p) in request.paragraphs.iter().enumerate().skip(1) {
        if p.break_before {
            if let Some(state) = preview.checkpoints.get(index) {
                for slot in [state.frame_index, state.frame_index.saturating_add(1)] {
                    if let Some(frame) = preview.frames.get(slot) {
                        break_pages.insert(frame.frame.page);
                    }
                }
            }
        }
    }
    for page in pages {
        crate::cancel::check_current_cancel("continuation pruning plan")?;
        let dict = page_dictionary(&engine, page)?;
        let Some(key) = marker(&dict, &request.story_id) else {
            continue;
        };
        let frames = preview
            .frames
            .iter()
            .filter(|f| f.frame.page == page && !f.created)
            .collect::<Vec<_>>();
        let owners = frames
            .iter()
            .filter_map(|f| f.frame.owner.as_ref().map(|o| o.key.clone()))
            .collect::<BTreeSet<_>>();
        let images = request
            .figures
            .iter()
            .filter_map(|f| match &f.source {
                crate::image_fragments::ImageFragmentSource::Owned { binding }
                    if binding.page == page =>
                {
                    Some(binding.key.clone())
                }
                _ => None,
            })
            .chain(
                request
                    .figure_removals
                    .iter()
                    .filter(|r| r.binding.page == page)
                    .map(|r| r.binding.key.clone()),
            )
            .collect();
        let reason = if !owners.contains(&key) {
            Some("continuation_owner_not_in_request")
        } else if frames.iter().any(|f| {
            !f.lines.is_empty()
                || !f.decorations.is_empty()
                || !f.table_cells.is_empty()
                || !f.figures.is_empty()
        }) {
            Some("continuation_still_has_layout_content")
        } else if preview.anchor_moves.iter().any(|m| m.target_page == page) {
            Some("annotation_destination")
        } else if break_pages.contains(&page) {
            Some("explicit_frame_break")
        } else if page_features(&dict) {
            Some("additional_page_features")
        } else if !content_clear(&engine, page, &owners, &images, true)? {
            Some("unrelated_content_or_owner")
        } else {
            None
        };
        let mut reason = reason.map(str::to_owned);
        if reason.is_none() {
            if let Some(value) = dict.get("Annots") {
                let value = engine.document().reader().resolve(value.clone())?;
                let annots = value
                    .as_array()
                    .ok_or_else(|| fail("invalid continuation Annots"))?;
                if annots.iter().any(|a| {
                    a.as_reference()
                        .is_none_or(|r| !departures.annotations.contains(&r))
                }) {
                    reason = Some("surviving_page_annotation".into());
                }
            }
        }
        if let Some(reason) = reason {
            plan.retained_pages
                .push(RetainedContinuation { page, reason });
        } else {
            let p = engine.document().get_page(page)?;
            candidates.insert((p.object_number, p.generation_number), page);
        }
    }
    let protected = writer::protected_pages(engine.document(), &candidates, &departures)?;
    for &page in candidates.values() {
        if let Some(reason) = protected.get(&page) {
            plan.retained_pages.push(RetainedContinuation {
                page,
                reason: reason.clone(),
            });
        } else {
            plan.removed_pages.push(page);
        }
    }
    plan.removed_pages.sort_unstable();
    plan.retained_pages.sort_by_key(|p| p.page);
    // A native story must retain at least one source frame for subsequent edits.
    if preview
        .frames
        .iter()
        .all(|f| plan.removed_pages.contains(&f.frame.page))
    {
        if let Some(page) = plan.removed_pages.first().copied() {
            plan.removed_pages.remove(0);
            plan.retained_pages.push(RetainedContinuation {
                page,
                reason: "last_story_frame".into(),
            });
        }
    }
    plan.output_page_count -= plan.removed_pages.len();
    writer::validate_labels(
        engine.document(),
        &plan.removed_pages.iter().copied().collect(),
    )?;
    Ok(plan)
}

pub(super) fn stamp_created(
    input: &[u8],
    request: &LinkedStoryRequest,
    preview: &LinkedStoryPreview,
) -> Result<Vec<u8>> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let mut updates = Vec::new();
    for frame in preview.frames.iter().filter(|f| f.created) {
        let p = engine.document().get_page(frame.frame.page)?;
        let mut dict = page_dictionary(&engine, frame.frame.page)?;
        let mut marker = PdfDictionary::empty();
        marker.insert("Version", PdfObject::Integer(1));
        marker.insert(
            "Story",
            PdfObject::String(hash(request.story_id.as_bytes()).into_bytes()),
        );
        marker.insert(
            "Frame",
            PdfObject::String(frame_key(&request.story_id, &frame.frame.id).into_bytes()),
        );
        dict.insert(MARKER, PdfObject::Dictionary(marker));
        updates.push(IncrementalObject {
            number: p.object_number,
            generation: p.generation_number,
            object: PdfObject::Dictionary(dict),
        });
    }
    if updates.is_empty() {
        Ok(input.to_vec())
    } else {
        write_incremental_update(engine.document().reader(), updates)
    }
}

pub(super) fn apply(
    input: &[u8],
    request: &LinkedStoryRequest,
    plan: &StoryPagePruning,
) -> Result<Vec<u8>> {
    if plan.removed_pages.is_empty() {
        return Ok(input.to_vec());
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    for &page in &plan.removed_pages {
        let dict = page_dictionary(&engine, page)?;
        let key = marker(&dict, &request.story_id)
            .ok_or_else(|| fail("pruning continuation ownership changed"))?;
        let owners = request
            .frames
            .iter()
            .filter(|f| f.page == page)
            .map(|f| frame_key(&request.story_id, &f.id))
            .collect::<BTreeSet<_>>();
        if !owners.contains(&key)
            || page_features(&dict)
            || !content_clear(&engine, page, &owners, &BTreeSet::new(), false)?
        {
            return Err(fail("planned continuation is not empty after editing"));
        }
        if let Some(annots) = dict.get("Annots") {
            let value = engine.document().reader().resolve(annots.clone())?;
            if value.as_array().is_none_or(|a| !a.is_empty()) {
                return Err(fail("planned continuation still has annotations"));
            }
        }
    }
    let output = writer::remove(
        engine.document(),
        &plan.removed_pages.iter().copied().collect(),
    )?;
    let reopened = ContentEngine::open_bytes(output.clone())?;
    if reopened.page_count()? != plan.output_page_count {
        return Err(fail("pruned output page count mismatch"));
    }
    if reopened
        .document()
        .get_catalog()?
        .contains_key("StructTreeRoot")
    {
        crate::tagged_structure::validate_parent_tree(&output)?;
    }
    Ok(output)
}

pub(super) fn project(
    input: &[u8],
    request: &LinkedStoryRequest,
    preview: &mut LinkedStoryPreview,
    plan: StoryPagePruning,
) -> Result<()> {
    let map = |page: usize| page - plan.removed_pages.partition_point(|&p| p < page);
    let removed = |page: usize| plan.removed_pages.binary_search(&page).is_ok();
    preview.frames.retain(|f| !removed(f.frame.page));
    for f in &mut preview.frames {
        f.frame.page = map(f.frame.page);
    }
    for a in &mut preview.anchor_moves {
        a.target_page = map(a.target_page);
    }
    for transition in &mut preview.page_breaks {
        transition.from_page = map(transition.from_page);
        transition.to_page = map(transition.to_page);
    }
    preview.changed_pages.retain(|&p| !removed(p));
    preview.changed_pages.iter_mut().for_each(|p| *p = map(*p));
    preview
        .full_page_invalidations
        .retain(|(p, _)| !removed(*p));
    preview
        .full_page_invalidations
        .iter_mut()
        .for_each(|(p, _)| *p = map(*p));
    if let Some(&first) = plan.removed_pages.first() {
        let engine = ContentEngine::open_bytes(input.to_vec())?;
        let pages = engine.document().get_pages()?;
        let after = request
            .frames
            .last()
            .ok_or_else(|| fail("missing pruning template"))?
            .page;
        for raw in first..=pages.len() + preview.generated_pages {
            if removed(raw) {
                continue;
            }
            let source = if raw <= after {
                raw
            } else if raw <= after + preview.generated_pages {
                after
            } else {
                raw - preview.generated_pages
            };
            preview
                .full_page_invalidations
                .push((map(raw), pages[source - 1].crop_box));
            preview.changed_pages.push(map(raw));
        }
        preview.exact_limits.push("Approved empty continuation pages are removed from the current page tree; original pages and pages with surviving content or dependencies are retained; historical bytes are not erased".into());
    }
    preview.changed_pages.sort_unstable();
    preview.changed_pages.dedup();
    preview.page_pruning = plan;
    Ok(())
}
