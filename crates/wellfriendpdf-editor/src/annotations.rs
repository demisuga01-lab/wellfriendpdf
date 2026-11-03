//! Markup annotations (highlight, underline, strike-out, sticky note, ink,
//! stamp), each with a normal appearance stream so every viewer, and the
//! engine's own renderer, draws the same thing. Nothing time-dependent is
//! written: the browser preview and the server replay produce equal bytes.

use lopdf::{dictionary, Dictionary, Object, Stream};

use crate::error::EditorError;
use crate::ops::{AnnotationKind, Paint, Rect};
use crate::overlay::{load, page_id, save};

fn real(value: f64) -> Object {
    Object::Real(((value * 1000.0).round() / 1000.0) as f32)
}

fn fmt(value: f64) -> String {
    let rounded = (value * 1000.0).round() / 1000.0;
    if rounded.fract() == 0.0 {
        format!("{}", rounded as i64)
    } else {
        format!("{rounded}")
    }
}

fn pdf_string(text: &str) -> Object {
    // UTF-16BE with BOM: any Unicode text in /Contents and /T.
    let mut bytes = vec![0xFE, 0xFF];
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }
    Object::String(bytes, lopdf::StringFormat::Hexadecimal)
}

/// WinAnsi-safe stamp label for the Helvetica-Bold appearance.
fn label_bytes(label: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for c in label.to_uppercase().chars().take(40) {
        let c = if c.is_ascii() && !c.is_ascii_control() { c } else { '?' };
        if matches!(c, '(' | ')' | '\\') {
            out.push(b'\\');
        }
        out.push(c as u8);
    }
    out
}

fn bounds_of(points: impl Iterator<Item = [f64; 2]>) -> Option<[f64; 4]> {
    let mut out: Option<[f64; 4]> = None;
    for [x, y] in points {
        out = Some(match out {
            None => [x, y, x, y],
            Some(b) => [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)],
        });
    }
    out
}

#[allow(clippy::too_many_arguments)]
pub fn add_annotation(
    input: &[u8],
    page: u32,
    kind: AnnotationKind,
    rect: &Rect,
    quads: &[[f64; 4]],
    ink: &[Vec<[f64; 2]>],
    contents: Option<&str>,
    label: Option<&str>,
    color: &Paint,
    author: Option<&str>,
) -> Result<Vec<u8>, EditorError> {
    let mut document = load(input)?;
    let page_object = page_id(&document, page)?;
    let [r, g, b] = color.approximate_rgb();
    let rgb = format!("{} {} {}", fmt(r), fmt(g), fmt(b));
    let mut box_ = [rect.x, rect.y, rect.x + rect.width, rect.y + rect.height];
    let (subtype, appearance, mut extra): (&str, String, Dictionary) = match kind {
        AnnotationKind::Highlight | AnnotationKind::Underline | AnnotationKind::Strikeout => {
            box_ = bounds_of(quads.iter().flat_map(|q| [[q[0], q[1]], [q[2], q[3]]])).unwrap_or(box_);
            let mut points = Vec::new();
            let mut drawing = String::new();
            for q in quads {
                let (x0, y0, x1, y1) = (q[0].min(q[2]), q[1].min(q[3]), q[0].max(q[2]), q[1].max(q[3]));
                // Upper-left, upper-right, lower-left, lower-right.
                points.extend([x0, y1, x1, y1, x0, y0, x1, y0].map(real));
                let h = y1 - y0;
                match kind {
                    AnnotationKind::Highlight => drawing.push_str(&format!(
                        "{} {} {} {} re f\n",
                        fmt(x0 - box_[0]),
                        fmt(y0 - box_[1]),
                        fmt(x1 - x0),
                        fmt(h)
                    )),
                    AnnotationKind::Underline => drawing.push_str(&format!(
                        "{} w {} {} m {} {} l S\n",
                        fmt((h * 0.07).max(0.5)),
                        fmt(x0 - box_[0]),
                        fmt(y0 - box_[1] + h * 0.08),
                        fmt(x1 - box_[0]),
                        fmt(y0 - box_[1] + h * 0.08)
                    )),
                    _ => drawing.push_str(&format!(
                        "{} w {} {} m {} {} l S\n",
                        fmt((h * 0.07).max(0.5)),
                        fmt(x0 - box_[0]),
                        fmt(y0 - box_[1] + h * 0.45),
                        fmt(x1 - box_[0]),
                        fmt(y0 - box_[1] + h * 0.45)
                    )),
                }
            }
            let appearance = if kind == AnnotationKind::Highlight {
                format!("/Mult gs {rgb} rg\n{drawing}")
            } else {
                format!("{rgb} RG\n{drawing}")
            };
            let subtype = match kind {
                AnnotationKind::Highlight => "Highlight",
                AnnotationKind::Underline => "Underline",
                _ => "StrikeOut",
            };
            (subtype, appearance, dictionary! { "QuadPoints" => points })
        }
        AnnotationKind::Note => {
            box_ = [rect.x, rect.y, rect.x + 20.0, rect.y + 20.0];
            // A folded-note icon: filled square in the note colour, three lines.
            let appearance = format!(
                "{rgb} rg 0.3 G 0.75 w 0.5 0.5 19 19 re B\n0 G 4 14 m 16 14 l S 4 10 m 16 10 l S 4 6 m 12 6 l S\n"
            );
            ("Text", appearance, dictionary! { "Name" => "Comment", "Open" => false })
        }
        AnnotationKind::Ink => {
            box_ = bounds_of(ink.iter().flatten().copied()).unwrap_or(box_);
            box_ = [box_[0] - 2.0, box_[1] - 2.0, box_[2] + 2.0, box_[3] + 2.0];
            let mut drawing = format!("{rgb} RG 2 w 1 J 1 j\n");
            let mut list = Vec::new();
            for stroke in ink.iter().filter(|s| s.len() >= 2) {
                let mut path = Vec::new();
                for (i, [x, y]) in stroke.iter().enumerate() {
                    drawing.push_str(&format!(
                        "{} {} {}\n",
                        fmt(x - box_[0]),
                        fmt(y - box_[1]),
                        if i == 0 { "m" } else { "l" }
                    ));
                    path.push(real(*x));
                    path.push(real(*y));
                }
                drawing.push_str("S\n");
                list.push(Object::Array(path));
            }
            (
                "Ink",
                drawing,
                dictionary! { "InkList" => list, "BS" => dictionary! { "W" => 2 } },
            )
        }
        AnnotationKind::Stamp => {
            let label = label.unwrap_or("APPROVED");
            let text = label_bytes(label);
            let (w, h) = (rect.width, rect.height);
            let size = (h * 0.5).min(w / (text.len().max(1) as f64 * 0.62)).max(6.0);
            let mut appearance = format!(
                "{rgb} RG 2.5 w 2 2 {} {} re S\nBT /Helv {} Tf {rgb} rg {} {} Td (",
                fmt(w - 4.0),
                fmt(h - 4.0),
                fmt(size),
                fmt(((w - text.len() as f64 * size * 0.62) / 2.0).max(4.0)),
                fmt((h - size * 0.72) / 2.0)
            );
            appearance.push_str(&String::from_utf8_lossy(&text));
            appearance.push_str(") Tj ET\n");
            ("Stamp", appearance, dictionary! { "Name" => "Approved" })
        }
    };
    let width = box_[2] - box_[0];
    let height = box_[3] - box_[1];
    let mut resources = Dictionary::new();
    if kind == AnnotationKind::Highlight {
        resources.set("ExtGState", dictionary! { "Mult" => dictionary! { "BM" => "Multiply" } });
    }
    if kind == AnnotationKind::Stamp {
        resources.set(
            "Font",
            dictionary! { "Helv" => dictionary! {
                "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica-Bold", "Encoding" => "WinAnsiEncoding",
            } },
        );
    }
    let mut stream = Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), real(width), real(height)],
            "Resources" => resources,
        },
        appearance.into_bytes(),
    );
    let _ = stream.compress();
    let appearance_id = document.add_object(stream);
    let mut annotation = dictionary! {
        "Type" => "Annot",
        "Subtype" => subtype,
        "Rect" => box_.iter().map(|v| real(*v)).collect::<Vec<_>>(),
        "C" => vec![real(r), real(g), real(b)],
        "F" => 4,
        "P" => Object::Reference(page_object),
        "AP" => dictionary! { "N" => Object::Reference(appearance_id) },
    };
    for (key, value) in extra.iter_mut() {
        annotation.set(key.clone(), value.clone());
    }
    if let Some(contents) = contents.filter(|c| !c.is_empty()) {
        annotation.set("Contents", pdf_string(contents));
    }
    if let Some(author) = author.filter(|a| !a.is_empty()) {
        annotation.set("T", pdf_string(author));
    }
    let annotation_id = document.add_object(annotation);
    let name = format!("wf-annotation-{}", annotation_id.0);
    if let Ok(dict) = document.get_object_mut(annotation_id).and_then(Object::as_dict_mut) {
        dict.set("NM", Object::string_literal(name));
    }
    let existing = document.get_dictionary(page_object).ok().and_then(|d| d.get(b"Annots").ok().cloned());
    let mut annots = match existing {
        Some(Object::Array(items)) => items,
        Some(Object::Reference(id)) => document.get_object(id).ok().and_then(|o| o.as_array().ok().cloned()).unwrap_or_default(),
        _ => Vec::new(),
    };
    annots.push(Object::Reference(annotation_id));
    document
        .get_object_mut(page_object)
        .and_then(Object::as_dict_mut)
        .map_err(|_| EditorError::new("EDITOR_ANNOTATION_REFUSED", "page dictionary missing"))?
        .set("Annots", Object::Array(annots));
    save(&mut document)
}
