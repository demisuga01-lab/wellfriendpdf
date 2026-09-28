//! Source table hierarchy and explicit reuse plan. Stable row/cell identities
//! are distinct from the leaf structure owners which receive generated MCRs.
use super::*;

pub(super) struct Tree {
    pub rows: BTreeSet<ObjectRef>,
    pub groups: BTreeSet<ObjectRef>,
    pub paths: BTreeMap<ObjectRef, Vec<ObjectRef>>,
    pub cell_paths: BTreeMap<ObjectRef, Vec<Vec<ObjectRef>>>,
    pub leaves: BTreeSet<ObjectRef>,
    pub nodes: BTreeSet<ObjectRef>,
    pub caption: Option<(ObjectRef, bool)>,
}
impl Tree {
    pub fn read(
        store: &Store<'_>,
        index: &StructureIndex,
        lookup: &Lookup,
        table: ObjectRef,
        request: &LinkedStoryRequest,
    ) -> Result<Self> {
        let mut tree = Self {
            rows: BTreeSet::new(),
            groups: BTreeSet::new(),
            paths: BTreeMap::new(),
            cell_paths: BTreeMap::new(),
            leaves: BTreeSet::new(),
            nodes: BTreeSet::from([table]),
            caption: None,
        };
        let mut parents = Vec::new();
        let mut phase = 0;
        let mut grouped = None;
        let children = kids(&store.dict(table)?);
        for (position, child) in children.iter().enumerate() {
            let child = child
                .as_reference()
                .filter(|id| index.nodes.contains(id))
                .ok_or_else(|| fail("table child is not a structure owner"))?;
            let d = store.dict(child)?;
            if d.get_reference("P") != Some(table) {
                return Err(fail("table child has a different parent"));
            }
            if d.get_name("S") == Some("Caption") {
                if tree.caption.is_some() || position != 0 && position + 1 != children.len() {
                    return Err(fail("table caption must be a single first/last child"));
                }
                // Opaque, unselected content: preserve its complete subtree and
                // page association, not just the caption's extracted wording.
                tree.caption = Some((child, position == 0));
                continue;
            }
            owner_check(store, child, request)?;
            match d.get_name("S") {
                Some("TR") if grouped != Some(true) => {
                    grouped = Some(false);
                    parents.push((child, table));
                }
                Some(role @ ("THead" | "TBody" | "TFoot")) if grouped != Some(false) => {
                    grouped = Some(true);
                    if !tree.nodes.insert(child)
                        || !tree.groups.insert(child)
                        || tree.groups.len() > 4096
                    {
                        return Err(fail("repeated/excessive table row groups"));
                    }
                    match role {
                        "THead" if phase == 0 => phase = 1,
                        "TBody" if phase <= 2 => phase = 2,
                        "TFoot" if phase == 2 => phase = 3,
                        _ => return Err(fail("table row groups have invalid order")),
                    }
                    for row in kids(&d) {
                        let row = row
                            .as_reference()
                            .filter(|id| index.nodes.contains(id))
                            .ok_or_else(|| fail("table row group owns a non-row item"))?;
                        parents.push((row, child));
                    }
                }
                _ => {
                    return Err(fail(
                        "table needs rows or THead/TBody/TFoot groups; mixed row/group hierarchies need an explicit model",
                    ));
                }
            }
        }
        if grouped == Some(true) && phase < 2 {
            return Err(fail("grouped table requires a body group"));
        }
        let mut path_slots = 0usize;
        for (row, parent) in parents {
            let d = owner_check(store, row, request)?;
            if d.get_name("S") != Some("TR")
                || d.get_reference("P") != Some(parent)
                || !tree.rows.insert(row)
                || !tree.nodes.insert(row)
                || tree.rows.len() > 4096
            {
                return Err(fail("invalid table row ownership/budget"));
            }
            for cell in kids(&d) {
                let cell = cell
                    .as_reference()
                    .filter(|id| index.nodes.contains(id))
                    .ok_or_else(|| fail("TR child is not a structure cell"))?;
                let d = owner_check(store, cell, request)?;
                if !matches!(d.get_name("S"), Some("TH" | "TD"))
                    || d.get_reference("P") != Some(row)
                {
                    return Err(fail("invalid table cell role/parent"));
                }
                let mut paths = Vec::new();
                let mut stack = vec![(cell, Vec::new())];
                while let Some((current, mut path)) = stack.pop() {
                    crate::cancel::check_current_cancel("table nested text ownership")?;
                    if !tree.nodes.insert(current)
                        || tree.nodes.len() > MAX_NODES
                        || path.len() > 128
                    {
                        return Err(fail(
                            "cyclic/repeated table descendants or depth budget exceeded",
                        ));
                    }
                    path.push(current);
                    let d = owner_check(store, current, request)?;
                    if current != cell
                        && matches!(
                            d.get_name("S"),
                            Some("Table" | "TR" | "TH" | "TD" | "THead" | "TBody" | "TFoot")
                        )
                    {
                        return Err(fail(
                            "nested table owners require a nested table layout, not paragraph flattening",
                        ));
                    }
                    let children = kids(&d)
                        .into_iter()
                        .filter_map(|kid| kid.as_reference().filter(|id| index.nodes.contains(id)))
                        .collect::<Vec<_>>();
                    if children.is_empty() {
                        if lookup.non_leaf.contains(&current) {
                            return Err(fail("cell leaf includes Form/OBJR ownership"));
                        }
                        path_slots = path_slots.saturating_add(path.len());
                        if path_slots > MAX_NODES || tree.leaves.len() >= 16_384 {
                            return Err(fail("table text path budget exceeded"));
                        }
                        tree.leaves.insert(current);
                        paths.push(path);
                        continue;
                    }
                    if children.len() != kids(&d).len() {
                        return Err(fail(
                            "cell container mixes structural children with paint ownership",
                        ));
                    }
                    for next in children.into_iter().rev() {
                        if store.dict(next)?.get_reference("P") != Some(current) {
                            return Err(fail("cell text path has a different parent"));
                        }
                        if stack.len() >= MAX_NODES {
                            return Err(fail("table structure traversal budget exceeded"));
                        }
                        stack.push((next, path.clone()));
                    }
                }
                if paths.len() == 1 {
                    tree.paths.insert(cell, paths[0].clone());
                }
                if tree.cell_paths.insert(cell, paths).is_some() || tree.cell_paths.len() > 4096 {
                    return Err(fail("repeated table cell or cell budget exceeded"));
                }
            }
        }
        if index.marked.values().any(|items| {
            items
                .values()
                .any(|node| tree.nodes.contains(node) && !tree.leaves.contains(node))
        }) || index.objects.values().any(|node| tree.nodes.contains(node))
        {
            return Err(fail(
                "table containers mix descendant text with paint/annotation ownership",
            ));
        }
        let resolver = attributes::Resolver::new(store)?;
        let mut budget = attributes::Budget::default();
        for node in &tree.nodes {
            resolver.entries(store, &store.dict(*node)?, &mut budget)?;
        }
        Ok(tree)
    }
}
pub(super) fn validate_groups(request: &LinkedStoryRequest) -> Result<()> {
    let tags = config(request).unwrap();
    let layout = request.table_layout.as_ref().unwrap();
    if tags.groups.is_empty() {
        return Ok(());
    }
    let mut seen = BTreeSet::new();
    let mut cursor: usize = 0;
    let mut phase = 0;
    for group in &tags.groups {
        if group.id.is_empty() || !seen.insert(&group.id) || group.rows.is_empty() {
            return Err(fail("invalid table row group identity/size"));
        }
        match group.role {
            RowGroupRole::THead if phase == 0 => phase = 1,
            RowGroupRole::TBody if phase <= 2 => phase = 2,
            RowGroupRole::TFoot if phase == 2 => phase = 3,
            _ => {
                return Err(fail(
                    "approved row groups require optional head, bodies, optional foot",
                ));
            }
        }
        let end = cursor
            .checked_add(group.rows.len())
            .ok_or_else(|| fail("row group size overflow"))?;
        if end > layout.rows.len()
            || layout.rows[cursor..end]
                .iter()
                .map(|r| &r.id)
                .ne(group.rows.iter())
        {
            return Err(fail(
                "table row groups must partition approved rows in grid order",
            ));
        }
        if group.role == RowGroupRole::THead && end != layout.header_rows {
            return Err(fail(
                "THead rows must match the approved repeated header prefix",
            ));
        }
        if layout.cells.iter().any(|cell| {
            cell.row >= cursor
                && cell.row < end
                && cell
                    .row
                    .checked_add(cell.row_span)
                    .is_none_or(|last| last > end)
        }) {
            return Err(fail("cell span crosses a semantic row group"));
        }
        cursor = end;
    }
    if cursor != layout.rows.len() || phase < 2 {
        return Err(fail("row groups do not cover the complete table/body"));
    }
    Ok(())
}

pub(super) struct Plan {
    pub tree: Tree,
    pub cells: BTreeMap<String, Vec<ObjectRef>>,
    pub rows: BTreeMap<String, ObjectRef>,
    pub groups: BTreeMap<String, ObjectRef>,
    pub blocks: BTreeMap<String, blocks::CellPlan>,
}
pub(super) fn plan(
    store: &Store<'_>,
    root: ObjectRef,
    index: &StructureIndex,
    lookup: &Lookup,
    table: ObjectRef,
    request: &LinkedStoryRequest,
) -> Result<Plan> {
    let tags = config(request).unwrap();
    let tree = Tree::read(store, index, lookup, table, request)?;
    let mut used = BTreeSet::from([table]);
    let mut rows = BTreeMap::new();
    let mut cells = BTreeMap::new();
    let mut groups = BTreeMap::new();
    let mut block_cells = BTreeMap::new();
    for (id, binding) in &tags.rows {
        if let Some(binding) = binding {
            let node = resolve_tag(root, index, lookup, binding)?;
            if !tree.rows.contains(&node) || !used.insert(node) {
                return Err(fail("row reuse is not unique/selected"));
            }
            semantic_review(store, &store.dict(node)?, tags.row_text.get(id))?;
            rows.insert(id.clone(), node);
        }
    }
    for group in &tags.groups {
        if let Some(binding) = &group.source {
            let node = resolve_tag(root, index, lookup, binding)?;
            if !tree.groups.contains(&node) || !used.insert(node) {
                return Err(fail("row-group reuse is not unique/selected"));
            }
            let d = store.dict(node)?;
            if d.get_name("S") != Some(group.role.name()) {
                return Err(fail("reused row-group role differs"));
            }
            semantic_review(store, &d, group.semantic_text.as_ref())?;
            groups.insert(group.id.clone(), node);
        }
    }
    let mut removed_groups = BTreeSet::new();
    for binding in &tags.removed_groups {
        let node = resolve_tag(root, index, lookup, binding)?;
        if !tree.groups.contains(&node) || used.contains(&node) || !removed_groups.insert(node) {
            return Err(fail("invalid row-group removal decision"));
        }
    }
    if tree
        .groups
        .iter()
        .any(|node| !used.contains(node) && !removed_groups.contains(node))
    {
        return Err(fail(
            "every imported row group requires explicit reuse or removal; no implicit flattening",
        ));
    }
    for cell in &request.table_layout.as_ref().unwrap().cells {
        let id = &cell.id;
        let binding = &tags.cells[id];
        if blocks::active(tags, cell) {
            let plan =
                blocks::plan_cell(store, root, index, lookup, request, cell, &tree, &mut used)?;
            block_cells.insert(id.clone(), plan);
            continue;
        }
        let approved = tags.content_paths.get(id).map(Vec::as_slice).unwrap_or(&[]);
        let Some(binding) = binding else {
            if !approved.is_empty() {
                return Err(fail("new cells cannot reuse an existing descendant path"));
            }
            continue;
        };
        let node = resolve_tag(root, index, lookup, binding)?;
        let path = tree
            .paths
            .get(&node)
            .ok_or_else(|| fail("selected cell is outside source table"))?;
        if path.len() != approved.len() + 1 {
            return Err(fail(
                "nested cell text needs its complete approved descendant path",
            ));
        }
        let d = store.dict(node)?;
        if d.get_name("S") != Some(tags.semantics[id].role.name()) {
            return Err(fail(
                "reused cell role differs; migrate external relationships explicitly",
            ));
        }
        semantic_review(store, &d, tags.semantic_text.get(id))?;
        for (node, decision) in path.iter().skip(1).zip(approved) {
            if resolve_tag(root, index, lookup, &decision.source)? != *node {
                return Err(fail(
                    "approved cell text path differs from actual hierarchy",
                ));
            }
            semantic_review(store, &store.dict(*node)?, decision.semantic_text.as_ref())?;
        }
        for node in path {
            if !used.insert(*node) {
                return Err(fail("a cell text owner is reused more than once"));
            }
        }
        cells.insert(id.clone(), path.clone());
    }
    let removed = tree.nodes.difference(&used).copied().collect();
    ensure_unreferenced_removals(store, index, &removed)?;
    Ok(Plan {
        tree,
        cells,
        rows,
        groups,
        blocks: block_cells,
    })
}
