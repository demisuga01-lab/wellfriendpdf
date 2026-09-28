//! Exact-occurrence materialization for annotations stored directly in Annots.
//! The input revision and slot, never dictionary equality alone, select a source.
use crate::annotation_identity::{self, IdentityIndex};
use crate::annotation_relationships::Graph;
use crate::writer::{write_incremental_update, IncrementalObject};
use crate::{PdfDictionary, PdfDocument, PdfObject, PdfReader, Result, WellfriendError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

type Ref = (u32, u16);

/// Read-only projection of the same strict field ownership graph used during
/// direct-owner materialization. Consumers must not infer ownership by /T.
#[derive(Clone)]
pub(crate) struct ReachableFieldNode {
    pub path: Vec<usize>,
    pub reference: Option<Ref>,
    pub dictionary: PdfDictionary,
    pub widget: bool,
}

pub(crate) fn reachable_field_nodes(document: &PdfDocument) -> Result<Vec<ReachableFieldNode>> {
    owners::reachable_field_nodes(document)
}
#[cfg(test)]
#[path = "annotation_promotion_owner_tests.rs"]
mod owner_tests;
#[path = "annotation_promotion_owners.rs"]
mod owners;
#[cfg(test)]
#[path = "annotation_promotion_tests.rs"]
mod tests;
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnnotationPromotionReport {
    pub input_sha256: String,
    pub output_sha256: String,
    /// Includes relationship neighbors whose identity must survive staging.
    pub persisted_ids: Vec<String>,
    pub promoted_ids: Vec<String>,
    pub changed_pages: Vec<usize>,
    pub source_order_verified: bool,
    pub relationship_graph_verified: bool,
    #[serde(default)]
    pub reused_owner_ids: Vec<String>,
    #[serde(default)]
    pub field_ownership_verified: bool,
    #[serde(default)]
    pub tagged_ownership_verified: bool,
    #[serde(default)]
    pub materialized_field_nodes: usize,
    #[serde(default)]
    pub repaired_field_parents: usize,
    #[serde(default)]
    pub dependent_widget_ids: Vec<String>,
    #[serde(default)]
    pub materialized_structure_nodes: usize,
    #[serde(default)]
    pub repaired_structure_parents: usize,
    #[serde(default)]
    pub normalized_structure_links: usize,
    #[serde(default)]
    pub dependent_tagged_annotation_ids: Vec<String>,
}
fn fail(message: &str) -> WellfriendError {
    WellfriendError::invalid_input(message)
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn reference(id: Ref) -> PdfObject {
    PdfObject::Reference {
        number: id.0,
        generation: id.1,
    }
}
fn annots(reader: &PdfReader, page: Ref) -> Result<(PdfDictionary, Vec<PdfObject>)> {
    let object = reader.get_object(page.0, page.1)?;
    let dict = object
        .as_dict()
        .ok_or_else(|| fail("invalid annotation page"))?
        .clone();
    let values = match dict.get("Annots") {
        Some(value) => reader
            .resolve(value.clone())?
            .as_array()
            .ok_or_else(|| fail("invalid annotation array"))?
            .to_vec(),
        None => Vec::new(),
    };
    Ok((dict, values))
}
fn object_hash(object: &PdfObject, budget: &mut usize) -> Result<String> {
    let mut bytes = Vec::new();
    crate::writer::serialize_object(object, &mut bytes);
    *budget = budget.saturating_add(bytes.len());
    if *budget > 64 * 1024 * 1024 {
        return Err(fail("annotation promotion dictionary budget exceeded"));
    }
    Ok(digest(&bytes))
}

/// Standalone source normalization, also used privately inside geometry/XFDF
/// transactions. This does not sanitize actions, flatten appearances, or remove
/// historical revisions. A source hash is mandatory for externally selected IDs.
pub fn promote_annotation_sources_pdf(
    input: &[u8],
    source_sha256: &str,
    annotation_ids: &[String],
) -> Result<(Vec<u8>, AnnotationPromotionReport)> {
    if !source_sha256.eq_ignore_ascii_case(&digest(input)) {
        return Err(fail("annotation promotion source revision changed"));
    }
    let selected = annotation_ids.iter().cloned().collect::<BTreeSet<_>>();
    if selected.is_empty() || selected.len() != annotation_ids.len() || selected.len() > 4096 {
        return Err(fail(
            "annotation promotion requires 1..4096 distinct source IDs",
        ));
    }
    stage(input, input, &selected, 100_000)
}

/// Already canonical indirect sources need no extra revision. Direct page
/// occurrences, direct field copies and direct field ancestors are normalized
/// with all original request/dependency identities bound before changing bytes.
pub(crate) fn prepare_if_direct(
    input: &[u8],
    ids: &IdentityIndex,
    selected: &BTreeSet<String>,
    limit: usize,
) -> Result<Option<(Vec<u8>, AnnotationPromotionReport)>> {
    if selected.is_empty() {
        return Ok(None);
    }
    let source = PdfDocument::open_bytes(input.to_vec())?;
    let graph = Graph::read(&source, ids)?;
    let components = component_ids(&graph, selected)?;
    let has_direct = components
        .iter()
        .any(|id| graph.nodes[id].reference.is_none());
    let mut owners = owners::Owners::new(&source, &source, ids);
    if !has_direct {
        let _ = expand_ownership_dependencies(&graph, &mut owners, &components)?;
    }
    if has_direct || owners.needs_fields() || owners.needs_structures() {
        stage(input, input, selected, limit).map(Some)
    } else {
        Ok(None)
    }
}

/// Materialize selected components before another writer renumbers objects.
/// An intermediate image transaction may have changed unrelated objects, but
/// selected dictionaries and the page/Annots slot must still match the input.
pub(crate) fn stage(
    input: &[u8],
    staged: &[u8],
    requested: &BTreeSet<String>,
    limit: usize,
) -> Result<(Vec<u8>, AnnotationPromotionReport)> {
    let source = PdfDocument::open_bytes(input.to_vec())?;
    let current = PdfDocument::open_bytes(staged.to_vec())?;
    if source.reader().root_reference() != current.reader().root_reference() {
        return Err(fail(
            "annotation catalog identity changed before source materialization",
        ));
    }
    let ids = annotation_identity::index(&source, limit)?;
    let graph = Graph::read(&source, &ids)?;
    let mut owners = owners::Owners::new(&source, &current, &ids);
    let selected = expand_ownership_dependencies(&graph, &mut owners, requested)?;
    let locations = ids
        .iter()
        .map(|(slot, id)| (id.id.clone(), *slot))
        .collect::<BTreeMap<_, _>>();
    let source_pages = source.get_pages()?;
    let pages = current.get_pages()?;
    if source_pages.len() != pages.len()
        || source_pages.iter().zip(&pages).any(|(a, b)| {
            (a.object_number, a.generation_number) != (b.object_number, b.generation_number)
        })
    {
        return Err(fail(
            "annotation pages changed before source materialization",
        ));
    }
    let mut page_updates = BTreeMap::<usize, (PdfDictionary, Vec<PdfObject>)>::new();
    let mut source_arrays = BTreeMap::<usize, Vec<PdfObject>>::new();
    let mut objects = BTreeMap::<Ref, PdfObject>::new();
    let mut expected_refs = BTreeMap::<String, Ref>::new();
    // Size includes free slots omitted by object_ids(). Never accidentally
    // reuse a previously freed number with generation zero.
    let declared_size = current
        .reader()
        .trailer()
        .get_integer("Size")
        .map(u32::try_from)
        .transpose()
        .map_err(|_| fail("invalid annotation xref Size"))?
        .unwrap_or(1);
    let mut next = current
        .reader()
        .object_ids()
        .iter()
        .map(|r| r.0)
        .max()
        .unwrap_or(0)
        .max(declared_size.saturating_sub(1));
    let mut promoted_ids = Vec::new();
    let mut reused_owner_ids = Vec::new();
    let mut changed_pages = BTreeSet::new();
    let mut source_budget = 0usize;
    for id in &selected {
        crate::cancel::check_current_cancel("annotation occurrence materialization")?;
        let &(page, order) = locations
            .get(id)
            .ok_or_else(|| fail("annotation source disappeared"))?;
        let page_object = &pages[page - 1];
        let page_ref = (page_object.object_number, page_object.generation_number);
        if owners.dependent_widgets().contains(id) || owners.structure_dependencies().contains(id) {
            changed_pages.insert(page);
        }
        if let std::collections::btree_map::Entry::Vacant(e) = page_updates.entry(page) {
            e.insert(annots(current.reader(), page_ref)?);
            source_arrays.insert(page, annots(source.reader(), page_ref)?.1);
        }
        let original = source_arrays[&page]
            .get(order)
            .ok_or_else(|| fail("annotation source slot disappeared"))?;
        let (_, values) = page_updates.get_mut(&page).unwrap();
        let actual = values
            .get(order)
            .ok_or_else(|| fail("staged annotation slot disappeared"))?;
        if actual != original {
            return Err(fail(
                "annotation source occurrence changed before materialization",
            ));
        }
        let source_object = source.reader().resolve(original.clone())?;
        let actual_object = current.reader().resolve(actual.clone())?;
        if source_object != actual_object {
            return Err(fail("annotation dictionary changed before materialization"));
        }
        object_hash(&actual_object, &mut source_budget)?;
        let dict = actual_object
            .as_dict()
            .ok_or_else(|| fail("annotation source is not a dictionary"))?;
        let mut stamped = annotation_identity::stamp(current.reader(), dict, id)?;
        let output_ref = if let Some(r) = original.as_reference() {
            if let Some(key) = owners.install_existing(id, r, &mut objects)? {
                if dict.get_integer("StructParent") != Some(key) {
                    stamped
                        .get_or_insert_with(|| dict.clone())
                        .insert("StructParent", PdfObject::Integer(key));
                }
            }
            r
        } else {
            if dict
                .get("Type")
                .map(|v| current.reader().resolve(v.clone()))
                .transpose()?
                .is_some_and(|v| !matches!(v, PdfObject::Null) && v.as_name() != Some("Annot"))
            {
                return Err(fail("direct annotation Type is not Annot"));
            }
            if let Some(owner) = dict.get("P") {
                if !matches!(current.reader().resolve(owner.clone())?, PdfObject::Null)
                    && owner.as_reference() != Some(page_ref)
                {
                    return Err(fail(
                        "direct annotation P disagrees with its selected page occurrence",
                    ));
                }
            }
            let binding = owners.bind(id, dict)?;
            // The owner index consumes scalar keys. Resolving a selected
            // indirect integer preserves its value, not a stale duplicate key.
            if let Some(key) = binding.parent_key {
                if dict.get_integer("StructParent") != Some(key) {
                    stamped
                        .get_or_insert_with(|| dict.clone())
                        .insert("StructParent", PdfObject::Integer(key));
                }
            }
            let r = if let Some(r) = binding.existing {
                reused_owner_ids.push(id.clone());
                r
            } else {
                next = next
                    .checked_add(1)
                    .ok_or_else(|| fail("annotation object number exhausted"))?;
                (next, 0)
            };
            owners.install(&binding, r, &mut objects)?;
            values[order] = reference(r);
            promoted_ids.push(id.clone());
            r
        };
        expected_refs.insert(id.clone(), output_ref);
        if let Some(dict) = stamped {
            objects.insert(output_ref, PdfObject::Dictionary(dict));
            changed_pages.insert(page);
        } else if original.as_reference().is_none() {
            objects.insert(output_ref, actual_object);
            changed_pages.insert(page);
        }
    }
    let (materialized_field_nodes, repaired_field_parents) =
        owners.finish_fields(&mut objects, &mut next, &expected_refs)?;
    let (materialized_structure_nodes, repaired_structure_parents, normalized_structure_links) =
        owners.finish_structures(&mut objects, &mut next)?;
    for (page, (mut dict, values)) in page_updates {
        if values != source_arrays[&page] {
            dict.insert("Annots", PdfObject::Array(values));
            let p = &pages[page - 1];
            objects.insert(
                (p.object_number, p.generation_number),
                PdfObject::Dictionary(dict),
            );
        }
    }
    let mut budget = 0usize;
    let expected = objects
        .iter()
        .map(|(r, o)| Ok((*r, object_hash(o, &mut budget)?)))
        .collect::<Result<BTreeMap<_, _>>>()?;
    let output = write_incremental_update(
        current.reader(),
        objects
            .into_iter()
            .map(|(r, object)| IncrementalObject {
                number: r.0,
                generation: r.1,
                object,
            })
            .collect(),
    )?;
    let reopened = PdfDocument::open_bytes(output.clone())?;
    let result_ids = annotation_identity::index(&reopened, limit)?;
    // Validate locations, not discovery order of generated/hash-based IDs.
    verify_occurrences(&ids, &result_ids, &selected, &expected_refs)?;
    let result_graph = Graph::read(&reopened, &result_ids)?;
    let renames = ids
        .iter()
        .map(|(slot, id)| (id.id.clone(), result_ids[slot].id.clone()))
        .collect::<BTreeMap<_, _>>();
    for (id, node) in &graph.nodes {
        crate::cancel::check_current_cancel("annotation promotion relationship verification")?;
        let mut expected = node.clone();
        expected.reference = expected_refs.get(id).copied().or(node.reference);
        for target in [
            &mut expected.reply_to,
            &mut expected.popup,
            &mut expected.parent,
        ]
        .into_iter()
        .flatten()
        {
            *target = renames
                .get(target)
                .ok_or_else(|| fail("annotation target disappeared"))?
                .clone();
        }
        if result_graph.nodes.get(&renames[id]) != Some(&expected) {
            return Err(fail("annotation promotion changed relationship ownership"));
        }
    }
    budget = 0;
    for (r, hash) in expected {
        if object_hash(&reopened.reader().get_object(r.0, r.1)?, &mut budget)? != hash {
            return Err(fail(
                "annotation materialization dictionary postcondition failed",
            ));
        }
    }
    let (field_ownership_verified, tagged_ownership_verified) = owners.verify(&output)?;
    let report = AnnotationPromotionReport {
        input_sha256: digest(input),
        output_sha256: digest(&output),
        persisted_ids: selected.into_iter().collect(),
        promoted_ids,
        changed_pages: changed_pages.into_iter().collect(),
        source_order_verified: true,
        relationship_graph_verified: true,
        reused_owner_ids,
        field_ownership_verified,
        tagged_ownership_verified,
        materialized_field_nodes,
        repaired_field_parents,
        dependent_widget_ids: owners.dependent_widgets().iter().cloned().collect(),
        materialized_structure_nodes,
        repaired_structure_parents,
        normalized_structure_links,
        dependent_tagged_annotation_ids: owners.structure_dependencies().iter().cloned().collect(),
    };
    Ok((output, report))
}

fn expand_ownership_dependencies(
    graph: &Graph,
    owners: &mut owners::Owners<'_>,
    requested: &BTreeSet<String>,
) -> Result<BTreeSet<String>> {
    let mut selected = component_ids(graph, requested)?;
    loop {
        crate::cancel::check_current_cancel("annotation ownership dependency fixed point")?;
        let previous = selected.len();
        let field_dependencies = owners.expand_fields(&selected)?;
        selected.extend(field_dependencies);
        selected = component_ids(graph, &selected)?;
        let structure_dependencies = owners.expand_structures(&selected)?;
        selected.extend(structure_dependencies);
        selected = component_ids(graph, &selected)?;
        if selected.len() == previous {
            return Ok(selected);
        }
        if selected.len() > graph.nodes.len() {
            return Err(fail(
                "annotation dependency closure exceeded source inventory",
            ));
        }
    }
}

pub(crate) fn component_ids(
    graph: &Graph,
    requested: &BTreeSet<String>,
) -> Result<BTreeSet<String>> {
    let mut edges = BTreeMap::<&str, BTreeSet<&str>>::new();
    for (id, node) in &graph.nodes {
        crate::cancel::check_current_cancel("annotation promotion dependency inventory")?;
        for target in node.targets() {
            edges.entry(id).or_default().insert(target);
            edges.entry(target).or_default().insert(id);
        }
    }
    let mut result = requested.clone();
    let mut pending = requested.iter().cloned().collect::<Vec<_>>();
    while let Some(id) = pending.pop() {
        crate::cancel::check_current_cancel("annotation promotion dependency closure")?;
        if !graph.nodes.contains_key(&id) {
            return Err(fail(
                "annotation promotion source identity missing; rediscover the source",
            ));
        }
        if let Some(neighbors) = edges.get(id.as_str()) {
            for neighbor in neighbors {
                if result.insert((*neighbor).to_string()) {
                    pending.push((*neighbor).to_string());
                }
            }
        }
    }
    Ok(result)
}

fn verify_occurrences(
    before: &IdentityIndex,
    after: &IdentityIndex,
    selected: &BTreeSet<String>,
    refs: &BTreeMap<String, Ref>,
) -> Result<()> {
    if before.len() != after.len() {
        return Err(fail("annotation promotion changed source count"));
    }
    for (slot, old) in before {
        crate::cancel::check_current_cancel("annotation promotion source order verification")?;
        let new = after
            .get(slot)
            .ok_or_else(|| fail("annotation promotion changed source order"))?;
        if new.name != old.name
            || new.reference != refs.get(&old.id).copied().or(old.reference)
            || (selected.contains(&old.id) && new.id != old.id)
        {
            return Err(fail(
                "annotation promotion changed source identity or page order",
            ));
        }
    }
    Ok(())
}
