//! Ownership migration for a staged annotation appearance clone. Source roles,
//! MCIDs and element identities are retained; no reading order is inferred.
use super::*;
use serde::Deserialize;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaggedClonePolicy {
    #[default]
    Reject,
    /// Move existing content-item carriers only when the old namespace is no
    /// longer executed by any page or any normal/rollover/down appearance.
    MoveExclusiveNamespaces,
    /// Explicitly approve retaining live old carriers and inserting each new
    /// carrier immediately after its old carrier under the SAME logical owner.
    SplitSharedNamespacesAfterSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructureActualTextUpdate {
    pub element: ObjectRef,
    pub expected_text: String,
    pub replacement_text: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaggedCloneOptions {
    #[serde(default)]
    pub policy: TaggedClonePolicy,
    /// Affected owner/ancestor ActualText is a separate semantic value. Require
    /// exact user-supplied old/new text rather than retaining stale logical text
    /// or guessing how an ancestor's complete wording should change.
    #[serde(default)]
    pub actual_text_updates: Vec<StructureActualTextUpdate>,
}

fn denied(message: &str) -> WellfriendError {
    WellfriendError::UnsupportedFeature(format!("tagged appearance clone: {message}"))
}

/// The legacy stream-only guard cannot see an OBJR targeting the annotation
/// itself. Preserve reject-by-default for that ownership route as well.
pub(crate) fn check_annotation_clone_ownership(
    engine: &ContentEngine,
    annotation: ObjectRef,
) -> Result<()> {
    crate::cancel::check_current_cancel("annotation clone structural ownership")?;
    let object = engine
        .document()
        .reader()
        .get_object(annotation.0, annotation.1)?;
    let marked = dictionary(&object).is_some_and(|dict| {
        ["StructParent", "StructParents"]
            .iter()
            .any(|key| dict.get(key).is_some_and(|value| !value.is_null()))
    });
    if !engine
        .document()
        .get_catalog()?
        .contains_key("StructTreeRoot")
    {
        if marked {
            return Err(fail("annotation has orphan structural ownership markers"));
        }
        return Ok(());
    }
    let (_, _, index) = index_document(engine, None)?;
    if marked || index.objects.contains_key(&annotation) {
        return Err(denied(
            "whole-annotation OBJR ownership requires an explicit tagged clone policy",
        ));
    }
    Ok(())
}

/// Every intermediate PDF is private in-memory staging; only validated final
/// bytes are returned. Mapping contains the edited leaf and cloned ancestors.
pub(crate) fn finish(
    original: &[u8],
    candidate: Vec<u8>,
    annotation: ObjectRef,
    page: ObjectRef,
    clones: &BTreeMap<ObjectRef, ObjectRef>,
    options: &TaggedCloneOptions,
) -> Result<Vec<u8>> {
    crate::cancel::check_current_cancel("tagged appearance clone migration")?;
    if options.policy == TaggedClonePolicy::Reject {
        return Err(denied("explicit clone ownership policy required"));
    }
    if clones.len() > 128 || options.actual_text_updates.len() > 1024 {
        return Err(fail("tagged clone request budget exceeded"));
    }
    let source = ContentEngine::open_bytes(original.to_vec())?;
    let source_numbers = source
        .document()
        .reader()
        .object_ids()
        .into_iter()
        .map(|id| id.0)
        .collect::<BTreeSet<_>>();
    let new_ids = clones.values().copied().collect::<BTreeSet<_>>();
    if clones.is_empty()
        || new_ids.len() != clones.len()
        || new_ids.iter().any(|id| source_numbers.contains(&id.0))
    {
        return Err(fail("clone identity map is not one-to-one and fresh"));
    }
    if !source
        .document()
        .get_catalog()?
        .contains_key("StructTreeRoot")
    {
        if dictionary(
            &source
                .document()
                .reader()
                .get_object(annotation.0, annotation.1)?,
        )
        .is_some_and(|dict| dict.contains_key("StructParent") || dict.contains_key("StructParents"))
        {
            return Err(fail(
                "untagged document has orphan annotation structural markers",
            ));
        }
        for id in clones.keys() {
            let object = source.document().reader().get_object(id.0, id.1)?;
            if dictionary(&object).is_some_and(|dict| {
                dict.contains_key("StructParents") || dict.contains_key("StructParent")
            }) {
                return Err(fail("tagged clone has orphan source structural markers"));
            }
        }
        if !options.actual_text_updates.is_empty() {
            return Err(fail("ActualText updates supplied for an untagged document"));
        }
        return Ok(candidate);
    }
    validate_parent_tree(original)?;
    let (source_store, root, index) = index_document(&source, None)?;
    let staged = ContentEngine::open_bytes(candidate.clone())?;
    let staged_reader = staged.document().reader();
    let mut live = ContentScopes::new(&staged);
    live.collect_live()?;
    let source_annotation = source_store.dict(annotation)?;
    let output_annotation = Store::new(staged_reader).dict(annotation)?;
    let source_root = crate::annotation_appearance::select_normal(
        &source_annotation,
        source.document().reader(),
    )?
    .and_then(|selected| selected.stream)
    .ok_or_else(|| fail("missing source appearance root"))?;
    let output_root =
        crate::annotation_appearance::select_normal(&output_annotation, staged_reader)?
            .and_then(|selected| selected.stream)
            .ok_or_else(|| fail("missing cloned appearance root"))?;
    let descendants = live.descendants(output_root)?;
    if clones.get(&source_root) != Some(&output_root) || !new_ids.is_subset(&descendants) {
        return Err(fail("clone map is outside the selected appearance graph"));
    }
    for document in [source.document(), staged.document()] {
        let page_object = document.reader().get_object(page.0, page.1)?;
        let annotations = page_object
            .as_dict()
            .and_then(|d| d.get("Annots"))
            .ok_or_else(|| fail("selected page has no annotation owner"))?;
        let annotations = document.reader().resolve(annotations.clone())?;
        if annotations.as_array().is_none_or(|items| {
            items
                .iter()
                .filter(|item| item.as_reference() == Some(annotation))
                .count()
                != 1
        }) {
            return Err(fail("selected annotation/page ownership is not unique"));
        }
    }
    let mut retained = BTreeSet::new();
    let mut affected = BTreeSet::new();
    // An OBJR can own the annotation as a whole even if the AP contains no
    // MCIDs. Its logical ActualText must not silently override the new paint.
    if let Some(owner) = index.objects.get(&annotation) {
        affected.insert(*owner);
    }
    let mut owned = BTreeMap::new();
    // The normal liveness scanner does not execute patterns, Type 3 glyph
    // programs or soft masks. A source reachable through those resources must
    // keep its existing carrier too; absence from page/AP Do calls is not proof
    // that it is unused. Conservatively retain even an unused resource here.
    let other_programs = other_program_references(&staged, &live, clones)?;
    for (&old, &new) in clones {
        crate::cancel::check_current_cancel("tagged clone namespace mapping")?;
        if source.document().reader().get_object(old.0, old.1)?
            != staged_reader.get_object(old.0, old.1)?
        {
            return Err(fail("staged clone changed an original source program"));
        }
        if !live.actual.contains_key(&new) {
            return Err(fail("new cloned namespace is not physically reachable"));
        }
        if let Some(items) = index.marked.get(&old) {
            let expected = items.keys().copied().collect::<BTreeSet<_>>();
            if live.actual.get(&new) != Some(&expected) {
                return Err(denied("edited clone changed the source MCID set; an explicit content-item remap is required"));
            }
            affected.extend(items.values().copied());
            owned.insert(old, new);
        }
        if let Some(owner) = index.objects.get(&old) {
            affected.insert(*owner);
            owned.insert(old, new);
        }
        if owned.contains_key(&old)
            && (live.actual.contains_key(&old) || other_programs.contains(&old))
        {
            if options.policy != TaggedClonePolicy::SplitSharedNamespacesAfterSource {
                return Err(denied("source namespace remains live; approve shared-namespace split and its logical order"));
            }
            retained.insert(old);
        }
    }
    if affected.is_empty() {
        if !options.actual_text_updates.is_empty() {
            return Err(fail(
                "no affected logical owners for requested ActualText updates",
            ));
        }
        return Ok(candidate);
    }
    // Close the ancestor set so a paragraph/section-level logical replacement
    // cannot silently override the newly written appearance glyphs.
    let mut pending = affected.iter().copied().collect::<Vec<_>>();
    while let Some(id) = pending.pop() {
        crate::cancel::check_current_cancel("tagged clone logical ancestor closure")?;
        if affected.len() > MAX_NODES {
            return Err(fail("tagged clone ancestor budget exceeded"));
        }
        let parent = source_store
            .dict(id)?
            .get_reference("P")
            .ok_or_else(|| fail("structure owner has no parent"))?;
        if parent != root && affected.insert(parent) {
            pending.push(parent);
        }
    }
    let mut logical_updates = BTreeMap::new();
    let mut text_bytes = 0usize;
    for update in &options.actual_text_updates {
        text_bytes = text_bytes
            .saturating_add(update.expected_text.len())
            .saturating_add(update.replacement_text.len());
        if text_bytes > 16 * 1024 * 1024
            || !affected.contains(&update.element)
            || logical_updates.insert(update.element, update).is_some()
        {
            return Err(fail(
                "out-of-scope, duplicate or excessive structural ActualText update",
            ));
        }
    }
    let mut updates = BTreeMap::new();
    let mut visits = 0usize;
    let mut rebound = BTreeSet::new();
    for &node in &index.nodes {
        crate::cancel::check_current_cancel("tagged clone content carrier rewrite")?;
        let mut dict = source_store.dict(node)?;
        let mut changed = false;
        if let Some(kids) = dict.get("K") {
            let (items, did_change) = rewrite_kids(
                &source_store,
                kids,
                &owned,
                &retained,
                annotation,
                source_root,
                page,
                0,
                &mut visits,
                &mut rebound,
            )?;
            if did_change {
                dict.insert("K", pack(items));
                changed = true;
            }
        }
        if affected.contains(&node) {
            let actual = dict
                .get("ActualText")
                .map(|value| source_store.resolve(value))
                .transpose()?;
            match actual {
                Some(PdfObject::String(bytes)) => {
                    let old = crate::info::decode_pdf_text_string(&bytes);
                    let update = logical_updates.remove(&node).ok_or_else(|| denied("affected structural ActualText requires an exact old/new logical-text decision"))?;
                    if update.expected_text != old {
                        return Err(fail("structural ActualText compare-and-swap mismatch"));
                    }
                    dict.insert(
                        "ActualText",
                        crate::annotation_identity::text_string(&update.replacement_text),
                    );
                    changed = true;
                }
                None | Some(PdfObject::Null) => {}
                _ => return Err(fail("structural ActualText is not a text string")),
            }
        }
        if changed {
            updates.insert(node, PdfObject::Dictionary(dict));
        }
    }
    if !logical_updates.is_empty() {
        return Err(fail(
            "ActualText update has no matching source logical replacement",
        ));
    }
    if rebound != owned.keys().copied().collect() {
        return Err(fail(
            "cloned tagged namespace has no rewritten reachable carrier",
        ));
    }
    let changes = updates
        .into_iter()
        .map(|((number, generation), object)| IncrementalObject {
            number,
            generation,
            object,
        })
        .collect();
    let migrated = write_incremental_update(staged_reader, changes)?;
    // Reuse the canonical complete owner/ParentTree/IDTree writer. It checks
    // actual program MCIDs, assigns collision-free per-container keys and
    // validates reciprocal output ownership before returning.
    let (output, _) = rebuild_owner_trees(&migrated, None)?;
    let final_engine = ContentEngine::open_bytes(output.clone())?;
    for old in clones.keys() {
        if source.document().reader().get_object(old.0, old.1)?
            != final_engine.document().reader().get_object(old.0, old.1)?
        {
            return Err(fail("tag migration changed an original source program"));
        }
    }
    Ok(output)
}

fn other_program_references(
    engine: &ContentEngine,
    live: &ContentScopes<'_>,
    clones: &BTreeMap<ObjectRef, ObjectRef>,
) -> Result<BTreeSet<ObjectRef>> {
    let reader = engine.document().reader();
    let mut roots = Vec::new();
    let mut visits = 0usize;
    {
        let mut add_resources = |resources: &PdfDictionary| -> Result<()> {
            // ExtGState includes SMask/G; Font includes Type 3 CharProcs. Traverse
            // their reference graphs, not content bytes. All Forms reached from a
            // normal page/AP invocation already have a namespace in `live`.
            for key in ["Pattern", "ExtGState", "Font"] {
                if let Some(value) = resources.get(key) {
                    if roots.len() >= MAX_NODES {
                        return Err(fail("tagged clone alternate resource root budget exceeded"));
                    }
                    roots.push(value.clone());
                }
            }
            Ok(())
        };
        for page in engine.document().get_pages()? {
            crate::cancel::check_current_cancel("tagged clone alternate resource roots")?;
            add_resources(&page.resources)?;
        }
        for id in live.actual.keys() {
            crate::cancel::check_current_cancel("tagged clone alternate resource roots")?;
            let value = reader.get_object(id.0, id.1)?;
            if let Some(resources) = dictionary(&value).and_then(|dict| dict.get("Resources")) {
                match reader.resolve(resources.clone())? {
                    PdfObject::Dictionary(resources) => add_resources(&resources)?,
                    PdfObject::Null => {}
                    _ => return Err(fail("invalid alternate program resource dictionary")),
                }
            }
        }
    }
    let mut seen = BTreeSet::new();
    let mut referenced = BTreeSet::new();
    while let Some(value) = roots.pop() {
        crate::cancel::check_current_cancel("tagged clone alternate program references")?;
        visits = visits.saturating_add(1);
        if visits > MAX_NODES || roots.len() > MAX_NODES {
            return Err(fail(
                "tagged clone alternate program reference budget exceeded",
            ));
        }
        match value {
            PdfObject::Reference { number, generation } => {
                let id = (number, generation);
                if clones.contains_key(&id) {
                    referenced.insert(id);
                }
                if seen.insert(id) {
                    roots.push(reader.get_object(number, generation)?);
                }
            }
            PdfObject::Dictionary(dict) | PdfObject::Stream { dict, .. } => {
                if roots.len().saturating_add(dict.iter().count()) > MAX_NODES {
                    return Err(fail("tagged clone resource fan-out budget exceeded"));
                }
                roots.extend(dict.iter().map(|(_, value)| value.clone()));
            }
            PdfObject::Array(items) => {
                if roots.len().saturating_add(items.len()) > MAX_NODES {
                    return Err(fail("tagged clone resource array budget exceeded"));
                }
                roots.extend(items);
            }
            _ => {}
        }
    }
    Ok(referenced)
}

fn pack(mut items: Vec<PdfObject>) -> PdfObject {
    if items.len() == 1 {
        items.remove(0)
    } else {
        PdfObject::Array(items)
    }
}

#[allow(clippy::too_many_arguments)]
fn rewrite_kids(
    store: &Store<'_>,
    value: &PdfObject,
    clones: &BTreeMap<ObjectRef, ObjectRef>,
    retained: &BTreeSet<ObjectRef>,
    annotation: ObjectRef,
    appearance_root: ObjectRef,
    page: ObjectRef,
    depth: usize,
    visits: &mut usize,
    rebound: &mut BTreeSet<ObjectRef>,
) -> Result<(Vec<PdfObject>, bool)> {
    crate::cancel::check_current_cancel("tagged clone child traversal")?;
    *visits = visits.saturating_add(1);
    if depth > 128 || *visits > MAX_NODES {
        return Err(fail("tagged clone child traversal budget exceeded"));
    }
    let object = store.resolve(value)?;
    match object {
        PdfObject::Array(items) => {
            let mut result = Vec::new();
            let mut changed = false;
            for item in items {
                let (items, did_change) = rewrite_kids(
                    store,
                    &item,
                    clones,
                    retained,
                    annotation,
                    appearance_root,
                    page,
                    depth + 1,
                    visits,
                    rebound,
                )?;
                result.extend(items);
                changed |= did_change;
            }
            Ok((
                vec![if changed {
                    PdfObject::Array(result)
                } else {
                    value.clone()
                }],
                changed,
            ))
        }
        PdfObject::Dictionary(mut dict) => {
            let key = if dict.get_name("Type") == Some("OBJR") {
                "Obj"
            } else if dict.get_name("Type") == Some("MCR")
                || (dict.get_name("S").is_none() && dict.contains_key("MCID"))
            {
                "Stm"
            } else {
                return Ok((vec![value.clone()], false));
            };
            let Some(old) = dict.get_reference(key).filter(|id| clones.contains_key(id)) else {
                return Ok((vec![value.clone()], false));
            };
            let original = value.clone();
            dict.insert(key, reference(clones[&old]));
            dict.insert("Pg", reference(page));
            if key == "Stm" {
                if dict
                    .get_reference("StmOwn")
                    .is_some_and(|owner| owner != annotation)
                {
                    return Err(denied("MCR has a different stream owner"));
                }
                // The annotation directly references the root AP, not every
                // descendant Form. Keep omitted nested ownership omitted;
                // do not fabricate an annotation StmOwn on ordinary Forms.
                if old == appearance_root || dict.contains_key("StmOwn") {
                    dict.insert("StmOwn", reference(annotation));
                }
            }
            rebound.insert(old);
            let mut items = Vec::new();
            if retained.contains(&old) {
                items.push(original);
            }
            items.push(PdfObject::Dictionary(dict));
            Ok((items, true))
        }
        _ => Ok((vec![value.clone()], false)),
    }
}
