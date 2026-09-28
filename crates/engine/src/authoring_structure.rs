//! Minimal canonical structure-tree planning for freshly authored content.
//! Structure identities are stable during layout; MCIDs and PDF object numbers
//! are assigned only from the final materialized page sequence.
use super::*;

#[cfg(test)]
#[path = "authoring_structure_tests.rs"]
mod tests;

const MAX_STRUCTURE_ELEMENTS: usize = 100_000;
const MAX_STRUCTURE_TITLE_BYTES: usize = 16 * 1024;
const MAX_NUMBER_TREE_NODE_ENTRIES: usize = 64;
const MAX_TYPED_OWNER_SERIALIZED_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Role {
    Toc,
    Toci,
    Index,
    Paragraph,
    Heading,
    Heading1,
    Heading2,
    Heading3,
    Heading4,
    Heading5,
    Heading6,
    List,
    ListItem,
    ListLabel,
    ListBody,
    Figure,
    Table,
    Caption,
    TableHead,
    TableBody,
    TableRow,
    TableHeader,
    TableData,
    Span,
    Reference,
    Note,
}

impl Role {
    fn pdf_name(self) -> &'static str {
        match self {
            Self::Toc => "TOC",
            Self::Toci => "TOCI",
            Self::Index => "Index",
            Self::Paragraph => "P",
            Self::Heading => "H",
            Self::Heading1 => "H1",
            Self::Heading2 => "H2",
            Self::Heading3 => "H3",
            Self::Heading4 => "H4",
            Self::Heading5 => "H5",
            Self::Heading6 => "H6",
            Self::List => "L",
            Self::ListItem => "LI",
            Self::ListLabel => "Lbl",
            Self::ListBody => "LBody",
            Self::Figure => "Figure",
            Self::Table => "Table",
            Self::Caption => "Caption",
            Self::TableHead => "THead",
            Self::TableBody => "TBody",
            Self::TableRow => "TR",
            Self::TableHeader => "TH",
            Self::TableData => "TD",
            Self::Span => "Span",
            Self::Reference => "Reference",
            Self::Note => "Note",
        }
    }

    pub(super) fn heading(level: u8) -> Self {
        match level {
            1 => Self::Heading1,
            2 => Self::Heading2,
            3 => Self::Heading3,
            4 => Self::Heading4,
            5 => Self::Heading5,
            6 => Self::Heading6,
            _ => Self::Heading,
        }
    }

    fn may_be_container(self) -> bool {
        matches!(
            self,
            Self::Toc
                | Self::Index
                | Self::List
                | Self::ListItem
                | Self::Table
                | Self::TableHead
                | Self::TableBody
                | Self::TableRow
        )
    }
}

fn element_id(element: &Element) -> Option<Vec<u8>> {
    match element.role {
        Role::Note => Some(format!("WFNote-{}", element.id).into_bytes()),
        Role::TableHeader => Some(format!("WFTH-{}", element.id).into_bytes()),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub(super) struct Element {
    pub(super) id: u64,
    role: Role,
    parent: Option<u64>,
    title: Option<String>,
    alternate_text: Option<String>,
    allow_empty: bool,
    references: Vec<u64>,
    table_summary: Option<String>,
    table_scope: Option<TableHeaderScope>,
    table_headers: Vec<u64>,
    table_column_span: Option<usize>,
    table_row_span: Option<usize>,
    pub(super) typed_table_identity: Option<String>,
    pub(super) typed_cell_identity: Option<String>,
}

pub(super) fn register(
    builder: &mut PdfBuilder,
    role: Role,
    parent: Option<u64>,
    title: Option<String>,
) -> Result<u64> {
    register_full(builder, role, parent, title, None, false)
}

pub(super) fn register_figure(builder: &mut PdfBuilder, alternate_text: String) -> Result<u64> {
    register_full(
        builder,
        Role::Figure,
        None,
        None,
        Some(alternate_text),
        false,
    )
}

pub(super) fn register_table_cell(
    builder: &mut PdfBuilder,
    role: Role,
    parent: u64,
    allow_empty: bool,
) -> Result<u64> {
    if !matches!(role, Role::TableHeader | Role::TableData) {
        return Err(WellfriendError::invalid_input(
            "authored table cell requires a TH or TD role",
        ));
    }
    register_full(builder, role, Some(parent), None, None, allow_empty)
}

fn register_full(
    builder: &mut PdfBuilder,
    role: Role,
    parent: Option<u64>,
    title: Option<String>,
    alternate_text: Option<String>,
    allow_empty: bool,
) -> Result<u64> {
    if builder.structures.len() >= MAX_STRUCTURE_ELEMENTS {
        return Err(WellfriendError::ResourceLimit(
            "authored structure element count".into(),
        ));
    }
    if let Some(parent) = parent {
        if !builder
            .structures
            .iter()
            .any(|element| element.id == parent)
        {
            return Err(WellfriendError::invalid_input(
                "authored structure parent must be registered first",
            ));
        }
    }
    if let Some(title) = title.as_deref() {
        if title.is_empty()
            || title.len() > MAX_STRUCTURE_TITLE_BYTES
            || title.chars().any(|ch| ch == '\0')
        {
            return Err(WellfriendError::invalid_input(
                "authored structure title must be bounded and nonempty",
            ));
        }
    }
    if let Some(alternate_text) = alternate_text.as_deref() {
        if alternate_text.is_empty()
            || alternate_text.len() > MAX_STRUCTURE_TITLE_BYTES
            || alternate_text.chars().any(|ch| ch == '\0')
        {
            return Err(WellfriendError::invalid_input(
                "authored figure alternate text must be bounded and nonempty",
            ));
        }
    }
    let id = builder.next_structure_id;
    builder.next_structure_id = id.checked_add(1).ok_or_else(|| {
        WellfriendError::ResourceLimit("authored structure identity overflow".into())
    })?;
    builder.structures.push(Element {
        id,
        role,
        parent,
        title,
        alternate_text,
        allow_empty,
        references: Vec::new(),
        table_summary: None,
        table_scope: None,
        table_headers: Vec::new(),
        table_column_span: None,
        table_row_span: None,
        typed_table_identity: None,
        typed_cell_identity: None,
    });
    Ok(id)
}

pub(super) fn register_table(builder: &mut PdfBuilder, summary: Option<&str>) -> Result<u64> {
    let table = register(builder, Role::Table, None, None)?;
    if let Some(summary) = summary {
        if summary.is_empty()
            || summary.len() > MAX_STRUCTURE_TITLE_BYTES
            || summary.chars().any(|ch| ch == '\0')
        {
            return Err(WellfriendError::invalid_input(
                "authored table summary must be bounded and nonempty",
            ));
        }
        builder
            .structures
            .iter_mut()
            .find(|element| element.id == table)
            .expect("registered table")
            .table_summary = Some(summary.to_owned());
    }
    Ok(table)
}

pub(super) fn register_caption(builder: &mut PdfBuilder, table: u64) -> Result<u64> {
    if !builder
        .structures
        .iter()
        .any(|element| element.id == table && element.role == Role::Table)
    {
        return Err(WellfriendError::invalid_input(
            "authored table caption requires a Table owner",
        ));
    }
    register(builder, Role::Caption, Some(table), None)
}

pub(super) fn register_table_group(
    builder: &mut PdfBuilder,
    table: u64,
    role: Role,
) -> Result<u64> {
    if !matches!(role, Role::TableHead | Role::TableBody)
        || !builder
            .structures
            .iter()
            .any(|element| element.id == table && element.role == Role::Table)
    {
        return Err(WellfriendError::invalid_input(
            "authored table group requires THead/TBody and a Table owner",
        ));
    }
    register(builder, role, Some(table), None)
}

pub(super) fn set_table_header_scope(
    builder: &mut PdfBuilder,
    cell: u64,
    scope: TableHeaderScope,
) -> Result<()> {
    let element = builder
        .structures
        .iter_mut()
        .find(|element| element.id == cell)
        .ok_or_else(|| WellfriendError::invalid_input("unknown authored table header"))?;
    if element.role != Role::TableHeader {
        return Err(WellfriendError::invalid_input(
            "table header scope requires a TH element",
        ));
    }
    element.table_scope = Some(scope);
    Ok(())
}

pub(super) fn set_table_column_span(
    builder: &mut PdfBuilder,
    cell: u64,
    columns: usize,
) -> Result<()> {
    let element = builder
        .structures
        .iter_mut()
        .find(|element| element.id == cell)
        .ok_or_else(|| WellfriendError::invalid_input("unknown authored table cell"))?;
    if !matches!(element.role, Role::TableHeader | Role::TableData)
        || !(2..=10_000).contains(&columns)
    {
        return Err(WellfriendError::invalid_input(
            "table column span requires a TH/TD element and at least two bounded columns",
        ));
    }
    element.table_column_span = Some(columns);
    Ok(())
}

pub(super) fn set_table_row_span(builder: &mut PdfBuilder, cell: u64, rows: usize) -> Result<()> {
    let element = builder
        .structures
        .iter_mut()
        .find(|element| element.id == cell)
        .ok_or_else(|| WellfriendError::invalid_input("unknown authored table cell"))?;
    if !matches!(element.role, Role::TableHeader | Role::TableData)
        || !(2..=100_000).contains(&rows)
    {
        return Err(WellfriendError::invalid_input(
            "table row span requires a TH/TD element and at least two bounded rows",
        ));
    }
    element.table_row_span = Some(rows);
    Ok(())
}

/// Bind an authored typed value to the exact structure owner that carries its
/// visible PDF content. The pair is repeated on every marked-content fragment
/// of the same element, so a split cell remains one logical mutation target.
pub(super) fn set_typed_table_cell_owner(
    builder: &mut PdfBuilder,
    cell: u64,
    table_identity: &str,
    cell_identity: &str,
) -> Result<()> {
    let valid = |identity: &str| {
        !identity.is_empty()
            && identity.len() <= MAX_STRUCTURE_TITLE_BYTES
            && !identity.chars().any(|ch| ch == '\0')
    };
    if !valid(table_identity) || !valid(cell_identity) {
        return Err(WellfriendError::invalid_input(
            "typed table/cell ownership identities must be bounded and nonempty",
        ));
    }
    if builder.structures.iter().any(|element| {
        element.id != cell
            && element.typed_table_identity.as_deref() == Some(table_identity)
            && element.typed_cell_identity.as_deref() == Some(cell_identity)
    }) {
        return Err(WellfriendError::invalid_input(
            "typed table/cell ownership identities must be unique",
        ));
    }
    let element = builder
        .structures
        .iter_mut()
        .find(|element| element.id == cell)
        .ok_or_else(|| WellfriendError::invalid_input("unknown authored typed table cell"))?;
    if !matches!(element.role, Role::TableHeader | Role::TableData)
        || element.typed_table_identity.is_some()
        || element.typed_cell_identity.is_some()
    {
        return Err(WellfriendError::invalid_input(
            "typed table ownership requires one unbound TH/TD element",
        ));
    }
    element.typed_table_identity = Some(table_identity.to_owned());
    element.typed_cell_identity = Some(cell_identity.to_owned());
    Ok(())
}

pub(super) fn set_table_headers(
    builder: &mut PdfBuilder,
    cell: u64,
    headers: Vec<u64>,
) -> Result<()> {
    let unique = headers.iter().copied().collect::<BTreeSet<_>>();
    if unique.len() != headers.len()
        || headers.iter().any(|header| {
            !builder
                .structures
                .iter()
                .any(|element| element.id == *header && element.role == Role::TableHeader)
        })
    {
        return Err(WellfriendError::invalid_input(
            "table cell headers must be unique live TH elements",
        ));
    }
    let element = builder
        .structures
        .iter_mut()
        .find(|element| element.id == cell)
        .ok_or_else(|| WellfriendError::invalid_input("unknown authored table data cell"))?;
    if element.role != Role::TableData {
        return Err(WellfriendError::invalid_input(
            "explicit table headers require a TD element",
        ));
    }
    element.table_headers = headers;
    Ok(())
}

/// Register an exact logical footnote marker below its paragraph and connect
/// it bidirectionally to the corresponding Note structure element. `/Ref`
/// relationships are emitted only after every target survives final planning.
pub(super) fn register_reference(
    builder: &mut PdfBuilder,
    paragraph: u64,
    note: u64,
) -> Result<u64> {
    let paragraph_role = builder
        .structures
        .iter()
        .find(|element| element.id == paragraph)
        .map(|element| element.role);
    let note_role = builder
        .structures
        .iter()
        .find(|element| element.id == note)
        .map(|element| element.role);
    if paragraph_role != Some(Role::Paragraph) || note_role != Some(Role::Note) {
        return Err(WellfriendError::invalid_input(
            "authored footnote reference requires a paragraph and Note owner",
        ));
    }
    let reference = register(builder, Role::Reference, Some(paragraph), None)?;
    relate(builder, reference, note)?;
    Ok(reference)
}

pub(super) fn register_span(builder: &mut PdfBuilder, paragraph: u64) -> Result<u64> {
    if !builder
        .structures
        .iter()
        .any(|element| element.id == paragraph && element.role == Role::Paragraph)
    {
        return Err(WellfriendError::invalid_input(
            "authored text span requires a paragraph owner",
        ));
    }
    register(builder, Role::Span, Some(paragraph), None)
}

fn relate(builder: &mut PdfBuilder, left: u64, right: u64) -> Result<()> {
    if left == right {
        return Err(WellfriendError::invalid_input(
            "authored structure relationship cannot target itself",
        ));
    }
    let left_index = builder
        .structures
        .iter()
        .position(|element| element.id == left)
        .ok_or_else(|| WellfriendError::invalid_input("unknown authored relationship source"))?;
    let right_index = builder
        .structures
        .iter()
        .position(|element| element.id == right)
        .ok_or_else(|| WellfriendError::invalid_input("unknown authored relationship target"))?;
    if builder.structures[left_index].references.len() >= MAX_STRUCTURE_ELEMENTS
        || builder.structures[right_index].references.len() >= MAX_STRUCTURE_ELEMENTS
    {
        return Err(WellfriendError::ResourceLimit(
            "authored structure relationship count".into(),
        ));
    }
    builder.structures[left_index].references.push(right);
    builder.structures[right_index].references.push(left);
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct Mark {
    element: u64,
    mcid: u32,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct AnnotationMark {
    pub(super) element: u64,
    pub(super) page: usize,
    pub(super) object: u32,
    pub(super) struct_parent: u32,
}

pub(super) struct Built {
    pub(super) root: Option<u32>,
    pub(super) objects: Vec<OutputObject>,
    page_marks: Vec<Vec<Mark>>,
    roles: HashMap<u64, Role>,
    typed_cell_owners: HashMap<u64, (String, String)>,
}

#[derive(Debug, Clone, Copy)]
struct NumberTreeNode {
    number: u32,
    first: i64,
    last: i64,
}

#[derive(Debug, Clone)]
struct NameTreeNode {
    number: u32,
    first: Vec<u8>,
    last: Vec<u8>,
}

fn number_tree_limits(first: i64, last: i64) -> PdfObject {
    PdfObject::Array(vec![PdfObject::Integer(first), PdfObject::Integer(last)])
}

fn number_tree_leaf_dictionary(entries: &[(i64, PdfObject)]) -> PdfDictionary {
    let mut nums = Vec::with_capacity(entries.len() * 2);
    for (key, value) in entries {
        nums.push(PdfObject::Integer(*key));
        nums.push(value.clone());
    }
    let mut dictionary = dict(&[("Nums", PdfObject::Array(nums))]);
    if let (Some((first, _)), Some((last, _))) = (entries.first(), entries.last()) {
        dictionary.insert("Limits", number_tree_limits(*first, *last));
    }
    dictionary
}

fn number_tree_branch_dictionary(nodes: &[NumberTreeNode]) -> Result<PdfDictionary> {
    let Some(first) = nodes.first() else {
        return Err(WellfriendError::invalid_input(
            "authored ParentTree branch cannot be empty",
        ));
    };
    let last = nodes.last().unwrap();
    Ok(dict(&[
        (
            "Kids",
            PdfObject::Array(nodes.iter().map(|node| reference(node.number)).collect()),
        ),
        ("Limits", number_tree_limits(first.first, last.last)),
    ]))
}

fn build_number_tree(
    root: u32,
    entries: &[(i64, PdfObject)],
    next: &mut u32,
) -> Result<Vec<OutputObject>> {
    if entries.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
        return Err(WellfriendError::invalid_input(
            "authored ParentTree keys must be strictly increasing",
        ));
    }
    if entries.len() <= MAX_NUMBER_TREE_NODE_ENTRIES {
        return Ok(vec![OutputObject {
            number: root,
            object: PdfObject::Dictionary(number_tree_leaf_dictionary(entries)),
        }]);
    }

    let mut objects = Vec::new();
    let mut level = Vec::new();
    for chunk in entries.chunks(MAX_NUMBER_TREE_NODE_ENTRIES) {
        let number = alloc(next);
        objects.push(OutputObject {
            number,
            object: PdfObject::Dictionary(number_tree_leaf_dictionary(chunk)),
        });
        level.push(NumberTreeNode {
            number,
            first: chunk.first().unwrap().0,
            last: chunk.last().unwrap().0,
        });
    }
    while level.len() > MAX_NUMBER_TREE_NODE_ENTRIES {
        let mut parent_level = Vec::new();
        for chunk in level.chunks(MAX_NUMBER_TREE_NODE_ENTRIES) {
            let number = alloc(next);
            objects.push(OutputObject {
                number,
                object: PdfObject::Dictionary(number_tree_branch_dictionary(chunk)?),
            });
            parent_level.push(NumberTreeNode {
                number,
                first: chunk.first().unwrap().first,
                last: chunk.last().unwrap().last,
            });
        }
        level = parent_level;
    }
    objects.push(OutputObject {
        number: root,
        object: PdfObject::Dictionary(number_tree_branch_dictionary(&level)?),
    });
    Ok(objects)
}

fn name_tree_limits(first: &[u8], last: &[u8]) -> PdfObject {
    PdfObject::Array(vec![
        PdfObject::String(first.to_vec()),
        PdfObject::String(last.to_vec()),
    ])
}

fn name_tree_leaf_dictionary(entries: &[(Vec<u8>, PdfObject)]) -> PdfDictionary {
    let mut names = Vec::with_capacity(entries.len() * 2);
    for (key, value) in entries {
        names.push(PdfObject::String(key.clone()));
        names.push(value.clone());
    }
    let mut dictionary = dict(&[("Names", PdfObject::Array(names))]);
    if let (Some((first, _)), Some((last, _))) = (entries.first(), entries.last()) {
        dictionary.insert("Limits", name_tree_limits(first, last));
    }
    dictionary
}

fn name_tree_branch_dictionary(nodes: &[NameTreeNode]) -> Result<PdfDictionary> {
    let Some(first) = nodes.first() else {
        return Err(WellfriendError::invalid_input(
            "authored IDTree branch cannot be empty",
        ));
    };
    let last = nodes.last().unwrap();
    Ok(dict(&[
        (
            "Kids",
            PdfObject::Array(nodes.iter().map(|node| reference(node.number)).collect()),
        ),
        ("Limits", name_tree_limits(&first.first, &last.last)),
    ]))
}

fn build_name_tree(
    root: u32,
    entries: &[(Vec<u8>, PdfObject)],
    next: &mut u32,
) -> Result<Vec<OutputObject>> {
    if entries.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
        return Err(WellfriendError::invalid_input(
            "authored IDTree keys must be strictly increasing",
        ));
    }
    if entries.len() <= MAX_NUMBER_TREE_NODE_ENTRIES {
        return Ok(vec![OutputObject {
            number: root,
            object: PdfObject::Dictionary(name_tree_leaf_dictionary(entries)),
        }]);
    }

    let mut objects = Vec::new();
    let mut level = Vec::new();
    for chunk in entries.chunks(MAX_NUMBER_TREE_NODE_ENTRIES) {
        let number = alloc(next);
        objects.push(OutputObject {
            number,
            object: PdfObject::Dictionary(name_tree_leaf_dictionary(chunk)),
        });
        level.push(NameTreeNode {
            number,
            first: chunk.first().unwrap().0.clone(),
            last: chunk.last().unwrap().0.clone(),
        });
    }
    while level.len() > MAX_NUMBER_TREE_NODE_ENTRIES {
        let mut parent_level = Vec::new();
        for chunk in level.chunks(MAX_NUMBER_TREE_NODE_ENTRIES) {
            let number = alloc(next);
            objects.push(OutputObject {
                number,
                object: PdfObject::Dictionary(name_tree_branch_dictionary(chunk)?),
            });
            parent_level.push(NameTreeNode {
                number,
                first: chunk.first().unwrap().first.clone(),
                last: chunk.last().unwrap().last.clone(),
            });
        }
        level = parent_level;
    }
    objects.push(OutputObject {
        number: root,
        object: PdfObject::Dictionary(name_tree_branch_dictionary(&level)?),
    });
    Ok(objects)
}

impl Built {
    pub(super) fn untagged(page_count: usize) -> Self {
        Self {
            root: None,
            objects: Vec::new(),
            page_marks: vec![Vec::new(); page_count],
            roles: HashMap::new(),
            typed_cell_owners: HashMap::new(),
        }
    }

    pub(super) fn page_is_marked(&self, page: usize) -> bool {
        self.page_marks
            .get(page)
            .is_some_and(|marks| !marks.is_empty())
    }

    pub(super) fn role_name(&self, element: u64) -> Result<&'static str> {
        self.roles
            .get(&element)
            .copied()
            .map(Role::pdf_name)
            .ok_or_else(|| WellfriendError::invalid_input("unknown authored structure identity"))
    }

    pub(super) fn typed_cell_owner(&self, element: u64) -> Result<Option<(&str, &str)>> {
        if !self.roles.contains_key(&element) {
            return Err(WellfriendError::invalid_input(
                "unknown authored structure identity",
            ));
        }
        Ok(self
            .typed_cell_owners
            .get(&element)
            .map(|(table, cell)| (table.as_str(), cell.as_str())))
    }
}

fn scan_pages(builder: &PdfBuilder) -> Result<Vec<Vec<Mark>>> {
    let known = builder
        .structures
        .iter()
        .map(|element| element.id)
        .collect::<BTreeSet<_>>();
    let typed = builder
        .structures
        .iter()
        .filter(|element| {
            element.typed_table_identity.is_some() && element.typed_cell_identity.is_some()
        })
        .map(|element| element.id)
        .collect::<BTreeSet<_>>();
    let mut pages = Vec::with_capacity(builder.pages.len());
    for page in &builder.pages {
        let mut active = None;
        let mut artifact = false;
        let mut marks = Vec::new();
        for command in &page.commands {
            match command {
                PageCommand::BeginArtifact | PageCommand::BeginOwnedTableCellArtifact { .. } => {
                    if artifact || active.is_some() {
                        return Err(WellfriendError::invalid_input(
                            "nested or intersecting authored artifact scope",
                        ));
                    }
                    artifact = true;
                }
                PageCommand::EndArtifact => {
                    if !artifact {
                        return Err(WellfriendError::invalid_input(
                            "unbalanced authored artifact scope",
                        ));
                    }
                    artifact = false;
                }
                PageCommand::BeginStructure(element) => {
                    if artifact
                        || active.is_some()
                        || !known.contains(element)
                        || typed.contains(element)
                    {
                        return Err(WellfriendError::invalid_input(
                            "nested, intersecting, unknown or unbounded typed authored structure scope",
                        ));
                    }
                    let mcid = u32::try_from(marks.len()).map_err(|_| {
                        WellfriendError::ResourceLimit("authored page MCID count".into())
                    })?;
                    marks.push(Mark {
                        element: *element,
                        mcid,
                    });
                    active = Some(*element);
                }
                PageCommand::BeginTypedCellStructure { element, region } => {
                    if artifact
                        || active.is_some()
                        || !known.contains(element)
                        || !typed.contains(element)
                        || !region.iter().all(|value| value.is_finite())
                        || region[0] >= region[2]
                        || region[1] >= region[3]
                        || region[0] < -1e-7
                        || region[1] < -1e-7
                        || region[2] > page.size.width + 1e-7
                        || region[3] > page.size.height + 1e-7
                    {
                        return Err(WellfriendError::invalid_input(
                            "invalid bounded authored typed-cell structure scope",
                        ));
                    }
                    let mcid = u32::try_from(marks.len()).map_err(|_| {
                        WellfriendError::ResourceLimit("authored page MCID count".into())
                    })?;
                    marks.push(Mark {
                        element: *element,
                        mcid,
                    });
                    active = Some(*element);
                }
                PageCommand::EndStructure(element) => {
                    if active != Some(*element) {
                        return Err(WellfriendError::invalid_input(
                            "unbalanced authored structure scope",
                        ));
                    }
                    active = None;
                }
                _ => {
                    if !known.is_empty() && active.is_none() && !artifact {
                        return Err(WellfriendError::invalid_input(
                            "authored tagged document contains unowned page content",
                        ));
                    }
                }
            }
        }
        if active.is_some() || artifact {
            return Err(WellfriendError::invalid_input(
                "unclosed authored marked-content scope",
            ));
        }
        pages.push(marks);
    }
    Ok(pages)
}

pub(super) fn build(
    builder: &PdfBuilder,
    page_start: u32,
    next: &mut u32,
    annotation_marks: &[AnnotationMark],
    parent_tree_next_key: u32,
) -> Result<Built> {
    let page_marks = scan_pages(builder)?;
    if builder.structures.is_empty() {
        if page_marks.iter().any(|marks| !marks.is_empty()) || !annotation_marks.is_empty() {
            return Err(WellfriendError::invalid_input(
                "authored structure marks have no registry",
            ));
        }
        return Ok(Built::untagged(builder.pages.len()));
    }

    let page_parent_count = u32::try_from(builder.pages.len())
        .map_err(|_| WellfriendError::ResourceLimit("authored ParentTree page count".into()))?;
    if annotation_marks.iter().any(|mark| {
        mark.struct_parent < page_parent_count || mark.struct_parent >= parent_tree_next_key
    }) {
        return Err(WellfriendError::invalid_input(
            "authored annotation StructParent key is outside its reserved range",
        ));
    }
    let unique_annotation_keys = annotation_marks
        .iter()
        .map(|mark| mark.struct_parent)
        .collect::<BTreeSet<_>>();
    if unique_annotation_keys.len() != annotation_marks.len()
        || parent_tree_next_key
            != page_parent_count
                .checked_add(u32::try_from(annotation_marks.len()).map_err(|_| {
                    WellfriendError::ResourceLimit("authored annotation StructParent count".into())
                })?)
                .ok_or_else(|| {
                    WellfriendError::ResourceLimit("authored annotation StructParent range".into())
                })?
    {
        return Err(WellfriendError::invalid_input(
            "authored annotation StructParent keys must be unique and contiguous",
        ));
    }

    let root = alloc(next);
    let parent_tree = alloc(next);
    let id_tree = builder
        .structures
        .iter()
        .any(|element| element_id(element).is_some())
        .then(|| alloc(next));
    let mut element_numbers = HashMap::new();
    for element in &builder.structures {
        element_numbers.insert(element.id, alloc(next));
    }
    let roles = builder
        .structures
        .iter()
        .map(|element| (element.id, element.role))
        .collect::<HashMap<_, _>>();
    let typed_cell_owners = builder
        .structures
        .iter()
        .filter_map(|element| {
            element
                .typed_table_identity
                .as_ref()
                .zip(element.typed_cell_identity.as_ref())
                .map(|(table, cell)| (element.id, (table.clone(), cell.clone())))
        })
        .collect::<HashMap<_, _>>();
    let mut typed_owner_bytes = 0usize;
    for marks in &page_marks {
        for mark in marks {
            if let Some((table, cell)) = typed_cell_owners.get(&mark.element) {
                let bytes = table
                    .encode_utf16()
                    .count()
                    .checked_add(cell.encode_utf16().count())
                    .and_then(|units| units.checked_mul(4))
                    .and_then(|bytes| bytes.checked_add(256))
                    .ok_or_else(|| {
                        WellfriendError::ResourceLimit(
                            "authored typed ownership serialization bytes".into(),
                        )
                    })?;
                typed_owner_bytes = typed_owner_bytes.checked_add(bytes).ok_or_else(|| {
                    WellfriendError::ResourceLimit(
                        "authored typed ownership serialization bytes".into(),
                    )
                })?;
                if typed_owner_bytes > MAX_TYPED_OWNER_SERIALIZED_BYTES {
                    return Err(WellfriendError::ResourceLimit(
                        "authored typed ownership serialization exceeds 64 MiB".into(),
                    ));
                }
            }
        }
    }
    for mark in annotation_marks {
        if mark.page >= builder.pages.len() || !roles.contains_key(&mark.element) {
            return Err(WellfriendError::invalid_input(
                "authored annotation structure ownership is invalid",
            ));
        }
    }
    let mut children: HashMap<u64, Vec<u64>> = HashMap::new();
    let mut top = Vec::new();
    for element in &builder.structures {
        if let Some(parent) = element.parent {
            children.entry(parent).or_default().push(element.id);
        } else {
            top.push(element.id);
        }
    }
    let mut marks_by_element: HashMap<u64, Vec<(usize, u32)>> = HashMap::new();
    for (page, marks) in page_marks.iter().enumerate() {
        for mark in marks {
            marks_by_element
                .entry(mark.element)
                .or_default()
                .push((page, mark.mcid));
        }
    }
    let mut annotations_by_element: HashMap<u64, Vec<&AnnotationMark>> = HashMap::new();
    for mark in annotation_marks {
        annotations_by_element
            .entry(mark.element)
            .or_default()
            .push(mark);
    }
    let references_by_element = builder
        .structures
        .iter()
        .map(|element| (element.id, element.references.as_slice()))
        .collect::<HashMap<_, _>>();
    let parents = builder
        .structures
        .iter()
        .map(|element| (element.id, element.parent))
        .collect::<HashMap<_, _>>();
    let mut typed_owners = BTreeSet::new();
    for element in &builder.structures {
        let unique_references = element.references.iter().copied().collect::<BTreeSet<_>>();
        if unique_references.len() != element.references.len()
            || element.references.iter().any(|target| {
                !element_numbers.contains_key(target)
                    || !references_by_element
                        .get(target)
                        .is_some_and(|references| references.contains(&element.id))
            })
        {
            return Err(WellfriendError::invalid_input(
                "authored structure relationships must be unique, reciprocal and live",
            ));
        }
        if element.role == Role::Reference
            && (element.references.len() != 1
                || element
                    .parent
                    .and_then(|parent| roles.get(&parent).copied())
                    != Some(Role::Paragraph)
                || roles.get(&element.references[0]).copied() != Some(Role::Note))
        {
            return Err(WellfriendError::invalid_input(
                "authored Reference must be a paragraph child linked to one Note",
            ));
        }
        if element.role == Role::Note
            && element
                .references
                .iter()
                .any(|target| roles.get(target).copied() != Some(Role::Reference))
        {
            return Err(WellfriendError::invalid_input(
                "authored Note may only reference Reference elements",
            ));
        }
        if !matches!(element.role, Role::Reference | Role::Note) && !element.references.is_empty() {
            return Err(WellfriendError::invalid_input(
                "authored structure role does not permit /Ref relationships",
            ));
        }
        if (element.table_summary.is_some() && element.role != Role::Table)
            || element.table_scope.is_some() != (element.role == Role::TableHeader)
            || (!element.table_headers.is_empty() && element.role != Role::TableData)
            || (element.table_column_span.is_some()
                && !matches!(element.role, Role::TableHeader | Role::TableData))
            || (element.table_row_span.is_some()
                && !matches!(element.role, Role::TableHeader | Role::TableData))
            || (element.typed_table_identity.is_some() != element.typed_cell_identity.is_some())
            || (element.typed_table_identity.is_some()
                && !matches!(element.role, Role::TableHeader | Role::TableData))
        {
            return Err(WellfriendError::invalid_input(
                "authored table structure attributes are attached to the wrong role",
            ));
        }
        if let Some((table, cell)) = element
            .typed_table_identity
            .as_ref()
            .zip(element.typed_cell_identity.as_ref())
        {
            if table.is_empty()
                || table.len() > MAX_STRUCTURE_TITLE_BYTES
                || table.chars().any(|ch| ch == '\0')
                || cell.is_empty()
                || cell.len() > MAX_STRUCTURE_TITLE_BYTES
                || cell.chars().any(|ch| ch == '\0')
                || !typed_owners.insert((table.as_str(), cell.as_str()))
            {
                return Err(WellfriendError::invalid_input(
                    "authored typed table/cell structure ownership is invalid or duplicated",
                ));
            }
        }
        if element.role == Role::Caption
            && element
                .parent
                .and_then(|parent| roles.get(&parent).copied())
                != Some(Role::Table)
        {
            return Err(WellfriendError::invalid_input(
                "authored Caption must be a Table child",
            ));
        }
        if matches!(element.role, Role::TableHead | Role::TableBody)
            && element
                .parent
                .and_then(|parent| roles.get(&parent).copied())
                != Some(Role::Table)
        {
            return Err(WellfriendError::invalid_input(
                "authored table row group must be a Table child",
            ));
        }
        if element.role == Role::TableRow
            && !element
                .parent
                .and_then(|parent| roles.get(&parent).copied())
                .is_some_and(|role| matches!(role, Role::TableHead | Role::TableBody))
        {
            return Err(WellfriendError::invalid_input(
                "authored TR must be owned by THead or TBody",
            ));
        }
        if matches!(element.role, Role::TableHeader | Role::TableData) {
            let row = element.parent.ok_or_else(|| {
                WellfriendError::invalid_input("authored table cell has no row parent")
            })?;
            let group = parents
                .get(&row)
                .copied()
                .flatten()
                .filter(|group| {
                    roles
                        .get(group)
                        .is_some_and(|role| matches!(role, Role::TableHead | Role::TableBody))
                })
                .ok_or_else(|| {
                    WellfriendError::invalid_input("authored table cell has no row-group ancestor")
                })?;
            let table = parents
                .get(&group)
                .copied()
                .flatten()
                .filter(|table| roles.get(table).copied() == Some(Role::Table))
                .ok_or_else(|| {
                    WellfriendError::invalid_input("authored table cell has no Table ancestor")
                })?;
            for header in &element.table_headers {
                let header_row = parents.get(header).copied().flatten().ok_or_else(|| {
                    WellfriendError::invalid_input("authored table header has no row parent")
                })?;
                let header_group = parents.get(&header_row).copied().flatten();
                if header_group.and_then(|group| parents.get(&group).copied().flatten())
                    != Some(table)
                {
                    return Err(WellfriendError::invalid_input(
                        "authored data-cell header belongs to another Table",
                    ));
                }
            }
        }
        if !element.role.may_be_container()
            && !element.allow_empty
            && !marks_by_element.contains_key(&element.id)
            && !annotations_by_element.contains_key(&element.id)
            && !children.contains_key(&element.id)
        {
            return Err(WellfriendError::invalid_input(
                "authored leaf structure element owns no marked content",
            ));
        }
    }

    let mut objects = Vec::new();
    let mut root_dict = dict(&[
        ("Type", PdfObject::Name("StructTreeRoot".into())),
        (
            "K",
            PdfObject::Array(
                top.iter()
                    .map(|id| reference(element_numbers[id]))
                    .collect(),
            ),
        ),
        ("ParentTree", reference(parent_tree)),
        (
            "ParentTreeNextKey",
            PdfObject::Integer(i64::from(parent_tree_next_key)),
        ),
    ]);
    root_dict.insert("RoleMap", PdfObject::Dictionary(PdfDictionary::empty()));
    if let Some(id_tree) = id_tree {
        root_dict.insert("IDTree", reference(id_tree));
    }
    objects.push(OutputObject {
        number: root,
        object: PdfObject::Dictionary(root_dict),
    });

    let mut parent_entries = Vec::new();
    for (page, marks) in page_marks.iter().enumerate() {
        if marks.is_empty() {
            continue;
        }
        parent_entries.push((
            i64::try_from(page).map_err(|_| {
                WellfriendError::ResourceLimit("authored ParentTree page key".into())
            })?,
            PdfObject::Array(
                marks
                    .iter()
                    .map(|mark| reference(element_numbers[&mark.element]))
                    .collect(),
            ),
        ));
    }
    for mark in annotation_marks {
        parent_entries.push((
            i64::from(mark.struct_parent),
            reference(element_numbers[&mark.element]),
        ));
    }
    objects.extend(build_number_tree(parent_tree, &parent_entries, next)?);
    if let Some(id_tree) = id_tree {
        let mut entries = builder
            .structures
            .iter()
            .filter_map(|element| {
                element_id(element).map(|id| (id, reference(element_numbers[&element.id])))
            })
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        objects.extend(build_name_tree(id_tree, &entries, next)?);
    }

    for element in &builder.structures {
        let parent = element
            .parent
            .map(|id| element_numbers[&id])
            .unwrap_or(root);
        let mut kids = Vec::new();
        if let Some(child_ids) = children.get(&element.id) {
            kids.extend(child_ids.iter().map(|id| reference(element_numbers[id])));
        }
        if let Some(marks) = marks_by_element.get(&element.id) {
            for (page, mcid) in marks {
                let page_number = page_start
                    .checked_add(u32::try_from(*page).map_err(|_| {
                        WellfriendError::ResourceLimit("authored structure page index".into())
                    })?)
                    .ok_or_else(|| {
                        WellfriendError::ResourceLimit("authored structure page number".into())
                    })?;
                kids.push(PdfObject::Dictionary(dict(&[
                    ("Type", PdfObject::Name("MCR".into())),
                    ("Pg", reference(page_number)),
                    ("MCID", PdfObject::Integer(i64::from(*mcid))),
                ])));
            }
        }
        if let Some(annotation_marks) = annotations_by_element.get(&element.id) {
            for mark in annotation_marks {
                let page_number = page_start
                    .checked_add(u32::try_from(mark.page).map_err(|_| {
                        WellfriendError::ResourceLimit(
                            "authored annotation structure page index".into(),
                        )
                    })?)
                    .ok_or_else(|| {
                        WellfriendError::ResourceLimit(
                            "authored annotation structure page number".into(),
                        )
                    })?;
                kids.push(PdfObject::Dictionary(dict(&[
                    ("Type", PdfObject::Name("OBJR".into())),
                    ("Obj", reference(mark.object)),
                    ("Pg", reference(page_number)),
                ])));
            }
        }
        let kid = if kids.len() == 1 {
            kids.pop().unwrap()
        } else {
            PdfObject::Array(kids)
        };
        let mut dictionary = dict(&[
            ("Type", PdfObject::Name("StructElem".into())),
            ("S", PdfObject::Name(element.role.pdf_name().into())),
            ("P", reference(parent)),
            ("K", kid),
        ]);
        if let Some(title) = &element.title {
            dictionary.insert("T", PdfObject::String(pdf_text_string(title)));
        }
        if let Some(alternate_text) = &element.alternate_text {
            dictionary.insert("Alt", PdfObject::String(pdf_text_string(alternate_text)));
        }
        if let Some(summary) = &element.table_summary {
            dictionary.insert("Summary", PdfObject::String(pdf_text_string(summary)));
        }
        if let Some((table, cell)) = element
            .typed_table_identity
            .as_ref()
            .zip(element.typed_cell_identity.as_ref())
        {
            dictionary.insert("WFTableID", PdfObject::String(pdf_text_string(table)));
            dictionary.insert("WFCellID", PdfObject::String(pdf_text_string(cell)));
        }
        if !element.references.is_empty() {
            dictionary.insert(
                "Ref",
                PdfObject::Array(
                    element
                        .references
                        .iter()
                        .map(|target| reference(element_numbers[target]))
                        .collect(),
                ),
            );
        }
        if element.role == Role::TableHeader || element.role == Role::TableData {
            let mut attributes = dict(&[("O", PdfObject::Name("Table".into()))]);
            if let Some(scope) = element.table_scope {
                attributes.insert(
                    "Scope",
                    PdfObject::Name(
                        match scope {
                            TableHeaderScope::Row => "Row",
                            TableHeaderScope::Column => "Column",
                            TableHeaderScope::Both => "Both",
                        }
                        .into(),
                    ),
                );
            }
            if !element.table_headers.is_empty() {
                attributes.insert(
                    "Headers",
                    PdfObject::Array(
                        element
                            .table_headers
                            .iter()
                            .map(|header| PdfObject::String(format!("WFTH-{header}").into_bytes()))
                            .collect(),
                    ),
                );
            }
            if let Some(columns) = element.table_column_span {
                attributes.insert(
                    "ColSpan",
                    PdfObject::Integer(i64::try_from(columns).map_err(|_| {
                        WellfriendError::ResourceLimit(
                            "authored table column span serialization".into(),
                        )
                    })?),
                );
            }
            if let Some(rows) = element.table_row_span {
                attributes.insert(
                    "RowSpan",
                    PdfObject::Integer(i64::try_from(rows).map_err(|_| {
                        WellfriendError::ResourceLimit(
                            "authored table row span serialization".into(),
                        )
                    })?),
                );
            }
            dictionary.insert("A", PdfObject::Dictionary(attributes));
        }
        if let Some(id) = element_id(element) {
            dictionary.insert("ID", PdfObject::String(id));
        }
        objects.push(OutputObject {
            number: element_numbers[&element.id],
            object: PdfObject::Dictionary(dictionary),
        });
    }
    Ok(Built {
        root: Some(root),
        objects,
        page_marks,
        roles,
        typed_cell_owners,
    })
}
