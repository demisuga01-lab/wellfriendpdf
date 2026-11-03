//! Editing text inside a Form XObject.
//!
//! [`PageTextLayout`] lists the text of every form the page places
//! ([`EditorFormText`]). An edit in that text runs the ordinary text
//! operation on a scratch page whose content is the form's content program
//! and whose resources are the form's, then writes the result back into the
//! form. A form placed more than once (twice on a page, on another page, or
//! from another object) is copied first: the edited placement then invokes
//! the copy and every other placement keeps the original.

use lopdf::content::Content;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use wellfriendpdf_engine::{EditorFormText, PageTextLayout};

use crate::apply::OperationReport;
use crate::error::EditorError;
use crate::fonts::page_resources;
use crate::overlay;

/// The form holding all of `[start, end)` when the range is past the page's
/// own text; an error when it straddles page and form text.
pub(crate) fn target(layout: &PageTextLayout, start: usize, end: usize) -> Result<Option<EditorFormText>, EditorError> {
    let page_text = layout.page_text_len();
    // An insertion at the end of the page's text stays page text, unless
    // the page has no text of its own and a form's text starts there.
    if end <= page_text && (start < page_text || layout.forms.first().is_none_or(|f| f.range[0] > start)) {
        return Ok(None);
    }
    match layout.form_at([start, end]) {
        Some(form) if form.editable => Ok(Some(form.clone())),
        Some(_) => Err(EditorError::new(
            "EDITOR_TEXT_NOT_EDITABLE",
            "this text is inside a form whose fonts do not decode to exact text",
        )),
        None => Err(EditorError::new(
            "EDITOR_TEXT_SPANS_FORM",
            "an edit cannot span the page text and text inside a placed form; edit them separately",
        )),
    }
}

/// Run `edit` on a scratch page holding `form`'s content (in form space,
/// with offsets local to the form's text) and write its result back into
/// the form placed on `page`. `edit` gets the scratch document and its page
/// number. The report's region and caret are mapped back to the page.
pub(crate) fn edit_in_form(
    input: &[u8],
    page: u32,
    form: &EditorFormText,
    report: &mut OperationReport,
    edit: impl FnOnce(&[u8], u32, &mut OperationReport) -> Result<Vec<u8>, EditorError>,
) -> Result<Vec<u8>, EditorError> {
    let (scratch, scratch_page) = scratch_document(input, page, form)?;
    let edited = edit(&scratch, scratch_page, report)?;
    // The form's box clips what it draws: it grows to hold the edited text
    // (a larger size or a longer line) so no glyph is cut off.
    let drawn = PageTextLayout::from_bytes(&edited, scratch_page as usize)?
        .glyphs
        .iter()
        .filter(|g| !g.text.trim().is_empty() && !g.invisible)
        .map(|g| g.bbox)
        .reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]);
    let mut document = overlay::load(&edited)?;
    let pages = document.get_pages();
    let scratch_id = *pages
        .get(&scratch_page)
        .ok_or_else(|| EditorError::new("EDITOR_TEXT_LAYOUT_FAILED", "the edited form page went missing"))?;
    let page_id = overlay::page_id(&document, page)?;
    let content = document.get_page_content(scratch_id);
    let resources = page_resources(&document, scratch_id).unwrap_or_default();
    remove_page(&mut document, scratch_id)?;
    let form_id = placed_form(&document, page_id, form)?;
    if shared(&document, form_id)? {
        let mut copy = document
            .get_object(form_id)
            .and_then(Object::as_stream)
            .map_err(|e| EditorError::new("EDITOR_WRITE_FAILED", e.to_string()))?
            .dict
            .clone();
        for key in [b"Filter".as_slice(), b"DecodeParms", b"Length"] {
            copy.remove(key);
        }
        copy.set("Resources", Object::Dictionary(resources));
        grow_box(&mut copy, drawn);
        let mut stream = Stream::new(copy, content);
        let _ = stream.compress();
        let copy_id = document.add_object(stream);
        let name = invoke_copy(&mut document, page_id, form, copy_id)?;
        report.notes.push(format!(
            "The text is inside a form placed more than once; this placement now uses its own copy ({name}) and the others are unchanged."
        ));
    } else {
        let stream = document
            .get_object_mut(form_id)
            .and_then(Object::as_stream_mut)
            .map_err(|e| EditorError::new("EDITOR_WRITE_FAILED", e.to_string()))?;
        stream.dict.set("Resources", Object::Dictionary(resources));
        grow_box(&mut stream.dict, drawn);
        stream.set_plain_content(content);
        let _ = stream.compress();
    }
    if let Some(region) = report.changed_region.as_mut() {
        *region = transform_box(&form.matrix, *region);
    }
    if let Some(caret) = report.caret.as_mut() {
        *caret += form.range[0];
    }
    overlay::save(&mut document)
}

/// The input with one more page (the last) whose content is the form's
/// decoded content and whose resources are the form's (or, when it has none,
/// the placing page's), its box the form's `/BBox`.
fn scratch_document(input: &[u8], page: u32, form: &EditorFormText) -> Result<(Vec<u8>, u32), EditorError> {
    let mut document = overlay::load(input)?;
    let page_id = overlay::page_id(&document, page)?;
    let form_id = placed_form(&document, page_id, form)?;
    let stream = document
        .get_object(form_id)
        .and_then(Object::as_stream)
        .map_err(|e| EditorError::new("EDITOR_TEXT_STALE", e.to_string()))?;
    let content = stream.decompressed_content().unwrap_or_else(|_| stream.content.clone());
    let bbox: Vec<f64> = stream
        .dict
        .get(b"BBox")
        .and_then(Object::as_array)
        .map(|items| items.iter().filter_map(|o| o.as_float().ok().map(f64::from)).collect())
        .unwrap_or_default();
    let bbox = match bbox[..] {
        [a, b, c, d] => [a.min(c), b.min(d), a.max(c), b.max(d)],
        _ => [0.0, 0.0, 612.0, 792.0],
    };
    // The scratch page is the placing page seen in form space (so a
    // paragraph that grows meets the real page bottom, and the reflow
    // engine's margins leave room on a small form), widened to the form box.
    let m = form.matrix;
    let det = m[0] * m[3] - m[1] * m[2];
    let page_box = overlay::media_box(&document, page_id)?;
    let media = if det.abs() > 1e-9 {
        let corners = [[page_box[0], page_box[1]], [page_box[2], page_box[1]], [page_box[0], page_box[3]], [page_box[2], page_box[3]]]
            .map(|[x, y]| [((x - m[4]) * m[3] - (y - m[5]) * m[2]) / det, ((y - m[5]) * m[0] - (x - m[4]) * m[1]) / det]);
        corners.iter().fold(bbox, |a, p| [a[0].min(p[0]), a[1].min(p[1]), a[2].max(p[0]), a[3].max(p[1])])
    } else {
        [bbox[0].min(0.0), bbox[1].min(0.0), bbox[2].max(612.0), bbox[3].max(792.0)]
    };
    let array = |b: [f64; 4]| Object::Array(b.iter().map(|v| Object::Real(*v as f32)).collect());
    let resources = match stream.dict.get(b"Resources") {
        Ok(Object::Reference(id)) => document.get_dictionary(*id).ok().cloned(),
        Ok(Object::Dictionary(dict)) => Some(dict.clone()),
        _ => None,
    }
    .or_else(|| page_resources(&document, page_id))
    .unwrap_or_default();
    // Sub-dictionaries are copied so the edit's additions (a library font)
    // land in the scratch page only.
    let resources = direct_children(&document, resources);
    let root = document
        .catalog()
        .and_then(|catalog| catalog.get(b"Pages"))
        .and_then(Object::as_reference)
        .map_err(|e| EditorError::new("EDITOR_PDF_INVALID", e.to_string()))?;
    let contents = document.add_object(Stream::new(Dictionary::new(), content));
    let mut scratch = Dictionary::new();
    scratch.set("Type", Object::Name(b"Page".to_vec()));
    scratch.set("Parent", Object::Reference(root));
    scratch.set("MediaBox", array(media));
    scratch.set("Resources", Object::Dictionary(resources));
    scratch.set("Contents", Object::Reference(contents));
    let scratch_id = document.add_object(scratch);
    let pages = document
        .get_dictionary_mut(root)
        .map_err(|e| EditorError::new("EDITOR_PDF_INVALID", e.to_string()))?;
    match pages.get_mut(b"Kids") {
        Ok(Object::Array(kids)) => kids.push(Object::Reference(scratch_id)),
        _ => return Err(EditorError::new("EDITOR_PDF_INVALID", "the page tree has no Kids array")),
    }
    let count = pages.get(b"Count").and_then(Object::as_i64).unwrap_or(0);
    pages.set("Count", Object::Integer(count + 1));
    let number = document.get_pages().len() as u32;
    Ok((overlay::save(&mut document)?, number))
}

fn direct_children(document: &Document, mut resources: Dictionary) -> Dictionary {
    let keys: Vec<Vec<u8>> = resources.iter().map(|(k, _)| k.clone()).collect();
    for key in keys {
        if let Ok(Object::Reference(id)) = resources.get(&key) {
            if let Ok(dict) = document.get_dictionary(*id) {
                let dict = dict.clone();
                resources.set(key, Object::Dictionary(dict));
            }
        }
    }
    resources
}

/// The form object the page's `Do` number `form.placement` invokes, checked
/// against the layout's record.
fn placed_form(document: &Document, page_id: ObjectId, form: &EditorFormText) -> Result<ObjectId, EditorError> {
    let stale = || EditorError::new("EDITOR_TEXT_STALE", "the form no longer matches the page; reopen the current version");
    let name = nth_invocation(document, page_id, form.placement)?.ok_or_else(stale)?;
    if name != form.name.as_bytes() {
        return Err(stale());
    }
    let resources = page_resources(document, page_id).ok_or_else(stale)?;
    let xobjects = resolve_dict(document, resources.get(b"XObject").map_err(|_| stale())?).ok_or_else(stale)?;
    xobjects.get(&name).and_then(Object::as_reference).map_err(|_| stale())
}

fn resolve_dict<'a>(document: &'a Document, object: &'a Object) -> Option<&'a Dictionary> {
    match object {
        Object::Reference(id) => document.get_dictionary(*id).ok(),
        Object::Dictionary(dict) => Some(dict),
        _ => None,
    }
}

fn page_operations(document: &Document, page_id: ObjectId) -> Result<Vec<Vec<lopdf::content::Operation>>, EditorError> {
    let mut out = Vec::new();
    for id in document.get_page_contents(page_id) {
        let stream = document
            .get_object(id)
            .and_then(Object::as_stream)
            .map_err(|e| EditorError::new("EDITOR_PDF_INVALID", e.to_string()))?;
        let plain = stream.decompressed_content().unwrap_or_else(|_| stream.content.clone());
        let content = Content::decode(&plain).map_err(|e| EditorError::new("EDITOR_PDF_INVALID", e.to_string()))?;
        out.push(content.operations);
    }
    Ok(out)
}

/// The XObject name of the `n`-th `Do` of the page content.
fn nth_invocation(document: &Document, page_id: ObjectId, n: usize) -> Result<Option<Vec<u8>>, EditorError> {
    Ok(page_operations(document, page_id)?
        .into_iter()
        .flatten()
        .filter(|op| op.operator == "Do")
        .nth(n)
        .and_then(|op| op.operands.first().and_then(|o| o.as_name().ok()).map(<[u8]>::to_vec)))
}

/// Whether anything but one `Do` on one page draws `form_id`: more than one
/// reference to it in the file, or more than one invocation of the names it
/// has in the pages that reference it.
fn shared(document: &Document, form_id: ObjectId) -> Result<bool, EditorError> {
    let mut references = 0usize;
    for object in document.objects.values() {
        count_references(object, form_id, &mut references);
    }
    if references > 1 {
        return Ok(true);
    }
    let mut placements = 0usize;
    for page_id in document.get_pages().into_values() {
        let Some(resources) = page_resources(document, page_id) else { continue };
        let Some(xobjects) = resources.get(b"XObject").ok().and_then(|o| resolve_dict(document, o)) else {
            continue;
        };
        let names: Vec<Vec<u8>> = xobjects
            .iter()
            .filter(|(_, value)| value.as_reference().ok() == Some(form_id))
            .map(|(name, _)| name.clone())
            .collect();
        if names.is_empty() {
            continue;
        }
        placements += page_operations(document, page_id)?
            .into_iter()
            .flatten()
            .filter(|op| op.operator == "Do")
            .filter(|op| op.operands.first().and_then(|o| o.as_name().ok()).is_some_and(|n| names.iter().any(|m| m == n)))
            .count();
    }
    Ok(placements > 1)
}

fn count_references(object: &Object, id: ObjectId, count: &mut usize) {
    match object {
        Object::Reference(r) if *r == id => *count += 1,
        Object::Array(items) => items.iter().for_each(|item| count_references(item, id, count)),
        Object::Dictionary(dict) => dict.iter().for_each(|(_, value)| count_references(value, id, count)),
        Object::Stream(stream) => stream.dict.iter().for_each(|(_, value)| count_references(value, id, count)),
        _ => {}
    }
}

/// Make the page's `Do` number `form.placement` invoke `copy_id` under a new
/// resource name in page-local resources; returns the name.
fn invoke_copy(document: &mut Document, page_id: ObjectId, form: &EditorFormText, copy_id: ObjectId) -> Result<String, EditorError> {
    let mut resources = page_resources(document, page_id).unwrap_or_default();
    let mut xobjects = resources
        .get(b"XObject")
        .ok()
        .and_then(|o| resolve_dict(document, o))
        .cloned()
        .unwrap_or_default();
    let name = (1..)
        .map(|n| format!("{}e{n}", form.name))
        .find(|candidate| !xobjects.has(candidate.as_bytes()))
        .expect("an unused resource name");
    xobjects.set(name.as_bytes(), Object::Reference(copy_id));
    resources.set("XObject", Object::Dictionary(xobjects));
    let mut seen = 0usize;
    let mut streams = Vec::new();
    for mut operations in page_operations(document, page_id)? {
        for operation in operations.iter_mut().filter(|op| op.operator == "Do") {
            if seen == form.placement {
                operation.operands = vec![Object::Name(name.as_bytes().to_vec())];
            }
            seen += 1;
        }
        let encoded = Content { operations }
            .encode()
            .map_err(|e| EditorError::new("EDITOR_WRITE_FAILED", e.to_string()))?;
        let mut stream = Stream::new(Dictionary::new(), encoded);
        let _ = stream.compress();
        streams.push(Object::Reference(document.add_object(stream)));
    }
    let page = document
        .get_object_mut(page_id)
        .and_then(Object::as_dict_mut)
        .map_err(|e| EditorError::new("EDITOR_WRITE_FAILED", e.to_string()))?;
    page.set("Resources", Object::Dictionary(resources));
    page.set("Contents", Object::Array(streams));
    Ok(name)
}

/// Take a page out of the page tree (and drop its objects).
fn remove_page(document: &mut Document, page_id: ObjectId) -> Result<(), EditorError> {
    let invalid = |e: lopdf::Error| EditorError::new("EDITOR_WRITE_FAILED", e.to_string());
    let contents = document.get_page_contents(page_id);
    let mut parent = document.get_dictionary(page_id).and_then(|d| d.get(b"Parent")).and_then(Object::as_reference).map_err(invalid)?;
    if let Ok(Object::Array(kids)) = document.get_dictionary_mut(parent).map_err(invalid)?.get_mut(b"Kids") {
        kids.retain(|kid| kid.as_reference().ok() != Some(page_id));
    }
    for _ in 0..32 {
        let node = document.get_dictionary_mut(parent).map_err(invalid)?;
        let count = node.get(b"Count").and_then(Object::as_i64).unwrap_or(1);
        node.set("Count", Object::Integer((count - 1).max(0)));
        match node.get(b"Parent").and_then(Object::as_reference) {
            Ok(next) => parent = next,
            Err(_) => break,
        }
    }
    document.objects.remove(&page_id);
    for id in contents {
        document.objects.remove(&id);
    }
    Ok(())
}

/// Extend a form's `/BBox` (form space) to cover `drawn`, one unit beyond.
fn grow_box(dict: &mut Dictionary, drawn: Option<[f64; 4]>) {
    let Some(drawn) = drawn else { return };
    let current: Vec<f64> = dict
        .get(b"BBox")
        .and_then(Object::as_array)
        .map(|items| items.iter().filter_map(|o| o.as_float().ok().map(f64::from)).collect())
        .unwrap_or_default();
    let [a, b, c, d] = current[..] else { return };
    let (x0, y0, x1, y1) = (a.min(c), b.min(d), a.max(c), b.max(d));
    if drawn[0] >= x0 && drawn[1] >= y0 && drawn[2] <= x1 && drawn[3] <= y1 {
        return;
    }
    let grown = [x0.min(drawn[0] - 1.0), y0.min(drawn[1] - 1.0), x1.max(drawn[2] + 1.0), y1.max(drawn[3] + 1.0)];
    dict.set("BBox", Object::Array(grown.iter().map(|v| Object::Real(*v as f32)).collect()));
}

fn transform_box(m: &[f64; 6], b: [f64; 4]) -> [f64; 4] {
    let corners = [[b[0], b[1]], [b[2], b[1]], [b[0], b[3]], [b[2], b[3]]]
        .map(|[x, y]| [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]]);
    corners.iter().fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |a, p| {
        [a[0].min(p[0]), a[1].min(p[1]), a[2].max(p[0]), a[3].max(p[1])]
    })
}
