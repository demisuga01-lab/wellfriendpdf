//! Paragraph paths under stable table-cell containers. Source paths are explicit,
//! shared ancestors are retained, and an ordered tree must represent the approved
//! paragraph order exactly. No byte-offset or extracted-string tag matching.
use super::*;
use crate::linked_stories::tables::TableCell;

pub(super) fn active(tags: &TableTagging, cell: &TableCell) -> bool {
    cell.block_ids().any(|id| tags.blocks.contains_key(id))
}
pub(super) fn validate_config(request: &LinkedStoryRequest) -> Result<()> {
    let tags = config(request).unwrap();
    let mut paragraphs = BTreeSet::new();
    let mut bound = BTreeSet::new();
    for cell in &request.table_layout.as_ref().unwrap().cells {
        if cell.paragraph_ids.len() > 4096 {
            return Err(fail("table semantic block budget exceeded"));
        }
        let multiple = active(tags, cell);
        if !multiple && cell.block_ids().ne(std::iter::once(cell.id.as_str())) {
            return Err(fail(
                "explicit table paragraphs require per-block semantic ownership",
            ));
        }
        if multiple && tags.content_paths.contains_key(&cell.id) {
            return Err(fail(
                "a cell cannot combine legacy content_paths and per-block ownership",
            ));
        }
        for id in cell.block_ids() {
            if !paragraphs.insert(id) {
                return Err(fail("table semantic paragraph ownership is not unique"));
            }
            if !multiple {
                continue;
            }
            let block = tags.blocks.get(id).ok_or_else(|| {
                fail("every paragraph in a block-owned cell needs a reuse/create decision")
            })?;
            bound.insert(id);
            if !block.path.is_empty() && (block.new_role.is_some() || block.semantic_text.is_some())
            {
                return Err(fail(
                    "reused block roles are preserved; semantic review belongs on its source path",
                ));
            }
            if block.new_role.as_ref().is_some_and(|role| {
                !matches!(
                    role.as_str(),
                    "P" | "Span" | "H" | "H1" | "H2" | "H3" | "H4" | "H5" | "H6" | "Quote" | "Code"
                )
            }) {
                return Err(fail("new cell block needs an explicit supported text role"));
            }
        }
    }
    if request
        .paragraphs
        .iter()
        .map(|p| p.id.as_str())
        .collect::<BTreeSet<_>>()
        .len()
        != request.paragraphs.len()
        || tags.blocks.keys().any(|id| !bound.contains(id.as_str()))
        || paragraphs.len() != request.paragraphs.len()
        || request
            .paragraphs
            .iter()
            .any(|p| !paragraphs.contains(p.id.as_str()))
    {
        return Err(fail("table tag blocks and styled paragraphs disagree"));
    }
    Ok(())
}

pub(super) struct CellPlan {
    pub root: Option<ObjectRef>,
    /// Descendants only; no entry for a newly created leaf.
    pub paths: BTreeMap<String, Vec<ObjectRef>>,
}
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Token {
    Source(ObjectRef),
    New(usize),
}

/// Construct an ordered prefix tree and verify its leaf traversal. This rejects
/// A/leaf1, B/leaf2, A/leaf3: one retained A cannot appear in two sibling slots.
fn ordered_edges<T: Ord + Copy>(root: T, paths: &[Vec<T>]) -> Result<BTreeMap<T, Vec<T>>> {
    let mut edges = BTreeMap::<T, Vec<T>>::new();
    let mut edge_set = BTreeSet::new();
    let mut parents = BTreeMap::new();
    let mut expected = Vec::new();
    let mut leaves = BTreeSet::new();
    let mut slots = 0usize;
    for path in paths {
        crate::cancel::check_current_cancel("ordered table block tree")?;
        slots = slots.saturating_add(path.len());
        if path.first() != Some(&root) || path.len() < 2 || path.len() > 130 || slots > MAX_NODES {
            return Err(fail("invalid/budgeted table block path"));
        }
        let leaf = *path.last().unwrap();
        if !leaves.insert(leaf) || path.iter().copied().collect::<BTreeSet<_>>().len() != path.len()
        {
            return Err(fail("repeated/cyclic table block owner"));
        }
        expected.push(leaf);
        for pair in path.windows(2) {
            if parents
                .insert(pair[1], pair[0])
                .is_some_and(|old| old != pair[0])
            {
                return Err(fail("table block owner has multiple parents"));
            }
            if edge_set.insert((pair[0], pair[1])) {
                edges.entry(pair[0]).or_default().push(pair[1]);
            }
        }
    }
    let mut observed = Vec::new();
    let mut stack = vec![root];
    let mut visited = BTreeSet::new();
    while let Some(node) = stack.pop() {
        if visited.len() % 256 == 0 {
            crate::cancel::check_current_cancel("table block tree traversal")?;
        }
        if !visited.insert(node) || visited.len() > MAX_NODES {
            return Err(fail("cyclic table block graph"));
        }
        if let Some(children) = edges.get(&node) {
            if leaves.contains(&node) {
                return Err(fail("text leaf is also a block container"));
            }
            stack.extend(children.iter().rev().copied());
        } else {
            observed.push(node);
        }
    }
    if observed != expected {
        return Err(fail(
            "paragraph order interleaves a shared semantic ancestor; explicitly restructure that ancestor",
        ));
    }
    Ok(edges)
}

pub(super) fn plan_cell(
    store: &Store<'_>,
    root: ObjectRef,
    index: &StructureIndex,
    lookup: &Lookup,
    request: &LinkedStoryRequest,
    cell: &TableCell,
    tree: &structure::Tree,
    used: &mut BTreeSet<ObjectRef>,
) -> Result<CellPlan> {
    let tags = config(request).unwrap();
    let owner = tags.cells[&cell.id]
        .as_ref()
        .map(|v| resolve_tag(root, index, lookup, v))
        .transpose()?;
    let available = if let Some(node) = owner {
        let paths = tree
            .cell_paths
            .get(&node)
            .ok_or_else(|| fail("block cell is outside the selected table"))?;
        let dict = store.dict(node)?;
        if dict.get_name("S") != Some(tags.semantics[&cell.id].role.name()) || !used.insert(node) {
            return Err(fail("reused block cell role/identity differs"));
        }
        semantic_review(store, &dict, tags.semantic_text.get(&cell.id))?;
        paths
            .iter()
            .map(|path| (*path.last().unwrap(), path))
            .collect::<BTreeMap<_, _>>()
    } else {
        BTreeMap::new()
    };
    let mut paths = BTreeMap::new();
    let mut reviews = BTreeMap::<ObjectRef, Option<StorySemanticText>>::new();
    let mut local = BTreeSet::new();
    let mut ordered = Vec::new();
    let root_token = owner.map(Token::Source).unwrap_or(Token::New(0));
    for (ordinal, id) in cell.block_ids().enumerate() {
        let decision = &tags.blocks[id];
        if decision.path.is_empty() {
            ordered.push(vec![root_token, Token::New(ordinal + 1)]);
            continue;
        }
        let owner =
            owner.ok_or_else(|| fail("new cell cannot reuse another cell's paragraph path"))?;
        let path = decision
            .path
            .iter()
            .map(|item| resolve_tag(root, index, lookup, &item.source))
            .collect::<Result<Vec<_>>>()?;
        let leaf = *path.last().unwrap();
        let actual = available
            .get(&leaf)
            .ok_or_else(|| fail("paragraph leaf is outside its selected cell"))?;
        if actual.first() != Some(&owner) || actual.as_slice().get(1..) != Some(path.as_slice()) {
            return Err(fail("paragraph path differs from the source hierarchy"));
        }
        for (&node, approval) in path.iter().zip(&decision.path) {
            if let Some(previous) = reviews.get(&node) {
                if previous != &approval.semantic_text {
                    return Err(fail(
                        "shared block ancestor has conflicting semantic reviews",
                    ));
                }
            } else {
                semantic_review(store, &store.dict(node)?, approval.semantic_text.as_ref())?;
                reviews.insert(node, approval.semantic_text.clone());
            }
            if local.insert(node) && !used.insert(node) {
                return Err(fail("table block owner is reused by different cells"));
            }
        }
        let mut full = vec![root_token];
        full.extend(path.iter().copied().map(Token::Source));
        ordered.push(full);
        paths.insert(id.to_owned(), path);
    }
    ordered_edges(root_token, &ordered)?;
    Ok(CellPlan { root: owner, paths })
}

fn marker(
    store: &mut Store<'_>,
    node: ObjectRef,
    request: &LinkedStoryRequest,
    key: String,
    replace: bool,
) -> Result<()> {
    let mut d = store.dict(node)?;
    if replace || key_value(&d, "WFStoryTagKey").is_none() {
        d.insert("WFStoryTagKey", PdfObject::String(key.into_bytes()));
    }
    d.insert(
        "WFStoryID",
        PdfObject::String(story_key(request).into_bytes()),
    );
    store.replace_dict(node, d)
}

pub(super) fn stage_cell(
    store: &mut Store<'_>,
    request: &LinkedStoryRequest,
    cell: &TableCell,
    plan: &CellPlan,
    targets: &PdfDictionary,
    paths: &mut PdfDictionary,
    preimages: &mut Vec<PdfObject>,
) -> Result<ObjectRef> {
    let tags = config(request).unwrap();
    let root = if let Some(root) = plan.root {
        root
    } else {
        let mut d = PdfDictionary::empty();
        d.insert("Type", PdfObject::Name("StructElem".into()));
        d.insert(
            "S",
            PdfObject::Name(tags.semantics[&cell.id].role.name().into()),
        );
        store.add(PdfObject::Dictionary(d))?
    };
    let mut ancestors = BTreeSet::new();
    let mut full_paths = Vec::new();
    for id in cell.block_ids() {
        let target = targets
            .get_reference(&key(&request.story_id, id))
            .ok_or_else(|| fail("missing generated block leaf"))?;
        let mut full = vec![root];
        full.extend(plan.paths.get(id).cloned().unwrap_or_else(|| vec![target]));
        if full.last() != Some(&target) {
            return Err(fail("generated block leaf differs from approved source"));
        }
        for (depth, &node) in full[..full.len() - 1].iter().enumerate() {
            if ancestors.insert(node) {
                let mut preimage = PdfDictionary::empty();
                preimage.insert("Owner", reference(node));
                preimage.insert("Children", PdfObject::Array(kids(&store.dict(node)?)));
                preimages.push(PdfObject::Dictionary(preimage));
                marker(
                    store,
                    node,
                    request,
                    if depth == 0 {
                        node_key(request, "cell-root", &cell.id, 0)
                    } else {
                        node_key(request, "block-content", id, depth)
                    },
                    depth == 0,
                )?;
            }
        }
        paths.insert(
            key(&request.story_id, id),
            PdfObject::Array(full.iter().copied().map(reference).collect()),
        );
        full_paths.push(full);
    }
    ordered_edges(root, &full_paths)?;
    Ok(root)
}

pub(super) fn check_preimages(store: &Store<'_>, txn: &PdfDictionary) -> Result<()> {
    let Some(preimages) = txn.get("TableBlockPreimages").and_then(PdfObject::as_array) else {
        return Ok(());
    };
    if preimages.len() > MAX_NODES {
        return Err(fail("table block preimage budget exceeded"));
    }
    for item in preimages {
        crate::cancel::check_current_cancel("table block preimage validation")?;
        let d = item
            .as_dict()
            .ok_or_else(|| fail("invalid table block preimage"))?;
        let owner = d
            .get_reference("Owner")
            .ok_or_else(|| fail("missing block preimage owner"))?;
        let children = d
            .get("Children")
            .and_then(PdfObject::as_array)
            .ok_or_else(|| fail("missing block preimage children"))?;
        if kids(&store.dict(owner)?).as_slice() != children {
            return Err(fail(
                "block ancestor children changed during the transaction",
            ));
        }
    }
    Ok(())
}

pub(super) fn finish_cell(
    store: &mut Store<'_>,
    request: &LinkedStoryRequest,
    cell: &TableCell,
    root: ObjectRef,
    txn: &PdfDictionary,
    rebound: &BTreeMap<String, Option<TagReference>>,
    resolver: &attributes::Resolver,
    budget: &mut attributes::Budget,
) -> Result<(Vec<PdfObject>, BTreeMap<String, CellBlockTagging>)> {
    let tags = config(request).unwrap();
    let paths = txn
        .get("TableBlockPaths")
        .and_then(PdfObject::as_dict)
        .ok_or_else(|| fail("missing staged block paths"))?;
    let mut full_paths = Vec::new();
    let mut review = BTreeMap::new();
    let mut leaves = BTreeSet::new();
    for id in cell.block_ids() {
        let path = paths
            .get(&key(&request.story_id, id))
            .and_then(PdfObject::as_array)
            .ok_or_else(|| fail("missing staged paragraph path"))?
            .iter()
            .map(|v| {
                v.as_reference()
                    .ok_or_else(|| fail("invalid staged paragraph owner"))
            })
            .collect::<Result<Vec<_>>>()?;
        let leaf = rebound
            .get(id)
            .and_then(Option::as_ref)
            .ok_or_else(|| fail("missing rebound paragraph leaf"))?;
        let decision = &tags.blocks[id];
        let expected = if decision.path.is_empty() {
            2
        } else {
            decision.path.len() + 1
        };
        if path.len() != expected
            || path.first() != Some(&root)
            || path.last() != Some(&(leaf.object, leaf.generation))
        {
            return Err(fail("staged block path disagrees with approved target"));
        }
        for (depth, &node) in path.iter().enumerate().skip(1) {
            let approved = if decision.path.is_empty() {
                decision.semantic_text.clone()
            } else {
                decision.path[depth - 1].semantic_text.clone()
            };
            if review
                .insert(node, approved.clone())
                .is_some_and(|old| old != approved)
            {
                return Err(fail("conflicting staged ancestor reviews"));
            }
        }
        leaves.insert((leaf.object, leaf.generation));
        full_paths.push(path);
    }
    let edges = ordered_edges(root, &full_paths)?;
    let parents = edges
        .iter()
        .flat_map(|(&parent, children)| children.iter().map(move |&child| (child, parent)))
        .collect::<BTreeMap<_, _>>();
    for (node, approval) in &review {
        crate::cancel::check_current_cancel("table block semantic migration")?;
        let mut d = store.dict(*node)?;
        d.insert("P", reference(parents[node]));
        d.remove("Pg");
        if !leaves.contains(node) {
            d.insert(
                "K",
                PdfObject::Array(edges[node].iter().copied().map(reference).collect()),
            );
            d.remove("ActualText");
        }
        reviewed_text(&mut d, approval.as_ref());
        let rtl = request.paragraphs.iter().any(|p| {
            p.rtl
                && rebound
                    .get(&p.id)
                    .and_then(Option::as_ref)
                    .is_some_and(|tag| (tag.object, tag.generation) == *node)
        });
        resolver.rewrite_flow(store, &mut d, None, request.writing_mode, rtl, budget)?;
        store.replace_dict(*node, d)?;
    }
    let mut rebound_blocks = BTreeMap::new();
    for (id, path) in cell.block_ids().zip(full_paths) {
        let owners = path
            .iter()
            .skip(1)
            .map(|&node| {
                Ok(CellTextOwner {
                    source: tag_ref(node, &store.dict(node)?),
                    semantic_text: None,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        rebound_blocks.insert(
            id.to_owned(),
            CellBlockTagging {
                path: owners,
                new_role: None,
                semantic_text: None,
            },
        );
    }
    Ok((
        edges[&root].iter().copied().map(reference).collect(),
        rebound_blocks,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordered_prefix_tree_preserves_shared_ancestors_without_interleaving() {
        let paths = vec![vec![0, 1, 2], vec![0, 1, 3], vec![0, 4]];
        let edges = ordered_edges(0, &paths).unwrap();
        assert_eq!(edges[&0], vec![1, 4]);
        assert_eq!(edges[&1], vec![2, 3]);
        assert!(ordered_edges(0, &[vec![0, 1, 2], vec![0, 4], vec![0, 1, 3]]).is_err());
        assert!(ordered_edges(0, &[vec![0, 1], vec![0, 1]]).is_err());
        assert!(ordered_edges(0, &[vec![0, 1], vec![0, 1, 2]]).is_err());
        assert!(ordered_edges(0, &[vec![0, 1, 2], vec![0, 3, 2]]).is_err());
        assert!(ordered_edges(0, &[vec![0, 1, 0]]).is_err());
    }
}
