//! Bounded popup/reply topology and explicit whole-component approval.
//! ISO 32000-1 12.5.6: IRT/RT relationships and Popup/Parent are preserved,
//! not flattened into independent rectangles. Widget Parent is a field owner,
//! never an edge in this graph.
use super::*;

const MAX_GROUP: usize = 256;
const MAX_MEMBERSHIPS: usize = 262_144;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoryAnnotationGroupMember {
    pub annotation_id: String,
    pub geometry_sha256: String,
    pub page: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoryAnnotationGroup {
    /// Canonical stable-ID edges, reply types and popup ownership. Independent
    /// of object numbering and page indices, but not of topology changes.
    pub topology_sha256: String,
    /// Sorted complete component, including the chosen anchor itself.
    pub members: Vec<StoryAnnotationGroupMember>,
    /// Relative order in the source page's Annots array.
    pub paint_order: Vec<String>,
}

pub(super) fn discover(
    entries: &mut BTreeMap<String, Entry>,
    relationships: &crate::annotation_relationships::Graph,
) -> Result<()> {
    let mut graph = BTreeMap::<String, BTreeSet<String>>::new();
    let mut edges = BTreeMap::<String, Vec<(String, String)>>::new();
    for id in entries.keys() {
        let node = relationships
            .nodes
            .get(id)
            .ok_or_else(|| fail("annotation relationship source missing"))?;
        for (key, target) in [
            ("Popup", node.popup.as_ref()),
            ("IRT", node.reply_to.as_ref()),
            (
                "Parent",
                if node.parent_explicit {
                    node.parent.as_ref()
                } else {
                    None
                },
            ),
        ] {
            let Some(target) = target else { continue };
            if !entries.contains_key(target) {
                return Err(fail(
                    "annotation relationship member has no editable source geometry",
                ));
            }
            graph.entry(id.clone()).or_default().insert(target.clone());
            graph.entry(target.clone()).or_default().insert(id.clone());
            edges
                .entry(id.clone())
                .or_default()
                .push((key.into(), target.clone()));
        }
    }
    let mut done = BTreeSet::new();
    let mut memberships = 0usize;
    let mut receipt_bytes = 0usize;
    for start in graph.keys() {
        crate::cancel::check_current_cancel("annotation group topology")?;
        if done.contains(start) {
            continue;
        }
        let mut pending = vec![start.clone()];
        let mut component = BTreeSet::new();
        while let Some(id) = pending.pop() {
            if !component.insert(id.clone()) {
                continue;
            }
            if component.len() > MAX_GROUP {
                return Err(fail("annotation group member limit exceeded"));
            }
            if let Some(adjacent) = graph.get(&id) {
                pending.extend(adjacent.iter().cloned());
            }
        }
        memberships = memberships.saturating_add(component.len().saturating_mul(component.len()));
        if memberships > MAX_MEMBERSHIPS {
            return Err(fail("annotation group receipt budget exceeded"));
        }
        let member_bytes = component.iter().map(|id| 2 * id.len() + 200).sum::<usize>();
        receipt_bytes = receipt_bytes.saturating_add(member_bytes.saturating_mul(component.len()));
        if receipt_bytes > 32 * 1024 * 1024 {
            return Err(fail("annotation group receipt byte budget exceeded"));
        }
        let topology = component
            .iter()
            .map(|id| {
                (
                    id,
                    &entries[id].source.subtype,
                    edges.get(id),
                    relationships.nodes[id].reply_type.as_deref(),
                )
            })
            .collect::<Vec<_>>();
        let mut paint_order = component.iter().collect::<Vec<_>>();
        paint_order.sort_by_key(|id| entries[*id].annotation_order);
        let encoded =
            serde_json::to_vec(&(topology, &paint_order)).map_err(|e| fail(&e.to_string()))?;
        let group = StoryAnnotationGroup {
            topology_sha256: format!("{:x}", Sha256::digest(encoded)),
            paint_order: paint_order.into_iter().cloned().collect(),
            members: component
                .iter()
                .map(|id| StoryAnnotationGroupMember {
                    annotation_id: id.clone(),
                    geometry_sha256: entries[id].source.geometry_sha256.clone(),
                    page: entries[id].source.page,
                })
                .collect(),
        };
        for id in component {
            done.insert(id.clone());
            entries.get_mut(&id).unwrap().source.group = Some(group.clone());
        }
    }
    Ok(())
}

pub(super) fn approved_ids(anchor: &StoryAnnotationAnchor, entry: &Entry) -> Result<Vec<String>> {
    if anchor.group != entry.source.group {
        return Err(fail(
            "annotation popup/reply group requires complete current approval",
        ));
    }
    Ok(entry
        .source
        .group
        .as_ref()
        .map(|g| g.members.iter().map(|m| m.annotation_id.clone()).collect())
        .unwrap_or_else(|| vec![anchor.annotation_id.clone()]))
}

/// Persist selected object identities, before canonical page insertion.
/// Original approval is bound to `input`; intermediate image staging may have
/// changed bytes, but must not have changed any selected annotation dictionary.
pub(super) fn stage(input: &[u8], staged: &[u8], moves: &[StoryAnnotationMove]) -> Result<Vec<u8>> {
    if moves.is_empty() {
        return Ok(staged.to_vec());
    }
    let source = ContentEngine::open_bytes(input.to_vec())?;
    let entries = inventory(&source)?;
    validate_moves(&entries, moves)?;
    let mut selected = BTreeSet::new();
    for movement in moves {
        crate::cancel::check_current_cancel("annotation identity staging")?;
        let e = entries
            .get(&movement.annotation_id)
            .ok_or_else(|| fail("staged annotation missing"))?;
        if !selected.insert(movement.annotation_id.clone()) || e.source.rect != movement.old_rect {
            return Err(fail("duplicate or stale staged annotation"));
        }
    }
    crate::annotation_promotion::stage(input, staged, &selected, 100_000).map(|(bytes, _)| bytes)
}

/// Defend the internal writer too: no partial group, split destinations or
/// independently moved replies, even if a caller bypasses preview assembly.
pub(super) fn validate_moves(
    entries: &BTreeMap<String, Entry>,
    moves: &[StoryAnnotationMove],
) -> Result<()> {
    validate_geometry_moves(entries, moves, false)
}

pub(super) fn validate_geometry_moves(
    entries: &BTreeMap<String, Entry>,
    moves: &[StoryAnnotationMove],
    allow_resize: bool,
) -> Result<()> {
    let selected = moves
        .iter()
        .map(|m| (m.annotation_id.as_str(), m))
        .collect::<BTreeMap<_, _>>();
    if selected.len() != moves.len() {
        return Err(fail("duplicate annotation movement"));
    }
    for m in moves {
        let entry = entries
            .get(&m.annotation_id)
            .ok_or_else(|| fail("annotation move source missing"))?;
        if let Some(group) = &entry.source.group {
            let transform = super::geometry::affine(m.old_rect, m.new_rect)?;
            for member in &group.members {
                let other = selected
                    .get(member.annotation_id.as_str())
                    .ok_or_else(|| fail("annotation movement omitted a popup/reply member"))?;
                let other_transform = super::geometry::affine(other.old_rect, other.new_rect)?;
                if other.target_page != m.target_page
                    || transform
                        .iter()
                        .zip(other_transform)
                        .any(|(a, b)| (a - b).abs() > 1e-7)
                    || (!allow_resize
                        && (transform[0] - 1.0).abs().max((transform[1] - 1.0).abs()) > 1e-7)
                {
                    return Err(fail(
                        "annotation group requires one common approved transform and destination",
                    ));
                }
            }
        }
    }
    Ok(())
}
