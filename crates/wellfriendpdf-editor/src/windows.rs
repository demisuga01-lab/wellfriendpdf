//! Large documents as page windows.
//!
//! A page window is a one-page PDF holding page `n` of a document and only
//! the objects that page uses (`ContentEngine::extract_pages_to`, which reads
//! the source lazily: tens of milliseconds on a 500 MB file). A window of a
//! page with form widgets also carries the widgets' field dictionaries and an
//! `/AcroForm` naming them, so a field can be filled on the window.
//!
//! The browser edits windows; the server replays each window's operations on
//! a freshly extracted window and builds a [`Plan`]: the edited page objects,
//! field dictionaries rewritten in place, pages inserted from other PDFs and
//! the rewritten `/Pages` nodes of a changed page order. Every object the
//! edit did not change keeps its original number and bytes.
//!
//! A plan is written one of two ways:
//! - [`Plan::write_update`]: an incremental update appended to the original
//!   bytes (ordinary edits; the original stays a byte prefix).
//! - [`Plan::write_rewrite`]: a full rewrite (redaction). Only the newest
//!   definition of every object still reachable from the catalogue and the
//!   document information is written, unchanged objects streamed through
//!   without decoding their streams. Superseded definitions, earlier
//!   incremental sections, object and cross-reference streams and
//!   linearization data are not carried over, so content a redaction removed
//!   exists nowhere in the result.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::io::Write;

use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, StringFormat};
use serde::Serialize;
use wellfriendpdf_engine::{ContentEngine, PdfDictionary, PdfObject, PdfReader};

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
    with_form_fields(engine, out)
}

/// A one-page PDF with an empty page of `width` x `height` points: the
/// window of a blank page inserted into a windowed document.
pub fn blank_window(width: f64, height: f64) -> Result<Vec<u8>, EditorError> {
    let valid = |v: f64| v.is_finite() && (3.0..=14_400.0).contains(&v);
    if !valid(width) || !valid(height) {
        return Err(EditorError::new("EDITOR_OPERATION_INVALID", "page sizes are 3 to 14400 points"));
    }
    let mut document = Document::with_version("1.7");
    let pages = document.new_object_id();
    let contents = document.add_object(lopdf::Stream::new(Dictionary::new(), Vec::new()));
    let page = document.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => Object::Reference(pages),
        "MediaBox" => vec![0.into(), 0.into(), Object::Real(width as f32), Object::Real(height as f32)],
        "Resources" => Dictionary::new(),
        "Contents" => Object::Reference(contents),
    });
    document.objects.insert(
        pages,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![Object::Reference(page)],
            "Count" => 1,
        }),
    );
    let catalog = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => Object::Reference(pages) });
    document.trailer.set("Root", Object::Reference(catalog));
    crate::overlay::save(&mut document)
}

/// The windows of `pages` (all when empty) of another PDF, inserted into a
/// windowed document: each is the page's own window.
pub fn inserted_windows(pdf: &[u8], pages: &[u32]) -> Result<Vec<Vec<u8>>, EditorError> {
    let engine = ContentEngine::open_bytes(pdf.to_vec())?;
    let count = engine.page_count()? as u32;
    let chosen: Vec<u32> = if pages.is_empty() { (1..=count).collect() } else { pages.to_vec() };
    chosen.into_iter().map(|page| extract(&engine, page)).collect()
}

// ---- form fields in a window ----

fn is_annotation(dictionary: &Dictionary) -> bool {
    dictionary.get(b"Type").ok().and_then(|t| t.as_name().ok()) == Some(b"Annot".as_slice())
        || dictionary.get(b"Subtype").ok().and_then(|t| t.as_name().ok()) == Some(b"Widget".as_slice())
}

fn is_page_tree(dictionary: &Dictionary) -> bool {
    matches!(
        dictionary.get(b"Type").ok().and_then(|t| t.as_name().ok()),
        Some(b"Page") | Some(b"Pages")
    )
}

/// A form field dictionary or widget: it has an identity in the document's
/// field tree, so an edit rewrites it in place instead of copying it.
fn is_field(dictionary: &Dictionary) -> bool {
    dictionary.has(b"FT")
        || dictionary.has(b"T")
        || (dictionary.get(b"Subtype").ok().and_then(|t| t.as_name().ok()) == Some(b"Widget".as_slice())
            && dictionary.has(b"Parent"))
}

fn parent_of(document: &Document, id: ObjectId) -> Option<ObjectId> {
    document
        .get_dictionary(id)
        .ok()?
        .get(b"Parent")
        .ok()?
        .as_reference()
        .ok()
}

fn annotation_ids(document: &Document, page: ObjectId) -> Vec<ObjectId> {
    let Ok(dictionary) = document.get_dictionary(page) else { return Vec::new() };
    let annots = match dictionary.get(b"Annots") {
        Ok(Object::Reference(id)) => document.get_object(*id).ok(),
        Ok(other) => Some(other),
        Err(_) => None,
    };
    annots
        .and_then(|a| a.as_array().ok())
        .map(|items| items.iter().filter_map(|item| item.as_reference().ok()).collect())
        .unwrap_or_default()
}

/// Convert an engine object into lopdf, importing the objects it references
/// into `document` (at most `budget` of them).
fn import(
    reader: &PdfReader,
    object: &PdfObject,
    document: &mut Document,
    map: &mut HashMap<(u32, u16), ObjectId>,
    budget: &mut usize,
) -> Object {
    match object {
        PdfObject::Reference { number, generation } => {
            let key = (*number, *generation);
            if let Some(id) = map.get(&key) {
                return Object::Reference(*id);
            }
            if *budget == 0 {
                return Object::Null;
            }
            *budget -= 1;
            let Ok(target) = reader.get_object(*number, *generation) else { return Object::Null };
            let id = document.new_object_id();
            map.insert(key, id);
            let value = import(reader, &target, document, map, budget);
            document.objects.insert(id, value);
            Object::Reference(id)
        }
        PdfObject::Array(items) => Object::Array(items.iter().map(|i| import(reader, i, document, map, budget)).collect()),
        PdfObject::Dictionary(dictionary) => Object::Dictionary(import_dict(reader, dictionary, document, map, budget)),
        PdfObject::Stream { dict, raw } => {
            let mut dictionary = import_dict(reader, dict, document, map, budget);
            dictionary.set("Length", Object::Integer(raw.len() as i64));
            Object::Stream(lopdf::Stream::new(dictionary, raw.clone()))
        }
        other => to_lopdf(other.clone()),
    }
}

fn import_dict(
    reader: &PdfReader,
    dictionary: &PdfDictionary,
    document: &mut Document,
    map: &mut HashMap<(u32, u16), ObjectId>,
    budget: &mut usize,
) -> Dictionary {
    let mut out = Dictionary::new();
    for (key, value) in dictionary.iter() {
        if key == "Length" {
            continue;
        }
        out.set(key.as_bytes().to_vec(), import(reader, value, document, map, budget));
    }
    out
}

/// The document's `/AcroForm` dictionary, read lazily.
fn acroform(reader: &PdfReader) -> Option<PdfDictionary> {
    let (number, generation) = reader.root_reference()?;
    let catalog = reader.get_and_resolve(number, generation).ok()?;
    let form = catalog.as_dict()?.get("AcroForm")?.clone();
    reader.resolve(form).ok()?.as_dict().cloned()
}

/// Add an `/AcroForm` to a window whose page has widgets, naming the fields
/// those widgets belong to (the extraction already copied each widget's
/// field ancestry and sibling widgets) with the document's default
/// appearance and resources, so the engine can fill them on the window.
fn with_form_fields(engine: &ContentEngine, window: Vec<u8>) -> Result<Vec<u8>, EditorError> {
    let reader = engine.document().reader();
    let Some(form) = acroform(reader) else { return Ok(window) };
    let Ok(mut document) = Document::load_mem(&window) else { return Ok(window) };
    let Some(&page) = document.get_pages().get(&1) else { return Ok(window) };
    let mut tops = Vec::new();
    for annotation in annotation_ids(&document, page) {
        let Ok(dictionary) = document.get_dictionary(annotation) else { continue };
        if dictionary.get(b"Subtype").ok().and_then(|t| t.as_name().ok()) != Some(b"Widget".as_slice()) {
            continue;
        }
        let mut top = annotation;
        let mut depth = 0;
        while let Some(parent) = parent_of(&document, top) {
            depth += 1;
            if depth > 32 || document.get_dictionary(parent).is_err() {
                break;
            }
            top = parent;
        }
        let named = document.get_dictionary(top).is_ok_and(|d| d.has(b"T") || d.has(b"FT"));
        if named && !tops.contains(&top) {
            tops.push(top);
        }
    }
    if tops.is_empty() {
        return Ok(window);
    }
    let mut acro = Dictionary::new();
    acro.set("Fields", Object::Array(tops.into_iter().map(Object::Reference).collect()));
    let mut map = HashMap::new();
    let mut budget = 4096;
    for key in ["DA", "Q", "NeedAppearances", "DR"] {
        if let Some(value) = form.get(key) {
            let value = import(reader, value, &mut document, &mut map, &mut budget);
            acro.set(key, value);
        }
    }
    let root = document
        .trailer
        .get(b"Root")
        .and_then(Object::as_reference)
        .map_err(|_| refused("the window has no catalogue"))?;
    // An indirect form dictionary, as the field inspection expects.
    let acro = document.add_object(acro);
    document
        .get_object_mut(root)
        .and_then(Object::as_dict_mut)
        .map_err(|_| refused("the window has no catalogue"))?
        .set("AcroForm", Object::Reference(acro));
    crate::overlay::save(&mut document)
}

// ---- serialization ----

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

fn write_real(value: f64, out: &mut Vec<u8>) {
    if !value.is_finite() {
        out.push(b'0');
    } else if value.fract() == 0.0 && value.abs() < 1e15 {
        let _ = write!(out, "{}", value as i64);
    } else {
        let mut text = format!("{value:.6}");
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
        out.extend_from_slice(text.as_bytes());
    }
}

fn write_literal(bytes: &[u8], out: &mut Vec<u8>) {
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

fn write_hex(bytes: &[u8], out: &mut Vec<u8>) {
    out.push(b'<');
    for byte in bytes {
        let _ = write!(out, "{byte:02X}");
    }
    out.push(b'>');
}

/// Serialize one lopdf object as it appears between `obj` and `endobj`.
fn write_object(object: &Object, out: &mut Vec<u8>) {
    match object {
        Object::Null => out.extend_from_slice(b"null"),
        Object::Boolean(value) => out.extend_from_slice(if *value { b"true" } else { b"false" }),
        Object::Integer(value) => {
            let _ = write!(out, "{value}");
        }
        Object::Real(value) => write_real(f64::from(*value), out),
        Object::Name(name) => write_name(name, out),
        Object::String(bytes, format) => match format {
            StringFormat::Hexadecimal => write_hex(bytes, out),
            StringFormat::Literal => write_literal(bytes, out),
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

/// Serialize one engine object (reals at full precision; a stream's encoded
/// bytes as they are, with a direct `/Length`).
fn write_pdf(object: &PdfObject, out: &mut Vec<u8>) {
    match object {
        PdfObject::Null => out.extend_from_slice(b"null"),
        PdfObject::Boolean(value) => out.extend_from_slice(if *value { b"true" } else { b"false" }),
        PdfObject::Integer(value) => {
            let _ = write!(out, "{value}");
        }
        PdfObject::Real(value) => write_real(*value, out),
        PdfObject::Name(name) => write_name(name.as_bytes(), out),
        PdfObject::String(bytes) => {
            if bytes.iter().all(|b| (0x20..0x7f).contains(b) || matches!(b, b'\n' | b'\r' | b'\t')) {
                write_literal(bytes, out);
            } else {
                write_hex(bytes, out);
            }
        }
        PdfObject::Array(items) => {
            out.push(b'[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(b' ');
                }
                write_pdf(item, out);
            }
            out.push(b']');
        }
        PdfObject::Dictionary(dictionary) => write_pdf_dictionary(dictionary, None, out),
        PdfObject::Stream { dict, raw } => {
            write_pdf_dictionary(dict, Some(raw.len()), out);
            out.extend_from_slice(b"\nstream\n");
            out.extend_from_slice(raw);
            out.extend_from_slice(b"\nendstream");
        }
        PdfObject::Reference { number, generation } => {
            let _ = write!(out, "{number} {generation} R");
        }
    }
}

fn write_pdf_dictionary(dictionary: &PdfDictionary, length: Option<usize>, out: &mut Vec<u8>) {
    out.extend_from_slice(b"<<");
    for (key, value) in dictionary.iter() {
        if length.is_some() && key == "Length" {
            continue;
        }
        write_name(key.as_bytes(), out);
        out.push(b' ');
        write_pdf(value, out);
        out.push(b'\n');
    }
    if let Some(length) = length {
        let _ = writeln!(out, "/Length {length}");
    }
    out.extend_from_slice(b">>");
}

/// An engine object as lopdf.
pub(crate) fn to_lopdf(object: PdfObject) -> Object {
    match object {
        PdfObject::Null => Object::Null,
        PdfObject::Boolean(value) => Object::Boolean(value),
        PdfObject::Integer(value) => Object::Integer(value),
        PdfObject::Real(value) => Object::Real(value as f32),
        PdfObject::String(bytes) => Object::String(bytes, StringFormat::Literal),
        PdfObject::Name(name) => Object::Name(name.into_bytes()),
        PdfObject::Array(items) => Object::Array(items.into_iter().map(to_lopdf).collect()),
        PdfObject::Dictionary(dictionary) => Object::Dictionary(to_lopdf_dict(dictionary)),
        PdfObject::Stream { dict, raw } => {
            let mut dictionary = to_lopdf_dict(dict);
            dictionary.set("Length", Object::Integer(raw.len() as i64));
            Object::Stream(lopdf::Stream::new(dictionary, raw))
        }
        PdfObject::Reference { number, generation } => Object::Reference((number, generation)),
    }
}

fn to_lopdf_dict(dictionary: PdfDictionary) -> Dictionary {
    let mut out = Dictionary::new();
    for (key, value) in dictionary.iter() {
        out.set(key.as_bytes().to_vec(), to_lopdf(value.clone()));
    }
    out
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

/// The references an object makes, skipping a page tree node's `/Parent`
/// and an annotation's `/P` (which would pull in the page tree).
fn children(object: &Object, out: &mut Vec<ObjectId>) {
    match object {
        Object::Reference(id) => out.push(*id),
        Object::Array(items) => items.iter().for_each(|item| children(item, out)),
        Object::Dictionary(d) => {
            let (tree, annotation) = (is_page_tree(d), is_annotation(d));
            d.iter()
                .filter(|(k, _)| !(tree && k.as_slice() == b"Parent") && !(annotation && k.as_slice() == b"P"))
                .for_each(|(_, v)| children(v, out));
        }
        Object::Stream(s) => s.dict.iter().for_each(|(_, v)| children(v, out)),
        _ => {}
    }
}

/// Objects of `document` reachable from `start` (see [`children`]).
fn reachable(document: &Document, start: ObjectId) -> Vec<ObjectId> {
    let mut seen = BTreeSet::from([start]);
    let mut order = vec![start];
    let mut queue = VecDeque::from([start]);
    while let Some(id) = queue.pop_front() {
        let Ok(object) = document.get_object(id) else { continue };
        let mut kids = Vec::new();
        children(object, &mut kids);
        for kid in kids {
            if document.get_object(kid).is_ok() && seen.insert(kid) {
                order.push(kid);
                queue.push_back(kid);
            }
        }
    }
    order
}

/// Whether the catalogue declares signatures (AcroForm `/SigFlags` bit 1),
/// read lazily so a large document is not parsed whole.
pub fn signed_lazily(engine: &wellfriendpdf_engine::ContentEngine) -> bool {
    let reader = engine.document().reader();
    let Some(form) = acroform(reader) else { return false };
    form.get("SigFlags")
        .and_then(|f| reader.resolve(f.clone()).ok())
        .is_some_and(|f| matches!(f, PdfObject::Integer(flags) if flags & 1 == 1))
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
fn pair_objects(window: &Document, window_page: ObjectId, reader: &PdfReader, page: ObjectId) -> BTreeMap<ObjectId, ObjectId> {
    fn walk(a: &Object, b: &PdfObject, pairs: &mut BTreeMap<ObjectId, ObjectId>, queue: &mut VecDeque<(ObjectId, ObjectId)>) {
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
        y: &PdfDictionary,
        pairs: &mut BTreeMap<ObjectId, ObjectId>,
        queue: &mut VecDeque<(ObjectId, ObjectId)>,
    ) {
        let (tree, annotation) = (is_page_tree(x), is_annotation(x));
        for (key, value) in x.iter() {
            if (tree && key.as_slice() == b"Parent") || (annotation && key.as_slice() == b"P") {
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
fn reusable(before: &Document, after: &Document, objects: &[ObjectId], window_page: ObjectId) -> BTreeSet<ObjectId> {
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
            if kids.iter().any(|kid| !keep.contains(kid) && after.get_object(*kid).is_ok()) {
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

/// Where a page of the saved document comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Slot {
    /// Page `n` (1-based) of the source document.
    Source(u32),
    /// The `k`-th page inserted by the save's structure operations.
    Insert(usize),
}

/// What a windowed save changes.
pub struct Save<'a> {
    /// Edited windows of source pages.
    pub windows: &'a [EditedWindow],
    /// The (edited) window of every inserted page, by insert index.
    pub inserts: &'a [Vec<u8>],
    /// The saved page order, when the structure changed.
    pub order: Option<&'a [Slot]>,
    /// Source pages a redaction edited: their thumbnails are dropped and the
    /// annotations it removed are dropped from the field tree as well.
    pub redacted: &'a BTreeSet<u32>,
}

/// The objects a windowed save writes over the original document.
pub struct Plan {
    /// New and rewritten objects by number (with their generation).
    objects: BTreeMap<u32, (u16, Object)>,
    next: u32,
    root: ObjectId,
    info: Option<ObjectId>,
    id: Option<Vec<u8>>,
    previous: usize,
    /// Annotations a redaction removed from their page: in a full rewrite
    /// every reference to them is dropped too (field tree, popups).
    removed: BTreeSet<ObjectId>,
    /// `/Pages` nodes whose `/Kids` changed.
    touched: BTreeSet<ObjectId>,
    /// Pages of the saved document.
    pub page_count: usize,
}

impl Plan {
    /// The newest definition of `id`: the plan's, else the original's.
    fn current(&self, reader: &PdfReader, id: ObjectId) -> Result<Object, EditorError> {
        if let Some((generation, object)) = self.objects.get(&id.0) {
            if *generation == id.1 {
                return Ok(object.clone());
            }
        }
        Ok(to_lopdf(reader.get_object(id.0, id.1)?))
    }

    fn dict(&self, reader: &PdfReader, id: ObjectId) -> Result<Dictionary, EditorError> {
        match self.current(reader, id)? {
            Object::Dictionary(dictionary) => Ok(dictionary),
            _ => Err(refused(format!("object {} is not a dictionary", id.0))),
        }
    }

    fn put(&mut self, id: ObjectId, object: Object) {
        self.objects.insert(id.0, (id.1, object));
    }

    fn allocate(&mut self) -> ObjectId {
        let id = (self.next, 0);
        self.next += 1;
        id
    }

    /// Splice an edited source-page window: the page dictionary keeps its
    /// number, field objects are rewritten in place, every other changed
    /// object is appended under a new number.
    fn splice_window(
        &mut self,
        reader: &PdfReader,
        target_id: ObjectId,
        window: &EditedWindow,
        redacted: bool,
    ) -> Result<(), EditorError> {
        let parent = match reader.get_object(target_id.0, target_id.1)? {
            PdfObject::Dictionary(dictionary) => match dictionary.get("Parent") {
                Some(PdfObject::Reference { number, generation }) => (*number, *generation),
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
        // A changed field dictionary or widget is the document's field: it is
        // rewritten under its own number so the field tree sees the edit.
        let in_place: BTreeSet<ObjectId> = objects
            .iter()
            .copied()
            .filter(|id| *id != window_page && !keep.contains(id) && pairs.contains_key(id))
            .filter(|id| edited.get_dictionary(*id).is_ok_and(is_field))
            .collect();
        let mut map = BTreeMap::new();
        map.insert(window_page, target_id);
        for object in &objects {
            if *object == window_page {
                continue;
            }
            if keep.contains(object) || in_place.contains(object) {
                map.insert(*object, pairs[object]);
            } else {
                let id = self.allocate();
                map.insert(*object, id);
            }
        }
        for object in objects.into_iter().filter(|id| !keep.contains(id)) {
            let mut value = edited.get_object(object).map_err(|_| refused("a window object is missing"))?.clone();
            remap(&mut value, &map);
            let new_id = map[&object];
            if let Object::Dictionary(dictionary) = &mut value {
                if object == window_page {
                    dictionary.set("Parent", Object::Reference(parent));
                    // The page's own rotation: an ancestor's never applies.
                    if !dictionary.has(b"Rotate") {
                        dictionary.set("Rotate", Object::Integer(0));
                    }
                    if redacted {
                        // An embedded thumbnail still shows what was removed.
                        dictionary.remove(b"Thumb");
                    }
                } else if in_place.contains(&object) {
                    // The window cut the widget's page link; keep the original's.
                    if let Ok(PdfObject::Dictionary(original)) = reader.get_object(new_id.0, new_id.1) {
                        if let Some(link) = original.get("P") {
                            dictionary.set("P", to_lopdf(link.clone()));
                        }
                    }
                }
            }
            self.put(new_id, value);
        }
        if redacted {
            let original: BTreeSet<ObjectId> = match reader.get_object(target_id.0, target_id.1)? {
                PdfObject::Dictionary(dictionary) => match dictionary.get("Annots").map(|a| reader.resolve(a.clone())) {
                    Some(Ok(PdfObject::Array(items))) => items.iter().filter_map(PdfObject::as_reference).collect(),
                    _ => BTreeSet::new(),
                },
                _ => BTreeSet::new(),
            };
            let page = self.dict(reader, target_id)?;
            let kept: BTreeSet<ObjectId> = match page.get(b"Annots") {
                Ok(Object::Array(items)) => items.iter().filter_map(|i| i.as_reference().ok()).collect(),
                Ok(Object::Reference(id)) => match self.current(reader, *id) {
                    Ok(Object::Array(items)) => items.iter().filter_map(|i| i.as_reference().ok()).collect(),
                    _ => BTreeSet::new(),
                },
                _ => BTreeSet::new(),
            };
            self.removed.extend(original.difference(&kept).copied());
        }
        Ok(())
    }

    /// Append an inserted page's window; returns its new page id (its
    /// `/Parent` is set when the page tree is rewritten).
    fn append_page(&mut self, window: &[u8]) -> Result<ObjectId, EditorError> {
        let document = Document::load_mem(window).map_err(|e| refused(format!("an inserted page is invalid: {e}")))?;
        let page = *document.get_pages().get(&1).ok_or_else(|| refused("an inserted page is missing"))?;
        let objects = reachable(&document, page);
        let mut map = BTreeMap::new();
        for object in &objects {
            let id = self.allocate();
            map.insert(*object, id);
        }
        for object in objects {
            let mut value = document.get_object(object).map_err(|_| refused("an inserted object is missing"))?.clone();
            remap(&mut value, &map);
            if object == page {
                if let Object::Dictionary(dictionary) = &mut value {
                    // Self-contained: nothing is inherited from its new parent.
                    if !dictionary.has(b"Rotate") {
                        dictionary.set("Rotate", Object::Integer(0));
                    }
                    if !dictionary.has(b"CropBox") {
                        if let Ok(media) = dictionary.get(b"MediaBox").cloned() {
                            dictionary.set("CropBox", media);
                        }
                    }
                    if !dictionary.has(b"Resources") {
                        dictionary.set("Resources", Dictionary::new());
                    }
                }
            }
            self.put(map[&object], value);
        }
        Ok(map[&page])
    }

    fn kids(&self, reader: &PdfReader, node: ObjectId) -> Result<Vec<ObjectId>, EditorError> {
        let dictionary = self.dict(reader, node)?;
        let kids = match dictionary.get(b"Kids") {
            Ok(Object::Reference(id)) => self.current(reader, *id)?,
            Ok(other) => other.clone(),
            Err(_) => Object::Array(Vec::new()),
        };
        Ok(kids
            .as_array()
            .map(|items| items.iter().filter_map(|i| i.as_reference().ok()).collect())
            .unwrap_or_default())
    }

    fn set_kids(&mut self, reader: &PdfReader, node: ObjectId, kids: &[ObjectId]) -> Result<(), EditorError> {
        let mut dictionary = self.dict(reader, node)?;
        dictionary.set("Kids", Object::Array(kids.iter().map(|id| Object::Reference(*id)).collect()));
        self.put(node, Object::Dictionary(dictionary));
        self.touched.insert(node);
        Ok(())
    }

    fn parent(&self, reader: &PdfReader, id: ObjectId) -> Result<ObjectId, EditorError> {
        self.dict(reader, id)?
            .get(b"Parent")
            .and_then(Object::as_reference)
            .map_err(|_| refused("a page has no parent node"))
    }

    /// Remove `leaf` from its parent's `/Kids`; a node left empty (not the
    /// root) is removed from its own parent.
    fn detach(&mut self, reader: &PdfReader, leaf: ObjectId, root: ObjectId) -> Result<(), EditorError> {
        let mut current = leaf;
        for _ in 0..64 {
            let node = self.parent(reader, current)?;
            let mut kids = self.kids(reader, node)?;
            kids.retain(|kid| *kid != current);
            self.set_kids(reader, node, &kids)?;
            if !kids.is_empty() || node == root {
                return Ok(());
            }
            current = node;
        }
        Err(refused("the page tree is too deep"))
    }

    /// Insert `leaf` into `node`'s `/Kids` at `index` and point it at `node`.
    fn attach(&mut self, reader: &PdfReader, leaf: ObjectId, node: ObjectId, index: usize) -> Result<(), EditorError> {
        let mut kids = self.kids(reader, node)?;
        kids.insert(index.min(kids.len()), leaf);
        self.set_kids(reader, node, &kids)?;
        let mut page = self.dict(reader, leaf)?;
        page.set("Parent", Object::Reference(node));
        self.put(leaf, Object::Dictionary(page));
        Ok(())
    }

    /// Give a page that changes parent its inherited attributes as its own.
    fn materialize(&mut self, reader: &PdfReader, page: &wellfriendpdf_engine::PdfPage) -> Result<(), EditorError> {
        let id = (page.object_number, page.generation_number);
        let mut dictionary = self.dict(reader, id)?;
        let boxed = |b: [f64; 4]| Object::Array(b.iter().map(|v| Object::Real(*v as f32)).collect());
        if !dictionary.has(b"Resources") {
            dictionary.set("Resources", to_lopdf(PdfObject::Dictionary(page.resources.clone())));
        }
        if !dictionary.has(b"MediaBox") {
            dictionary.set("MediaBox", boxed(page.media_box));
        }
        if !dictionary.has(b"CropBox") {
            dictionary.set("CropBox", boxed(page.crop_box));
        }
        if !dictionary.has(b"Rotate") {
            dictionary.set("Rotate", Object::Integer(i64::from(page.rotate)));
        }
        self.put(id, Object::Dictionary(dictionary));
        Ok(())
    }

    /// Rewrite the page tree to `order`, touching only the `/Pages` nodes
    /// whose `/Kids` change and their ancestors' `/Count`. Pages that keep
    /// their relative order (the longest increasing run) stay where they
    /// are; deleted pages are detached; moved and inserted pages are placed
    /// next to their neighbour in the new order.
    fn rewrite_tree(
        &mut self,
        reader: &PdfReader,
        pages: &[wellfriendpdf_engine::PdfPage],
        order: &[Slot],
        inserted: &BTreeMap<usize, ObjectId>,
    ) -> Result<(), EditorError> {
        let catalog = self.dict(reader, self.root)?;
        let root = catalog
            .get(b"Pages")
            .and_then(Object::as_reference)
            .map_err(|_| refused("the document has no page tree"))?;
        let sources: Vec<u32> = order
            .iter()
            .filter_map(|slot| match slot {
                Slot::Source(page) => Some(*page),
                Slot::Insert(_) => None,
            })
            .collect();
        let anchors: BTreeSet<u32> = longest_increasing(&sources).into_iter().collect();
        let leaf = |slot: &Slot| -> Result<ObjectId, EditorError> {
            match slot {
                Slot::Source(page) => pages
                    .get(*page as usize - 1)
                    .map(|p| (p.object_number, p.generation_number))
                    .ok_or_else(|| EditorError::new("EDITOR_PAGE_NOT_FOUND", format!("page {page} does not exist"))),
                Slot::Insert(k) => inserted.get(k).copied().ok_or_else(|| refused("an inserted page is missing")),
            }
        };
        for (index, page) in pages.iter().enumerate() {
            if !anchors.contains(&(index as u32 + 1)) {
                self.detach(reader, (page.object_number, page.generation_number), root)?;
            }
        }
        let mut cursor: Option<ObjectId> = None;
        for (position, slot) in order.iter().enumerate() {
            let id = leaf(slot)?;
            if let Slot::Source(page) = slot {
                if anchors.contains(page) {
                    cursor = Some(id);
                    continue;
                }
                self.materialize(reader, &pages[*page as usize - 1])?;
            }
            let (node, index) = match cursor {
                Some(previous) => {
                    let node = self.parent(reader, previous)?;
                    let at = self.kids(reader, node)?.iter().position(|k| *k == previous).map_or(usize::MAX, |i| i + 1);
                    (node, at)
                }
                None => match order[position..].iter().find(|s| matches!(s, Slot::Source(p) if anchors.contains(p))) {
                    Some(next) => {
                        let next = leaf(next)?;
                        let node = self.parent(reader, next)?;
                        let at = self.kids(reader, node)?.iter().position(|k| *k == next).unwrap_or(0);
                        (node, at)
                    }
                    None => (root, usize::MAX),
                },
            };
            self.attach(reader, id, node, index)?;
            cursor = Some(id);
        }
        self.recount(reader, root)
    }

    /// Recompute `/Count` of every touched node and its ancestors.
    fn recount(&mut self, reader: &PdfReader, root: ObjectId) -> Result<(), EditorError> {
        let mut stale = BTreeSet::new();
        for node in self.touched.clone() {
            let mut current = node;
            for _ in 0..64 {
                if !stale.insert(current) || current == root {
                    break;
                }
                match self.parent(reader, current) {
                    Ok(parent) => current = parent,
                    Err(_) => break,
                }
            }
        }
        stale.insert(root);
        let mut counts: HashMap<ObjectId, i64> = HashMap::new();
        fn count(
            plan: &Plan,
            reader: &PdfReader,
            node: ObjectId,
            stale: &BTreeSet<ObjectId>,
            counts: &mut HashMap<ObjectId, i64>,
            depth: usize,
        ) -> Result<i64, EditorError> {
            if let Some(value) = counts.get(&node) {
                return Ok(*value);
            }
            if depth > 64 {
                return Err(refused("the page tree is too deep"));
            }
            let dictionary = plan.dict(reader, node)?;
            let is_node = dictionary.has(b"Kids") || is_page_tree(&dictionary) && dictionary.get(b"Type").ok().and_then(|t| t.as_name().ok()) == Some(b"Pages".as_slice());
            if !is_node {
                return Ok(1);
            }
            let total = if stale.contains(&node) {
                let mut total = 0;
                for kid in plan.kids(reader, node)? {
                    total += count(plan, reader, kid, stale, counts, depth + 1)?;
                }
                total
            } else {
                dictionary.get(b"Count").and_then(Object::as_i64).unwrap_or(0)
            };
            counts.insert(node, total);
            Ok(total)
        }
        for node in &stale {
            let total = count(self, reader, *node, &stale, &mut counts, 0)?;
            let mut dictionary = self.dict(reader, *node)?;
            dictionary.set("Count", Object::Integer(total));
            self.put(*node, Object::Dictionary(dictionary));
        }
        Ok(())
    }

    /// Append the plan as an incremental update to an original of
    /// `base_len` bytes (`newline`: whether it ends with one). Returns the
    /// bytes written.
    pub fn write_update(&self, base_len: usize, newline: bool, out: &mut dyn Write) -> std::io::Result<u64> {
        let mut position = base_len;
        let mut buffer = Vec::new();
        if !newline {
            buffer.push(b'\n');
        }
        let mut offsets = BTreeMap::new();
        for (number, (generation, object)) in &self.objects {
            offsets.insert(*number, (position + buffer.len(), *generation));
            let _ = writeln!(buffer, "{number} {generation} obj");
            write_object(object, &mut buffer);
            buffer.extend_from_slice(b"\nendobj\n");
            if buffer.len() >= 1 << 20 {
                out.write_all(&buffer)?;
                position += buffer.len();
                buffer.clear();
            }
        }
        let xref = position + buffer.len();
        self.write_xref(&offsets, xref, true, &mut buffer);
        out.write_all(&buffer)?;
        position += buffer.len();
        Ok((position - base_len) as u64)
    }

    /// The cross-reference table and trailer. An update lists the written
    /// objects and links to the previous section; a rewrite lists every
    /// number up to its size.
    fn write_xref(&self, offsets: &BTreeMap<u32, (usize, u16)>, xref: usize, update: bool, out: &mut Vec<u8>) {
        out.extend_from_slice(b"xref\n");
        let size;
        if update {
            let numbers: Vec<u32> = offsets.keys().copied().collect();
            let mut index = 0;
            while index < numbers.len() {
                let mut end = index + 1;
                while end < numbers.len() && numbers[end] == numbers[end - 1] + 1 {
                    end += 1;
                }
                let _ = writeln!(out, "{} {}", numbers[index], end - index);
                for number in &numbers[index..end] {
                    let (offset, generation) = offsets[number];
                    let _ = write!(out, "{offset:010} {generation:05} n\r\n");
                }
                index = end;
            }
            size = self.next.max(offsets.keys().next_back().map_or(0, |n| n + 1));
        } else {
            size = offsets.keys().next_back().map_or(1, |n| n + 1);
            let free: Vec<u32> = (1..size).filter(|n| !offsets.contains_key(n)).collect();
            let _ = writeln!(out, "0 {size}");
            let _ = write!(out, "{:010} 65535 f\r\n", free.first().copied().unwrap_or(0));
            let mut next_free = free.iter().skip(1);
            for number in 1..size {
                match offsets.get(&number) {
                    Some((offset, generation)) => {
                        let _ = write!(out, "{offset:010} {generation:05} n\r\n");
                    }
                    None => {
                        let _ = write!(out, "{:010} 00000 f\r\n", next_free.next().copied().unwrap_or(0));
                    }
                }
            }
        }
        let _ = write!(out, "trailer\n<</Size {size} /Root {} {} R", self.root.0, self.root.1);
        if update {
            let _ = write!(out, " /Prev {}", self.previous);
        }
        if let Some((number, generation)) = self.info {
            let _ = write!(out, " /Info {number} {generation} R");
        }
        if let Some(id) = &self.id {
            let first: String = id.iter().map(|b| format!("{b:02X}")).collect();
            let second: String = if update {
                first.clone()
            } else {
                use sha2::Digest;
                let mut hasher = sha2::Sha256::new();
                hasher.update(id);
                hasher.update(b"wellfriend-full-rewrite");
                for (number, (offset, _)) in offsets.iter().take(64) {
                    hasher.update(number.to_le_bytes());
                    hasher.update(offset.to_le_bytes());
                }
                hasher.finalize()[..16].iter().map(|b| format!("{b:02X}")).collect()
            };
            let _ = write!(out, " /ID [<{first}> <{second}>]");
        }
        let _ = write!(out, ">>\nstartxref\n{xref}\n%%EOF\n");
    }

    /// Write the whole document anew (see the module docs): every object
    /// reachable from the catalogue and the document information, newest
    /// definition only, streams copied encoded; references to annotations a
    /// redaction removed are dropped; with `scrub`, document-level strings
    /// lose the redacted text. Returns the bytes written.
    pub fn write_rewrite(&self, reader: &PdfReader, scrub: &Scrub, out: &mut dyn Write) -> Result<u64, EditorError> {
        let io = |e: std::io::Error| EditorError::new("EDITOR_IO_FAILED", format!("the rewrite could not be written: {e}"));
        let mut position = 0usize;
        let mut buffer = Vec::with_capacity(1 << 20);
        let version = reader.version();
        let version = if version.len() == 3 && version.starts_with("1.") || version == "2.0" { version } else { "1.7" };
        let _ = writeln!(buffer, "%PDF-{version}");
        buffer.extend_from_slice(b"%\xE2\xE3\xCF\xD3\n");
        let mut offsets: BTreeMap<u32, (usize, u16)> = BTreeMap::new();
        let mut queue: VecDeque<ObjectId> = VecDeque::from([self.root]);
        if let Some(info) = self.info {
            queue.push_back(info);
        }
        let mut seen = BTreeSet::new();
        while let Some(id) = queue.pop_front() {
            if self.removed.contains(&id) || !seen.insert(id.0) {
                continue;
            }
            let start = buffer.len();
            let _ = writeln!(buffer, "{} {} obj", id.0, id.1);
            let mut kids = Vec::new();
            match self.objects.get(&id.0) {
                Some((generation, object)) if *generation == id.1 => {
                    let mut object = object.clone();
                    prune_lopdf(&mut object, &self.removed);
                    if !scrub.is_empty() {
                        scrub.lopdf(&mut object, Some(id) == self.info);
                    }
                    children_all(&object, &mut kids);
                    write_object(&object, &mut buffer);
                }
                _ => {
                    let Ok(mut object) = reader.get_object(id.0, id.1) else {
                        // A dangling reference: keep the number as null.
                        buffer.truncate(start);
                        let _ = writeln!(buffer, "{} {} obj", id.0, id.1);
                        buffer.extend_from_slice(b"null\nendobj\n");
                        offsets.insert(id.0, (position + start, id.1));
                        continue;
                    };
                    if let PdfObject::Stream { dict, .. } = &mut object {
                        // A direct length: the length object is not needed.
                        dict.remove("Length");
                    }
                    prune_pdf(&mut object, &self.removed);
                    if !scrub.is_empty() {
                        scrub.pdf(&mut object, Some(id) == self.info);
                    }
                    pdf_children(&object, &mut kids);
                    write_pdf(&object, &mut buffer);
                }
            }
            buffer.extend_from_slice(b"\nendobj\n");
            offsets.insert(id.0, (position + start, id.1));
            for kid in kids {
                if !seen.contains(&kid.0) && !self.removed.contains(&kid) {
                    queue.push_back(kid);
                }
            }
            if buffer.len() >= 1 << 20 {
                out.write_all(&buffer).map_err(io)?;
                position += buffer.len();
                buffer.clear();
            }
        }
        let xref = position + buffer.len();
        self.write_xref(&offsets, xref, false, &mut buffer);
        out.write_all(&buffer).map_err(io)?;
        position += buffer.len();
        Ok(position as u64)
    }
}

/// Every reference an object makes (the rewrite keeps whatever is reachable).
fn children_all(object: &Object, out: &mut Vec<ObjectId>) {
    match object {
        Object::Reference(id) => out.push(*id),
        Object::Array(items) => items.iter().for_each(|item| children_all(item, out)),
        Object::Dictionary(d) => d.iter().for_each(|(_, v)| children_all(v, out)),
        Object::Stream(s) => s.dict.iter().for_each(|(_, v)| children_all(v, out)),
        _ => {}
    }
}

fn pdf_children(object: &PdfObject, out: &mut Vec<ObjectId>) {
    match object {
        PdfObject::Reference { number, generation } => out.push((*number, *generation)),
        PdfObject::Array(items) => items.iter().for_each(|item| pdf_children(item, out)),
        PdfObject::Dictionary(d) => d.iter().for_each(|(_, v)| pdf_children(v, out)),
        PdfObject::Stream { dict, .. } => dict.iter().for_each(|(_, v)| pdf_children(v, out)),
        _ => {}
    }
}

/// Keys whose string values a redaction scrubs anywhere in the document.
const SCRUBBED_KEYS: [&str; 12] = [
    "Title", "Subject", "Keywords", "Author", "Contents", "RC", "V", "DV", "TU", "ActualText", "Alt", "E",
];

/// Drop references to removed annotations; a field that lost a widget also
/// loses its value (the value was shown where the redaction removed it).
fn prune_pdf(object: &mut PdfObject, removed: &BTreeSet<ObjectId>) {
    if removed.is_empty() {
        return;
    }
    let gone = |o: &PdfObject| o.as_reference().is_some_and(|id| removed.contains(&id));
    match object {
        PdfObject::Array(items) => {
            items.retain(|item| !gone(item));
            items.iter_mut().for_each(|item| prune_pdf(item, removed));
        }
        PdfObject::Dictionary(dictionary) | PdfObject::Stream { dict: dictionary, .. } => {
            let keys: Vec<String> = dictionary.iter().filter(|(_, v)| gone(v)).map(|(k, _)| k.clone()).collect();
            for key in keys {
                dictionary.remove(&key);
            }
            let lost_kid = dictionary.get("Kids").and_then(PdfObject::as_array).is_some_and(|kids| kids.iter().any(gone));
            for (_, value) in dictionary.entries_mut() {
                prune_pdf(value, removed);
            }
            if lost_kid && (dictionary.contains_key("FT") || dictionary.contains_key("T")) {
                dictionary.remove("V");
                dictionary.remove("DV");
            }
        }
        _ => {}
    }
}

fn prune_lopdf(object: &mut Object, removed: &BTreeSet<ObjectId>) {
    if removed.is_empty() {
        return;
    }
    let gone = |o: &Object| o.as_reference().is_ok_and(|id| removed.contains(&id));
    match object {
        Object::Array(items) => {
            items.retain(|item| !gone(item));
            items.iter_mut().for_each(|item| prune_lopdf(item, removed));
        }
        Object::Dictionary(dictionary) | Object::Stream(lopdf::Stream { dict: dictionary, .. }) => {
            let keys: Vec<Vec<u8>> = dictionary.iter().filter(|(_, v)| gone(v)).map(|(k, _)| k.clone()).collect();
            for key in keys {
                dictionary.remove(&key);
            }
            let lost_kid = dictionary.get(b"Kids").and_then(Object::as_array).is_ok_and(|kids| kids.iter().any(gone));
            for (_, value) in dictionary.iter_mut() {
                prune_lopdf(value, removed);
            }
            if lost_kid && (dictionary.has(b"FT") || dictionary.has(b"T")) {
                dictionary.remove(b"V");
                dictionary.remove(b"DV");
            }
        }
        _ => {}
    }
}

/// Redacted text removed from document-level strings during a rewrite:
/// every run of text a redaction removed and every word of five or more
/// characters in it, wherever it appears in an outline title, the document
/// information, an annotation's contents, a field value or tooltip, or
/// structure alternate text, and in XMP metadata.
#[derive(Debug, Clone, Default)]
pub struct Scrub {
    secrets: Vec<String>,
}

impl Scrub {
    pub fn new(runs: impl IntoIterator<Item = String>) -> Self {
        let mut secrets = BTreeSet::new();
        for run in runs {
            let run = run.trim().to_owned();
            for word in run.split_whitespace() {
                let word = word.trim_matches(|c: char| c.is_ascii_punctuation() && c != '-');
                if word.chars().count() >= 5 {
                    secrets.insert(word.to_owned());
                }
            }
            if run.chars().count() >= 2 {
                secrets.insert(run);
            }
        }
        let mut secrets: Vec<String> = secrets.into_iter().collect();
        // Longest first, so a run goes before the words inside it.
        secrets.sort_by_key(|s| std::cmp::Reverse(s.chars().count()));
        Self { secrets }
    }

    pub fn is_empty(&self) -> bool {
        self.secrets.is_empty()
    }

    fn text(&self, bytes: &[u8]) -> Option<Vec<u8>> {
        let text = decode_text(bytes);
        let scrubbed = self.secrets.iter().fold(text.clone(), |acc, secret| acc.replace(secret.as_str(), ""));
        (scrubbed != text).then(|| encode_text(&scrubbed))
    }

    fn pdf(&self, object: &mut PdfObject, every_string: bool) {
        match object {
            PdfObject::String(bytes) if every_string => {
                if let Some(scrubbed) = self.text(bytes) {
                    *bytes = scrubbed;
                }
            }
            PdfObject::Array(items) => items.iter_mut().for_each(|item| self.pdf(item, every_string)),
            PdfObject::Dictionary(dictionary) => {
                for (key, value) in dictionary.entries_mut() {
                    self.pdf(value, every_string || SCRUBBED_KEYS.contains(&key.as_str()));
                }
            }
            PdfObject::Stream { dict, raw } if dict.get_name("Type").is_some_and(|t| t.eq_ignore_ascii_case("Metadata")) => {
                let lopdf_dict = to_lopdf_dict(dict.clone());
                let stream = lopdf::Stream::new(lopdf_dict.clone(), raw.clone());
                let decoded = if lopdf_dict.has(b"Filter") { stream.decompressed_content().ok() } else { Some(raw.clone()) };
                if let Some(scrubbed) = decoded.and_then(|data| self.bytes(&data)) {
                    dict.remove("Filter");
                    dict.remove("DecodeParms");
                    *raw = scrubbed;
                }
            }
            _ => {}
        }
    }

    fn lopdf(&self, object: &mut Object, every_string: bool) {
        match object {
            Object::String(bytes, format) if every_string => {
                if let Some(scrubbed) = self.text(bytes) {
                    *bytes = scrubbed;
                    *format = StringFormat::Literal;
                }
            }
            Object::Array(items) => items.iter_mut().for_each(|item| self.lopdf(item, every_string)),
            Object::Dictionary(dictionary) => {
                for (key, value) in dictionary.iter_mut() {
                    let keyed = std::str::from_utf8(key).is_ok_and(|k| SCRUBBED_KEYS.contains(&k));
                    self.lopdf(value, every_string || keyed);
                }
            }
            Object::Stream(stream)
                if stream.dict.get(b"Type").and_then(Object::as_name).is_ok_and(|t| t.eq_ignore_ascii_case(b"Metadata")) =>
            {
                let decoded = if stream.dict.has(b"Filter") { stream.decompressed_content().ok() } else { Some(stream.content.clone()) };
                if let Some(scrubbed) = decoded.and_then(|data| self.bytes(&data)) {
                    stream.dict.remove(b"Filter");
                    stream.dict.remove(b"DecodeParms");
                    stream.set_content(scrubbed);
                }
            }
            _ => {}
        }
    }

    /// A metadata payload (UTF-8 XML) without the redacted text.
    fn bytes(&self, data: &[u8]) -> Option<Vec<u8>> {
        let mut current = data.to_vec();
        let mut changed = false;
        for secret in &self.secrets {
            let escaped = secret.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
            for needle in [secret.as_bytes(), escaped.as_bytes()] {
                if needle.is_empty() {
                    continue;
                }
                while let Some(at) = current.windows(needle.len()).position(|w| w == needle) {
                    current.drain(at..at + needle.len());
                    changed = true;
                }
            }
        }
        changed.then_some(current)
    }
}

/// A PDF text string as Unicode (UTF-16BE or UTF-8 with a byte order mark,
/// else PDFDocEncoding read as Latin-1).
fn decode_text(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
        String::from_utf16_lossy(&units)
    } else if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8_lossy(rest).into_owned()
    } else {
        bytes.iter().map(|&b| b as char).collect()
    }
}

fn encode_text(text: &str) -> Vec<u8> {
    if text.is_ascii() {
        text.as_bytes().to_vec()
    } else {
        let mut out = vec![0xFE, 0xFF];
        for unit in text.encode_utf16() {
            out.extend_from_slice(&unit.to_be_bytes());
        }
        out
    }
}

/// The longest strictly increasing subsequence of `values`.
fn longest_increasing(values: &[u32]) -> Vec<u32> {
    let mut tails: Vec<usize> = Vec::new();
    let mut previous = vec![usize::MAX; values.len()];
    for (index, value) in values.iter().enumerate() {
        let at = tails.partition_point(|&t| values[t] < *value);
        if at > 0 {
            previous[index] = tails[at - 1];
        }
        if at == tails.len() {
            tails.push(index);
        } else {
            tails[at] = index;
        }
    }
    let mut out = Vec::with_capacity(tails.len());
    let mut current = tails.last().copied().unwrap_or(usize::MAX);
    while current != usize::MAX {
        out.push(values[current]);
        current = previous[current];
    }
    out.reverse();
    out
}

/// Build the plan of a windowed save over the document `engine` opened.
pub fn plan(engine: &ContentEngine, save: &Save<'_>) -> Result<Plan, EditorError> {
    let reader = engine.document().reader();
    if reader.is_encrypted() {
        return Err(refused("encrypted PDFs are edited with a full rewrite, not page windows"));
    }
    let root = reader.root_reference().ok_or_else(|| refused("the document has no catalogue"))?;
    let next = reader
        .size()
        .map(|size| size as u32)
        .into_iter()
        .chain(reader.object_ids().iter().map(|(n, _)| n + 1).max())
        .max()
        .ok_or_else(|| refused("the document size is unknown"))?;
    let pages = engine.document().get_pages()?;
    let identity: Vec<Slot> = (1..=pages.len() as u32).map(Slot::Source).collect();
    let order = save.order.unwrap_or(&identity);
    if order.is_empty() {
        return Err(EditorError::new("EDITOR_PAGES_REFUSED", "a PDF must keep at least one page"));
    }
    let kept: BTreeSet<u32> = order
        .iter()
        .filter_map(|s| match s {
            Slot::Source(p) => Some(*p),
            Slot::Insert(_) => None,
        })
        .collect();
    let mut plan = Plan {
        objects: BTreeMap::new(),
        next,
        root,
        info: reader.info_reference(),
        id: reader.first_file_id(),
        previous: reader.startxref_offset(),
        removed: BTreeSet::new(),
        touched: BTreeSet::new(),
        page_count: order.len(),
    };
    for window in save.windows {
        if !kept.contains(&window.page) {
            continue;
        }
        let target = pages
            .get(window.page as usize - 1)
            .ok_or_else(|| EditorError::new("EDITOR_PAGE_NOT_FOUND", format!("page {} does not exist", window.page)))?;
        plan.splice_window(
            reader,
            (target.object_number, target.generation_number),
            window,
            save.redacted.contains(&window.page),
        )?;
    }
    let mut inserted = BTreeMap::new();
    for slot in order {
        if let Slot::Insert(k) = slot {
            let bytes = save.inserts.get(*k).ok_or_else(|| refused("an inserted page is missing"))?;
            let id = plan.append_page(bytes)?;
            inserted.insert(*k, id);
        }
    }
    if order != identity.as_slice() {
        plan.rewrite_tree(reader, &pages, order, &inserted)?;
    }
    Ok(plan)
}

/// Append the edited windows' pages to `original` as one incremental update.
pub fn splice_incremental(original: &[u8], windows: &[EditedWindow]) -> Result<Vec<u8>, EditorError> {
    let engine = ContentEngine::open_bytes(original.to_vec())?;
    let none = BTreeSet::new();
    let plan = plan(
        &engine,
        &Save {
            windows,
            inserts: &[],
            order: None,
            redacted: &none,
        },
    )?;
    let mut out = original.to_vec();
    plan.write_update(original.len(), original.ends_with(b"\n"), &mut out)
        .map_err(|e| EditorError::new("EDITOR_IO_FAILED", e.to_string()))?;
    Ok(out)
}
