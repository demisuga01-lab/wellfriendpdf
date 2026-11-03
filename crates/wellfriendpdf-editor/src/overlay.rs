//! Added content (text boxes, images, shapes, whiteouts) is authored by the
//! engine's PDF builder on a page the size of the target page, then placed on
//! the target page as one Form XObject drawn after the existing content.
//! Fonts are embedded subsets with ToUnicode, so added text is real,
//! searchable PDF text. The existing content streams are not modified; they
//! are wrapped in `q`/`Q` so an unbalanced graphics state cannot leak into
//! the overlay.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream};

use crate::error::EditorError;
use crate::ops::SpotColor;
use crate::fonts::page_resources;

fn refused(message: impl Into<String>) -> EditorError {
    EditorError::new("EDITOR_OVERLAY_REFUSED", message)
}

fn number(object: &Object) -> Option<f64> {
    match object {
        Object::Integer(value) => Some(*value as f64),
        Object::Real(value) => Some(f64::from(*value)),
        _ => None,
    }
}

pub fn load(bytes: &[u8]) -> Result<Document, EditorError> {
    let document = Document::load_mem(bytes).map_err(|error| {
        EditorError::new(
            "EDITOR_PDF_INVALID",
            format!("the PDF could not be parsed: {error}"),
        )
    })?;
    if document.is_encrypted() {
        return Err(EditorError::new(
            "EDITOR_PDF_ENCRYPTED",
            "encrypted PDFs must be unlocked before editing",
        ));
    }
    Ok(document)
}

pub fn save(document: &mut Document) -> Result<Vec<u8>, EditorError> {
    let mut out = Vec::new();
    document
        .save_to(&mut out)
        .map_err(|error| EditorError::new("EDITOR_WRITE_FAILED", error.to_string()))?;
    Ok(out)
}

pub fn page_id(document: &Document, page: u32) -> Result<ObjectId, EditorError> {
    document.get_pages().get(&page).copied().ok_or_else(|| {
        EditorError::new(
            "EDITOR_PAGE_NOT_FOUND",
            format!("page {page} does not exist"),
        )
    })
}

/// The page's MediaBox `[x0, y0, x1, y1]` (own or inherited), normalised.
pub fn media_box(document: &Document, page: ObjectId) -> Result<[f64; 4], EditorError> {
    let mut current = document
        .get_dictionary(page)
        .map_err(|_| refused("page dictionary missing"))?;
    for _ in 0..32 {
        if let Ok(value) = current.get(b"MediaBox") {
            let value = match value {
                Object::Reference(id) => document
                    .get_object(*id)
                    .map_err(|_| refused("MediaBox missing"))?,
                other => other,
            };
            let array = value
                .as_array()
                .map_err(|_| refused("MediaBox is not an array"))?;
            let numbers: Vec<f64> = array.iter().filter_map(number).collect();
            if numbers.len() != 4 {
                return Err(refused("MediaBox is malformed"));
            }
            return Ok([
                numbers[0].min(numbers[2]),
                numbers[1].min(numbers[3]),
                numbers[0].max(numbers[2]),
                numbers[1].max(numbers[3]),
            ]);
        }
        let Ok(parent) = current.get(b"Parent").and_then(Object::as_reference) else {
            break;
        };
        current = document
            .get_dictionary(parent)
            .map_err(|_| refused("page tree is malformed"))?;
    }
    Ok([0.0, 0.0, 612.0, 792.0])
}

/// References reachable from `root` inside `document`.
fn reachable(document: &Document, root: &Object) -> BTreeSet<ObjectId> {
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::new();
    queue.push_back(root.clone());
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
            Object::Dictionary(dictionary) => queue.extend(dictionary.into_iter().map(|(_, v)| v)),
            Object::Stream(stream) => queue.extend(stream.dict.into_iter().map(|(_, v)| v)),
            _ => {}
        }
    }
    seen
}

/// Draw page 1 of `overlay` (authored at the target MediaBox's size, origin
/// at its lower-left corner) over page `page` of `target`.
///
/// The authored operators are appended to the page's own content (inside
/// `q`/`Q`, translated to the MediaBox origin) and its resources are merged
/// under fresh names, so added text is ordinary page text that the editor
/// can select and edit again later.
pub fn merge(
    target: &[u8],
    page: u32,
    overlay: &[u8],
    spots: &[SpotColor],
) -> Result<Vec<u8>, EditorError> {
    let mut document = load(target)?;
    let target_page = page_id(&document, page)?;
    let [x0, y0, _, _] = media_box(&document, target_page)?;

    let mut source = Document::load_mem(overlay)
        .map_err(|e| refused(format!("authored overlay is invalid: {e}")))?;
    source.renumber_objects_with(document.max_id + 1);
    let source_page = *source
        .get_pages()
        .get(&1)
        .ok_or_else(|| refused("authored overlay has no page"))?;
    let content = source.get_page_content(source_page);
    let mut content = Content::decode(&content)
        .map_err(|e| refused(format!("authored overlay content is unreadable: {e}")))?;
    let resources = page_resources(&source, source_page).unwrap_or_default();
    for id in reachable(&source, &Object::Dictionary(resources.clone())) {
        if let Ok(object) = source.get_object(id) {
            document.objects.insert(id, object.clone());
        }
    }
    document.max_id = document.max_id.max(source.max_id);
    let suffix = document.max_id;

    // The page gets its own copy of its resources so a dictionary shared
    // with other pages is never changed.
    let mut page_resources = page_resources(&document, target_page).unwrap_or_default();
    let mut renamed: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
    for (category, entries) in resources.iter() {
        let entries = match entries {
            Object::Dictionary(d) => d.clone(),
            Object::Reference(id) => match source.get_dictionary(*id) {
                Ok(d) => d.clone(),
                Err(_) => continue,
            },
            _ => continue, // ProcSet
        };
        let mut merged = match page_resources.get(category) {
            Ok(Object::Dictionary(d)) => d.clone(),
            Ok(Object::Reference(id)) => document.get_dictionary(*id).cloned().unwrap_or_default(),
            _ => Dictionary::new(),
        };
        for (name, value) in entries.iter() {
            let mut fresh = name.clone();
            fresh.extend_from_slice(format!("_wf{suffix}").as_bytes());
            renamed.insert(name.clone(), fresh.clone());
            merged.set(fresh, value.clone());
        }
        page_resources.set(category.clone(), Object::Dictionary(merged));
    }
    for operation in &mut content.operations {
        for operand in &mut operation.operands {
            if let Object::Name(name) = operand {
                if let Some(fresh) = renamed.get(name) {
                    *name = fresh.clone();
                }
            }
        }
    }
    if !spots.is_empty() {
        apply_spots(&mut document, &mut page_resources, &mut content, spots, suffix);
    }
    let mut operations = vec![
        Operation::new("Q", vec![]),
        Operation::new("q", vec![]),
        Operation::new(
            "cm",
            vec![
                1.into(),
                0.into(),
                0.into(),
                1.into(),
                Object::Real(x0 as f32),
                Object::Real(y0 as f32),
            ],
        ),
    ];
    operations.append(&mut content.operations);
    operations.push(Operation::new("Q", vec![]));
    let draw = Content { operations }
        .encode()
        .map_err(|e| refused(format!("overlay content could not be encoded: {e}")))?;

    let mut contents: Vec<Object> = Vec::new();
    let before = document.add_object(Stream::new(Dictionary::new(), b"q\n".to_vec()));
    contents.push(Object::Reference(before));
    let existing = document
        .get_dictionary(target_page)
        .ok()
        .and_then(|d| d.get(b"Contents").ok().cloned());
    match existing {
        Some(Object::Reference(id)) => match document.get_object(id) {
            Ok(Object::Array(items)) => contents.extend(items.clone()),
            _ => contents.push(Object::Reference(id)),
        },
        Some(Object::Array(items)) => contents.extend(items),
        _ => {}
    }
    let mut after = Stream::new(Dictionary::new(), draw);
    let _ = after.compress();
    let after = document.add_object(after);
    contents.push(Object::Reference(after));

    let page_dictionary = document
        .get_object_mut(target_page)
        .and_then(Object::as_dict_mut)
        .map_err(|_| refused("page dictionary missing"))?;
    page_dictionary.set("Resources", Object::Dictionary(page_resources));
    page_dictionary.set("Contents", Object::Array(contents));
    save(&mut document)
}

/// Add a link annotation (URL or internal page) to `page`.
pub fn add_link(
    target: &[u8],
    page: u32,
    rect: [f64; 4],
    url: Option<&str>,
    target_page: Option<u32>,
) -> Result<Vec<u8>, EditorError> {
    let mut document = load(target)?;
    let page_object = page_id(&document, page)?;
    let action_or_dest: (&str, Object) = match (url, target_page) {
        (Some(url), _) => (
            "A",
            Object::Dictionary(dictionary! {
                "S" => "URI",
                "URI" => Object::string_literal(url.as_bytes().to_vec()),
            }),
        ),
        (None, Some(target)) => {
            let destination = page_id(&document, target)?;
            (
                "Dest",
                Object::Array(vec![
                    Object::Reference(destination),
                    "XYZ".into(),
                    Object::Null,
                    Object::Null,
                    Object::Null,
                ]),
            )
        }
        (None, None) => return Err(refused("a link needs a URL or a page")),
    };
    let mut annotation = dictionary! {
        "Type" => "Annot",
        "Subtype" => "Link",
        "Rect" => rect.iter().map(|v| Object::Real(*v as f32)).collect::<Vec<_>>(),
        "Border" => vec![0.into(), 0.into(), 0.into()],
        "P" => Object::Reference(page_object),
    };
    annotation.set(action_or_dest.0, action_or_dest.1);
    let annotation_id = document.add_object(annotation);
    let existing = document
        .get_dictionary(page_object)
        .ok()
        .and_then(|d| d.get(b"Annots").ok().cloned());
    let mut annots = match existing {
        Some(Object::Array(items)) => items,
        Some(Object::Reference(id)) => document
            .get_object(id)
            .ok()
            .and_then(|o| o.as_array().ok().cloned())
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    annots.push(Object::Reference(annotation_id));
    document
        .get_object_mut(page_object)
        .and_then(Object::as_dict_mut)
        .map_err(|_| refused("page dictionary missing"))?
        .set("Annots", Object::Array(annots));
    save(&mut document)
}

fn operand_number(object: &Object) -> Option<f64> {
    match object {
        Object::Integer(value) => Some(*value as f64),
        Object::Real(value) => Some(f64::from(*value)),
        _ => None,
    }
}

/// Rewrite the authored CMYK paint of each spot colour (its alternate scaled
/// by the tint) into the `/Separation` colour space the spot names, so the
/// PDF carries the real spot plate with its CMYK appearance as alternate.
fn apply_spots(
    document: &mut Document,
    page_resources: &mut Dictionary,
    content: &mut Content,
    spots: &[SpotColor],
    suffix: u32,
) {
    let mut spaces = match page_resources.get(b"ColorSpace") {
        Ok(Object::Dictionary(d)) => d.clone(),
        Ok(Object::Reference(id)) => document.get_dictionary(*id).cloned().unwrap_or_default(),
        _ => Dictionary::new(),
    };
    for (index, spot) in spots.iter().enumerate() {
        let function = dictionary! {
            "FunctionType" => 2,
            "Domain" => vec![0.into(), 1.into()],
            "C0" => vec![0.into(), 0.into(), 0.into(), 0.into()],
            "C1" => spot.alternate.iter().map(|v| Object::Real(*v as f32)).collect::<Vec<_>>(),
            "N" => 1,
        };
        let separation = document.add_object(Object::Array(vec![
            "Separation".into(),
            Object::Name(spot.name.replace(' ', "#20").into_bytes()),
            "DeviceCMYK".into(),
            Object::Dictionary(function),
        ]));
        let name = format!("WfSpot{index}_wf{suffix}");
        spaces.set(name.as_bytes().to_vec(), Object::Reference(separation));
        let expected: Vec<f64> = spot.alternate.iter().map(|v| v * spot.tint).collect();
        let mut rewritten = Vec::with_capacity(content.operations.len());
        for operation in content.operations.drain(..) {
            let stroke = operation.operator == "K";
            let matches = (operation.operator == "k" || stroke)
                && operation.operands.len() == 4
                && operation
                    .operands
                    .iter()
                    .zip(&expected)
                    .all(|(o, e)| operand_number(o).is_some_and(|v| (v - e).abs() < 0.002));
            if matches {
                rewritten.push(Operation::new(
                    if stroke { "CS" } else { "cs" },
                    vec![Object::Name(name.as_bytes().to_vec())],
                ));
                rewritten.push(Operation::new(
                    if stroke { "SCN" } else { "scn" },
                    vec![Object::Real(spot.tint as f32)],
                ));
            } else {
                rewritten.push(operation);
            }
        }
        content.operations = rewritten;
    }
    page_resources.set("ColorSpace", Object::Dictionary(spaces));
}
