//! Hierarchical document outlines for fresh authoring. Targets are stable
//! authored anchor names; object identities and all tree links are assigned in
//! one final serialization transaction.
use super::*;

#[cfg(test)]
#[path = "authoring_outline_tests.rs"]
mod tests;

const MAX_OUTLINE_ITEMS: usize = 100_000;
const MAX_OUTLINE_DEPTH: usize = 128;
const MAX_OUTLINE_TEXT_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfOutlineEntry {
    pub title: String,
    pub anchor: String,
    pub open: bool,
    pub children: Vec<PdfOutlineEntry>,
}

impl PdfOutlineEntry {
    pub fn new(title: impl Into<String>, anchor: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            anchor: anchor.into(),
            open: true,
            children: Vec::new(),
        }
    }

    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    pub fn children(mut self, children: Vec<PdfOutlineEntry>) -> Self {
        self.children = children;
        self
    }
}

#[derive(Debug)]
pub(super) struct OutlineObjects {
    pub(super) root: Option<u32>,
    pub(super) objects: Vec<OutputObject>,
}

#[derive(Debug, Clone, Copy)]
enum Parent {
    Root,
    Item(usize),
}

#[derive(Debug)]
struct Node {
    title: String,
    anchor: String,
    open: bool,
    parent: Parent,
    previous: Option<usize>,
    next: Option<usize>,
    children: Vec<usize>,
}

fn fail(message: &str) -> WellfriendError {
    WellfriendError::invalid_input(message)
}

fn validate_text(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > MAX_OUTLINE_TEXT_BYTES
        || value
            .chars()
            .any(|ch| ch == '\0' || crate::fonts::hard_break::is_hard_break(ch))
    {
        return Err(fail(label));
    }
    Ok(())
}

fn validate_entries(entries: &[PdfOutlineEntry], depth: usize, count: &mut usize) -> Result<()> {
    if !entries.is_empty() && depth > MAX_OUTLINE_DEPTH {
        return Err(WellfriendError::ResourceLimit(
            "authored outline nesting depth".into(),
        ));
    }
    for entry in entries {
        *count = count
            .checked_add(1)
            .ok_or_else(|| WellfriendError::ResourceLimit("authored outline item count".into()))?;
        if *count % 256 == 1 {
            crate::cancel::check_current_cancel("authoring outline validation")?;
        }
        if *count > MAX_OUTLINE_ITEMS {
            return Err(WellfriendError::ResourceLimit(
                "authored outline item count".into(),
            ));
        }
        validate_text(
            &entry.title,
            "outline title must be a bounded nonempty single line",
        )?;
        validate_text(
            &entry.anchor,
            "outline anchor must be a bounded nonempty single line",
        )?;
        validate_entries(&entry.children, depth + 1, count)?;
    }
    Ok(())
}

pub(super) fn validate(entries: &[PdfOutlineEntry]) -> Result<usize> {
    let mut count = 0usize;
    validate_entries(entries, 1, &mut count)?;
    Ok(count)
}

fn children_at_path<'a>(
    entries: &'a [PdfOutlineEntry],
    path: &[String],
) -> Result<&'a [PdfOutlineEntry]> {
    let Some((head, tail)) = path.split_first() else {
        return Ok(entries);
    };
    let entry = entries
        .iter()
        .find(|entry| entry.anchor == *head)
        .ok_or_else(|| fail("captured outline heading path no longer exists"))?;
    children_at_path(&entry.children, tail)
}

fn children_at_path_mut<'a>(
    entries: &'a mut Vec<PdfOutlineEntry>,
    path: &[String],
) -> Result<&'a mut Vec<PdfOutlineEntry>> {
    let Some((head, tail)) = path.split_first() else {
        return Ok(entries);
    };
    let entry = entries
        .iter_mut()
        .find(|entry| entry.anchor == *head)
        .ok_or_else(|| fail("captured outline heading path no longer exists"))?;
    children_at_path_mut(&mut entry.children, tail)
}

pub(super) fn validate_heading_append(
    builder: &PdfBuilder,
    path: &[String],
    title: &str,
    anchor: &str,
    level: u8,
) -> Result<()> {
    let level = usize::from(level);
    if level == 0 || level > MAX_OUTLINE_DEPTH {
        return Err(fail("outlined heading level must be in 1..=128"));
    }
    validate_text(
        title,
        "outlined heading title must be a bounded nonempty single line",
    )?;
    super::fields::validate_anchor_name(anchor)?;
    if builder.anchors.contains_key(anchor) {
        return Err(fail("outlined heading anchor is already defined"));
    }
    if builder.outline_item_count >= MAX_OUTLINE_ITEMS {
        return Err(WellfriendError::ResourceLimit(
            "authored outline item count".into(),
        ));
    }
    if level > path.len() + 1 {
        return Err(fail("outlined heading cannot skip a hierarchy level"));
    }
    let parent_path = &path[..level - 1];
    let _ = children_at_path(&builder.outline, parent_path)?;
    Ok(())
}

pub(super) fn append_heading(
    builder: &mut PdfBuilder,
    path: &mut Vec<String>,
    title: String,
    anchor: String,
    level: u8,
) -> Result<()> {
    validate_heading_append(builder, path, &title, &anchor, level)?;
    let parent_len = usize::from(level) - 1;
    path.truncate(parent_len);
    children_at_path_mut(&mut builder.outline, path)?
        .push(PdfOutlineEntry::new(title, anchor.clone()));
    builder.outline_item_count = builder
        .outline_item_count
        .checked_add(1)
        .ok_or_else(|| WellfriendError::ResourceLimit("authored outline item count".into()))?;
    path.push(anchor);
    Ok(())
}

pub(super) fn rollback_heading(
    builder: &mut PdfBuilder,
    path: &mut Vec<String>,
    previous_path: Vec<String>,
    anchor: &str,
) -> Result<()> {
    let parent_len = path
        .len()
        .checked_sub(1)
        .ok_or_else(|| fail("captured outline rollback has no item"))?;
    let removed = children_at_path_mut(&mut builder.outline, &path[..parent_len])?
        .pop()
        .ok_or_else(|| fail("captured outline rollback item disappeared"))?;
    if removed.anchor != anchor {
        return Err(fail("captured outline rollback identity changed"));
    }
    builder.outline_item_count = builder
        .outline_item_count
        .checked_sub(1)
        .ok_or_else(|| fail("captured outline count underflow"))?;
    *path = previous_path;
    Ok(())
}

fn flatten_siblings(
    entries: &[PdfOutlineEntry],
    parent: Parent,
    nodes: &mut Vec<Node>,
) -> Result<Vec<usize>> {
    let mut siblings = Vec::with_capacity(entries.len());
    for entry in entries {
        crate::cancel::check_current_cancel("authoring outline flatten")?;
        let index = nodes.len();
        nodes.push(Node {
            title: entry.title.clone(),
            anchor: entry.anchor.clone(),
            open: entry.open,
            parent,
            previous: None,
            next: None,
            children: Vec::new(),
        });
        let children = flatten_siblings(&entry.children, Parent::Item(index), nodes)?;
        nodes[index].children = children;
        siblings.push(index);
    }
    for (position, index) in siblings.iter().copied().enumerate() {
        nodes[index].previous = position.checked_sub(1).map(|previous| siblings[previous]);
        nodes[index].next = siblings.get(position + 1).copied();
    }
    Ok(siblings)
}

fn visible_descendants(index: usize, nodes: &[Node], memo: &mut [Option<usize>]) -> Result<usize> {
    if let Some(value) = memo[index] {
        return Ok(value);
    }
    let mut count = 0usize;
    for (position, child) in nodes[index].children.iter().copied().enumerate() {
        if position % 256 == 0 {
            crate::cancel::check_current_cancel("authoring outline count")?;
        }
        count = count
            .checked_add(1)
            .ok_or_else(|| WellfriendError::ResourceLimit("outline visible count".into()))?;
        if nodes[child].open {
            count = count
                .checked_add(visible_descendants(child, nodes, memo)?)
                .ok_or_else(|| WellfriendError::ResourceLimit("outline visible count".into()))?;
        }
    }
    memo[index] = Some(count);
    Ok(count)
}

fn allocate(next: &mut u32) -> Result<u32> {
    let number = *next;
    *next = next
        .checked_add(1)
        .ok_or_else(|| WellfriendError::ResourceLimit("authored outline object number".into()))?;
    Ok(number)
}

pub(super) fn build(builder: &PdfBuilder, next: &mut u32) -> Result<OutlineObjects> {
    let count = validate(&builder.outline)?;
    if count != builder.outline_item_count {
        return Err(fail("authored outline count invariant changed"));
    }
    if builder.outline.is_empty() {
        return Ok(OutlineObjects {
            root: None,
            objects: Vec::new(),
        });
    }
    let mut nodes = Vec::new();
    let top = flatten_siblings(&builder.outline, Parent::Root, &mut nodes)?;
    if nodes.is_empty() || top.is_empty() {
        return Err(fail("nonempty outline did not produce items"));
    }
    for node in &nodes {
        if !builder.anchors.contains_key(&node.anchor) {
            return Err(fail("outline target is not a declared authored anchor"));
        }
    }

    let root_number = allocate(next)?;
    let mut numbers = Vec::with_capacity(nodes.len());
    for _ in &nodes {
        numbers.push(allocate(next)?);
    }
    let mut memo = vec![None; nodes.len()];
    let mut root_count = 0usize;
    for index in top.iter().copied() {
        root_count = root_count
            .checked_add(1)
            .ok_or_else(|| WellfriendError::ResourceLimit("outline root count".into()))?;
        if nodes[index].open {
            root_count = root_count
                .checked_add(visible_descendants(index, &nodes, &mut memo)?)
                .ok_or_else(|| WellfriendError::ResourceLimit("outline root count".into()))?;
        }
    }

    let mut root = PdfDictionary::empty();
    root.insert("Type", PdfObject::Name("Outlines".into()));
    root.insert("First", reference(numbers[top[0]]));
    root.insert("Last", reference(numbers[*top.last().unwrap()]));
    root.insert(
        "Count",
        PdfObject::Integer(
            i64::try_from(root_count)
                .map_err(|_| WellfriendError::ResourceLimit("outline root count".into()))?,
        ),
    );
    let mut objects = Vec::with_capacity(nodes.len() + 1);
    objects.push(OutputObject {
        number: root_number,
        object: PdfObject::Dictionary(root),
    });

    for (index, node) in nodes.iter().enumerate() {
        crate::cancel::check_current_cancel("authoring outline object")?;
        let mut dictionary = PdfDictionary::empty();
        dictionary.insert("Title", PdfObject::String(pdf_text_string(&node.title)));
        dictionary.insert(
            "Parent",
            reference(match node.parent {
                Parent::Root => root_number,
                Parent::Item(parent) => numbers[parent],
            }),
        );
        dictionary.insert("Dest", PdfObject::String(pdf_text_string(&node.anchor)));
        if let Some(previous) = node.previous {
            dictionary.insert("Prev", reference(numbers[previous]));
        }
        if let Some(next) = node.next {
            dictionary.insert("Next", reference(numbers[next]));
        }
        if let (Some(first), Some(last)) = (node.children.first(), node.children.last()) {
            dictionary.insert("First", reference(numbers[*first]));
            dictionary.insert("Last", reference(numbers[*last]));
            let visible = visible_descendants(index, &nodes, &mut memo)?;
            let visible = i64::try_from(visible)
                .map_err(|_| WellfriendError::ResourceLimit("outline item count".into()))?;
            dictionary.insert(
                "Count",
                PdfObject::Integer(if node.open { visible } else { -visible }),
            );
        }
        objects.push(OutputObject {
            number: numbers[index],
            object: PdfObject::Dictionary(dictionary),
        });
    }
    Ok(OutlineObjects {
        root: Some(root_number),
        objects,
    })
}
