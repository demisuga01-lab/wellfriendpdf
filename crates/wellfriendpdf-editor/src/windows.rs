//! Large documents as page windows.
//!
//! A page window is a one-page PDF holding page `n` of a document and only
//! the objects that page uses (`ContentEngine::extract_pages_to`, which reads
//! the source lazily: tens of milliseconds on a 500 MB file). The browser
//! edits windows; the server replays each window's operations on a freshly
//! extracted window and splices the edited page back into the document as an
//! incremental update: the original bytes stay untouched, and only the
//! edited page dictionary and the objects it now uses are appended.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::Write;

use lopdf::{Dictionary, Document, Object, ObjectId, StringFormat};
use serde::Serialize;
use wellfriendpdf_engine::ContentEngine;

use crate::error::EditorError;

fn refused(message: impl Into<String>) -> EditorError {
    EditorError::new("EDITOR_WINDOW_REFUSED", message)
}

/// Page count and every page's displayed size (points).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowIndex {
    pub page_count: usize,
    pub page_sizes: Vec<[f64; 2]>,
    /// The catalogue declares signatures (AcroForm `/SigFlags`).
    pub signed: bool,
}

pub fn index(engine: &ContentEngine) -> Result<WindowIndex, EditorError> {
    let page_count = engine.page_count()?;
    let mut page_sizes = Vec::with_capacity(page_count);
    for page in 1..=page_count {
        let viewport = engine.page_viewport(page, 720)?;
        page_sizes.push([f64::from(viewport.width_px) / 10.0, f64::from(viewport.height_px) / 10.0]);
    }
    Ok(WindowIndex {
        page_count,
        page_sizes,
        signed: signed_lazily(engine),
    })
}

/// The one-page window of `page` (1-based).
pub fn extract(engine: &ContentEngine, page: u32) -> Result<Vec<u8>, EditorError> {
    let count = engine.page_count()?;
    if page == 0 || page as usize > count {
        return Err(EditorError::new("EDITOR_PAGE_NOT_FOUND", format!("page {page} does not exist")));
    }
    let mut out = Vec::new();
    engine.extract_pages_to(&[page as usize], &mut out)?;
    Ok(out)
}

fn write_name(name: &[u8], out: &mut Vec<u8>) {
    out.push(b'/');
    for &byte in name {
        if byte.is_ascii_graphic() && !b"()<>[]{}/%#".contains(&byte) {
            out.push(byte);
        } else {
            let _ = write!(out, "#{byte:02X}");
        }
    }
}

fn write_real(value: f32, out: &mut Vec<u8>) {
    if value.fract() == 0.0 && value.abs() < 1e9 {
        let _ = write!(out, "{}", value as i64);
    } else {
        let mut text = format!("{value:.5}");
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
        out.extend_from_slice(text.as_bytes());
    }
}

/// Serialize one object as it appears between `obj` and `endobj`.
fn write_object(object: &Object, out: &mut Vec<u8>) {
    match object {
        Object::Null => out.extend_from_slice(b"null"),
        Object::Boolean(value) => out.extend_from_slice(if *value { b"true" } else { b"false" }),
        Object::Integer(value) => {
            let _ = write!(out, "{value}");
        }
        Object::Real(value) => write_real(*value, out),
        Object::Name(name) => write_name(name, out),
        Object::String(bytes, format) => match format {
            StringFormat::Hexadecimal => {
                out.push(b'<');
                for byte in bytes {
                    let _ = write!(out, "{byte:02X}");
                }
                out.push(b'>');
            }
            StringFormat::Literal => {
                out.push(b'(');
                for &byte in bytes {
                    match byte {
                        b'(' | b')' | b'\\' => {
                            out.push(b'\\');
                            out.push(byte);
                        }
                        b'\r' => out.extend_from_slice(b"\\r"),
                        b'\n' => out.extend_from_slice(b"\\n"),
                        _ => out.push(byte),
                    }
                }
                out.push(b')');
            }
        },
        Object::Array(items) => {
            out.push(b'[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(b' ');
                }
                write_object(item, out);
            }
            out.push(b']');
        }
        Object::Dictionary(dictionary) => write_dictionary(dictionary, out),
        Object::Stream(stream) => {
            let mut dictionary = stream.dict.clone();
            dictionary.set("Length", Object::Integer(stream.content.len() as i64));
            write_dictionary(&dictionary, out);
            out.extend_from_slice(b"\nstream\n");
            out.extend_from_slice(&stream.content);
            out.extend_from_slice(b"\nendstream");
        }
        Object::Reference((number, generation)) => {
            let _ = write!(out, "{number} {generation} R");
        }
    }
}

fn write_dictionary(dictionary: &Dictionary, out: &mut Vec<u8>) {
    out.extend_from_slice(b"<<");
    for (key, value) in dictionary.iter() {
        write_name(key, out);
        out.push(b' ');
        write_object(value, out);
        out.push(b'\n');
    }
    out.extend_from_slice(b">>");
}

fn remap(object: &mut Object, map: &BTreeMap<ObjectId, ObjectId>) {
    match object {
        Object::Reference(id) => {
            if let Some(new) = map.get(id) {
                *id = *new;
            }
        }
        Object::Array(items) => items.iter_mut().for_each(|item| remap(item, map)),
        Object::Dictionary(dictionary) => dictionary.iter_mut().for_each(|(_, value)| remap(value, map)),
        Object::Stream(stream) => stream.dict.iter_mut().for_each(|(_, value)| remap(value, map)),
        _ => {}
    }
}

/// Objects reachable from `start` without following `/Parent`.
fn reachable(document: &Document, start: ObjectId) -> Vec<ObjectId> {
    let mut seen = BTreeSet::new();
    let mut order = Vec::new();
    let mut queue = VecDeque::from([Object::Reference(start)]);
    while let Some(object) = queue.pop_front() {
        match object {
            Object::Reference(id) => {
                if seen.insert(id) {
                    order.push(id);
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
    order
}

/// Append the edited windows' pages to `original` as one incremental update.
/// Whether the catalogue declares signatures (AcroForm `/SigFlags` bit 1),
/// read lazily so a large document is not parsed whole.
pub fn signed_lazily(engine: &wellfriendpdf_engine::ContentEngine) -> bool {
    let Ok(catalog) = engine.document().get_catalog() else {
        return false;
    };
    let reader = engine.document().reader();
    let Some(form) = catalog.get("AcroForm").and_then(|f| reader.resolve(f.clone()).ok()) else {
        return false;
    };
    match form {
        wellfriendpdf_engine::PdfObject::Dictionary(d) => d
            .get("SigFlags")
            .and_then(|f| reader.resolve(f.clone()).ok())
            .is_some_and(|f| matches!(f, wellfriendpdf_engine::PdfObject::Integer(flags) if flags & 1 == 1)),
        _ => false,
    }
}

/// A window before and after its edits.
pub struct EditedWindow {
    pub page: u32,
    pub before: Vec<u8>,
    pub after: Vec<u8>,
}

/// Pair window object ids with the original document's object ids by
/// walking both graphs from their page dictionaries in step (the window is
/// a structural copy of the original page).
fn pair_objects(
    window: &Document,
    window_page: ObjectId,
    reader: &wellfriendpdf_engine::PdfReader,
    page: ObjectId,
) -> BTreeMap<ObjectId, ObjectId> {
    use wellfriendpdf_engine::PdfObject;
    fn walk(
        a: &Object,
        b: &PdfObject,
        pairs: &mut BTreeMap<ObjectId, ObjectId>,
        queue: &mut VecDeque<(ObjectId, ObjectId)>,
    ) {
        match (a, b) {
            (Object::Reference(w), PdfObject::Reference { number, generation }) => {
                if !pairs.contains_key(w) {
                    pairs.insert(*w, (*number, *generation));
                    queue.push_back((*w, (*number, *generation)));
                }
            }
            (Object::Array(x), PdfObject::Array(y)) => {
                for (a, b) in x.iter().zip(y) {
                    walk(a, b, pairs, queue);
                }
            }
            (Object::Dictionary(x), PdfObject::Dictionary(y)) => walk_dict(x, y, pairs, queue),
            (Object::Stream(x), PdfObject::Stream { dict, .. }) => walk_dict(&x.dict, dict, pairs, queue),
            _ => {}
        }
    }
    fn walk_dict(
        x: &Dictionary,
        y: &wellfriendpdf_engine::PdfDictionary,
        pairs: &mut BTreeMap<ObjectId, ObjectId>,
        queue: &mut VecDeque<(ObjectId, ObjectId)>,
    ) {
        for (key, value) in x.iter() {
            if key.as_slice() == b"Parent" {
                continue;
            }
            if let Some(other) = y.get(&String::from_utf8_lossy(key)) {
                walk(value, other, pairs, queue);
            }
        }
    }
    let mut pairs = BTreeMap::new();
    pairs.insert(window_page, page);
    let mut queue = VecDeque::from([(window_page, page)]);
    while let Some((w, o)) = queue.pop_front() {
        let (Ok(a), Ok(b)) = (window.get_object(w), reader.get_object(o.0, o.1)) else { continue };
        walk(a, &b, &mut pairs, &mut queue);
    }
    pairs
}

fn serialized(document: &Document, id: ObjectId) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    write_object(document.get_object(id).ok()?, &mut out);
    Some(out)
}

/// Objects of the edited window that are unchanged from the extracted
/// window, together with everything they reach: those keep pointing at the
/// original document's objects instead of being appended again.
fn reusable(
    before: &Document,
    after: &Document,
    objects: &[ObjectId],
    window_page: ObjectId,
) -> BTreeSet<ObjectId> {
    fn children(object: &Object, out: &mut Vec<ObjectId>) {
        match object {
            Object::Reference(id) => out.push(*id),
            Object::Array(items) => items.iter().for_each(|item| children(item, out)),
            Object::Dictionary(d) => d.iter().filter(|(k, _)| k.as_slice() != b"Parent").for_each(|(_, v)| children(v, out)),
            Object::Stream(s) => s.dict.iter().for_each(|(_, v)| children(v, out)),
            _ => {}
        }
    }
    let identical: BTreeSet<ObjectId> = objects
        .iter()
        .copied()
        .filter(|id| *id != window_page && serialized(before, *id).is_some() && serialized(before, *id) == serialized(after, *id))
        .collect();
    // Drop any object reaching a changed one (or the page), to a fixed point.
    let mut keep = identical.clone();
    loop {
        let mut changed = false;
        for id in keep.clone() {
            let mut kids = Vec::new();
            if let Ok(object) = after.get_object(id) {
                children(object, &mut kids);
            }
            if kids.iter().any(|kid| !keep.contains(kid)) {
                keep.remove(&id);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    keep
}

pub fn splice_incremental(original: &[u8], windows: &[EditedWindow]) -> Result<Vec<u8>, EditorError> {
    let engine = ContentEngine::open_bytes(original.to_vec())?;
    let reader = engine.document().reader();
    if reader.is_encrypted() {
        return Err(refused("encrypted PDFs are edited with a full rewrite, not page windows"));
    }
    let root = reader.root_reference().ok_or_else(|| refused("the document has no catalogue"))?;
    let mut next = reader
        .size()
        .map(|size| size as u32)
        .or_else(|| reader.object_ids().iter().map(|(n, _)| *n).max().map(|n| n + 1))
        .ok_or_else(|| refused("the document size is unknown"))?;
    let previous = reader.startxref_offset();
    let info = reader.info_reference();
    let id = reader.first_file_id();

    let mut appended: BTreeMap<u32, (u16, Vec<u8>)> = BTreeMap::new();
    for window in windows {
        let page = &window.page;
        let target = engine.document().get_page(*page as usize)?;
        let target_id: ObjectId = (target.object_number, target.generation_number);
        let parent = match reader.get_object(target.object_number, target.generation_number)? {
            wellfriendpdf_engine::PdfObject::Dictionary(dictionary) => match dictionary.get("Parent") {
                Some(wellfriendpdf_engine::PdfObject::Reference { number, generation }) => (*number, *generation),
                _ => return Err(refused("the page has no parent")),
            },
            _ => return Err(refused("the page is not a dictionary")),
        };
        let edited = Document::load_mem(&window.after).map_err(|e| refused(format!("the edited window is invalid: {e}")))?;
        let before = Document::load_mem(&window.before).map_err(|e| refused(format!("the source window is invalid: {e}")))?;
        let window_page = *edited.get_pages().get(&1).ok_or_else(|| refused("the edited window has no page"))?;
        let before_page = *before.get_pages().get(&1).ok_or_else(|| refused("the source window has no page"))?;
        let objects = reachable(&edited, window_page);
        // Unchanged objects keep their original identity.
        let pairs = if before_page == window_page {
            pair_objects(&before, before_page, reader, target_id)
        } else {
            BTreeMap::new()
        };
        let keep: BTreeSet<ObjectId> = reusable(&before, &edited, &objects, window_page)
            .into_iter()
            .filter(|id| pairs.contains_key(id))
            .collect();
        let mut map = BTreeMap::new();
        map.insert(window_page, target_id);
        for object in &objects {
            if *object == window_page {
                continue;
            }
            if keep.contains(object) {
                map.insert(*object, pairs[object]);
            } else {
                map.insert(*object, (next, 0));
                next += 1;
            }
        }
        for object in objects.into_iter().filter(|id| !keep.contains(id)) {
            let mut value = edited.get_object(object).map_err(|_| refused("a window object is missing"))?.clone();
            remap(&mut value, &map);
            if object == window_page {
                if let Object::Dictionary(dictionary) = &mut value {
                    dictionary.set("Parent", Object::Reference(parent));
                }
            }
            let new_id = map[&object];
            let mut body = Vec::new();
            write_object(&value, &mut body);
            appended.insert(new_id.0, (new_id.1, body));
        }
    }

    let mut out = original.to_vec();
    if !out.ends_with(b"\n") {
        out.push(b'\n');
    }
    let mut offsets = BTreeMap::new();
    for (number, (generation, body)) in &appended {
        offsets.insert(*number, (out.len(), *generation));
        let _ = write!(out, "{number} {generation} obj\n");
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(b"xref\n");
    let numbers: Vec<u32> = offsets.keys().copied().collect();
    let mut index = 0;
    while index < numbers.len() {
        let mut end = index + 1;
        while end < numbers.len() && numbers[end] == numbers[end - 1] + 1 {
            end += 1;
        }
        let _ = write!(out, "{} {}\n", numbers[index], end - index);
        for number in &numbers[index..end] {
            let (offset, generation) = offsets[number];
            let _ = write!(out, "{offset:010} {generation:05} n\r\n");
        }
        index = end;
    }
    let _ = write!(out, "trailer\n<</Size {next} /Root {} {} R /Prev {previous}", root.0, root.1);
    if let Some((number, generation)) = info {
        let _ = write!(out, " /Info {number} {generation} R");
    }
    if let Some(id) = id {
        let hex: String = id.iter().map(|b| format!("{b:02X}")).collect();
        let _ = write!(out, " /ID [<{hex}> <{hex}>]");
    }
    let _ = write!(out, ">>\nstartxref\n{xref}\n%%EOF\n");
    Ok(out)
}
