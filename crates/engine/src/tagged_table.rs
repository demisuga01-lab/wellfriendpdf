//! Explicit whole-table structure migration. Cell identity and header links stay
//! distinct from paragraph-leaf ownership while MCRs move between fragments.
use super::*;
#[path = "tagged_table_blocks.rs"]
mod blocks;
#[path = "tagged_table_structure.rs"]
mod structure;
#[cfg(test)]
#[path = "tagged_table_tests.rs"]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CellRole {
    TH,
    TD,
}
impl CellRole {
    fn name(self) -> &'static str {
        match self {
            Self::TH => "TH",
            Self::TD => "TD",
        }
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum HeaderScope {
    Row,
    Column,
    Both,
}
impl HeaderScope {
    fn name(self) -> &'static str {
        match self {
            Self::Row => "Row",
            Self::Column => "Column",
            Self::Both => "Both",
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CellSemantics {
    pub role: CellRole,
    #[serde(default)]
    pub scope: Option<HeaderScope>,
    /// Logical cell IDs, not PDF object numbers or guessed adjacent headings.
    #[serde(default)]
    pub headers: Vec<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RowGroupRole {
    THead,
    TBody,
    TFoot,
}
impl RowGroupRole {
    fn name(self) -> &'static str {
        match self {
            Self::THead => "THead",
            Self::TBody => "TBody",
            Self::TFoot => "TFoot",
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RowGroupBinding {
    pub id: String,
    pub role: RowGroupRole,
    pub rows: Vec<String>,
    #[serde(default)]
    pub source: Option<TagReference>,
    #[serde(default)]
    pub semantic_text: Option<StorySemanticText>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CellTextOwner {
    pub source: TagReference,
    #[serde(default)]
    pub semantic_text: Option<StorySemanticText>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CellBlockTagging {
    /// Complete descendant path below its TH/TD, ending at one reusable text
    /// leaf. Empty explicitly creates a new direct child paragraph.
    #[serde(default)]
    pub path: Vec<CellTextOwner>,
    #[serde(default)]
    pub new_role: Option<String>,
    /// New leaf description; reused leaf reviews live on the path's last item.
    #[serde(default)]
    pub semantic_text: Option<StorySemanticText>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableTagging {
    pub source: TagReference,
    /// Every desired row/cell has an explicit reuse (Some) or create (None).
    pub rows: BTreeMap<String, Option<TagReference>>,
    pub cells: BTreeMap<String, Option<TagReference>>,
    pub semantics: BTreeMap<String, CellSemantics>,
    /// Explicit descendants between a reused TH/TD and its text leaf, in order.
    #[serde(default)]
    pub content_paths: BTreeMap<String, Vec<CellTextOwner>>,
    /// Per-paragraph ownership for a cell whose root is a structural container.
    /// Every block of such a cell must have an entry, even a new/empty paragraph.
    #[serde(default)]
    pub blocks: BTreeMap<String, CellBlockTagging>,
    /// Empty means direct TR children. Otherwise partitions rows in grid order.
    #[serde(default)]
    pub groups: Vec<RowGroupBinding>,
    /// Removing an imported group is a separate decision, never an implicit flatten.
    #[serde(default)]
    pub removed_groups: Vec<TagReference>,
    #[serde(default)]
    pub semantic_text: BTreeMap<String, StorySemanticText>,
    #[serde(default)]
    pub row_text: BTreeMap<String, StorySemanticText>,
    #[serde(default)]
    pub table_text: Option<StorySemanticText>,
}
pub(super) fn config(request: &LinkedStoryRequest) -> Option<&TableTagging> {
    request.table_layout.as_ref()?.tagging.as_ref()
}
pub(super) fn new_role(request: &LinkedStoryRequest, id: &str) -> Option<String> {
    if let Some(block) = config(request)?.blocks.get(id) {
        return Some(block.new_role.clone().unwrap_or_else(|| "P".into()));
    }
    Some(config(request)?.semantics.get(id)?.role.name().into())
}
pub(super) fn text_review<'a>(
    request: &'a LinkedStoryRequest,
    id: &str,
) -> Option<&'a StorySemanticText> {
    let tags = config(request)?;
    if let Some(block) = tags.blocks.get(id) {
        return block
            .path
            .last()
            .and_then(|node| node.semantic_text.as_ref())
            .or(block.semantic_text.as_ref());
    }
    match tags.content_paths.get(id).and_then(|path| path.last()) {
        Some(leaf) => leaf.semantic_text.as_ref(),
        None => tags.semantic_text.get(id),
    }
}
fn node_key(request: &LinkedStoryRequest, kind: &str, id: &str, ordinal: usize) -> String {
    let mut h = Sha256::new();
    h.update(b"Wellfriend.Table.Semantics.v1\0");
    for part in [request.story_id.as_str(), kind, id] {
        h.update((part.len() as u64).to_be_bytes());
        h.update(part.as_bytes());
    }
    h.update((ordinal as u64).to_be_bytes());
    format!("{:x}", h.finalize())
}
fn row_key(request: &LinkedStoryRequest, id: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"Wellfriend.Table.Row.v1\0");
    for part in [request.story_id.as_str(), id] {
        h.update((part.len() as u64).to_be_bytes());
        h.update(part.as_bytes());
    }
    format!("{:x}", h.finalize())
}
fn semantic_review(
    store: &Store<'_>,
    dict: &PdfDictionary,
    review: Option<&StorySemanticText>,
) -> Result<()> {
    if (semantic_string(store, dict, "Alt")?.is_some()
        || semantic_string(store, dict, "E")?.is_some())
        && review.is_none()
    {
        return Err(fail(
            "table owner has Alt/E; approve its replacement/removal explicitly",
        ));
    }
    Ok(())
}
fn reviewed_text(dict: &mut PdfDictionary, review: Option<&StorySemanticText>) {
    if let Some(review) = review {
        for (name, text) in [("Alt", &review.alternate), ("E", &review.expansion)] {
            if let Some(text) = text {
                dict.insert(name, logical_string(text));
            } else {
                dict.remove(name);
            }
        }
    }
}
fn owner_check(
    store: &Store<'_>,
    node: ObjectRef,
    request: &LinkedStoryRequest,
) -> Result<PdfDictionary> {
    let dict = store.dict(node)?;
    if key_value(&dict, "WFStoryID").is_some_and(|id| id != story_key(request)) {
        return Err(fail("table subtree includes another story's owner"));
    }
    Ok(dict)
}
/// Bound the semantic graph before shape/paint work. No header role or scope is
/// inferred from bold text, row location, colour, or a repeated visual header.
pub(crate) fn validate_config(request: &LinkedStoryRequest) -> Result<()> {
    let Some(tags) = config(request) else {
        return Ok(());
    };
    let layout = request.table_layout.as_ref().unwrap();
    if layout.rows.len() > 4096
        || layout.cells.len() > 4096
        || request.paragraphs.len() > 16_384
        || tags.blocks.len() > 16_384
        || request.source_tags.is_some()
        || tags.rows.len() != layout.rows.len()
        || tags.cells.len() != layout.cells.len()
        || tags.semantics.len() != layout.cells.len()
        || tags.row_text.len() > layout.rows.len()
        || tags.semantic_text.len() > layout.cells.len()
        || tags.content_paths.len() > layout.cells.len()
        || tags.groups.len() > 4096
        || tags.removed_groups.len() > 4096
    {
        return Err(fail(
            "table tagging must bind the complete approved topology",
        ));
    }
    blocks::validate_config(request)?;
    let row_ids = layout
        .rows
        .iter()
        .map(|row| row.id.as_str())
        .collect::<BTreeSet<_>>();
    let cell_ids = layout
        .cells
        .iter()
        .map(|cell| cell.id.as_str())
        .collect::<BTreeSet<_>>();
    if tags
        .rows
        .keys()
        .chain(tags.row_text.keys())
        .any(|id| !row_ids.contains(id.as_str()))
        || tags
            .cells
            .keys()
            .chain(tags.semantic_text.keys())
            .chain(tags.semantics.keys())
            .chain(tags.content_paths.keys())
            .any(|id| !cell_ids.contains(id.as_str()))
    {
        return Err(fail("table tagging references a missing row/cell"));
    }
    if tags
        .content_paths
        .values()
        .chain(tags.blocks.values().map(|b| &b.path))
        .any(|path| path.len() > 128)
        || tags
            .content_paths
            .values()
            .chain(tags.blocks.values().map(|b| &b.path))
            .map(Vec::len)
            .sum::<usize>()
            > MAX_NODES
    {
        return Err(fail("table descendant path budget exceeded"));
    }
    let semantic_bytes = tags
        .semantic_text
        .values()
        .chain(tags.row_text.values())
        .chain(tags.table_text.iter())
        .chain(tags.groups.iter().filter_map(|g| g.semantic_text.as_ref()))
        .chain(
            tags.content_paths
                .values()
                .chain(tags.blocks.values().map(|b| &b.path))
                .flatten()
                .filter_map(|p| p.semantic_text.as_ref()),
        )
        .chain(
            tags.blocks
                .values()
                .filter_map(|b| b.semantic_text.as_ref()),
        )
        .fold(0usize, |n, text| {
            n.saturating_add(text.alternate.as_ref().map_or(0, String::len))
                .saturating_add(text.expansion.as_ref().map_or(0, String::len))
        });
    if semantic_bytes > 4_000_000 {
        return Err(fail("table semantic text budget exceeded"));
    }
    structure::validate_groups(request)?;
    let mut edge_count = 0usize;
    for (id, semantics) in &tags.semantics {
        edge_count = edge_count.saturating_add(semantics.headers.len());
        let mut seen = BTreeSet::new();
        if edge_count > 65_536
            || semantics.role == CellRole::TD && semantics.scope.is_some()
            || semantics.headers.iter().any(|header| {
                header == id
                    || !seen.insert(header)
                    || tags
                        .semantics
                        .get(header)
                        .is_none_or(|value| value.role != CellRole::TH)
            })
        {
            return Err(fail("invalid table header role/scope/relationship"));
        }
    }
    // Kahn's algorithm also bounds traversal and rejects cyclic header graphs.
    let mut indegree = tags
        .semantics
        .keys()
        .map(|id| (id.as_str(), 0usize))
        .collect::<BTreeMap<_, _>>();
    for semantics in tags.semantics.values() {
        for header in &semantics.headers {
            *indegree.get_mut(header.as_str()).unwrap() += 1;
        }
    }
    let mut ready = indegree
        .iter()
        .filter(|(_, n)| **n == 0)
        .map(|(id, _)| *id)
        .collect::<Vec<_>>();
    let mut count = 0;
    while let Some(id) = ready.pop() {
        count += 1;
        for header in &tags.semantics[id].headers {
            let n = indegree.get_mut(header.as_str()).unwrap();
            *n -= 1;
            if *n == 0 {
                ready.push(header.as_str());
            }
        }
    }
    if count != tags.semantics.len() {
        return Err(fail("cyclic table header relationships"));
    }
    Ok(())
}

pub(super) fn selection(
    store: &Store<'_>,
    root: ObjectRef,
    index: &StructureIndex,
    lookup: &Lookup,
    request: &LinkedStoryRequest,
) -> Result<Selection> {
    validate_config(request)?;
    let tags = config(request).unwrap();
    let table = resolve_tag(root, index, lookup, &tags.source)?;
    if lookup
        .keys
        .get(&story_key(request))
        .is_some_and(|owner| *owner != table)
    {
        return Err(fail(
            "story identity already belongs to a different structural parent",
        ));
    }
    let dict = owner_check(store, table, request)?;
    if dict.get_name("S") != Some("Table") {
        return Err(fail("source table must have the Table role"));
    }
    semantic_review(store, &dict, tags.table_text.as_ref())?;
    let mut ancestor = dict.get_reference("P");
    let mut ancestors = BTreeSet::new();
    while let Some(id) = ancestor {
        if !ancestors.insert(id) || ancestors.len() > MAX_NODES {
            return Err(fail("cyclic table ancestors"));
        }
        let d = store.dict(id)?;
        if d.get("ActualText")
            .is_some_and(|v| !matches!(v, PdfObject::Null))
        {
            return Err(fail(
                "table ancestor ActualText needs an explicit semantic rewrite",
            ));
        }
        ancestor = if id == root {
            None
        } else {
            d.get_reference("P")
        };
    }
    let plan = structure::plan(store, root, index, lookup, table, request)?;
    let selected = plan.tree.leaves.iter().copied().collect();
    let mut targets = BTreeMap::new();
    for cell in &request.table_layout.as_ref().unwrap().cells {
        if let Some(blocks) = plan.blocks.get(&cell.id) {
            for id in cell.block_ids() {
                targets.insert(
                    id.to_owned(),
                    blocks.paths.get(id).and_then(|p| p.last()).copied(),
                );
            }
        } else {
            targets.insert(
                cell.id.clone(),
                plan.cells.get(&cell.id).and_then(|p| p.last()).copied(),
            );
        }
    }
    Ok(Selection {
        parent: table,
        selected,
        start: 0,
        targets,
        figures: BTreeMap::new(),
        figure_ocr_owners: BTreeMap::new(),
    })
}

pub(super) fn stage(
    store: &mut Store<'_>,
    root: ObjectRef,
    index: &StructureIndex,
    lookup: &Lookup,
    request: &LinkedStoryRequest,
    txn: &mut PdfDictionary,
) -> Result<()> {
    let Some(tags) = config(request) else {
        return Ok(());
    };
    let parent = txn
        .get_reference("Parent")
        .ok_or_else(|| fail("missing table parent"))?;
    let plan = structure::plan(store, root, index, lookup, parent, request)?;
    if let Some((caption, first)) = plan.tree.caption {
        let mut d = store.dict(caption)?;
        if !d.contains_key("Pg") {
            let mut ancestor = Some(parent);
            let mut seen = BTreeSet::new();
            while let Some(node) = ancestor {
                if !seen.insert(node) || seen.len() > MAX_NODES {
                    return Err(fail("cyclic caption page inheritance"));
                }
                let source = store.dict(node)?;
                if let Some(page) = source.get_reference("Pg") {
                    d.insert("Pg", reference(page));
                    break;
                }
                ancestor = source.get_reference("P");
            }
        }
        store.replace_dict(caption, d)?;
        let mut binding = PdfDictionary::empty();
        binding.insert("Owner", reference(caption));
        binding.insert("First", PdfObject::Boolean(first));
        txn.insert("TableCaption", PdfObject::Dictionary(binding));
    }
    txn.insert(
        "TableSourceChildren",
        PdfObject::Array(kids(&store.dict(parent)?)),
    );
    let mut rows = PdfDictionary::empty();
    for row in &request.table_layout.as_ref().unwrap().rows {
        let id = if let Some(&id) = plan.rows.get(&row.id) {
            id
        } else {
            let mut dict = PdfDictionary::empty();
            dict.insert("Type", PdfObject::Name("StructElem".into()));
            dict.insert("S", PdfObject::Name("TR".into()));
            dict.insert("P", reference(parent));
            store.add(PdfObject::Dictionary(dict))?
        };
        let mut dict = store.dict(id)?;
        let key = row_key(request, &row.id);
        dict.insert("WFStoryTagKey", PdfObject::String(key.as_bytes().to_vec()));
        dict.insert(
            "WFStoryID",
            PdfObject::String(story_key(request).into_bytes()),
        );
        store.replace_dict(id, dict)?;
        rows.insert(key, reference(id));
    }
    txn.insert("TableRows", PdfObject::Dictionary(rows));
    let mut groups = PdfDictionary::empty();
    for group in &tags.groups {
        let id = if let Some(&id) = plan.groups.get(&group.id) {
            id
        } else {
            let mut d = PdfDictionary::empty();
            d.insert("Type", PdfObject::Name("StructElem".into()));
            d.insert("S", PdfObject::Name(group.role.name().into()));
            d.insert("P", reference(parent));
            store.add(PdfObject::Dictionary(d))?
        };
        let key = node_key(request, "row-group", &group.id, 0);
        let mut d = store.dict(id)?;
        d.insert("WFStoryTagKey", PdfObject::String(key.as_bytes().to_vec()));
        d.insert(
            "WFStoryID",
            PdfObject::String(story_key(request).into_bytes()),
        );
        store.replace_dict(id, d)?;
        groups.insert(key, reference(id));
    }
    txn.insert("TableGroups", PdfObject::Dictionary(groups));
    let targets = txn
        .get("Targets")
        .and_then(PdfObject::as_dict)
        .ok_or_else(|| fail("missing cell text targets"))?;
    let mut paths = PdfDictionary::empty();
    let mut block_paths = PdfDictionary::empty();
    let mut block_preimages = Vec::new();
    for cell in &request.table_layout.as_ref().unwrap().cells {
        let key = key(&request.story_id, &cell.id);
        if let Some(blocks) = plan.blocks.get(&cell.id) {
            let root = blocks::stage_cell(
                store,
                request,
                cell,
                blocks,
                targets,
                &mut block_paths,
                &mut block_preimages,
            )?;
            paths.insert(key, PdfObject::Array(vec![reference(root)]));
            continue;
        }
        let leaf = targets
            .get_reference(&key)
            .ok_or_else(|| fail("missing cell text target"))?;
        let path = plan
            .cells
            .get(&cell.id)
            .cloned()
            .unwrap_or_else(|| vec![leaf]);
        if path.last() != Some(&leaf) {
            return Err(fail("cell leaf changed during tag staging"));
        }
        for (depth, &node) in path.iter().enumerate().take(path.len().saturating_sub(1)) {
            let mut d = store.dict(node)?;
            d.insert(
                "WFStoryTagKey",
                PdfObject::String(node_key(request, "cell-content", &cell.id, depth).into_bytes()),
            );
            d.insert(
                "WFStoryID",
                PdfObject::String(story_key(request).into_bytes()),
            );
            store.replace_dict(node, d)?;
        }
        paths.insert(
            key,
            PdfObject::Array(path.into_iter().map(reference).collect()),
        );
    }
    txn.insert("TableCellPaths", PdfObject::Dictionary(paths));
    txn.insert("TableBlockPaths", PdfObject::Dictionary(block_paths));
    txn.insert("TableBlockPreimages", PdfObject::Array(block_preimages));
    Ok(())
}

pub(super) fn finish(
    store: &mut Store<'_>,
    request: &LinkedStoryRequest,
    txn: &PdfDictionary,
    rebound: &BTreeMap<String, Option<TagReference>>,
) -> Result<TableTagging> {
    let tags = config(request).unwrap();
    let layout = request.table_layout.as_ref().unwrap();
    blocks::check_preimages(store, txn)?;
    let table = txn
        .get_reference("Parent")
        .ok_or_else(|| fail("missing table parent"))?;
    let mut table_dict = store.dict(table)?;
    let old_rows = txn
        .get("TableSourceChildren")
        .and_then(PdfObject::as_array)
        .ok_or_else(|| fail("missing table row preimage"))?;
    if kids(&table_dict)
        .iter()
        .map(PdfObject::as_reference)
        .ne(old_rows.iter().map(PdfObject::as_reference))
    {
        return Err(fail("table row ownership changed inside transaction"));
    }
    let row_targets = txn
        .get("TableRows")
        .and_then(PdfObject::as_dict)
        .ok_or_else(|| fail("missing table row targets"))?;
    let group_targets = txn
        .get("TableGroups")
        .and_then(PdfObject::as_dict)
        .ok_or_else(|| fail("missing table groups"))?;
    let path_targets = txn
        .get("TableCellPaths")
        .and_then(PdfObject::as_dict)
        .ok_or_else(|| fail("missing table cell paths"))?;
    let resolver = attributes::Resolver::new(store)?;
    let mut attribute_budget = attributes::Budget::default();
    let mut nodes = BTreeMap::new();
    let mut paths = BTreeMap::new();
    let mut ids = BTreeMap::new();
    for cell in &layout.cells {
        let path = path_targets
            .get(&key(&request.story_id, &cell.id))
            .and_then(PdfObject::as_array)
            .ok_or_else(|| fail("missing cell owner path"))?
            .iter()
            .map(|v| {
                v.as_reference()
                    .ok_or_else(|| fail("invalid cell owner path"))
            })
            .collect::<Result<Vec<_>>>()?;
        if blocks::active(tags, cell) {
            if path.len() != 1 {
                return Err(fail("block cell requires a distinct cell-root binding"));
            }
        } else {
            let source = rebound
                .get(&cell.id)
                .and_then(Option::as_ref)
                .ok_or_else(|| fail("missing table cell rebound owner"))?;
            if path.last() != Some(&(source.object, source.generation))
                || path.len() != tags.content_paths.get(&cell.id).map_or(0, Vec::len) + 1
            {
                return Err(fail("cell owner path/leaf disagrees with staged bindings"));
            }
        }
        for pair in path.windows(2) {
            let k = kids(&store.dict(pair[0])?);
            if k.len() != 1 || k[0].as_reference() != Some(pair[1]) {
                return Err(fail("cell descendant path changed inside transaction"));
            }
        }
        let node = path[0];
        let d = store.dict(node)?;
        let id = match d.get("ID").map(|v| store.resolve(v)).transpose()? {
            Some(PdfObject::String(bytes)) => bytes,
            None | Some(PdfObject::Null) => {
                format!("WFTableCell_{}", key(&request.story_id, &cell.id)).into_bytes()
            }
            _ => return Err(fail("table cell ID is not a string")),
        };
        nodes.insert(cell.id.clone(), node);
        paths.insert(cell.id.clone(), path);
        ids.insert(cell.id.clone(), id);
    }
    let mut row_children = vec![Vec::new(); layout.rows.len()];
    let mut rebound_cells = BTreeMap::new();
    let mut rebound_paths = BTreeMap::new();
    let mut rebound_blocks = BTreeMap::new();
    let mut cells = layout.cells.iter().collect::<Vec<_>>();
    cells.sort_by_key(|cell| (cell.row, cell.column));
    for cell in cells {
        crate::cancel::check_current_cancel("table cell semantic migration")?;
        let node = nodes[&cell.id];
        let parent = row_targets
            .get_reference(&row_key(request, &layout.rows[cell.row].id))
            .ok_or_else(|| fail("missing table row owner"))?;
        let mut dict = store.dict(node)?;
        let semantics = &tags.semantics[&cell.id];
        dict.insert("P", reference(parent));
        dict.insert("S", PdfObject::Name(semantics.role.name().into()));
        dict.insert("ID", PdfObject::String(ids[&cell.id].clone()));
        let mut a = PdfDictionary::empty();
        a.insert("O", PdfObject::Name("Table".into()));
        a.insert("RowSpan", PdfObject::Integer(cell.row_span as i64));
        a.insert("ColSpan", PdfObject::Integer(cell.column_span as i64));
        if let Some(scope) = semantics.scope {
            a.insert("Scope", PdfObject::Name(scope.name().into()));
        }
        if !semantics.headers.is_empty() {
            a.insert(
                "Headers",
                PdfObject::Array(
                    semantics
                        .headers
                        .iter()
                        .map(|id| PdfObject::String(ids[id].clone()))
                        .collect(),
                ),
            );
        }
        let path = &paths[&cell.id];
        let multiple = blocks::active(tags, cell);
        if multiple {
            let (children, bindings) = blocks::finish_cell(
                store,
                request,
                cell,
                node,
                txn,
                rebound,
                &resolver,
                &mut attribute_budget,
            )?;
            dict.insert("K", PdfObject::Array(children));
            rebound_blocks.extend(bindings);
        }
        if path.len() > 1 || multiple {
            dict.remove("Pg");
            dict.remove("ActualText");
        }
        reviewed_text(&mut dict, tags.semantic_text.get(&cell.id));
        let rtl = request
            .paragraphs
            .iter()
            .find(|p| p.id == cell.id)
            .is_some_and(|p| p.rtl);
        resolver.rewrite_flow(
            store,
            &mut dict,
            Some(a),
            request.writing_mode,
            rtl,
            &mut attribute_budget,
        )?;
        rebound_cells.insert(cell.id.clone(), Some(tag_ref(node, &dict)));
        store.replace_dict(node, dict)?;
        let mut rebound_path = Vec::new();
        for (depth, &node) in path.iter().enumerate().skip(1) {
            let mut dict = store.dict(node)?;
            dict.insert("P", reference(path[depth - 1]));
            dict.remove("Pg");
            if depth + 1 < path.len() {
                dict.remove("ActualText");
            }
            reviewed_text(
                &mut dict,
                tags.content_paths[&cell.id][depth - 1]
                    .semantic_text
                    .as_ref(),
            );
            resolver.rewrite_flow(
                store,
                &mut dict,
                None,
                request.writing_mode,
                rtl,
                &mut attribute_budget,
            )?;
            rebound_path.push(CellTextOwner {
                source: tag_ref(node, &dict),
                semantic_text: None,
            });
            store.replace_dict(node, dict)?;
        }
        if !rebound_path.is_empty() {
            rebound_paths.insert(cell.id.clone(), rebound_path);
        }
        row_children[cell.row].push(reference(node));
    }
    let mut row_parents = BTreeMap::new();
    for group in &tags.groups {
        let node = group_targets
            .get_reference(&node_key(request, "row-group", &group.id, 0))
            .ok_or_else(|| fail("missing row group target"))?;
        for row in &group.rows {
            row_parents.insert(row.as_str(), node);
        }
    }
    let mut rebound_rows = BTreeMap::new();
    let mut children = Vec::new();
    for (row, kids) in layout.rows.iter().zip(row_children) {
        let key = row_key(request, &row.id);
        let id = row_targets
            .get_reference(&key)
            .ok_or_else(|| fail("missing final table row"))?;
        let mut dict = store.dict(id)?;
        dict.insert("K", PdfObject::Array(kids));
        dict.insert(
            "P",
            reference(row_parents.get(row.id.as_str()).copied().unwrap_or(table)),
        );
        dict.remove("Pg");
        dict.remove("ActualText");
        resolver.rewrite_flow(
            store,
            &mut dict,
            None,
            request.writing_mode,
            false,
            &mut attribute_budget,
        )?;
        reviewed_text(&mut dict, tags.row_text.get(&row.id));
        store.replace_dict(id, dict)?;
        children.push(reference(id));
        rebound_rows.insert(
            row.id.clone(),
            Some(TagReference {
                object: id.0,
                generation: id.1,
                key: Some(key),
            }),
        );
    }
    let mut rebound_groups = Vec::new();
    if !tags.groups.is_empty() {
        children.clear();
        for group in &tags.groups {
            let node = group_targets
                .get_reference(&node_key(request, "row-group", &group.id, 0))
                .ok_or_else(|| fail("missing final row group"))?;
            let mut dict = store.dict(node)?;
            let rows = group
                .rows
                .iter()
                .map(|id| {
                    row_targets
                        .get_reference(&row_key(request, id))
                        .map(reference)
                        .ok_or_else(|| fail("missing grouped row"))
                })
                .collect::<Result<Vec<_>>>()?;
            dict.insert("K", PdfObject::Array(rows));
            dict.insert("P", reference(table));
            dict.remove("Pg");
            dict.remove("ActualText");
            reviewed_text(&mut dict, group.semantic_text.as_ref());
            resolver.rewrite_flow(
                store,
                &mut dict,
                None,
                request.writing_mode,
                false,
                &mut attribute_budget,
            )?;
            rebound_groups.push(RowGroupBinding {
                id: group.id.clone(),
                role: group.role,
                rows: group.rows.clone(),
                source: Some(tag_ref(node, &dict)),
                semantic_text: None,
            });
            store.replace_dict(node, dict)?;
            children.push(reference(node));
        }
    }
    if let Some(caption) = txn.get("TableCaption").and_then(PdfObject::as_dict) {
        let owner = caption
            .get_reference("Owner")
            .ok_or_else(|| fail("missing preserved caption owner"))?;
        if store.dict(owner)?.get_reference("P") != Some(table) {
            return Err(fail("caption ownership changed inside transaction"));
        }
        if matches!(caption.get("First"), Some(PdfObject::Boolean(true))) {
            children.insert(0, reference(owner));
        } else {
            children.push(reference(owner));
        }
    }
    table_dict.insert("K", PdfObject::Array(children));
    table_dict.insert(
        "WFStoryID",
        PdfObject::String(story_key(request).into_bytes()),
    );
    table_dict.remove("Pg");
    table_dict.remove("ActualText");
    resolver.rewrite_flow(
        store,
        &mut table_dict,
        None,
        request.writing_mode,
        false,
        &mut attribute_budget,
    )?;
    reviewed_text(&mut table_dict, tags.table_text.as_ref());
    store.replace_dict(table, table_dict)?;
    Ok(TableTagging {
        source: TagReference {
            object: table.0,
            generation: table.1,
            key: Some(story_key(request)),
        },
        rows: rebound_rows,
        cells: rebound_cells,
        semantics: tags.semantics.clone(),
        content_paths: rebound_paths,
        blocks: rebound_blocks,
        groups: rebound_groups,
        removed_groups: Vec::new(),
        semantic_text: BTreeMap::new(),
        row_text: BTreeMap::new(),
        table_text: None,
    })
}
