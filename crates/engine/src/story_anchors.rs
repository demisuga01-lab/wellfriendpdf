//! Explicit annotation-to-paragraph anchors. Geometry is translated in the
//! original dictionaries; appearance streams, actions and field owners are not
//! reconstructed through an interchange format. No automatic anchor inference.
use crate::linked_stories::{LinkedStoryPreview, LinkedStoryRequest};
use crate::{ContentEngine, PdfDictionary, PdfObject, Result, WellfriendError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[path = "story_annotation_groups.rs"]
mod groups;
pub use groups::{StoryAnnotationGroup, StoryAnnotationGroupMember};
#[path = "story_annotation_identity.rs"]
mod identity;
pub use identity::StoryAnnotationNameChange;
#[path = "annotation_geometry.rs"]
mod geometry;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryAnnotationAnchor {
    pub annotation_id: String,
    pub paragraph_id: String,
    /// Hash from annotation_anchor_sources: identity + geometry, not a promise
    /// to fingerprint an entire appearance/resource graph.
    pub geometry_sha256: String,
    /// Lower-left offset from the paragraph's first painted line origin.
    pub offset: [f64; 2],
    /// Exact connected popup/reply group from discovery. Omission never
    /// authorizes moving other annotations implicitly.
    #[serde(default)]
    pub group: Option<StoryAnnotationGroup>,
    /// Explicit permission to rename this anchor's members only when movement
    /// would duplicate an existing destination-page NM. Preview lists changes.
    #[serde(default)]
    pub rename_conflicting_names: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryAnnotationSource {
    pub annotation_id: String,
    /// PDF page-local NM, distinct from the stable editing identity.
    #[serde(default)]
    pub name: Option<String>,
    pub page: usize,
    pub subtype: String,
    pub rect: [f64; 4],
    pub geometry_sha256: String,
    #[serde(default)]
    pub group: Option<StoryAnnotationGroup>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryAnnotationMove {
    pub annotation_id: String,
    pub source_page: usize,
    pub target_page: usize,
    pub old_rect: [f64; 4],
    pub new_rect: [f64; 4],
    #[serde(default)]
    pub name_change: Option<StoryAnnotationNameChange>,
}
struct Entry {
    source: StoryAnnotationSource,
    reference: Option<(u32, u16)>,
    dict: PdfDictionary,
    annotation_order: usize,
}
fn fail(s: &str) -> WellfriendError {
    WellfriendError::invalid_input(s)
}
fn finite_array(reader: &crate::reader::PdfReader, value: &PdfObject) -> Result<Vec<f64>> {
    let value = reader.resolve(value.clone())?;
    let items = value
        .as_array()
        .ok_or_else(|| fail("annotation geometry must be an array"))?;
    if items.len() > 100_000 {
        return Err(fail("annotation geometry budget exceeded"));
    }
    items
        .iter()
        .map(|v| {
            let v = reader.resolve(v.clone())?;
            let n = v
                .as_number()
                .ok_or_else(|| fail("annotation coordinate must be numeric"))?;
            if n.is_finite() {
                Ok(n)
            } else {
                Err(fail("nonfinite annotation geometry"))
            }
        })
        .collect()
}
fn array(values: impl IntoIterator<Item = f64>) -> PdfObject {
    PdfObject::Array(
        values
            .into_iter()
            .map(|value| {
                if value == value.trunc() && value.abs() < 1e15 {
                    PdfObject::Integer(value as i64)
                } else {
                    PdfObject::Real(value)
                }
            })
            .collect(),
    )
}
fn page_annots(
    engine: &ContentEngine,
    page: &crate::document::PdfPage,
) -> Result<(PdfDictionary, Vec<PdfObject>)> {
    let reader = engine.document().reader();
    let dict = reader
        .get_object(page.object_number, page.generation_number)?
        .as_dict()
        .cloned()
        .ok_or_else(|| fail("invalid annotation page"))?;
    let annots = match dict.get("Annots") {
        None => Vec::new(),
        Some(v) => reader
            .resolve(v.clone())?
            .as_array()
            .ok_or_else(|| fail("invalid Annots array"))?
            .to_vec(),
    };
    Ok((dict, annots))
}
fn inventory(engine: &ContentEngine) -> Result<BTreeMap<String, Entry>> {
    let reader = engine.document().reader();
    let mut result = BTreeMap::new();
    let identities = crate::annotation_identity::index(engine.document(), 100_000)?;
    let mut count = 0usize;
    for page in engine.document().get_pages()? {
        crate::cancel::check_current_cancel("story annotation inventory")?;
        let (_, annots) = page_annots(engine, &page)?;
        for (annotation_order, value) in annots.into_iter().enumerate() {
            count += 1;
            if count > 100_000 {
                return Err(fail("annotation inventory budget exceeded"));
            }
            let reference = value.as_reference();
            let object = reader.resolve(value)?;
            let Some(dict) = object.as_dict() else {
                continue;
            };
            let identity = identities
                .get(&(page.page_number, annotation_order))
                .ok_or_else(|| fail("annotation source identity disappeared"))?;
            let Some(rect) = dict.get("Rect") else {
                continue;
            };
            let coordinates = finite_array(reader, rect)?;
            let raw_rect: [f64; 4] = coordinates
                .try_into()
                .map_err(|_| fail("annotation Rect must have four coordinates"))?;
            let rect = [
                raw_rect[0].min(raw_rect[2]),
                raw_rect[1].min(raw_rect[3]),
                raw_rect[0].max(raw_rect[2]),
                raw_rect[1].max(raw_rect[3]),
            ];
            if rect[0] == rect[2] || rect[1] == rect[3] {
                return Err(fail("anchor requires a nonempty rectangle"));
            }
            // Stable under canonical object renumbering and page insertion.
            let mut geometry = PdfDictionary::empty();
            let mut budget = (100_000usize, 4 * 1024 * 1024usize);
            for key in [
                "NM",
                "Subtype",
                "Rect",
                "QuadPoints",
                "Vertices",
                "L",
                "CL",
                "InkList",
                "RD",
                "LL",
                "LLE",
                "LLO",
                "CO",
                "F",
                "Contents",
            ] {
                if let Some(v) = dict.get(key) {
                    geometry.insert(key, resolve_geometry_value(reader, v, 0, &mut budget)?);
                }
            }
            let mut bytes = Vec::new();
            crate::writer::serialize_object(&PdfObject::Dictionary(geometry), &mut bytes);
            if bytes.len() > 4 * 1024 * 1024 {
                return Err(fail("anchor fingerprint byte budget exceeded"));
            }
            let source = StoryAnnotationSource {
                annotation_id: identity.id.clone(),
                name: identity.name.clone(),
                page: page.page_number,
                subtype: crate::annotation_relationships::subtype(reader, dict)?,
                rect,
                geometry_sha256: format!("{:x}", Sha256::digest(bytes)),
                group: None,
            };
            result.insert(
                identity.id.clone(),
                Entry {
                    source,
                    reference,
                    dict: dict.clone(),
                    annotation_order,
                },
            );
        }
    }
    let relationships =
        crate::annotation_relationships::Graph::read(engine.document(), &identities)?;
    groups::discover(&mut result, &relationships)?;
    Ok(result)
}
fn resolve_geometry_value(
    reader: &crate::reader::PdfReader,
    v: &PdfObject,
    depth: usize,
    budget: &mut (usize, usize),
) -> Result<PdfObject> {
    if depth > 8 || budget.0 == 0 {
        return Err(fail("annotation geometry nesting limit exceeded"));
    }
    budget.0 -= 1;
    let v = reader.resolve(v.clone())?;
    match v {
        PdfObject::Array(items) => {
            if items.len() > 100_000 {
                return Err(fail("annotation geometry array budget exceeded"));
            }
            Ok(PdfObject::Array(
                items
                    .iter()
                    .map(|v| resolve_geometry_value(reader, v, depth + 1, budget))
                    .collect::<Result<_>>()?,
            ))
        }
        PdfObject::String(ref b) => {
            budget.1 = budget
                .1
                .checked_sub(b.len())
                .ok_or_else(|| fail("annotation text budget exceeded"))?;
            Ok(v)
        }
        PdfObject::Name(ref n) => {
            budget.1 = budget
                .1
                .checked_sub(n.len())
                .ok_or_else(|| fail("annotation name budget exceeded"))?;
            Ok(v)
        }
        PdfObject::Dictionary(_) | PdfObject::Stream { .. } => {
            Err(fail("unexpected object in annotation geometry"))
        }
        _ => Ok(v),
    }
}

fn unrepresented_names(
    engine: &ContentEngine,
    entries: &BTreeMap<String, Entry>,
) -> Result<Vec<(usize, String)>> {
    Ok(
        crate::annotation_identity::index(engine.document(), 100_000)?
            .into_values()
            .filter(|identity| !entries.contains_key(&identity.id))
            .filter_map(|identity| identity.name.map(|name| (identity.page, name)))
            .collect(),
    )
}
pub fn annotation_anchor_sources(input: &[u8]) -> Result<Vec<StoryAnnotationSource>> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    Ok(inventory(&engine)?
        .into_values()
        .map(|e| e.source)
        .collect())
}

pub(crate) fn stage_identities(
    input: &[u8],
    staged: &[u8],
    moves: &[StoryAnnotationMove],
) -> Result<Vec<u8>> {
    groups::stage(input, staged, moves)
}

pub(crate) fn departing_objects(
    input: &[u8],
    moves: &[StoryAnnotationMove],
) -> Result<BTreeSet<(u32, u16)>> {
    if moves.is_empty() {
        return Ok(BTreeSet::new());
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let entries = inventory(&engine)?;
    let references = moves
        .iter()
        .filter(|m| m.source_page != m.target_page)
        .map(|m| {
            let entry = entries
                .get(&m.annotation_id)
                .ok_or_else(|| fail("pruning anchor disappeared"))?;
            if entry.source.page != m.source_page || entry.source.rect != m.old_rect {
                return Err(fail("pruning anchor differs from its verified preview"));
            }
            Ok(entry.reference)
        })
        .collect::<Result<Vec<_>>>()?;
    // A direct occurrence has no reference to exempt in the page-reference
    // walker. Until its promotion is staged, pruning conservatively retains
    // that source page; it does not prevent moving the annotation itself.
    Ok(references.into_iter().flatten().collect())
}
pub(crate) fn attach_preview(
    input: &[u8],
    request: &LinkedStoryRequest,
    preview: &mut LinkedStoryPreview,
) -> Result<()> {
    if request.annotation_anchors.is_empty() && preview.generated_pages == 0 {
        return Ok(());
    }
    if request.annotation_anchors.len() > 4096 {
        return Err(fail("story anchor limit exceeded"));
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    if preview.generated_pages > 0 {
        let after = request
            .frames
            .last()
            .ok_or_else(|| fail("continuation has no source frame"))?
            .page;
        let pages = engine.document().get_pages()?;
        for output_page in after + 1..=pages.len() + preview.generated_pages {
            let source_page = if output_page <= after + preview.generated_pages {
                after
            } else {
                output_page - preview.generated_pages
            };
            let page = pages
                .get(source_page - 1)
                .ok_or_else(|| fail("invalid continuation page mapping"))?;
            preview
                .full_page_invalidations
                .push((output_page, page.crop_box));
            preview.changed_pages.push(output_page);
        }
        preview.changed_pages.sort_unstable();
        preview.changed_pages.dedup();
    }
    if request.annotation_anchors.is_empty() {
        return Ok(());
    }
    let entries = inventory(&engine)?;
    let mut owned_elsewhere = BTreeSet::new();
    for saved in crate::linked_stories::load_linked_stories(input)? {
        if saved.request.story_id != request.story_id {
            for owned in saved.request.annotation_anchors {
                owned_elsewhere.insert(owned.annotation_id);
                if let Some(group) = owned.group {
                    owned_elsewhere.extend(group.members.into_iter().map(|m| m.annotation_id));
                }
            }
        }
    }
    let mut seen = BTreeSet::new();
    let mut rename_approved = BTreeSet::new();
    for anchor in &request.annotation_anchors {
        let entry = entries
            .get(&anchor.annotation_id)
            .ok_or_else(|| fail("anchored annotation is missing"))?;
        if entry.source.geometry_sha256 != anchor.geometry_sha256
            || anchor.offset.iter().any(|n| !n.is_finite())
        {
            return Err(fail("duplicate/stale annotation anchor or invalid offset"));
        }
        let members = groups::approved_ids(anchor, entry)?;
        let (frame, line) = preview
            .frames
            .iter()
            .find_map(|f| {
                f.paragraph_ids
                    .iter()
                    .position(|id| id == &anchor.paragraph_id)
                    .map(|i| (f, &f.lines[i]))
            })
            .ok_or_else(|| fail("anchor paragraph has no painted line"))?;
        let old = entry.source.rect;
        let x = line.x + anchor.offset[0];
        let y = line.baseline + anchor.offset[1];
        let dx = x - old[0];
        let dy = y - old[1];
        let page = if frame.created {
            request
                .frames
                .last()
                .ok_or_else(|| fail("story has no source frame"))?
                .page
        } else {
            frame.frame.page
        };
        let destination = engine.document().get_page(page)?;
        for id in members {
            if !seen.insert(id.clone()) || owned_elsewhere.contains(&id) {
                return Err(fail(
                    "annotation group overlaps another anchor or saved story",
                ));
            }
            let member = &entries[&id];
            if anchor.rename_conflicting_names {
                rename_approved.insert(id.clone());
            }
            translation_supported(member)?;
            let old = member.source.rect;
            let new = [old[0] + dx, old[1] + dy, old[2] + dx, old[3] + dy];
            let origin = engine.document().get_page(member.source.page)?;
            validate_destination(&origin, &destination, old, new)?;
            preview.anchor_moves.push(StoryAnnotationMove {
                annotation_id: id,
                source_page: member.source.page,
                target_page: frame.frame.page,
                old_rect: old,
                new_rect: new,
                name_change: None,
            });
            preview
                .changed_pages
                .extend([member.source.page, frame.frame.page]);
        }
    }
    let after = request.frames.last().map(|f| f.page).unwrap_or(0);
    identity::plan_names(
        &entries,
        &mut preview.anchor_moves,
        Some((after, preview.generated_pages)),
        &unrepresented_names(&engine, &entries)?,
    )?;
    if preview
        .anchor_moves
        .iter()
        .any(|m| m.name_change.is_some() && !rename_approved.contains(&m.annotation_id))
    {
        return Err(fail("annotation NM conflicts on the destination page; approve rename_conflicting_names and review exact name changes"));
    }
    preview.changed_pages.sort_unstable();
    preview.changed_pages.dedup();
    Ok(())
}

fn translation_supported(entry: &Entry) -> Result<()> {
    if ["Path", "Measure", "ExData"]
        .iter()
        .any(|k| entry.dict.contains_key(k))
    {
        return Err(fail(
            "extended annotation geometry needs explicit semantic migration",
        ));
    }
    if !matches!(
        entry.source.subtype.as_str(),
        "Link"
            | "Widget"
            | "Square"
            | "Circle"
            | "Highlight"
            | "Underline"
            | "StrikeOut"
            | "Squiggly"
            | "Line"
            | "Polygon"
            | "PolyLine"
            | "Ink"
            | "FreeText"
            | "Stamp"
            | "Text"
            | "Popup"
            | "Caret"
    ) {
        return Err(fail(
            "annotation subtype has no supported translation geometry",
        ));
    }
    Ok(())
}

fn validate_destination(
    origin: &crate::document::PdfPage,
    target: &crate::document::PdfPage,
    old: [f64; 4],
    new: [f64; 4],
) -> Result<()> {
    validate_geometry_destination(origin, target, old, new, false)
}

fn validate_geometry_destination(
    origin: &crate::document::PdfPage,
    target: &crate::document::PdfPage,
    old: [f64; 4],
    new: [f64; 4],
    allow_resize: bool,
) -> Result<()> {
    if origin.rotate.rem_euclid(360) != target.rotate.rem_euclid(360)
        || (origin.user_unit - target.user_unit).abs() > 1e-9
    {
        return Err(fail(
            "annotation anchors require matching page rotation and UserUnit",
        ));
    }
    let crop = target.crop_box;
    if new.iter().any(|v| !v.is_finite())
        || new[0] >= new[2]
        || new[1] >= new[3]
        || new[0] < crop[0]
        || new[1] < crop[1]
        || new[2] > crop[2]
        || new[3] > crop[3]
        || (!allow_resize
            && (0..2).any(|i| ((new[i + 2] - new[i]) - (old[i + 2] - old[i])).abs() > 1e-7))
    {
        return Err(fail(
            "annotation geometry is invalid, leaves the target crop box, or resizes a translation-only anchor",
        ));
    }
    Ok(())
}

pub(crate) fn apply_moves(input: &[u8], moves: &[StoryAnnotationMove]) -> Result<Vec<u8>> {
    apply_geometry(input, moves, false)
}

/// Both story translation and standalone resizing use the same source-object,
/// page-membership and tag-owner transaction. Resize never regenerates AP.
pub(crate) fn apply_geometry(
    input: &[u8],
    moves: &[StoryAnnotationMove],
    allow_resize: bool,
) -> Result<Vec<u8>> {
    if moves.is_empty() {
        return Ok(input.to_vec());
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let reader = engine.document().reader();
    let entries = inventory(&engine)?;
    groups::validate_geometry_moves(&entries, moves, allow_resize)?;
    identity::validate_names(&entries, moves, &unrepresented_names(&engine, &entries)?)?;
    let pages = engine.document().get_pages()?;
    let mut page_updates = BTreeMap::new();
    let mut updates = Vec::new();
    let mut structure_moves = BTreeMap::new();
    let mut selected = BTreeSet::new();
    let mut incoming = BTreeMap::<usize, Vec<(usize, usize, (u32, u16))>>::new();
    for movement in moves {
        crate::cancel::check_current_cancel("story annotation translation")?;
        let entry = entries
            .get(&movement.annotation_id)
            .ok_or_else(|| fail("anchor disappeared during story mutation"))?;
        let reference = entry.reference.ok_or_else(|| {
            fail("annotation source requires materialization before geometry writing")
        })?;
        if !selected.insert(reference) {
            return Err(fail("duplicate annotation movement"));
        }
        // Story page insertion changes ordinals before this transaction. The
        // standalone path has no such intermediate revision.
        if entry.source.rect != movement.old_rect
            || (allow_resize && entry.source.page != movement.source_page)
        {
            return Err(fail("anchor geometry changed during story mutation"));
        }
        translation_supported(entry)?;
        let target = pages
            .get(
                movement
                    .target_page
                    .checked_sub(1)
                    .ok_or_else(|| fail("invalid anchor page"))?,
            )
            .ok_or_else(|| fail("missing anchor target page"))?;
        let origin = &pages[entry.source.page - 1];
        validate_geometry_destination(
            origin,
            target,
            movement.old_rect,
            movement.new_rect,
            allow_resize,
        )?;
        let mut dict = geometry::transform(reader, entry, movement.new_rect)?;
        if let Some(stamped) =
            crate::annotation_identity::stamp(reader, &dict, &movement.annotation_id)?
        {
            dict = stamped;
        }
        if let Some(change) = &movement.name_change {
            dict.insert("NM", identity::text_string(&change.replacement));
        }
        dict.insert(
            "P",
            PdfObject::Reference {
                number: target.object_number,
                generation: target.generation_number,
            },
        );
        if entry.source.page != movement.target_page {
            structure_moves.insert(reference, (target.object_number, target.generation_number));
            for page_number in [entry.source.page, movement.target_page] {
                if let std::collections::btree_map::Entry::Vacant(e) =
                    page_updates.entry(page_number)
                {
                    e.insert(page_annots(&engine, &pages[page_number - 1])?);
                }
            }
            let (_, source) = page_updates
                .get_mut(&entry.source.page)
                .ok_or_else(|| fail("missing source Annots update"))?;
            source.retain(|v| v.as_reference() != Some(reference));
            incoming.entry(movement.target_page).or_default().push((
                entry.source.page,
                entry.annotation_order,
                reference,
            ));
        }
        updates.push(crate::writer::IncrementalObject {
            number: reference.0,
            generation: reference.1,
            object: PdfObject::Dictionary(dict),
        });
    }
    for (number, (mut dict, mut annots)) in page_updates {
        let page = &pages[number - 1];
        if let Some(arrivals) = incoming.get_mut(&number) {
            // Request/group ID ordering is not PDF paint ordering. Keep each
            // source page's annotation order, after unchanged target entries.
            arrivals.sort_unstable();
            annots.extend(arrivals.iter().map(|(_, _, id)| PdfObject::Reference {
                number: id.0,
                generation: id.1,
            }));
        }
        if annots.is_empty() {
            dict.remove("Annots");
        } else {
            dict.insert("Annots", PdfObject::Array(annots));
        }
        updates.push(crate::writer::IncrementalObject {
            number: page.object_number,
            generation: page.generation_number,
            object: PdfObject::Dictionary(dict),
        });
    }
    updates.extend(crate::tagged_structure::annotation_page_migration(
        input,
        &structure_moves,
    )?);
    // Bind every touched annotation dictionary, not only the displayed Rect.
    // Serializing both sides also normalizes equivalent integer/real syntax.
    let mut expected_dictionaries = BTreeMap::new();
    let mut fingerprint_bytes = 0usize;
    for update in &updates {
        if selected.contains(&(update.number, update.generation)) {
            let mut bytes = Vec::new();
            crate::writer::serialize_object(&update.object, &mut bytes);
            fingerprint_bytes = fingerprint_bytes.saturating_add(bytes.len());
            if fingerprint_bytes > 64 * 1024 * 1024 {
                return Err(fail("annotation dictionary verification budget exceeded"));
            }
            expected_dictionaries.insert(
                (update.number, update.generation),
                format!("{:x}", Sha256::digest(bytes)),
            );
        }
    }
    let output = crate::writer::write_incremental_update(reader, updates)?;
    if engine
        .document()
        .get_catalog()?
        .contains_key("StructTreeRoot")
    {
        crate::tagged_structure::validate_parent_tree(&output)?;
    }
    let reopened = ContentEngine::open_bytes(output.clone())?;
    for ((number, generation), expected) in expected_dictionaries {
        crate::cancel::check_current_cancel("annotation dictionary verification")?;
        let object = reopened
            .document()
            .reader()
            .get_object(number, generation)?;
        let mut bytes = Vec::new();
        crate::writer::serialize_object(&object, &mut bytes);
        if format!("{:x}", Sha256::digest(bytes)) != expected {
            return Err(fail("annotation dictionary differs after saving"));
        }
    }
    let final_entries = inventory(&reopened)?;
    for movement in moves {
        let current = &final_entries
            .get(&movement.annotation_id)
            .ok_or_else(|| fail("translated anchor missing after reopen"))?
            .source;
        if current.group.as_ref().map(|g| &g.topology_sha256)
            != entries[&movement.annotation_id]
                .source
                .group
                .as_ref()
                .map(|g| &g.topology_sha256)
        {
            return Err(fail("annotation group topology changed during translation"));
        }
        if current.page != movement.target_page
            || current.name
                != movement
                    .name_change
                    .as_ref()
                    .map(|c| c.replacement.clone())
                    .or_else(|| entries[&movement.annotation_id].source.name.clone())
            || current
                .rect
                .iter()
                .zip(movement.new_rect)
                .any(|(a, b)| (a - b).abs() > 1e-5)
        {
            return Err(fail("anchor translation postcondition failed"));
        }
    }
    Ok(output)
}
pub(crate) fn rebind(
    input: &[u8],
    anchors: &[StoryAnnotationAnchor],
) -> Result<Vec<StoryAnnotationAnchor>> {
    if anchors.is_empty() {
        return Ok(Vec::new());
    }
    let sources = annotation_anchor_sources(input)?
        .into_iter()
        .map(|s| (s.annotation_id.clone(), s))
        .collect::<BTreeMap<_, _>>();
    anchors
        .iter()
        .map(|a| {
            let mut a = a.clone();
            let current = sources
                .get(&a.annotation_id)
                .ok_or_else(|| fail("saved anchor missing"))?;
            a.geometry_sha256 = current.geometry_sha256.clone();
            a.group = current.group.clone();
            Ok(a)
        })
        .collect()
}

/// Page ordinals are projections, not annotation ownership. Inserting/pruning
/// pages for a different story can change them without changing these objects.
/// Discovery/load refreshes ordinals; exact membership, geometry and order
/// remain authoritative and cannot be refreshed silently from altered content.
pub(crate) fn same_saved_binding(a: &StoryAnnotationAnchor, b: &StoryAnnotationAnchor) -> bool {
    a.annotation_id == b.annotation_id
        && a.geometry_sha256 == b.geometry_sha256
        && match (&a.group, &b.group) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                a.topology_sha256 == b.topology_sha256
                    && a.paint_order == b.paint_order
                    && a.members.len() == b.members.len()
                    && a.members.iter().zip(&b.members).all(|(a, b)| {
                        a.annotation_id == b.annotation_id && a.geometry_sha256 == b.geometry_sha256
                    })
            }
            _ => false,
        }
}

#[cfg(test)]
#[path = "story_annotation_group_tests.rs"]
mod group_tests;

#[cfg(test)]
#[path = "story_annotation_identity_tests.rs"]
mod identity_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writer::{write_incremental_update, IncrementalObject};
    #[test]
    fn cross_page_translation_preserves_appearance_and_actions_and_rebinds() {
        use crate::authoring::{PageSize, PdfBuilder};
        let mut builder = PdfBuilder::new();
        builder.add_page(PageSize::custom(200.0, 200.0));
        builder.add_page(PageSize::custom(200.0, 200.0));
        let engine = ContentEngine::open_bytes(builder.to_bytes().unwrap()).unwrap();
        let pages = engine.document().get_pages().unwrap();
        let reader = engine.document().reader();
        let n = reader.object_ids().iter().map(|(n, _)| *n).max().unwrap() + 1;
        let mut annotation = PdfDictionary::empty();
        annotation.insert("Type", PdfObject::Name("Annot".into()));
        annotation.insert("Subtype", PdfObject::Name("Link".into()));
        annotation.insert("NM", PdfObject::String(b"stable-link".to_vec()));
        annotation.insert("Rect", array([10.0, 20.0, 40.0, 30.0]));
        annotation.insert(
            "QuadPoints",
            array([10.0, 30.0, 40.0, 30.0, 10.0, 20.0, 40.0, 20.0]),
        );
        let mut action = PdfDictionary::empty();
        action.insert("S", PdfObject::Name("URI".into()));
        action.insert(
            "URI",
            PdfObject::String(b"https://example.invalid/".to_vec()),
        );
        annotation.insert("A", PdfObject::Dictionary(action.clone()));
        let mut ap = PdfDictionary::empty();
        ap.insert(
            "N",
            PdfObject::Reference {
                number: n + 1,
                generation: 0,
            },
        );
        annotation.insert("AP", PdfObject::Dictionary(ap.clone()));
        let raw = b"0 0 30 10 re f".to_vec();
        let mut ap_dict = PdfDictionary::empty();
        ap_dict.insert("Type", PdfObject::Name("XObject".into()));
        ap_dict.insert("Subtype", PdfObject::Name("Form".into()));
        ap_dict.insert("BBox", array([0.0, 0.0, 30.0, 10.0]));
        ap_dict.insert("Length", PdfObject::Integer(raw.len() as i64));
        let (mut page, _) = page_annots(&engine, &pages[0]).unwrap();
        page.insert(
            "Annots",
            PdfObject::Array(vec![PdfObject::Reference {
                number: n,
                generation: 0,
            }]),
        );
        let input = write_incremental_update(
            reader,
            vec![
                IncrementalObject {
                    number: n,
                    generation: 0,
                    object: PdfObject::Dictionary(annotation),
                },
                IncrementalObject {
                    number: n + 1,
                    generation: 0,
                    object: PdfObject::Stream {
                        dict: ap_dict,
                        raw: raw.clone(),
                    },
                },
                IncrementalObject {
                    number: pages[0].object_number,
                    generation: pages[0].generation_number,
                    object: PdfObject::Dictionary(page),
                },
            ],
        )
        .unwrap();
        let before = annotation_anchor_sources(&input).unwrap().remove(0);
        let output = apply_moves(
            &input,
            &[StoryAnnotationMove {
                annotation_id: "stable-link".into(),
                source_page: 1,
                target_page: 2,
                old_rect: before.rect,
                new_rect: [50.0, 60.0, 80.0, 70.0],
                name_change: None,
            }],
        )
        .unwrap();
        let reopened = ContentEngine::open_bytes(output.clone()).unwrap();
        let entries = inventory(&reopened).unwrap();
        let link = &entries["stable-link"];
        assert_eq!(link.source.page, 2);
        assert_eq!(link.dict.get("A"), Some(&PdfObject::Dictionary(action)));
        assert_eq!(link.dict.get("AP"), Some(&PdfObject::Dictionary(ap)));
        let PdfObject::Stream { raw: unchanged, .. } =
            reopened.document().reader().get_object(n + 1, 0).unwrap()
        else {
            panic!("appearance missing")
        };
        assert_eq!(unchanged, raw);
        assert_eq!(
            finite_array(
                reopened.document().reader(),
                link.dict.get("QuadPoints").unwrap()
            )
            .unwrap(),
            vec![50.0, 70.0, 80.0, 70.0, 50.0, 60.0, 80.0, 60.0]
        );
        assert!(
            page_annots(&reopened, &reopened.document().get_page(1).unwrap())
                .unwrap()
                .1
                .is_empty()
        );
        assert_ne!(before.geometry_sha256, link.source.geometry_sha256);
        assert!(output.starts_with(&input));
    }
}
