//! Page-owned Figure leaves share the paragraph transaction, but never borrow
//! a caption's semantic identity. Source paint, logical ownership and output
//! MCIDs are independently checked on the exact revision.
use super::*;
use crate::image_fragments::ImageFragmentSource;
#[path = "tagged_figure_ocr.rs"]
pub(super) mod ocr;
#[cfg(test)]
#[path = "tagged_figure_tests.rs"]
mod tests;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FigureTagBinding {
    #[serde(default)]
    pub source: Option<TagReference>,
    /// Required for a new Figure. For an unchanged source image, None preserves
    /// its existing descriptions; Some explicitly replaces/removes Alt and E.
    #[serde(default)]
    pub semantic_text: Option<StorySemanticText>,
    /// A reused Form MCR denotes one shared content namespace rather than one
    /// painted invocation. When explicitly approved, retain a residual Figure
    /// leaf and any separately selected content-only OCR owner for the
    /// unselected invocations while this story moves the existing Figure owner
    /// with the selected occurrence.
    #[serde(default)]
    pub split_reused_form_semantics: bool,
    /// `/Ref` is semantic ownership rather than Form content. Splitting a
    /// reused Form cannot infer which result should own external relationships,
    /// so this one-shot policy applies to the Figure and every preserved
    /// descendant. Internal subtree references follow their corresponding copy.
    #[serde(default)]
    pub outbound_ref_split: Option<FigureOutboundReferenceSplit>,
    /// Other structure elements can point at this Figure or a preserved
    /// descendant through `/Ref`. A reused-Form split requires one decision for
    /// those external incoming links; all affected referrers are rewritten in
    /// one batch while internal links remain within their selected/clone tree.
    #[serde(default)]
    pub inbound_ref_split: Option<FigureIncomingReferenceSplit>,
    /// Preserve a bounded descendant structure tree whose children own no
    /// independent content or OBJR items. The Figure's direct source-content
    /// entries are replaced in place by its generated page MCR, and explicit
    /// descendant page bindings follow that destination page.
    #[serde(default)]
    pub preserve_semantic_subtree: bool,
    /// One-shot approval to remove the complete validated contentless subtree
    /// when this Figure appears in `figure_removals`. External relationships
    /// still require their own migration and therefore refuse publication.
    #[serde(default)]
    pub delete_semantic_subtree: bool,
    /// One-shot approval to clone a validated contentless descendant tree for
    /// the residual owner created by a reused-Form occurrence split. Clone IDs
    /// and page bindings are removed. `/Ref` links inside the complete Figure
    /// tree are rewritten to its clones; external links use the explicit
    /// outbound/inbound split policies above.
    #[serde(default)]
    pub clone_semantic_subtree_for_reused_form: bool,
    /// Backward-compatible one-owner spelling. Empty `span_ids` means every OCR
    /// span selected for this Figure; otherwise only those exact spans move from
    /// this owner. Do not combine it with `separate_ocr_owners`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub separate_ocr_owner: Option<FigureOcrOwnerBinding>,
    /// Separately tagged, content-only P/Span leaves whose exact invisible OCR
    /// spans are merged into this Figure. Each entry needs nonempty, disjoint
    /// `span_ids`. Source owners are removed from the selected sibling interval;
    /// no association is inferred.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub separate_ocr_owners: Vec<FigureOcrOwnerBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FigureOcrOwnerBinding {
    pub source: TagReference,
    pub policy: FigureOcrOwnerPolicy,
    /// Exact subset of `StoryFigure.ocr.span_ids` owned by this source. The
    /// legacy singular field may omit it to mean all selected spans.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub span_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FigureOcrOwnerPolicy {
    /// Remove the now-empty source owner and let the moved Figure's page MCR
    /// own the visual plus exact native search carrier.
    MergeIntoFigure,
}

fn ocr_owner_bindings(binding: &FigureTagBinding) -> Result<Vec<&FigureOcrOwnerBinding>> {
    if binding.separate_ocr_owner.is_some() && !binding.separate_ocr_owners.is_empty() {
        return Err(fail(
            "use either separate_ocr_owner or separate_ocr_owners, not both",
        ));
    }
    if let Some(owner) = &binding.separate_ocr_owner {
        Ok(vec![owner])
    } else {
        Ok(binding.separate_ocr_owners.iter().collect())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FigureOutboundReferenceSplit {
    MoveWithSelected,
    RetainWithResidual,
    CopyToBoth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FigureIncomingReferenceSplit {
    FollowSelected,
    RetargetResidual,
    ReferenceBoth,
}

pub(crate) fn owner_key(story: &str, figure: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"WFStoryFigure\0");
    hash.update((story.len() as u64).to_be_bytes());
    hash.update(story.as_bytes());
    hash.update(figure.as_bytes());
    format!("{:x}", hash.finalize())
}

fn mapped_role(store: &Store<'_>, root: ObjectRef, node: ObjectRef) -> Result<String> {
    let mut role = store.dict(node)?.get_name("S").unwrap_or("").to_owned();
    let root = store.dict(root)?;
    let map = root.get("RoleMap").map(|v| store.resolve(v)).transpose()?;
    let mut seen = BTreeSet::new();
    while role != "Figure" {
        if seen.len() >= 128 || !seen.insert(role.clone()) {
            return Err(fail("cyclic Figure RoleMap"));
        }
        let Some(next) = map
            .as_ref()
            .and_then(PdfObject::as_dict)
            .and_then(|d| d.get_name(&role))
        else {
            return Ok(role);
        };
        role = next.into();
    }
    Ok(role)
}

pub(super) fn figure_role(store: &Store<'_>, root: ObjectRef, node: ObjectRef) -> Result<bool> {
    Ok(mapped_role(store, root, node)? == "Figure")
}

fn content_leaf_value(
    store: &Store<'_>,
    index: &StructureIndex,
    value: &PdfObject,
) -> Result<bool> {
    content_leaf_value_with_additional_nodes(store, index, value, &BTreeSet::new())
}

fn content_leaf_value_with_additional_nodes(
    store: &Store<'_>,
    index: &StructureIndex,
    value: &PdfObject,
    additional_nodes: &BTreeSet<ObjectRef>,
) -> Result<bool> {
    let (value, identity) = store.resolve_identity(value)?;
    if identity.is_some_and(|identity| {
        index.nodes.contains(&identity) || additional_nodes.contains(&identity)
    }) {
        return Ok(false);
    }
    match value {
        PdfObject::Null | PdfObject::Integer(_) => Ok(true),
        PdfObject::Array(values) => {
            for value in values {
                if !content_leaf_value_with_additional_nodes(
                    store,
                    index,
                    &value,
                    additional_nodes,
                )? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        PdfObject::Dictionary(dictionary) => Ok((dictionary.get_name("Type") == Some("MCR")
            || dictionary.get_name("S").is_none() && dictionary.contains_key("MCID"))
            && !dictionary.contains_key("Obj")),
        _ => Ok(false),
    }
}

fn figure_content_leaf(store: &Store<'_>, index: &StructureIndex, node: ObjectRef) -> Result<bool> {
    if index.objects.values().any(|owner| *owner == node) {
        return Ok(false);
    }
    for value in kids(&store.dict(node)?) {
        if !content_leaf_value(store, index, &value)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn structure_child(
    store: &Store<'_>,
    index: &StructureIndex,
    value: &PdfObject,
) -> Result<Option<ObjectRef>> {
    structure_child_with_additional_nodes(store, index, value, &BTreeSet::new())
}

fn structure_child_with_additional_nodes(
    store: &Store<'_>,
    index: &StructureIndex,
    value: &PdfObject,
    additional_nodes: &BTreeSet<ObjectRef>,
) -> Result<Option<ObjectRef>> {
    let (_, identity) = store.resolve_identity(value)?;
    Ok(identity
        .filter(|identity| index.nodes.contains(identity) || additional_nodes.contains(identity)))
}

pub(super) fn validate_semantic_subtree(
    store: &Store<'_>,
    index: &StructureIndex,
    root: ObjectRef,
) -> Result<Vec<ObjectRef>> {
    validate_semantic_subtree_with_additional_nodes(store, index, root, &BTreeSet::new())
}

fn validate_semantic_subtree_with_additional_nodes(
    store: &Store<'_>,
    index: &StructureIndex,
    root: ObjectRef,
    additional_nodes: &BTreeSet<ObjectRef>,
) -> Result<Vec<ObjectRef>> {
    if index.objects.values().any(|owner| *owner == root) {
        return Err(fail("Figure subtree root owns an OBJR item"));
    }
    let mut pending = Vec::new();
    let mut has_content = false;
    for value in kids(&store.dict(root)?) {
        if let Some(child) =
            structure_child_with_additional_nodes(store, index, &value, additional_nodes)?
        {
            pending.push((child, root, 1usize));
        } else if !matches!(value, PdfObject::Null) {
            if !content_leaf_value_with_additional_nodes(store, index, &value, additional_nodes)? {
                return Err(fail(
                    "Figure subtree root has unsupported content ownership",
                ));
            }
            has_content = true;
        }
    }
    if pending.is_empty() || !has_content {
        return Err(fail(
            "semantic-subtree preservation requires both direct Figure content and descendants",
        ));
    }
    let mut seen = BTreeSet::new();
    let mut descendants = Vec::new();
    while let Some((node, parent, depth)) = pending.pop() {
        crate::cancel::check_current_cancel("Figure semantic subtree validation")?;
        if depth > 64 || seen.len() >= 4096 || !seen.insert(node) {
            return Err(fail("cyclic, shared or excessive Figure semantic subtree"));
        }
        let dictionary = store.dict(node)?;
        if dictionary.get_reference("P") != Some(parent)
            || index.objects.values().any(|owner| *owner == node)
            || index
                .marked
                .values()
                .any(|items| items.values().any(|owner| *owner == node))
        {
            return Err(fail(
                "Figure semantic subtree descendant has external parent or content ownership",
            ));
        }
        if let Some(page) = dictionary.get("Pg") {
            if !matches!(page, PdfObject::Null) {
                let valid = match page.as_reference() {
                    Some(page) => match store.dict(page) {
                        Ok(dictionary) => dictionary.get_name("Type") == Some("Page"),
                        Err(_) => false,
                    },
                    None => false,
                };
                if !valid {
                    return Err(fail(
                        "Figure semantic subtree descendant has malformed page binding",
                    ));
                }
            }
        }
        descendants.push(node);
        for value in kids(&dictionary) {
            if let Some(child) =
                structure_child_with_additional_nodes(store, index, &value, additional_nodes)?
            {
                pending.push((child, node, depth + 1));
            } else if !matches!(value, PdfObject::Null) {
                return Err(fail(
                    "Figure semantic subtree descendants must be contentless",
                ));
            }
        }
    }
    let root_parent = store
        .dict(root)?
        .get_reference("P")
        .ok_or_else(|| fail("Figure semantic subtree root has no structure parent"))?;
    let mut root_links = 0usize;
    for value in kids(&store.dict(root_parent)?) {
        if structure_child_with_additional_nodes(store, index, &value, additional_nodes)?
            == Some(root)
        {
            root_links += 1;
        }
    }
    if root_links != 1 && !(additional_nodes.contains(&root) && root_links == 0) {
        return Err(fail(
            "Figure semantic subtree root is shared by its structure parent",
        ));
    }
    let mut owned = seen;
    owned.insert(root);
    for &owner in &index.nodes {
        if owned.contains(&owner) {
            continue;
        }
        for value in kids(&store.dict(owner)?) {
            let Some(child) =
                structure_child_with_additional_nodes(store, index, &value, additional_nodes)?
            else {
                continue;
            };
            if owned.contains(&child) && !(child == root && root_parent == owner) {
                return Err(fail(
                    "Figure semantic subtree is shared by another structure owner",
                ));
            }
        }
    }
    Ok(descendants)
}

fn rebind_semantic_subtree_pages(
    store: &mut Store<'_>,
    descendants: &[ObjectRef],
    page: ObjectRef,
) -> Result<()> {
    for &node in descendants {
        crate::cancel::check_current_cancel("Figure semantic subtree page rebinding")?;
        let mut dictionary = store.dict(node)?;
        if dictionary
            .get("Pg")
            .is_some_and(|value| !matches!(value, PdfObject::Null))
        {
            dictionary.insert("Pg", reference(page));
            store.replace_dict(node, dictionary)?;
        }
    }
    Ok(())
}

fn clone_semantic_k_value(
    store: &Store<'_>,
    index: &StructureIndex,
    value: &PdfObject,
    clones: &BTreeMap<ObjectRef, ObjectRef>,
) -> Result<PdfObject> {
    if let Some(child) = structure_child(store, index, value)? {
        return clones
            .get(&child)
            .copied()
            .map(reference)
            .ok_or_else(|| fail("Figure semantic subtree clone map is incomplete"));
    }
    Ok(match value {
        PdfObject::Array(values) => PdfObject::Array(
            values
                .iter()
                .map(|value| clone_semantic_k_value(store, index, value, clones))
                .collect::<Result<Vec<_>>>()?,
        ),
        value => value.clone(),
    })
}

fn clone_semantic_subtree(
    store: &mut Store<'_>,
    index: &StructureIndex,
    root: ObjectRef,
    residual_root: ObjectRef,
    descendants: &[ObjectRef],
) -> Result<BTreeMap<ObjectRef, ObjectRef>> {
    let mut clones = BTreeMap::new();
    for &source in descendants {
        crate::cancel::check_current_cancel("Figure semantic subtree allocation")?;
        let dictionary = store.dict(source)?;
        let clone = store.add(PdfObject::Dictionary(dictionary))?;
        if clones.insert(source, clone).is_some() {
            return Err(fail("duplicate Figure semantic subtree clone source"));
        }
    }
    for (&source, &clone) in &clones {
        crate::cancel::check_current_cancel("Figure semantic subtree cloning")?;
        let source_dictionary = store.dict(source)?;
        let parent = source_dictionary
            .get_reference("P")
            .ok_or_else(|| fail("Figure semantic subtree clone source has no parent"))?;
        let parent = if parent == root {
            residual_root
        } else {
            clones
                .get(&parent)
                .copied()
                .ok_or_else(|| fail("Figure semantic subtree clone parent is outside the tree"))?
        };
        let mut dictionary = source_dictionary.clone();
        dictionary.insert("P", reference(parent));
        if let Some(value) = source_dictionary.get("K") {
            dictionary.insert("K", clone_semantic_k_value(store, index, value, &clones)?);
        }
        dictionary.remove("ID");
        dictionary.remove("Pg");
        dictionary.remove("WFStoryTagKey");
        dictionary.remove("WFStoryID");
        store.replace_dict(clone, dictionary)?;
    }
    let mut residual = store.dict(residual_root)?;
    let content = residual
        .get("K")
        .cloned()
        .ok_or_else(|| fail("reused Figure residual has no content ownership"))?;
    residual.insert(
        "K",
        clone_semantic_k_value(store, index, &content, &clones)?,
    );
    store.replace_dict(residual_root, residual)?;
    Ok(clones)
}

pub(super) fn select(
    store: &Store<'_>,
    root: ObjectRef,
    index: &StructureIndex,
    lookup: &Lookup,
    request: &LinkedStoryRequest,
    selected: &BTreeSet<ObjectRef>,
    used: &mut BTreeSet<ObjectRef>,
    removed_descendants: &mut BTreeSet<ObjectRef>,
) -> Result<BTreeMap<String, Option<ObjectRef>>> {
    let config = request
        .source_tags
        .as_ref()
        .ok_or_else(|| fail("missing story tags"))?;
    if request
        .figures
        .len()
        .saturating_add(request.figure_removals.len())
        > 1024
        || config.figures.len() > 1024
    {
        return Err(fail("tagged figure count budget exceeded"));
    }
    let mut result = BTreeMap::new();
    let mut all_owners = BTreeSet::new();
    let mut text_budget = 0usize;
    for (id, removing) in request
        .figures
        .iter()
        .map(|f| (&f.id, false))
        .chain(request.figure_removals.iter().map(|r| (&r.figure_id, true)))
    {
        if id.is_empty() || id.len() > 256 {
            return Err(fail("invalid tagged Figure identity"));
        }
        let binding = config.figures.get(id).ok_or_else(|| {
            fail("every retained/removed figure requires explicit Figure tag ownership")
        })?;
        let node = binding
            .source
            .as_ref()
            .map(|r| resolve_tag(root, index, lookup, r))
            .transpose()?;
        if binding.delete_semantic_subtree && (!removing || !binding.preserve_semantic_subtree) {
            return Err(fail(
                "Figure semantic subtree deletion approval requires removal of an approved subtree",
            ));
        }
        if binding.clone_semantic_subtree_for_reused_form
            && (removing
                || !binding.preserve_semantic_subtree
                || !binding.split_reused_form_semantics)
        {
            return Err(fail(
                "Figure semantic subtree clone approval requires a retained reused-Form subtree split",
            ));
        }
        if let Some(node) = node {
            if binding.preserve_semantic_subtree
                && binding.split_reused_form_semantics
                && !binding.clone_semantic_subtree_for_reused_form
            {
                return Err(fail(
                    "Figure semantic subtree reused-Form splitting requires explicit clone approval",
                ));
            }
            let valid_shape = if binding.preserve_semantic_subtree {
                let descendants = validate_semantic_subtree(store, index, node)?;
                if removing {
                    if !binding.delete_semantic_subtree {
                        return Err(fail(
                            "Figure semantic subtree deletion requires explicit complete-subtree approval",
                        ));
                    }
                    removed_descendants.extend(descendants);
                }
                true
            } else {
                figure_content_leaf(store, index, node)?
            };
            if !selected.contains(&node)
                || !all_owners.insert(node)
                || used.contains(&node)
                || !valid_shape
                || !figure_role(store, root, node)?
            {
                return Err(fail(
                    "Figure reuse requires a unique selected content owner or explicitly approved contentless semantic subtree, distinct from caption text",
                ));
            }
            if !removing {
                used.insert(node);
            }
        } else if removing || binding.semantic_text.is_none() || binding.preserve_semantic_subtree {
            return Err(fail(
                "new Figure requires explicit alternate-text review; removal requires an existing owner",
            ));
        }
        if let Some(text) = &binding.semantic_text {
            for value in [&text.alternate, &text.expansion].into_iter().flatten() {
                if value.len() > 1_000_000 {
                    return Err(fail("Figure semantic text budget exceeded"));
                }
                text_budget = text_budget.saturating_add(value.len());
            }
        }
        if text_budget > 4_000_000 || result.insert(id.clone(), node).is_some() {
            return Err(fail(
                "duplicate tagged figure or semantic-text budget exceeded",
            ));
        }
    }
    if result.len() != config.figures.len() {
        return Err(fail("Figure tag binding names a missing figure"));
    }
    Ok(result)
}

pub(super) fn select_ocr_owners(
    store: &Store<'_>,
    root: ObjectRef,
    index: &StructureIndex,
    lookup: &Lookup,
    request: &LinkedStoryRequest,
    parent: ObjectRef,
    selected: &BTreeSet<ObjectRef>,
    used: &BTreeSet<ObjectRef>,
    figures: &BTreeMap<String, Option<ObjectRef>>,
) -> Result<BTreeMap<String, BTreeMap<String, ObjectRef>>> {
    let config = request
        .source_tags
        .as_ref()
        .ok_or_else(|| fail("missing story tags"))?;
    let mut result = BTreeMap::new();
    let mut owners = BTreeSet::new();
    for figure in &request.figures {
        let binding = config
            .figures
            .get(&figure.id)
            .ok_or_else(|| fail("missing Figure semantic binding"))?;
        let bindings = ocr_owner_bindings(binding)?;
        if bindings.is_empty() {
            continue;
        }
        let Some(ocr) = &figure.ocr else {
            return Err(fail(
                "separate OCR ownership requires an existing Figure and an exact OCR selection",
            ));
        };
        if figures.get(&figure.id).copied().flatten().is_none() || bindings.len() > 4096 {
            return Err(fail(
                "separate OCR ownership requires an existing Figure and a bounded exact owner set",
            ));
        }
        let selected_spans = ocr
            .span_ids
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if selected_spans.len() != ocr.span_ids.len() {
            return Err(fail("duplicate Figure OCR span identity"));
        }
        let figure_owner = figures.get(&figure.id).copied().flatten();
        let mut span_owners = BTreeMap::new();
        for separate in bindings {
            match separate.policy {
                FigureOcrOwnerPolicy::MergeIntoFigure => {}
            }
            let owner = resolve_tag(root, index, lookup, &separate.source)?;
            let dictionary = store.dict(owner)?;
            let role = mapped_role(store, root, owner)?;
            if !selected.contains(&owner)
                || used.contains(&owner)
                || Some(owner) == figure_owner
                || !owners.insert(owner)
                || dictionary.get_reference("P") != Some(parent)
                || !figure_content_leaf(store, index, owner)?
                || !matches!(role.as_str(), "P" | "Span")
            {
                return Err(fail(
                    "separate OCR owners must be unique selected content-only P/Span siblings",
                ));
            }
            if dictionary
                .iter()
                .any(|(name, _)| !matches!(name.as_str(), "Type" | "S" | "P" | "K" | "Pg" | "ID"))
            {
                return Err(fail(
                    "separate OCR owner carries semantics/attributes requiring an explicit preservation policy",
                ));
            }
            let spans = if separate.span_ids.is_empty() {
                if binding.separate_ocr_owner.is_none() {
                    return Err(fail(
                        "each multi-owner OCR binding requires nonempty exact span_ids",
                    ));
                }
                &ocr.span_ids
            } else {
                &separate.span_ids
            };
            if spans.len() > 4096 {
                return Err(fail("separate OCR owner span budget exceeded"));
            }
            for span in spans {
                if !selected_spans.contains(span.as_str())
                    || span_owners.insert(span.clone(), owner).is_some()
                {
                    return Err(fail(
                        "separate OCR owner span_ids must be selected and disjoint",
                    ));
                }
            }
        }
        result.insert(figure.id.clone(), span_owners);
    }
    if config.figures.iter().any(|(id, binding)| {
        (binding.separate_ocr_owner.is_some() || !binding.separate_ocr_owners.is_empty())
            && !request.figures.iter().any(|figure| &figure.id == id)
    }) {
        return Err(fail(
            "separate OCR owner binding names a removed or missing Figure",
        ));
    }
    Ok(result)
}

enum Locator {
    Occurrence {
        stream: ObjectRef,
        stream_index: usize,
        range: [usize; 2],
    },
    Nested {
        occurrence: crate::universal_editing::UniversalImageOccurrenceV2,
        resources: PdfDictionary,
    },
    Native(String),
}
pub(super) struct SourceBinding {
    id: String,
    pub page: usize,
    owner: Option<ObjectRef>,
    locator: Locator,
}

fn resource_dictionary(reader: &PdfReader, value: Option<&PdfObject>) -> Result<PdfDictionary> {
    match value {
        None | Some(PdfObject::Null) => Ok(PdfDictionary::empty()),
        Some(value) => reader
            .resolve(value.clone())?
            .as_dict()
            .cloned()
            .ok_or_else(|| fail("nested Figure resource is not a dictionary")),
    }
}

fn nested_resources(
    reader: &PdfReader,
    page: &crate::document::PdfPage,
    occurrence: &crate::universal_editing::UniversalImageOccurrenceV2,
) -> Result<PdfDictionary> {
    let path = &occurrence.invocation_path;
    let outer = path
        .first()
        .ok_or_else(|| fail("nested Figure invocation path is empty"))?;
    if page.contents.get(occurrence.content_stream_index)
        != Some(&(outer.owner_stream_object, outer.owner_stream_generation))
    {
        return Err(fail("nested Figure page owner changed"));
    }
    let mut resources = page.resources.clone();
    for (depth, invocation) in path.iter().enumerate() {
        if depth > 0
            && (
                invocation.owner_stream_object,
                invocation.owner_stream_generation,
            ) != (path[depth - 1].form_object, path[depth - 1].form_generation)
        {
            return Err(fail("nested Figure owner chain changed"));
        }
        let xobjects = resource_dictionary(reader, resources.get("XObject"))?;
        if xobjects
            .get(&invocation.resource_name)
            .and_then(PdfObject::as_reference)
            != Some((invocation.form_object, invocation.form_generation))
        {
            return Err(fail("nested Figure resource chain changed"));
        }
        let object = reader.get_object(invocation.form_object, invocation.form_generation)?;
        let (form, _) = object
            .as_stream()
            .ok_or_else(|| fail("nested Figure target is not a Form stream"))?;
        if form.get_name("Subtype") != Some("Form") {
            return Err(fail("nested Figure target is not a Form XObject"));
        }
        if let Some(local) = form.get("Resources") {
            resources = resource_dictionary(reader, Some(local))?;
        }
    }
    Ok(resources)
}

/// Lossless, bounded view of a structure-element `/Ref` value.  Real-world
/// producers sometimes wrap the specification's reference/array forms in
/// additional direct or indirect arrays.  Treating those wrappers as a flat
/// list loses the ownership boundary of shared indirect containers, so the
/// split transaction keeps their topology and copy-on-writes every traversed
/// indirect array.
#[derive(Debug, Clone)]
enum RefGraph {
    Target(ObjectRef),
    Direct(Vec<RefGraph>),
    Indirect {
        source: ObjectRef,
        children: Vec<RefGraph>,
    },
}

fn ref_graph_mentions_any(
    store: &Store<'_>,
    value: &PdfObject,
    targets: &BTreeSet<ObjectRef>,
    visits: &mut usize,
    active: &mut BTreeSet<ObjectRef>,
    depth: usize,
) -> Result<bool> {
    *visits = (*visits).saturating_add(1);
    if depth > 16 || *visits > 16_384 {
        return Err(fail("overlong or excessive Figure /Ref container graph"));
    }
    match value {
        PdfObject::Reference { number, generation } => {
            let source = (*number, *generation);
            if targets.contains(&source) {
                return Ok(true);
            }
            if !active.insert(source) {
                return Err(fail("cyclic Figure /Ref container graph"));
            }
            let mentioned = match store.get(source)? {
                PdfObject::Array(values) => {
                    if values.is_empty() || values.len() > 4096 {
                        active.remove(&source);
                        return Err(fail("Figure /Ref container item budget exceeded"));
                    }
                    let mut mentioned = false;
                    for value in &values {
                        if ref_graph_mentions_any(store, value, targets, visits, active, depth + 1)?
                        {
                            mentioned = true;
                            break;
                        }
                    }
                    mentioned
                }
                _ => false,
            };
            active.remove(&source);
            Ok(mentioned)
        }
        PdfObject::Array(values) => {
            if values.is_empty() || values.len() > 4096 {
                return Err(fail("Figure /Ref container item budget exceeded"));
            }
            for value in values {
                if ref_graph_mentions_any(store, value, targets, visits, active, depth + 1)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        _ => Ok(false),
    }
}

fn ref_value_mentions_any(
    store: &Store<'_>,
    value: &PdfObject,
    targets: &BTreeSet<ObjectRef>,
) -> Result<bool> {
    let mut visits = 0;
    let mut active = BTreeSet::new();
    ref_graph_mentions_any(store, value, targets, &mut visits, &mut active, 0)
}

fn parse_ref_graph(
    store: &Store<'_>,
    value: &PdfObject,
    index: &StructureIndex,
    additional_targets: &BTreeSet<ObjectRef>,
    visits: &mut usize,
    active: &mut BTreeSet<ObjectRef>,
    depth: usize,
) -> Result<RefGraph> {
    *visits = (*visits).saturating_add(1);
    if depth > 16 || *visits > 16_384 {
        return Err(fail("overlong or excessive Figure /Ref container graph"));
    }
    let parse_children = |values: &[PdfObject],
                          visits: &mut usize,
                          active: &mut BTreeSet<ObjectRef>|
     -> Result<Vec<RefGraph>> {
        if values.is_empty() || values.len() > 4096 {
            return Err(fail("Figure /Ref container item budget exceeded"));
        }
        values
            .iter()
            .map(|value| {
                parse_ref_graph(
                    store,
                    value,
                    index,
                    additional_targets,
                    visits,
                    active,
                    depth + 1,
                )
            })
            .collect()
    };
    match value {
        PdfObject::Reference { number, generation } => {
            let source = (*number, *generation);
            if index.nodes.contains(&source) || additional_targets.contains(&source) {
                return Ok(RefGraph::Target(source));
            }
            if !active.insert(source) {
                return Err(fail("cyclic Figure /Ref container graph"));
            }
            let parsed = match store.get(source)? {
                PdfObject::Array(values) => RefGraph::Indirect {
                    source,
                    children: parse_children(&values, visits, active)?,
                },
                _ => {
                    active.remove(&source);
                    return Err(fail(
                        "Figure /Ref indirect container is not an array or structure element",
                    ));
                }
            };
            active.remove(&source);
            Ok(parsed)
        }
        PdfObject::Array(values) => Ok(RefGraph::Direct(parse_children(values, visits, active)?)),
        _ => Err(fail(
            "Figure semantic subtree /Ref uses a malformed container",
        )),
    }
}

fn ref_graph(store: &Store<'_>, value: &PdfObject, index: &StructureIndex) -> Result<RefGraph> {
    ref_graph_with_additional_targets(store, value, index, &BTreeSet::new())
}

fn ref_graph_with_additional_targets(
    store: &Store<'_>,
    value: &PdfObject,
    index: &StructureIndex,
    additional_targets: &BTreeSet<ObjectRef>,
) -> Result<RefGraph> {
    let mut visits = 0;
    let mut active = BTreeSet::new();
    parse_ref_graph(
        store,
        value,
        index,
        additional_targets,
        &mut visits,
        &mut active,
        0,
    )
}

fn ref_graph_targets(graph: &RefGraph, targets: &mut Vec<ObjectRef>) {
    match graph {
        RefGraph::Target(target) => targets.push(*target),
        RefGraph::Direct(children) | RefGraph::Indirect { children, .. } => {
            for child in children {
                ref_graph_targets(child, targets);
            }
        }
    }
}

fn parsed_ref_graph_signature(graph: &RefGraph) -> Vec<u8> {
    fn append(graph: &RefGraph, output: &mut Vec<u8>) {
        match graph {
            RefGraph::Target((number, generation)) => {
                output.extend_from_slice(format!("T{number}:{generation};").as_bytes());
            }
            RefGraph::Direct(children) => {
                output.extend_from_slice(format!("D{}[", children.len()).as_bytes());
                for child in children {
                    append(child, output);
                }
                output.push(b']');
            }
            RefGraph::Indirect {
                source: (number, generation),
                children,
            } => {
                output.extend_from_slice(
                    format!("I{number}:{generation}:{}[", children.len()).as_bytes(),
                );
                for child in children {
                    append(child, output);
                }
                output.push(b']');
            }
        }
    }
    let mut signature = Vec::new();
    append(graph, &mut signature);
    signature
}

fn optional_ref_graph_signature_with_additional_targets(
    store: &Store<'_>,
    value: Option<&PdfObject>,
    index: &StructureIndex,
    additional_targets: &BTreeSet<ObjectRef>,
) -> Result<PdfObject> {
    value
        .map(|value| {
            let graph = ref_graph_with_additional_targets(store, value, index, additional_targets)?;
            Ok::<_, WellfriendError>(PdfObject::String(parsed_ref_graph_signature(&graph)))
        })
        .transpose()
        .map(|signature| signature.unwrap_or(PdfObject::Null))
}

fn materialize_ref_graph(store: &mut Store<'_>, graph: RefGraph) -> Result<PdfObject> {
    fn materialize(
        store: &mut Store<'_>,
        graph: RefGraph,
        copied: &mut BTreeMap<ObjectRef, ObjectRef>,
    ) -> Result<PdfObject> {
        match graph {
            RefGraph::Target(target) => Ok(reference(target)),
            RefGraph::Direct(children) => Ok(PdfObject::Array(
                children
                    .into_iter()
                    .map(|child| materialize(store, child, copied))
                    .collect::<Result<Vec<_>>>()?,
            )),
            RefGraph::Indirect { source, children } => {
                if let Some(existing) = copied.get(&source) {
                    return Ok(reference(*existing));
                }
                let values = children
                    .into_iter()
                    .map(|child| materialize(store, child, copied))
                    .collect::<Result<Vec<_>>>()?;
                let clone = store.add(PdfObject::Array(values))?;
                copied.insert(source, clone);
                Ok(reference(clone))
            }
        }
    }
    materialize(store, graph, &mut BTreeMap::new())
}

fn inbound_referrers(
    store: &Store<'_>,
    index: &StructureIndex,
    target: ObjectRef,
) -> Result<Vec<ObjectRef>> {
    let mut result = Vec::new();
    let targets = BTreeSet::from([target]);
    for &node in &index.nodes {
        if node == target {
            continue;
        }
        let dictionary = store.dict(node)?;
        if let Some(value) = dictionary.get("Ref") {
            if !ref_value_mentions_any(store, value, &targets)? {
                continue;
            }
            let graph = ref_graph(store, value, index)?;
            let mut graph_targets = Vec::new();
            ref_graph_targets(&graph, &mut graph_targets);
            if graph_targets.contains(&target) {
                result.push(node);
            }
        }
    }
    Ok(result)
}

#[derive(Debug, Clone, Copy, Default)]
struct SemanticRelationshipSummary {
    outbound_external: bool,
    inbound_external: bool,
}

fn validate_semantic_relationship_domain(
    store: &Store<'_>,
    index: &StructureIndex,
    domain: &BTreeSet<ObjectRef>,
    coordinated: &BTreeSet<ObjectRef>,
) -> Result<SemanticRelationshipSummary> {
    let mut summary = SemanticRelationshipSummary::default();
    for &node in domain {
        crate::cancel::check_current_cancel("Figure semantic subtree relationship validation")?;
        let dictionary = store.dict(node)?;
        if let Some(value) = dictionary.get("Ref") {
            let graph = ref_graph(store, value, index)?;
            let mut targets = Vec::new();
            ref_graph_targets(&graph, &mut targets);
            summary.outbound_external |= targets.iter().any(|target| !coordinated.contains(target));
        }
        summary.inbound_external |= inbound_referrers(store, index, node)?
            .into_iter()
            .any(|referrer| !coordinated.contains(&referrer));
    }
    Ok(summary)
}

fn split_semantic_ref_value(
    store: &mut Store<'_>,
    index: &StructureIndex,
    value: &PdfObject,
    clones: &BTreeMap<ObjectRef, ObjectRef>,
    policy: Option<FigureOutboundReferenceSplit>,
) -> Result<(Option<PdfObject>, Option<PdfObject>)> {
    fn split_graph(
        graph: RefGraph,
        clones: &BTreeMap<ObjectRef, ObjectRef>,
        policy: Option<FigureOutboundReferenceSplit>,
    ) -> Result<(Option<RefGraph>, Option<RefGraph>)> {
        match graph {
            RefGraph::Target(target) => {
                if let Some(clone) = clones.get(&target) {
                    return Ok((
                        Some(RefGraph::Target(target)),
                        Some(RefGraph::Target(*clone)),
                    ));
                }
                Ok(match policy {
                    Some(FigureOutboundReferenceSplit::MoveWithSelected) => {
                        (Some(RefGraph::Target(target)), None)
                    }
                    Some(FigureOutboundReferenceSplit::RetainWithResidual) => {
                        (None, Some(RefGraph::Target(target)))
                    }
                    Some(FigureOutboundReferenceSplit::CopyToBoth) => (
                        Some(RefGraph::Target(target)),
                        Some(RefGraph::Target(target)),
                    ),
                    None => return Err(fail(
                        "external Figure subtree relationships require an explicit outbound split policy",
                    )),
                })
            }
            RefGraph::Direct(children) => {
                let mut selected = Vec::new();
                let mut residual = Vec::new();
                for child in children {
                    let (selected_child, residual_child) = split_graph(child, clones, policy)?;
                    selected.extend(selected_child);
                    residual.extend(residual_child);
                }
                Ok((
                    (!selected.is_empty()).then_some(RefGraph::Direct(selected)),
                    (!residual.is_empty()).then_some(RefGraph::Direct(residual)),
                ))
            }
            RefGraph::Indirect { source, children } => {
                let mut selected = Vec::new();
                let mut residual = Vec::new();
                for child in children {
                    let (selected_child, residual_child) = split_graph(child, clones, policy)?;
                    selected.extend(selected_child);
                    residual.extend(residual_child);
                }
                Ok((
                    (!selected.is_empty()).then_some(RefGraph::Indirect {
                        source,
                        children: selected,
                    }),
                    (!residual.is_empty()).then_some(RefGraph::Indirect {
                        source,
                        children: residual,
                    }),
                ))
            }
        }
    }

    let graph = ref_graph(store, value, index)?;
    let (selected, residual) = split_graph(graph, clones, policy)?;
    Ok((
        selected
            .map(|graph| materialize_ref_graph(store, graph))
            .transpose()?,
        residual
            .map(|graph| materialize_ref_graph(store, graph))
            .transpose()?,
    ))
}

fn split_semantic_relationships(
    store: &mut Store<'_>,
    index: &StructureIndex,
    domain_clones: &BTreeMap<ObjectRef, ObjectRef>,
    coordinated_clones: &BTreeMap<ObjectRef, ObjectRef>,
    policy: Option<FigureOutboundReferenceSplit>,
) -> Result<Vec<PdfDictionary>> {
    let mut receipts = Vec::with_capacity(domain_clones.len());
    let clone_targets = coordinated_clones
        .values()
        .copied()
        .collect::<BTreeSet<_>>();
    for (&source, &clone) in domain_clones {
        crate::cancel::check_current_cancel("Figure semantic subtree relationship split")?;
        let mut selected_dictionary = store.dict(source)?;
        let mut residual_dictionary = store.dict(clone)?;
        let (selected, residual) = if let Some(value) = selected_dictionary.get("Ref") {
            split_semantic_ref_value(store, index, value, coordinated_clones, policy)?
        } else {
            (None, None)
        };
        match &selected {
            Some(value) => selected_dictionary.insert("Ref", value.clone()),
            None => selected_dictionary.remove("Ref"),
        };
        match &residual {
            Some(value) => residual_dictionary.insert("Ref", value.clone()),
            None => residual_dictionary.remove("Ref"),
        };
        store.replace_dict(source, selected_dictionary)?;
        store.replace_dict(clone, residual_dictionary)?;
        let selected_signature = optional_ref_graph_signature_with_additional_targets(
            store,
            selected.as_ref(),
            index,
            &clone_targets,
        )?;
        let residual_signature = optional_ref_graph_signature_with_additional_targets(
            store,
            residual.as_ref(),
            index,
            &clone_targets,
        )?;
        let mut receipt = PdfDictionary::empty();
        receipt.insert("Source", reference(source));
        receipt.insert("Clone", reference(clone));
        receipt.insert("SelectedRefGraph", selected_signature);
        receipt.insert("CloneRefGraph", residual_signature);
        receipt.insert("SelectedRef", selected.unwrap_or(PdfObject::Null));
        receipt.insert("CloneRef", residual.unwrap_or(PdfObject::Null));
        receipts.push(receipt);
    }
    Ok(receipts)
}

pub(super) fn source_bindings(
    input: &[u8],
    request: &LinkedStoryRequest,
    selection: &Selection,
    store: &Store<'_>,
    index: &StructureIndex,
) -> Result<Vec<SourceBinding>> {
    if request.figures.is_empty() && request.figure_removals.is_empty() {
        return Ok(Vec::new());
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let source_pages = request
        .figures
        .iter()
        .filter_map(|f| match &f.source {
            ImageFragmentSource::Occurrence { page, .. } => Some(*page),
            _ => None,
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let inventory = if source_pages.is_empty() {
        Vec::new()
    } else {
        crate::universal_editing::universal_image_occurrences_v2(input, &source_pages)?
    };
    let config = request
        .source_tags
        .as_ref()
        .ok_or_else(|| fail("missing Figure configuration"))?;
    let mut split_domains = BTreeMap::new();
    for figure in &request.figures {
        let Some(binding) = config.figures.get(&figure.id) else {
            continue;
        };
        if !binding.split_reused_form_semantics {
            continue;
        }
        let root = selection
            .figures
            .get(&figure.id)
            .copied()
            .flatten()
            .ok_or_else(|| fail("reused Form semantic split has no Figure owner"))?;
        let mut domain = BTreeSet::from([root]);
        if binding.preserve_semantic_subtree {
            domain.extend(validate_semantic_subtree(store, index, root)?);
        }
        split_domains.insert(figure.id.as_str(), domain);
    }
    let coordinated_split_nodes = split_domains
        .values()
        .flat_map(|domain| domain.iter().copied())
        .collect::<BTreeSet<_>>();
    let mut result = Vec::new();
    let mut document_inventory = None;
    for (id, source) in request
        .figures
        .iter()
        .map(|f| (&f.id, f.source.clone()))
        .chain(request.figure_removals.iter().map(|r| {
            (
                &r.figure_id,
                ImageFragmentSource::Owned {
                    binding: r.binding.clone(),
                },
            )
        }))
    {
        let tag_binding = request
            .source_tags
            .as_ref()
            .and_then(|tags| tags.figures.get(id))
            .ok_or_else(|| fail("Figure tag binding disappeared"))?;
        let split_reused = tag_binding.split_reused_form_semantics;
        if !split_reused
            && (tag_binding.outbound_ref_split.is_some() || tag_binding.inbound_ref_split.is_some())
        {
            return Err(fail(
                "Figure relationship split policy requires a reused-Form semantic split",
            ));
        }
        let (page, locator) = match source {
            ImageFragmentSource::Occurrence {
                page,
                content_stream_index,
                occurrence_id,
            } => {
                let matches = inventory
                    .iter()
                    .filter(|i| {
                        i.page == page
                            && i.content_stream_index == content_stream_index
                            && i.occurrence_id == occurrence_id
                    })
                    .collect::<Vec<_>>();
                if matches.len() != 1 {
                    return Err(fail(format!(
                        "tagged Figure source must identify one exact image occurrence; found {} for page {page}, stream {content_stream_index}, occurrence {occurrence_id}",
                        matches.len()
                    )));
                }
                let image = matches[0];
                let figure_ocr = request
                    .figures
                    .iter()
                    .find(|figure| figure.id.as_str() == id.as_str())
                    .and_then(|figure| figure.ocr.as_ref());
                let locator = if image.invocation_path.is_empty() {
                    if figure_ocr.is_some_and(|ocr| ocr.form_target.is_some()) {
                        return Err(fail("page-owned Figure OCR cannot use a Form-local target"));
                    }
                    if split_reused {
                        return Err(fail(
                            "reused-Form semantic splitting was approved for a page-owned Figure",
                        ));
                    }
                    Locator::Occurrence {
                        stream: (image.owner_stream_object, image.owner_stream_generation),
                        stream_index: content_stream_index,
                        range: [image.operation_byte_start, image.operation_byte_end],
                    }
                } else {
                    if let Some(ocr) = figure_ocr {
                        let target = ocr.form_target.as_ref().ok_or_else(|| {
                            fail("tagged nested Form OCR requires its exact Form target")
                        })?;
                        if target.input_sha256 != request.input_sha256
                            || target.page != page
                            || target.content_stream_index != content_stream_index
                            || target.invocation_path != image.invocation_path
                        {
                            return Err(fail(
                                "tagged nested Form OCR target does not match its image occurrence",
                            ));
                        }
                    }
                    if document_inventory.is_none() {
                        let pages = engine
                            .document()
                            .get_pages()?
                            .into_iter()
                            .map(|page| page.page_number)
                            .collect::<Vec<_>>();
                        document_inventory =
                            Some(crate::universal_editing::universal_image_occurrences_v2(
                                input, &pages,
                            )?);
                    }
                    let all = document_inventory
                        .as_ref()
                        .ok_or_else(|| fail("nested Figure inventory disappeared"))?;
                    let uses = all
                        .iter()
                        .filter(|candidate| {
                            candidate.owner_stream_object == image.owner_stream_object
                                && candidate.owner_stream_generation
                                    == image.owner_stream_generation
                                && candidate.operation_byte_start == image.operation_byte_start
                                && candidate.operation_byte_end == image.operation_byte_end
                        })
                        .count();
                    if uses > 1 && (!split_reused || tag_binding.source.is_none()) {
                        return Err(fail(
                            "tagged nested Figure is reused; occurrence-specific semantic splitting requires an explicit owner decision",
                        ));
                    }
                    if uses == 1 && split_reused {
                        return Err(fail(
                            "reused-Form semantic splitting was approved for a unique occurrence",
                        ));
                    }
                    if uses > 1 {
                        let owner_id = selection
                            .figures
                            .get(id)
                            .copied()
                            .flatten()
                            .ok_or_else(|| fail("reused Figure owner disappeared"))?;
                        if tag_binding.preserve_semantic_subtree {
                            validate_semantic_subtree(store, index, owner_id)?;
                            if !tag_binding.clone_semantic_subtree_for_reused_form {
                                return Err(fail(
                                    "Figure semantic subtree reused-Form splitting requires explicit clone approval",
                                ));
                            }
                        }
                        let current_domain = split_domains.get(id.as_str()).ok_or_else(|| {
                            fail("reused Figure semantic split domain disappeared")
                        })?;
                        let relationship_summary = validate_semantic_relationship_domain(
                            store,
                            index,
                            current_domain,
                            &coordinated_split_nodes,
                        )?;
                        match (
                            relationship_summary.outbound_external,
                            tag_binding.outbound_ref_split,
                        ) {
                            (true, None) => {
                                return Err(fail(
                                    "reused Figure has outbound /Ref relationships requiring an explicit split policy",
                                ))
                            }
                            (false, Some(_)) => {
                                return Err(fail(
                                    "Figure outbound-reference split policy was supplied without an external source /Ref",
                                ))
                            }
                            _ => {}
                        }
                        match (
                            relationship_summary.inbound_external,
                            tag_binding.inbound_ref_split,
                        ) {
                            (true, None) => {
                                return Err(fail(
                                    "reused Figure has incoming /Ref relationships requiring an explicit split policy",
                                ))
                            }
                            (false, Some(_)) => {
                                return Err(fail(
                                    "Figure incoming-reference split policy was supplied without an external incoming /Ref",
                                ))
                            }
                            _ => {}
                        }
                    }
                    let page = engine.document().get_page(page)?;
                    Locator::Nested {
                        occurrence: image.clone(),
                        resources: nested_resources(engine.document().reader(), &page, image)?,
                    }
                };
                (page, locator)
            }
            ImageFragmentSource::Owned { binding } => {
                if split_reused {
                    return Err(fail(
                        "reused-Form semantic splitting is only valid for an initial nested occurrence",
                    ));
                }
                (binding.page, Locator::Native(binding.key))
            }
        };
        result.push(SourceBinding {
            id: id.clone(),
            page,
            owner: *selection
                .figures
                .get(id)
                .ok_or_else(|| fail("Figure source owner missing"))?,
            locator,
        });
    }
    Ok(result)
}

pub(super) fn validate_paint(
    page: usize,
    page_id: ObjectRef,
    marks: &PageMarks,
    index: &StructureIndex,
    selected: &BTreeSet<ObjectRef>,
    sources: &[SourceBinding],
    ocr: &ocr::PageOcr,
    seen: &mut BTreeSet<String>,
) -> Result<()> {
    let mut direct = BTreeMap::new();
    let mut native = BTreeMap::new();
    for source in sources.iter().filter(|s| s.page == page) {
        let duplicate = match &source.locator {
            Locator::Occurrence {
                stream,
                stream_index,
                range,
            } => direct
                .insert((*stream, *stream_index, *range), source)
                .is_some(),
            Locator::Nested { .. } => false,
            Locator::Native(key) => native.insert(key.as_str(), source).is_some(),
        };
        if duplicate {
            return Err(fail("duplicate Figure source occurrence"));
        }
    }
    for paint in &marks.nontext {
        crate::cancel::check_current_cancel("Figure source semantic coverage")?;
        let mut candidates = direct
            .get(&(paint.stream, paint.stream_index, paint.range))
            .copied()
            .into_iter()
            .collect::<Vec<_>>();
        for m in paint.marks.iter().map(|i| &marks.marks[*i]) {
            if let Some(source) = m.image_key.as_deref().and_then(|k| native.get(k)) {
                candidates.push(*source);
            }
        }
        let owners = paint
            .marks
            .iter()
            .filter_map(|i| marks.marks[*i].mcid)
            .filter_map(|m| index.marked.get(&page_id).and_then(|v| v.get(&m)))
            .copied()
            .collect::<BTreeSet<_>>();
        if candidates.is_empty() {
            if owners.iter().any(|o| selected.contains(o)) {
                return Err(fail(
                    "selected paragraph/Figure tag also owns unselected artwork",
                ));
            }
            continue;
        }
        if candidates.len() != 1 || !paint.image {
            return Err(fail("Figure binding selects ambiguous or non-image paint"));
        }
        let source = candidates[0];
        if owners != source.owner.into_iter().collect() || !seen.insert(source.id.clone()) {
            return Err(fail(
                "Figure paint does not belong exclusively to its approved logical owner",
            ));
        }
        for &slot in &paint.marks {
            let mark = &marks.marks[slot];
            if (mark.actual_text && !ocr.carries_actual_text(&source.id, slot))
                || mark.frame.is_some()
                || (!mark.safe && !(mark.image_key.is_some() || mark.artifact))
                || !(mark.mcid.is_some()
                    || mark.image_key.is_some()
                    || mark.artifact
                    || mark.actual_text && ocr.carries_actual_text(&source.id, slot))
            {
                return Err(fail(
                    "Figure paint has unapproved logical/optional content or frame ownership",
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn validate_nested_paint(
    engine: &ContentEngine,
    page: usize,
    request: &LinkedStoryRequest,
    index: &StructureIndex,
    sources: &[SourceBinding],
    ocr_selection: &mut ocr::OcrSelection,
    seen: &mut BTreeSet<String>,
) -> Result<()> {
    for source in sources.iter().filter(|source| source.page == page) {
        let Locator::Nested {
            occurrence,
            resources,
        } = &source.locator
        else {
            continue;
        };
        let scope = (
            occurrence.owner_stream_object,
            occurrence.owner_stream_generation,
        );
        let marks = form_marks(engine, scope, occurrence.content_stream_index, resources)?;
        let figure = request.figures.iter().find(|figure| figure.id == source.id);
        let form_ocr = if let Some(selection) = figure.and_then(|figure| figure.ocr.as_ref()) {
            let target = selection
                .form_target
                .as_ref()
                .ok_or_else(|| fail("nested Figure OCR target disappeared"))?;
            let capture_scope = crate::advanced_editing::form_text::ocr_capture_scope(
                engine,
                target,
                &request.input_sha256,
            )?;
            if capture_scope.source_page.contents != vec![scope] {
                return Err(fail("nested Figure OCR target leaf owner changed"));
            }
            let mut model = crate::advanced_editing::analyze_multi_run_source(
                engine,
                &capture_scope.source_page,
                capture_scope.initial,
            )?;
            model.paragraph_block_id = capture_scope.span_prefix.clone();
            for span in &mut model.source_spans {
                span.span_id = format!("{}:{}", capture_scope.span_prefix, span.span_id);
            }
            ocr_selection.form(page, &source.id, scope, &marks, index, &model)?
        } else {
            ocr::PageOcr::default()
        };
        let selected = marks
            .nontext
            .iter()
            .filter(|paint| {
                paint.stream == scope
                    && paint.stream_index == occurrence.content_stream_index
                    && paint.range
                        == [
                            occurrence.operation_byte_start,
                            occurrence.operation_byte_end,
                        ]
            })
            .collect::<Vec<_>>();
        if selected.len() != 1 || !selected[0].image {
            return Err(fail(
                "tagged nested Figure source is not one exact image paint",
            ));
        }
        let owners = |slots: &[usize]| -> Result<BTreeSet<ObjectRef>> {
            slots
                .iter()
                .filter_map(|slot| marks.marks[*slot].mcid)
                .map(|mcid| {
                    index
                        .marked
                        .get(&scope)
                        .and_then(|owners| owners.get(&mcid))
                        .copied()
                        .ok_or_else(|| fail("nested Figure MCID has no structure owner"))
                })
                .collect()
        };
        let expected = source.owner.into_iter().collect::<BTreeSet<_>>();
        if owners(&selected[0].marks)? != expected || !seen.insert(source.id.clone()) {
            return Err(fail(
                "nested Figure paint does not belong exclusively to its approved logical owner",
            ));
        }
        let mut selected_items = selected[0]
            .marks
            .iter()
            .filter_map(|slot| marks.marks[*slot].mcid)
            .map(|mcid| (scope, mcid))
            .collect::<BTreeSet<_>>();
        for text in marks
            .text
            .iter()
            .filter(|text| form_ocr.contains(text) && form_ocr.owner_of(text) == source.owner)
        {
            selected_items.extend(
                text.marks
                    .iter()
                    .filter_map(|slot| marks.marks[*slot].mcid)
                    .map(|mcid| (scope, mcid)),
            );
        }
        let mut all_items = BTreeSet::new();
        if let Some(owner) = source.owner {
            for (container, items) in &index.marked {
                all_items.extend(
                    items
                        .iter()
                        .filter(|(_, candidate)| **candidate == owner)
                        .map(|(mcid, _)| (*container, *mcid)),
                );
            }
        }
        if (source.owner.is_some() && selected_items.is_empty()) || selected_items != all_items {
            return Err(fail(
                "nested Figure structure owner contains additional content items",
            ));
        }
        let separate_ocr_owners = form_ocr
            .owners(&source.id)
            .into_iter()
            .filter(|owner| Some(*owner) != source.owner)
            .collect::<BTreeSet<_>>();
        for &owner in &separate_ocr_owners {
            let selected_ocr_items = marks
                .text
                .iter()
                .filter(|text| form_ocr.owner_of(text) == Some(owner))
                .flat_map(|text| {
                    text.marks
                        .iter()
                        .filter_map(|slot| marks.marks[*slot].mcid)
                        .map(|mcid| (scope, mcid))
                })
                .collect::<BTreeSet<_>>();
            let all_ocr_items = index
                .marked
                .iter()
                .flat_map(|(container, items)| {
                    items
                        .iter()
                        .filter(move |(_, candidate)| **candidate == owner)
                        .map(move |(mcid, _)| (*container, *mcid))
                })
                .collect::<BTreeSet<_>>();
            if selected_ocr_items.is_empty() || selected_ocr_items != all_ocr_items {
                return Err(fail(
                    "separate nested OCR owner contains additional content items",
                ));
            }
        }
        for &slot in &selected[0].marks {
            let mark = &marks.marks[slot];
            if mark.actual_text
                || mark.frame.is_some()
                || mark.image_key.is_some()
                || (!mark.safe && !mark.artifact)
                || !(mark.mcid.is_some() || mark.artifact)
            {
                return Err(fail(
                    "nested Figure image has unapproved marked/optional ownership",
                ));
            }
        }
        for paint in &marks.nontext {
            if std::ptr::eq(paint, selected[0]) {
                continue;
            }
            let paint_owners = owners(&paint.marks)?;
            if !paint_owners.is_disjoint(&expected) {
                return Err(fail(
                    "nested Figure owner contains additional painted content",
                ));
            }
            if !paint_owners.is_disjoint(&separate_ocr_owners) {
                return Err(fail("separate nested OCR owner contains painted content"));
            }
        }
        for text in &marks.text {
            if !text.bytes_empty && !form_ocr.contains(text) {
                let text_owners = owners(&text.marks)?;
                if !text_owners.is_disjoint(&expected) {
                    return Err(fail(
                        "nested Figure owner contains text outside its approved OCR selection",
                    ));
                }
                if !text_owners.is_disjoint(&separate_ocr_owners) {
                    return Err(fail(
                        "separate nested OCR owner contains text outside its approved selection",
                    ));
                }
            }
        }
    }
    Ok(())
}

fn rewrite_inbound_ref_value(
    store: &mut Store<'_>,
    index: &StructureIndex,
    value: &PdfObject,
    splits: &BTreeMap<ObjectRef, (ObjectRef, FigureIncomingReferenceSplit)>,
) -> Result<(bool, PdfObject)> {
    let targets = splits.keys().copied().collect::<BTreeSet<_>>();
    if !ref_value_mentions_any(store, value, &targets)? {
        return Ok((false, value.clone()));
    }
    fn rewrite_graph(
        graph: RefGraph,
        splits: &BTreeMap<ObjectRef, (ObjectRef, FigureIncomingReferenceSplit)>,
    ) -> (bool, Vec<RefGraph>) {
        match graph {
            RefGraph::Target(source) => {
                let Some((residual, policy)) = splits.get(&source) else {
                    return (false, vec![RefGraph::Target(source)]);
                };
                let rewritten = match policy {
                    FigureIncomingReferenceSplit::FollowSelected => {
                        vec![RefGraph::Target(source)]
                    }
                    FigureIncomingReferenceSplit::RetargetResidual => {
                        vec![RefGraph::Target(*residual)]
                    }
                    FigureIncomingReferenceSplit::ReferenceBoth => {
                        vec![RefGraph::Target(source), RefGraph::Target(*residual)]
                    }
                };
                (true, rewritten)
            }
            RefGraph::Direct(children) => {
                let mut mentioned = false;
                let mut rewritten = Vec::new();
                for child in children {
                    let (child_mentioned, child_rewritten) = rewrite_graph(child, splits);
                    mentioned |= child_mentioned;
                    rewritten.extend(child_rewritten);
                }
                (mentioned, vec![RefGraph::Direct(rewritten)])
            }
            RefGraph::Indirect { source, children } => {
                let mut mentioned = false;
                let mut rewritten = Vec::new();
                for child in children {
                    let (child_mentioned, child_rewritten) = rewrite_graph(child, splits);
                    mentioned |= child_mentioned;
                    rewritten.extend(child_rewritten);
                }
                (
                    mentioned,
                    vec![RefGraph::Indirect {
                        source,
                        children: rewritten,
                    }],
                )
            }
        }
    }

    // Earlier rewrites in this same atomic transaction can place freshly
    // allocated residual structure elements in a shared /Ref container.  They
    // are governed targets, even though the immutable source index cannot yet
    // contain them.
    let residual_targets = splits
        .values()
        .map(|(residual, _)| *residual)
        .collect::<BTreeSet<_>>();
    let graph = ref_graph_with_additional_targets(store, value, index, &residual_targets)?;
    let (mentioned, mut rewritten) = rewrite_graph(graph, splits);
    if !mentioned {
        return Ok((false, value.clone()));
    }
    let rewritten = if rewritten.len() == 1 {
        rewritten.remove(0)
    } else {
        RefGraph::Direct(rewritten)
    };
    Ok((true, materialize_ref_graph(store, rewritten)?))
}

fn stage_inbound_refs(
    store: &mut Store<'_>,
    index: &StructureIndex,
    splits: &BTreeMap<ObjectRef, (ObjectRef, FigureIncomingReferenceSplit)>,
    residuals: &BTreeSet<ObjectRef>,
    internal_nodes: &BTreeSet<ObjectRef>,
) -> Result<Vec<PdfDictionary>> {
    if splits.is_empty() {
        return Ok(Vec::new());
    }
    let mut nodes = index.nodes.iter().copied().collect::<BTreeSet<_>>();
    nodes.extend(residuals.iter().copied());
    let mut receipts = Vec::new();
    for node in nodes {
        if internal_nodes.contains(&node) {
            continue;
        }
        let mut dictionary = store.dict(node)?;
        let Some(current) = dictionary.get("Ref").cloned() else {
            continue;
        };
        let (mentioned, rewritten) = rewrite_inbound_ref_value(store, index, &current, splits)?;
        if !mentioned {
            continue;
        }
        dictionary.insert("Ref", rewritten.clone());
        store.replace_dict(node, dictionary)?;
        let mut receipt = PdfDictionary::empty();
        receipt.insert("Node", reference(node));
        let graph = ref_graph_with_additional_targets(store, &rewritten, index, residuals)?;
        receipt.insert(
            "RefGraph",
            PdfObject::String(parsed_ref_graph_signature(&graph)),
        );
        receipt.insert("Ref", rewritten);
        receipts.push(receipt);
    }
    Ok(receipts)
}

struct SemanticSplitStage {
    source: ObjectRef,
    residual: ObjectRef,
    descendants: Vec<ObjectRef>,
    clones: BTreeMap<ObjectRef, ObjectRef>,
    relationship_receipts: Vec<PdfDictionary>,
}

pub(super) fn stage(
    store: &mut Store<'_>,
    index: &StructureIndex,
    request: &LinkedStoryRequest,
    selection: &Selection,
    story: &str,
    targets: &mut PdfDictionary,
) -> Result<(Vec<PdfDictionary>, Vec<PdfDictionary>)> {
    if request.figures.is_empty() && request.figure_removals.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let tags = request
        .source_tags
        .as_ref()
        .ok_or_else(|| fail("missing Figure configuration"))?;
    let mut residuals = Vec::new();
    let mut residual_nodes = BTreeSet::new();
    let mut internal_relationship_nodes = BTreeSet::new();
    let mut inbound_splits = BTreeMap::new();
    let mut coordinated_clones = BTreeMap::new();
    let mut semantic_splits = BTreeMap::new();
    for figure in request
        .figures
        .iter()
        .filter(|figure| tags.figures[&figure.id].split_reused_form_semantics)
    {
        let binding = &tags.figures[&figure.id];
        let source = selection.figures[&figure.id].ok_or_else(|| {
            fail("reused Form semantic splitting requires an existing Figure owner")
        })?;
        let descendants = if binding.preserve_semantic_subtree {
            if !binding.clone_semantic_subtree_for_reused_form {
                return Err(fail(
                    "Figure semantic subtree reused-Form splitting requires explicit clone approval",
                ));
            }
            validate_semantic_subtree(store, index, source)?
        } else {
            Vec::new()
        };
        let mut residual_dictionary = store.dict(source)?;
        residual_dictionary.remove("ID");
        residual_dictionary.remove("WFStoryTagKey");
        residual_dictionary.remove("WFStoryID");
        residual_dictionary.insert("P", reference(selection.parent));
        let residual = store.add(PdfObject::Dictionary(residual_dictionary))?;
        let mut clones = if descendants.is_empty() {
            BTreeMap::new()
        } else {
            clone_semantic_subtree(store, index, source, residual, &descendants)?
        };
        clones.insert(source, residual);
        for (&semantic_source, &semantic_clone) in &clones {
            if coordinated_clones
                .insert(semantic_source, semantic_clone)
                .is_some()
            {
                return Err(fail("simultaneously split Figure semantic trees overlap"));
            }
        }
        residual_nodes.extend(clones.values().copied());
        internal_relationship_nodes.extend(clones.keys().copied());
        internal_relationship_nodes.extend(clones.values().copied());
        semantic_splits.insert(
            figure.id.clone(),
            SemanticSplitStage {
                source,
                residual,
                descendants,
                clones,
                relationship_receipts: Vec::new(),
            },
        );
    }
    for figure in request
        .figures
        .iter()
        .filter(|figure| tags.figures[&figure.id].split_reused_form_semantics)
    {
        let binding = &tags.figures[&figure.id];
        let split = semantic_splits
            .get(&figure.id)
            .ok_or_else(|| fail("reused Figure semantic split stage disappeared"))?;
        let receipts = split_semantic_relationships(
            store,
            index,
            &split.clones,
            &coordinated_clones,
            binding.outbound_ref_split,
        )?;
        if let Some(policy) = binding.inbound_ref_split {
            for (&semantic_source, &semantic_clone) in &split.clones {
                if inbound_splits
                    .insert(semantic_source, (semantic_clone, policy))
                    .is_some()
                {
                    return Err(fail("duplicate reused Figure incoming-reference split"));
                }
            }
        }
        semantic_splits
            .get_mut(&figure.id)
            .ok_or_else(|| fail("reused Figure semantic split stage disappeared"))?
            .relationship_receipts = receipts;
    }
    for figure in &request.figures {
        let owner = owner_key(&request.story_id, &figure.id);
        let existing = selection.figures[&figure.id];
        if tags.figures[&figure.id].split_reused_form_semantics {
            let split = semantic_splits
                .get(&figure.id)
                .ok_or_else(|| fail("reused Figure semantic split stage disappeared"))?;
            let source = split.source;
            let residual = split.residual;
            let residual_dictionary = store.dict(residual)?;
            let content = residual_dictionary
                .get("K")
                .cloned()
                .ok_or_else(|| fail("reused Figure residual has no content ownership"))?;
            let selected_reference = store
                .dict(source)?
                .get("Ref")
                .cloned()
                .unwrap_or(PdfObject::Null);
            let residual_reference = residual_dictionary
                .get("Ref")
                .cloned()
                .unwrap_or(PdfObject::Null);
            let mut entry = PdfDictionary::empty();
            entry.insert("Owner", PdfObject::String(owner.as_bytes().to_vec()));
            entry.insert("Node", reference(residual));
            entry.insert("Content", content);
            entry.insert("SelectedRef", selected_reference);
            entry.insert("ResidualRef", residual_reference);
            if !split.descendants.is_empty() {
                entry.insert(
                    "SemanticClones",
                    PdfObject::Array(
                        split
                            .descendants
                            .iter()
                            .map(|source| reference(split.clones[source]))
                            .collect(),
                    ),
                );
            }
            entry.insert(
                "SemanticRefSplits",
                PdfObject::Array(
                    split
                        .relationship_receipts
                        .iter()
                        .cloned()
                        .map(PdfObject::Dictionary)
                        .collect(),
                ),
            );
            let ocr_sources = selection
                .figure_ocr_owners
                .get(&figure.id)
                .into_iter()
                .flat_map(|owners| owners.values().copied())
                .collect::<BTreeSet<_>>();
            let mut ocr_residuals = Vec::new();
            for source in selection
                .selected
                .iter()
                .copied()
                .filter(|source| ocr_sources.contains(source))
            {
                let mut ocr_residual = store.dict(source)?;
                ocr_residual.remove("ID");
                ocr_residual.remove("WFStoryTagKey");
                ocr_residual.remove("WFStoryID");
                ocr_residual.insert("P", reference(selection.parent));
                let content = ocr_residual
                    .get("K")
                    .cloned()
                    .ok_or_else(|| fail("reused OCR residual has no content ownership"))?;
                let residual = store.add(PdfObject::Dictionary(ocr_residual))?;
                let mut receipt = PdfDictionary::empty();
                receipt.insert("Node", reference(residual));
                receipt.insert("Content", content);
                ocr_residuals.push(PdfObject::Dictionary(receipt));
            }
            if ocr_residuals.len() != ocr_sources.len() {
                return Err(fail("reused OCR residual source order changed"));
            }
            if !ocr_residuals.is_empty() {
                entry.insert("OcrResiduals", PdfObject::Array(ocr_residuals));
            }
            residuals.push(entry);
        }
        let node = if let Some(node) = existing {
            node
        } else {
            let mut dict = PdfDictionary::empty();
            dict.insert("Type", PdfObject::Name("StructElem".into()));
            dict.insert("S", PdfObject::Name("Figure".into()));
            dict.insert("P", reference(selection.parent));
            store.add(PdfObject::Dictionary(dict))?
        };
        let mut dict = store.dict(node)?;
        dict.insert(
            "WFStoryTagKey",
            PdfObject::String(owner.as_bytes().to_vec()),
        );
        dict.insert("WFStoryID", PdfObject::String(story.as_bytes().to_vec()));
        store.replace_dict(node, dict)?;
        if targets.contains_key(&owner) {
            return Err(fail("Figure/paragraph owner collision"));
        }
        targets.insert(owner, reference(node));
    }
    let inbound = stage_inbound_refs(
        store,
        index,
        &inbound_splits,
        &residual_nodes,
        &internal_relationship_nodes,
    )?;
    for entry in &mut residuals {
        let owner = entry
            .get("Owner")
            .and_then(PdfObject::as_string)
            .ok_or_else(|| fail("reused Figure residual owner disappeared"))?;
        let owner = std::str::from_utf8(owner)
            .map_err(|_| fail("reused Figure residual owner encoding changed"))?
            .to_owned();
        let target = targets
            .get_reference(&owner)
            .ok_or_else(|| fail("reused Figure moved owner disappeared"))?;
        let residual = entry
            .get_reference("Node")
            .ok_or_else(|| fail("reused Figure residual node disappeared"))?;
        let selected_dictionary = store.dict(target)?;
        let residual_dictionary = store.dict(residual)?;
        entry.insert(
            "SelectedRef",
            selected_dictionary
                .get("Ref")
                .cloned()
                .unwrap_or(PdfObject::Null),
        );
        entry.insert(
            "ResidualRef",
            residual_dictionary
                .get("Ref")
                .cloned()
                .unwrap_or(PdfObject::Null),
        );
    }
    Ok((residuals, inbound))
}

pub(super) fn residuals(
    store: &Store<'_>,
    root: ObjectRef,
    index: &StructureIndex,
    request: &LinkedStoryRequest,
    transaction: &PdfDictionary,
    targets: &PdfDictionary,
    parent: ObjectRef,
) -> Result<Vec<ObjectRef>> {
    let actual = transaction
        .get("FigureResiduals")
        .and_then(PdfObject::as_array)
        .unwrap_or_default();
    let governed_residual_nodes = actual
        .iter()
        .filter_map(PdfObject::as_dict)
        .flat_map(|entry| {
            entry
                .get_reference("Node")
                .into_iter()
                .chain(
                    entry
                        .get("SemanticClones")
                        .and_then(PdfObject::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(PdfObject::as_reference),
                )
                .chain(
                    entry
                        .get("OcrResiduals")
                        .and_then(PdfObject::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(PdfObject::as_dict)
                        .filter_map(|receipt| receipt.get_reference("Node")),
                )
        })
        .collect::<BTreeSet<_>>();
    let expected = request
        .figures
        .iter()
        .filter(|figure| {
            request
                .source_tags
                .as_ref()
                .is_some_and(|tags| tags.figures[&figure.id].split_reused_form_semantics)
        })
        .collect::<Vec<_>>();
    if actual.len() != expected.len() {
        return Err(fail("reused Figure residual count changed"));
    }
    let inbound = transaction
        .get("FigureInboundRefs")
        .and_then(PdfObject::as_array)
        .ok_or_else(|| fail("missing reused Figure incoming-reference receipt"))?;
    let expects_inbound = expected.iter().any(|figure| {
        request
            .source_tags
            .as_ref()
            .and_then(|tags| tags.figures.get(&figure.id))
            .and_then(|binding| binding.inbound_ref_split)
            .is_some()
    });
    if expects_inbound == inbound.is_empty() {
        return Err(fail(
            "reused Figure incoming-reference receipt count changed",
        ));
    }
    let mut inbound_nodes = BTreeSet::new();
    for receipt in inbound {
        let receipt = receipt
            .as_dict()
            .ok_or_else(|| fail("invalid reused Figure incoming-reference receipt"))?;
        let node = receipt
            .get_reference("Node")
            .ok_or_else(|| fail("reused Figure incoming referrer missing"))?;
        let dictionary = store.dict(node)?;
        let current_ref = dictionary
            .get("Ref")
            .ok_or_else(|| fail("reused Figure incoming relationship disappeared"))?;
        let current_graph = PdfObject::String(parsed_ref_graph_signature(
            &ref_graph_with_additional_targets(
                store,
                current_ref,
                index,
                &governed_residual_nodes,
            )?,
        ));
        if !inbound_nodes.insert(node)
            || dictionary.get("S").is_none()
            || receipt.get("Ref") != Some(current_ref)
            || receipt.get("RefGraph") != Some(&current_graph)
        {
            return Err(fail("reused Figure incoming relationship changed"));
        }
    }
    let mut seen = BTreeSet::new();
    let mut result = Vec::with_capacity(actual.len());
    for (entry, figure) in actual.iter().zip(expected) {
        let entry = entry
            .as_dict()
            .ok_or_else(|| fail("invalid reused Figure residual entry"))?;
        let owner = owner_key(&request.story_id, &figure.id);
        if !matches!(entry.get("Owner"), Some(PdfObject::String(value)) if value == owner.as_bytes())
        {
            return Err(fail("reused Figure residual owner changed"));
        }
        let node = entry
            .get_reference("Node")
            .ok_or_else(|| fail("reused Figure residual node missing"))?;
        if targets.get_reference(&owner) == Some(node) || !seen.insert(node) {
            return Err(fail("reused Figure residual aliases another owner"));
        }
        let dictionary = store.dict(node)?;
        let target = targets
            .get_reference(&owner)
            .ok_or_else(|| fail("reused Figure moved owner missing"))?;
        let target_dictionary = store.dict(target)?;
        let residual_reference = dictionary.get("Ref").cloned().unwrap_or(PdfObject::Null);
        let selected_reference = target_dictionary
            .get("Ref")
            .cloned()
            .unwrap_or(PdfObject::Null);
        if !figure_role(store, root, node)?
            || dictionary.get_reference("P") != Some(parent)
            || dictionary.contains_key("ID")
            || dictionary.contains_key("WFStoryTagKey")
            || dictionary.contains_key("WFStoryID")
            || kids(&dictionary).is_empty()
            || entry.get("Content") != dictionary.get("K")
            || entry.get("ResidualRef") != Some(&residual_reference)
            || entry.get("SelectedRef") != Some(&selected_reference)
        {
            return Err(fail("reused Figure residual structure changed"));
        }
        let binding = &request
            .source_tags
            .as_ref()
            .ok_or_else(|| fail("missing Figure residual tag configuration"))?
            .figures[&figure.id];
        let clone_receipt = entry
            .get("SemanticClones")
            .and_then(PdfObject::as_array)
            .unwrap_or_default();
        let mut expected_selected_nodes = BTreeSet::from([target]);
        let mut expected_residual_nodes = BTreeSet::from([node]);
        if binding.clone_semantic_subtree_for_reused_form {
            let expected_clones = clone_receipt
                .iter()
                .map(PdfObject::as_reference)
                .collect::<Option<BTreeSet<_>>>()
                .ok_or_else(|| fail("invalid reused Figure semantic clone receipt"))?;
            let mut residual_domain = expected_clones.clone();
            residual_domain.insert(node);
            let descendants = validate_semantic_subtree_with_additional_nodes(
                store,
                index,
                node,
                &residual_domain,
            )?;
            let selected_descendants = validate_semantic_subtree(store, index, target)?;
            if expected_clones.len() != clone_receipt.len()
                || descendants.iter().copied().collect::<BTreeSet<_>>() != expected_clones
            {
                return Err(fail("reused Figure semantic clone set changed"));
            }
            expected_selected_nodes.extend(selected_descendants);
            expected_residual_nodes.extend(descendants.iter().copied());
            for clone in descendants {
                let dictionary = store.dict(clone)?;
                if dictionary.contains_key("ID")
                    || dictionary.contains_key("Pg")
                    || dictionary.contains_key("WFStoryTagKey")
                    || dictionary.contains_key("WFStoryID")
                {
                    return Err(fail("reused Figure semantic clone retained unique state"));
                }
            }
        } else if !clone_receipt.is_empty() {
            return Err(fail(
                "unexpected reused Figure semantic subtree clone receipt",
            ));
        }
        let relationship_receipts = entry
            .get("SemanticRefSplits")
            .and_then(PdfObject::as_array)
            .ok_or_else(|| fail("missing reused Figure semantic relationship receipts"))?;
        if relationship_receipts.len() != expected_selected_nodes.len() {
            return Err(fail(
                "reused Figure semantic relationship receipt count changed",
            ));
        }
        let mut receipt_sources = BTreeSet::new();
        let mut receipt_clones = BTreeSet::new();
        for relationship in relationship_receipts {
            let relationship = relationship
                .as_dict()
                .ok_or_else(|| fail("invalid reused Figure semantic relationship receipt"))?;
            let source = relationship
                .get_reference("Source")
                .ok_or_else(|| fail("reused Figure semantic relationship source missing"))?;
            let clone = relationship
                .get_reference("Clone")
                .ok_or_else(|| fail("reused Figure semantic relationship clone missing"))?;
            let selected_dictionary = store.dict(source)?;
            let clone_dictionary = store.dict(clone)?;
            let selected_ref = selected_dictionary
                .get("Ref")
                .cloned()
                .unwrap_or(PdfObject::Null);
            let clone_ref = clone_dictionary
                .get("Ref")
                .cloned()
                .unwrap_or(PdfObject::Null);
            let selected_graph = optional_ref_graph_signature_with_additional_targets(
                store,
                selected_dictionary.get("Ref"),
                index,
                &governed_residual_nodes,
            )?;
            let clone_graph = optional_ref_graph_signature_with_additional_targets(
                store,
                clone_dictionary.get("Ref"),
                index,
                &governed_residual_nodes,
            )?;
            if !receipt_sources.insert(source)
                || !receipt_clones.insert(clone)
                || relationship.get("SelectedRef") != Some(&selected_ref)
                || relationship.get("CloneRef") != Some(&clone_ref)
                || relationship.get("SelectedRefGraph") != Some(&selected_graph)
                || relationship.get("CloneRefGraph") != Some(&clone_graph)
            {
                return Err(fail("reused Figure semantic relationship split changed"));
            }
        }
        if receipt_sources != expected_selected_nodes || receipt_clones != expected_residual_nodes {
            return Err(fail(
                "reused Figure semantic relationship ownership changed",
            ));
        }
        result.push(node);
        let expected_ocr = request
            .source_tags
            .as_ref()
            .and_then(|tags| tags.figures.get(&figure.id))
            .map(ocr_owner_bindings)
            .transpose()?
            .map_or(0, |owners| owners.len());
        let ocr_residuals = entry
            .get("OcrResiduals")
            .and_then(PdfObject::as_array)
            .unwrap_or_default();
        if expected_ocr != ocr_residuals.len() {
            return Err(fail("reused OCR residual receipt count changed"));
        }
        for receipt in ocr_residuals {
            let receipt = receipt
                .as_dict()
                .ok_or_else(|| fail("invalid reused OCR residual receipt"))?;
            let ocr_node = receipt
                .get_reference("Node")
                .ok_or_else(|| fail("reused OCR residual node missing"))?;
            let dictionary = store.dict(ocr_node)?;
            let role = mapped_role(store, root, ocr_node)?;
            if !matches!(role.as_str(), "P" | "Span")
                || dictionary.get_reference("P") != Some(parent)
                || dictionary.contains_key("ID")
                || dictionary.contains_key("WFStoryTagKey")
                || dictionary.contains_key("WFStoryID")
                || kids(&dictionary).is_empty()
                || receipt.get("Content") != dictionary.get("K")
                || dictionary
                    .iter()
                    .any(|(name, _)| !matches!(name.as_str(), "Type" | "S" | "P" | "K" | "Pg"))
                || !seen.insert(ocr_node)
            {
                return Err(fail("reused OCR residual structure changed"));
            }
            result.push(ocr_node);
        }
    }
    Ok(result)
}

pub(crate) fn wrap_paint(
    request: &LinkedStoryRequest,
    id: &str,
    paint: Vec<u8>,
) -> Result<Vec<u8>> {
    let Some(tags) = &request.source_tags else {
        return Ok(paint);
    };
    if !tags.figures.contains_key(id) {
        return Err(fail("missing generated Figure semantic owner"));
    }
    let mut bytes = format!(
        "/Figure << /WFStoryFigure ({}) >> BDC\n",
        owner_key(&request.story_id, id)
    )
    .into_bytes();
    bytes.extend(paint);
    bytes.extend_from_slice(b"\nEMC\n");
    Ok(bytes)
}

pub(super) fn expected(
    request: &LinkedStoryRequest,
    preview: &LinkedStoryPreview,
) -> Result<BTreeMap<String, (usize, String)>> {
    let mut expected = BTreeMap::new();
    for frame in &preview.frames {
        for placement in &frame.figures {
            let figure = request
                .figures
                .iter()
                .find(|f| f.id == placement.figure_id)
                .ok_or_else(|| fail("Figure preview owner missing"))?;
            let native =
                crate::image_fragments::stories::source_key(&request.input_sha256, &figure.source);
            if expected
                .insert(
                    owner_key(&request.story_id, &figure.id),
                    (frame.frame.page, native),
                )
                .is_some()
            {
                return Err(fail("duplicated Figure preview owner"));
            }
        }
    }
    if expected.len() != request.figures.len() {
        return Err(fail("Figure preview omitted an owner"));
    }
    Ok(expected)
}

pub(super) fn stamp_page(
    page: usize,
    page_id: ObjectRef,
    marks: &PageMarks,
    expected: &BTreeMap<String, (usize, String)>,
    observed: &mut BTreeMap<String, PdfObject>,
    used: &mut BTreeSet<usize>,
    next: &mut usize,
    patches: &mut StreamPatches,
) -> Result<()> {
    for (slot, mark) in marks.marks.iter().enumerate() {
        let Some(owner) = mark
            .figure
            .as_ref()
            .filter(|key| expected.contains_key(*key))
        else {
            continue;
        };
        let (target, native) = &expected[owner];
        if *target != page
            || mark.mcid.is_some()
            || mark.actual_text
            || mark.enclosing_frame.is_some()
            || observed.contains_key(owner)
        {
            return Err(fail(
                "generated Figure marker location/ownership is invalid",
            ));
        }
        let paint = marks
            .nontext
            .iter()
            .filter(|p| p.marks.contains(&slot))
            .collect::<Vec<_>>();
        if paint.len() != 1
            || !paint[0].image
            || !paint[0]
                .marks
                .iter()
                .any(|i| marks.marks[*i].image_key.as_ref() == Some(native))
            || marks.text.iter().any(|p| p.marks.contains(&slot))
        {
            return Err(fail(
                "generated Figure marker does not own exactly its approved image",
            ));
        }
        while used.contains(next) {
            *next += 1;
        }
        if *next > MAX_MCID {
            return Err(fail("Figure page MCID capacity exceeded"));
        }
        let mcid = *next;
        used.insert(mcid);
        let end = mark
            .dictionary_end
            .ok_or_else(|| fail("generated Figure needs direct properties"))?;
        patches.entry(mark.stream).or_default().push((
            end,
            end,
            format!(" /MCID {mcid} ").into_bytes(),
        ));
        let mut mcr = PdfDictionary::empty();
        mcr.insert("Type", PdfObject::Name("MCR".into()));
        mcr.insert("Pg", reference(page_id));
        mcr.insert("MCID", PdfObject::Integer(mcid as i64));
        observed.insert(owner.clone(), PdfObject::Dictionary(mcr));
    }
    Ok(())
}

fn replace_figure_content(
    store: &Store<'_>,
    index: &StructureIndex,
    dictionary: &PdfDictionary,
    mcr: PdfObject,
    preserve_subtree: bool,
) -> Result<PdfObject> {
    if !preserve_subtree {
        return Ok(PdfObject::Array(vec![mcr]));
    }
    let mut replacement = Vec::new();
    let mut inserted = false;
    let mut descendants = 0usize;
    for value in kids(dictionary) {
        if structure_child(store, index, &value)?.is_some() {
            descendants += 1;
            replacement.push(value);
        } else if !matches!(value, PdfObject::Null) {
            if !content_leaf_value(store, index, &value)? {
                return Err(fail("Figure semantic subtree source content changed"));
            }
            if !inserted {
                replacement.push(mcr.clone());
                inserted = true;
            }
        }
    }
    if !inserted || descendants == 0 {
        return Err(fail(
            "Figure semantic subtree source shape changed before publication",
        ));
    }
    Ok(PdfObject::Array(replacement))
}

pub(super) fn finish(
    store: &mut Store<'_>,
    index: &StructureIndex,
    request: &LinkedStoryRequest,
    targets: &PdfDictionary,
    locations: &mut BTreeMap<String, PdfObject>,
    resolver: &attributes::Resolver,
    budget: &mut attributes::Budget,
    preview: &LinkedStoryPreview,
) -> Result<BTreeMap<String, FigureTagBinding>> {
    if request.figures.is_empty() && request.figure_removals.is_empty() {
        return Ok(BTreeMap::new());
    }
    let tags = request
        .source_tags
        .as_ref()
        .ok_or_else(|| fail("missing Figure configuration"))?;
    let mut rebound = BTreeMap::new();
    let placements = preview
        .frames
        .iter()
        .flat_map(|frame| frame.figures.iter())
        .map(|p| (p.figure_id.as_str(), p.rect))
        .collect::<BTreeMap<_, _>>();
    for figure in &request.figures {
        let key = owner_key(&request.story_id, &figure.id);
        let node = targets
            .get_reference(&key)
            .ok_or_else(|| fail("missing generated Figure owner"))?;
        let mcr = locations
            .remove(&key)
            .ok_or_else(|| fail("missing generated Figure MCID"))?;
        if tags.figures[&figure.id].preserve_semantic_subtree {
            let descendants = validate_semantic_subtree(store, index, node)?;
            let page = mcr
                .as_dict()
                .and_then(|dictionary| dictionary.get_reference("Pg"))
                .ok_or_else(|| fail("generated Figure MCR has no destination page"))?;
            rebind_semantic_subtree_pages(store, &descendants, page)?;
        }
        let mut dict = store.dict(node)?;
        let content = replace_figure_content(
            store,
            index,
            &dict,
            mcr,
            tags.figures[&figure.id].preserve_semantic_subtree,
        )?;
        dict.insert("K", content);
        dict.remove("Pg");
        if let Some(text) = &tags.figures[&figure.id].semantic_text {
            for (name, value) in [("Alt", &text.alternate), ("E", &text.expansion)] {
                if let Some(value) = value {
                    dict.insert(name, logical_string(value));
                } else {
                    dict.remove(name);
                }
            }
        }
        let rect = placements
            .get(figure.id.as_str())
            .ok_or_else(|| fail("Figure geometry is missing"))?;
        let mut layout = PdfDictionary::empty();
        layout.insert("O", PdfObject::Name("Layout".into()));
        layout.insert(
            "BBox",
            PdfObject::Array(rect.iter().copied().map(PdfObject::Real).collect()),
        );
        layout.insert("Width", PdfObject::Real(rect[2] - rect[0]));
        layout.insert("Height", PdfObject::Real(rect[3] - rect[1]));
        layout.insert("Placement", PdfObject::Name("Block".into()));
        resolver.rewrite(store, &mut dict, Some(layout), budget)?;
        store.replace_dict(node, dict)?;
        rebound.insert(
            figure.id.clone(),
            FigureTagBinding {
                source: Some(TagReference {
                    object: node.0,
                    generation: node.1,
                    key: Some(key),
                }),
                semantic_text: None,
                split_reused_form_semantics: false,
                outbound_ref_split: None,
                inbound_ref_split: None,
                preserve_semantic_subtree: tags.figures[&figure.id].preserve_semantic_subtree,
                delete_semantic_subtree: false,
                clone_semantic_subtree_for_reused_form: false,
                separate_ocr_owner: None,
                separate_ocr_owners: Vec::new(),
            },
        );
    }
    Ok(rebound)
}
