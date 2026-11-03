//! Page structure edits: rotate, delete, move, insert blank pages and insert
//! pages of another PDF. The page tree is rewritten as one flat `/Kids`
//! array under the root `/Pages`, after inheritable attributes (`Resources`,
//! `MediaBox`, `CropBox`, `Rotate`) are copied onto every page, so no page
//! changes its look when it leaves its original subtree.

use std::collections::{BTreeSet, VecDeque};

use lopdf::{dictionary, Document, Object, ObjectId, Stream};

use crate::error::EditorError;
use crate::overlay::{load, save};

const INHERITABLE: [&[u8]; 4] = [b"Resources", b"MediaBox", b"CropBox", b"Rotate"];

fn refused(message: impl Into<String>) -> EditorError {
    EditorError::new("EDITOR_PAGES_REFUSED", message)
}

fn root_pages(document: &Document) -> Result<ObjectId, EditorError> {
    document
        .catalog()
        .ok()
        .and_then(|catalog| catalog.get(b"Pages").ok())
        .and_then(|pages| pages.as_reference().ok())
        .ok_or_else(|| refused("the document has no page tree"))
}

/// Copy inherited attributes onto `page` (own values win).
fn materialize(document: &mut Document, page: ObjectId) -> Result<(), EditorError> {
    let mut inherited: Vec<(&[u8], Object)> = Vec::new();
    let mut parent = document
        .get_dictionary(page)
        .ok()
        .and_then(|d| d.get(b"Parent").ok())
        .and_then(|p| p.as_reference().ok());
    let mut depth = 0;
    while let Some(id) = parent {
        depth += 1;
        if depth > 64 {
            return Err(refused("the page tree is too deep"));
        }
        let Ok(node) = document.get_dictionary(id) else { break };
        for key in INHERITABLE {
            if inherited.iter().all(|(k, _)| *k != key) {
                if let Ok(value) = node.get(key) {
                    inherited.push((key, value.clone()));
                }
            }
        }
        parent = node.get(b"Parent").ok().and_then(|p| p.as_reference().ok());
    }
    let dictionary = document
        .get_object_mut(page)
        .and_then(Object::as_dict_mut)
        .map_err(|_| refused("a page dictionary is missing"))?;
    for (key, value) in inherited {
        if !dictionary.has(key) {
            dictionary.set(key.to_vec(), value);
        }
    }
    if !dictionary.has(b"Resources") {
        dictionary.set("Resources", Object::Dictionary(lopdf::Dictionary::new()));
    }
    Ok(())
}

/// The current page order, with every page made self-contained.
fn flatten(document: &mut Document) -> Result<(ObjectId, Vec<ObjectId>), EditorError> {
    let root = root_pages(document)?;
    let order: Vec<ObjectId> = document.get_pages().into_values().collect();
    for page in &order {
        materialize(document, *page)?;
    }
    Ok((root, order))
}

fn write_order(document: &mut Document, root: ObjectId, order: &[ObjectId]) -> Result<(), EditorError> {
    if order.is_empty() {
        return Err(refused("a PDF must keep at least one page"));
    }
    for page in order {
        document
            .get_object_mut(*page)
            .and_then(Object::as_dict_mut)
            .map_err(|_| refused("a page dictionary is missing"))?
            .set("Parent", Object::Reference(root));
    }
    let pages = document
        .get_object_mut(root)
        .and_then(Object::as_dict_mut)
        .map_err(|_| refused("the page tree root is missing"))?;
    pages.set("Kids", Object::Array(order.iter().map(|id| Object::Reference(*id)).collect()));
    pages.set("Count", Object::Integer(order.len() as i64));
    for key in INHERITABLE {
        pages.remove(key);
    }
    Ok(())
}

fn index_of(order: &[ObjectId], page: u32) -> Result<usize, EditorError> {
    let index = page as usize;
    if index == 0 || index > order.len() {
        return Err(EditorError::new("EDITOR_PAGE_NOT_FOUND", format!("page {page} does not exist")));
    }
    Ok(index - 1)
}

pub fn rotate_page(input: &[u8], page: u32, rotation: i32) -> Result<Vec<u8>, EditorError> {
    let mut document = load(input)?;
    let (_, order) = flatten(&mut document)?;
    let id = order[index_of(&order, page)?];
    document
        .get_object_mut(id)
        .and_then(Object::as_dict_mut)
        .map_err(|_| refused("a page dictionary is missing"))?
        .set("Rotate", Object::Integer(i64::from(rotation)));
    save(&mut document)
}

pub fn delete_page(input: &[u8], page: u32) -> Result<Vec<u8>, EditorError> {
    let mut document = load(input)?;
    let (root, mut order) = flatten(&mut document)?;
    let index = index_of(&order, page)?;
    order.remove(index);
    write_order(&mut document, root, &order)?;
    save(&mut document)
}

pub fn move_page(input: &[u8], page: u32, to: u32) -> Result<Vec<u8>, EditorError> {
    let mut document = load(input)?;
    let (root, mut order) = flatten(&mut document)?;
    let from = index_of(&order, page)?;
    let target = index_of(&order, to)?;
    let id = order.remove(from);
    order.insert(target, id);
    write_order(&mut document, root, &order)?;
    save(&mut document)
}

pub fn insert_blank_page(input: &[u8], after: u32, width: f64, height: f64) -> Result<Vec<u8>, EditorError> {
    let mut document = load(input)?;
    let (root, mut order) = flatten(&mut document)?;
    if after as usize > order.len() {
        return Err(EditorError::new("EDITOR_PAGE_NOT_FOUND", format!("page {after} does not exist")));
    }
    let contents = document.add_object(Stream::new(lopdf::Dictionary::new(), Vec::new()));
    let page = document.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => Object::Reference(root),
        "MediaBox" => vec![0.into(), 0.into(), Object::Real(width as f32), Object::Real(height as f32)],
        "Resources" => lopdf::Dictionary::new(),
        "Contents" => Object::Reference(contents),
    });
    order.insert(after as usize, page);
    write_order(&mut document, root, &order)?;
    save(&mut document)
}

/// Objects reachable from `start`, not following `/Parent` (which would pull
/// in the source document's whole page tree).
fn reachable_from_page(document: &Document, start: ObjectId) -> BTreeSet<ObjectId> {
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::from([Object::Reference(start)]);
    while let Some(object) = queue.pop_front() {
        match object {
            Object::Reference(id) => {
                if seen.insert(id) {
                    if let Ok(target) = document.get_object(id) {
                        queue.push_back(target.clone());
                    }
                }
            }
            Object::Array(items) => queue.extend(items),
            Object::Dictionary(dictionary) => queue.extend(
                dictionary
                    .into_iter()
                    .filter(|(key, _)| key.as_slice() != b"Parent")
                    .map(|(_, value)| value),
            ),
            Object::Stream(stream) => queue.extend(stream.dict.into_iter().map(|(_, value)| value)),
            _ => {}
        }
    }
    seen
}

pub fn insert_pages(input: &[u8], after: u32, source: &[u8], pages: &[u32]) -> Result<Vec<u8>, EditorError> {
    let mut document = load(input)?;
    let (root, mut order) = flatten(&mut document)?;
    if after as usize > order.len() {
        return Err(EditorError::new("EDITOR_PAGE_NOT_FOUND", format!("page {after} does not exist")));
    }
    let mut other = load(source)?;
    other.renumber_objects_with(document.max_id + 1);
    let (_, other_order) = flatten(&mut other)?;
    let chosen: Vec<ObjectId> = if pages.is_empty() {
        other_order.clone()
    } else {
        pages
            .iter()
            .map(|p| index_of(&other_order, *p).map(|i| other_order[i]))
            .collect::<Result<_, _>>()?
    };
    let mut copied = BTreeSet::new();
    for page in &chosen {
        copied.extend(reachable_from_page(&other, *page));
    }
    for id in copied {
        if let Ok(object) = other.get_object(id) {
            document.objects.insert(id, object.clone());
        }
    }
    document.max_id = document.max_id.max(other.max_id);
    for (offset, page) in chosen.iter().enumerate() {
        order.insert(after as usize + offset, *page);
    }
    write_order(&mut document, root, &order)?;
    save(&mut document)
}

/// A new PDF with `pages` of `input` (for "extract pages").
pub fn extract_pages(input: &[u8], pages: &[u32]) -> Result<Vec<u8>, EditorError> {
    let mut document = load(input)?;
    let (root, order) = flatten(&mut document)?;
    let keep: Vec<ObjectId> = pages
        .iter()
        .map(|p| index_of(&order, *p).map(|i| order[i]))
        .collect::<Result<_, _>>()?;
    write_order(&mut document, root, &keep)?;
    document.prune_objects();
    save(&mut document)
}
