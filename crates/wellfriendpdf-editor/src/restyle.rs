//! Restyling existing text: font face (family, bold, italic), size,
//! alignment, line spacing and colour of a `[start, end)` range.
//!
//! Font, size, alignment and spacing go through the engine's reflow
//! transaction with the chosen library face as the approved font, so the text
//! is re-set and re-flowed inside its own block (a table cell or column keeps
//! its box). Colour is then applied to exactly the text-showing operators that
//! draw the restyled run: they are the ones that select the font resource the
//! transaction added.

use lopdf::content::{Content, Operation};
use lopdf::Object;
use serde_json::json;
use wellfriendpdf_engine::{
    text_reflow::{apply_reflow_region, GeometricReflowRequest},
    ApprovedFontAsset, PageTextLayout,
};

use crate::apply::{apply_operation, OperationReport};
use crate::error::EditorError;
use crate::fonts::{page_fonts, FontSource};
use crate::ops::{Align, EditorOperation, Rect, Rgb};
use crate::overlay;

pub struct Restyle<'a> {
    pub page: u32,
    pub start: usize,
    pub end: usize,
    pub expected: &'a str,
    pub font: &'a str,
    pub size: Option<f64>,
    pub color: Option<Rgb>,
    pub align: Option<Align>,
    pub line_height: Option<f64>,
}

pub fn restyle_text(
    input: &[u8],
    restyle: &Restyle<'_>,
    fonts: &dyn FontSource,
    report: &mut OperationReport,
) -> Result<Vec<u8>, EditorError> {
    let page = restyle.page as usize;
    let before = PageTextLayout::from_bytes(input, page)?;
    if !before.editable {
        return Err(EditorError::new(
            "EDITOR_TEXT_NOT_EDITABLE",
            "this page's text cannot be edited in place (its fonts do not decode to exact text)",
        ));
    }
    if restyle.start >= restyle.end
        || restyle.end > before.len()
        || before.text_in([restyle.start, restyle.end]) != restyle.expected
    {
        return Err(EditorError::new(
            "EDITOR_TEXT_STALE",
            "the selected text no longer matches the page; reopen the current version",
        ));
    }
    let bytes = fonts.font_bytes(restyle.font).ok_or_else(|| {
        EditorError::new(
            "EDITOR_FONT_UNAVAILABLE",
            format!("font {} is not in the font library", restyle.font),
        )
    })?;
    let covering: Vec<_> = before
        .paragraphs
        .iter()
        .filter(|p| p.range[0] < restyle.end && p.range[1] > restyle.start)
        .collect();
    let whole = covering.first().is_some_and(|p| p.range[0] == restyle.start)
        && covering.last().is_some_and(|p| p.range[1] == restyle.end);
    if !whole {
        return Err(EditorError::new(
            "EDITOR_RESTYLE_NEEDS_PARAGRAPH",
            "restyling applies to whole paragraphs; select the paragraph",
        ));
    }
    // 1. The engine's transaction: in place for a face or colour change,
    //    re-flowed in the block's own box for size, alignment and spacing.
    match transaction(input, &before, restyle, &covering, bytes) {
        Ok((out, added)) => {
            report.fonts_added = added;
            report.notes.push(format!("The text is set in {} from the font library.", restyle.font));
            report.changed_region = Some(region(&before, restyle.start, restyle.end));
            Ok(out)
        }
        // 2. Tagged or clipped source text must stay inside its marked-content
        //    scopes, where the engine keeps each run's own size. Whole
        //    paragraphs are then re-set as new text: the old glyphs are
        //    removed and the paragraph is written again in the chosen style.
        Err(first) => reset_paragraphs(input, &before, restyle, &covering, fonts, report).map_err(|_| first),
    }
}

fn transaction(
    input: &[u8],
    before: &PageTextLayout,
    restyle: &Restyle<'_>,
    covering: &[&wellfriendpdf_engine::EditorParagraph],
    bytes: Vec<u8>,
) -> Result<(Vec<u8>, Vec<String>), EditorError> {
    let page = restyle.page as usize;
    let source_fonts: Vec<String> = overlay::load(input)
        .map(|d| page_fonts(&d, restyle.page).into_iter().map(|f| f.resource).collect())
        .unwrap_or_default();
    let mut request: GeometricReflowRequest = serde_json::from_value(json!({
        "page": page,
        "source_text": restyle.expected,
        "replacement_text": restyle.expected,
        "target_logical_scalar_range": [restyle.start, restyle.end],
    }))
    .map_err(|e| EditorError::new("EDITOR_OPERATION_INVALID", e.to_string()))?;
    request.font_policy = format!("approved_substitute:{}", restyle.font);
    request.approved_font_asset = Some(ApprovedFontAsset {
        lookup_name: restyle.font.to_owned(),
        bytes,
    });
    request.prefer_source_font = false;
    if let Some(size) = restyle.size {
        request.font_size = Some(size);
    }
    if restyle.size.is_some() || restyle.align.is_some() || restyle.line_height.is_some() {
        let (bbox, lines) = block(covering);
        // The block keeps its left edge and top. A one-line block (a
        // heading, a cell) widens with the size so it stays one line; a
        // longer paragraph keeps its width and wraps, growing downwards.
        let source_size = source_size(before, restyle.start, restyle.end).unwrap_or(12.0);
        let size = restyle.size.unwrap_or(source_size);
        let ratio = (size / source_size).max(1.0);
        let width = bbox[2] - bbox[0];
        let right = if lines == 1 { bbox[0] + width * ratio + size } else { bbox[2] + 1.0 };
        let height = bbox[3] - bbox[1];
        let bottom = bbox[1] - height * (ratio - 1.0) - 2.0 * size * 1.2;
        request.region = Some([bbox[0], bottom, right, bbox[3] + (size - source_size).max(0.0)]);
        request.explicit_style = true;
    }
    if let Some(align) = restyle.align {
        request.alignment = match align {
            Align::Left => "left",
            Align::Center => "center",
            Align::Right => "right",
        }
        .to_owned();
    }
    if let Some(line_height) = restyle.line_height {
        request.line_height = line_height;
    }
    let (edited, _) = apply_reflow_region(input, &request)?;
    let after = PageTextLayout::from_bytes(&edited, page)?;
    if after.text != before.text {
        return Err(EditorError::new(
            "EDITOR_TEXT_LAYOUT_FAILED",
            "the restyled page does not read back as the same text",
        ));
    }
    // The engine re-encodes text its source font can draw with that font even
    // under an approved substitute; a restyle must actually change the face.
    let kept_source_font = after
        .glyphs
        .iter()
        .filter(|g| g.range[0] >= restyle.start && g.range[1] <= restyle.end && !g.text.trim().is_empty())
        .any(|g| source_fonts.contains(&g.font_resource));
    if kept_source_font {
        return Err(EditorError::new("EDITOR_TEXT_LAYOUT_FAILED", "the engine kept the source font"));
    }
    if let Some(size) = restyle.size {
        let kept_source_size = after
            .glyphs
            .iter()
            .filter(|g| g.range[0] >= restyle.start && g.range[1] <= restyle.end && !g.text.trim().is_empty())
            .any(|g| (g.font_size - size).abs() > 0.2);
        if kept_source_size {
            return Err(EditorError::new("EDITOR_TEXT_LAYOUT_FAILED", "the engine kept the source size"));
        }
    }
    let added: Vec<String> = overlay::load(&edited)
        .map(|d| page_fonts(&d, restyle.page).into_iter().map(|f| f.resource).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .filter(|resource| !source_fonts.contains(resource))
        .collect();
    let mut out = edited;
    if let Some(color) = restyle.color.clone() {
        if added.is_empty() {
            return Err(EditorError::new(
                "EDITOR_RESTYLE_UNLOCATED",
                "the restyled run could not be located to colour it",
            ));
        }
        out = colour_runs(&out, restyle.page, &added, &color)?;
    }
    Ok((out, added))
}

fn block(covering: &[&wellfriendpdf_engine::EditorParagraph]) -> ([f64; 4], usize) {
    let mut bbox = covering[0].bbox;
    for p in &covering[1..] {
        bbox = [bbox[0].min(p.bbox[0]), bbox[1].min(p.bbox[1]), bbox[2].max(p.bbox[2]), bbox[3].max(p.bbox[3])];
    }
    (bbox, covering.iter().map(|p| p.lines[1] - p.lines[0]).sum())
}

/// The paragraphs' text as a reader sees it: lines joined with a space, a
/// line-end hyphen joined to the next word, paragraphs on new lines.
fn paragraph_text(layout: &PageTextLayout, covering: &[&wellfriendpdf_engine::EditorParagraph]) -> String {
    let mut out = String::new();
    for (index, paragraph) in covering.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        for (line_index, line) in layout.lines[paragraph.lines[0]..paragraph.lines[1]].iter().enumerate() {
            let text = layout.text_in(line.range);
            if line_index > 0 {
                if out.ends_with('-') {
                    out.pop();
                } else if !out.ends_with(char::is_whitespace) && !text.starts_with(char::is_whitespace) {
                    out.push(' ');
                }
            }
            out.push_str(&text);
        }
    }
    out
}

fn reset_paragraphs(
    input: &[u8],
    before: &PageTextLayout,
    restyle: &Restyle<'_>,
    covering: &[&wellfriendpdf_engine::EditorParagraph],
    fonts: &dyn FontSource,
    report: &mut OperationReport,
) -> Result<Vec<u8>, EditorError> {
    let (bbox, lines) = block(covering);
    let source_size = source_size(before, restyle.start, restyle.end).unwrap_or(12.0);
    let size = restyle.size.unwrap_or(source_size);
    let ratio = (size / source_size).max(1.0);
    let pitch = if lines > 1 { (bbox[3] - bbox[1]) / lines as f64 } else { source_size * 1.2 };
    let line_height = restyle.line_height.unwrap_or((pitch / source_size).clamp(0.8, 3.0));
    let text = paragraph_text(before, covering);
    let width = if lines == 1 { (bbox[2] - bbox[0]) * ratio + size } else { bbox[2] - bbox[0] + 1.0 };
    let rows = (lines as f64 * ratio).ceil() + 2.0;
    let remove = EditorOperation::ReplaceText {
        page: restyle.page,
        start: restyle.start,
        end: restyle.end,
        expected: restyle.expected.to_owned(),
        text: String::new(),
        fallback_font: None,
    };
    let (removed, _) = apply_operation(input, &remove, fonts, true)?;
    let left = PageTextLayout::from_bytes(&removed, restyle.page as usize)?;
    if restyle.expected.chars().count() > 3 && left.text.contains(restyle.expected) {
        return Err(EditorError::new("EDITOR_TEXT_LAYOUT_FAILED", "the paragraph could not be removed"));
    }
    // The first baseline lands where the source's did: the box top is the
    // paragraph top plus the growth of the first line.
    let top = bbox[3] + (size - source_size).max(0.0) * 0.25;
    let add = EditorOperation::AddText {
        page: restyle.page,
        rect: Rect {
            x: bbox[0],
            y: top - size * line_height * rows,
            width,
            height: size * line_height * rows,
        },
        text,
        font: restyle.font.to_owned(),
        size,
        color: restyle.color.clone().unwrap_or(Rgb::BLACK),
        align: restyle.align.unwrap_or_default(),
        line_height,
    };
    let (out, added) = apply_operation(&removed, &add, fonts, true)?;
    report.fonts_added = added.fonts_added;
    report.notes.push(format!(
        "The paragraph is tagged or clipped, so it was re-set as new text in {} (its old glyphs were removed).",
        restyle.font
    ));
    report.changed_region = Some([bbox[0], bbox[1] - (ratio - 1.0) * (bbox[3] - bbox[1]), bbox[0] + width, top]);
    Ok(out)
}

/// The most common glyph size in `[start, end)`.
fn source_size(layout: &PageTextLayout, start: usize, end: usize) -> Option<f64> {
    let mut sizes: Vec<(f64, usize)> = Vec::new();
    for glyph in layout.glyphs.iter().filter(|g| g.range[0] >= start && g.range[1] <= end && !g.text.trim().is_empty()) {
        match sizes.iter_mut().find(|(s, _)| (*s - glyph.font_size).abs() < 0.01) {
            Some(entry) => entry.1 += 1,
            None => sizes.push((glyph.font_size, 1)),
        }
    }
    sizes.into_iter().max_by_key(|(_, n)| *n).map(|(s, _)| s)
}

fn region(layout: &PageTextLayout, start: usize, end: usize) -> [f64; 4] {
    let mut out = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for glyph in layout.glyphs.iter().filter(|g| g.range[0] >= start && g.range[1] <= end) {
        out = [
            out[0].min(glyph.bbox[0]),
            out[1].min(glyph.bbox[1]),
            out[2].max(glyph.bbox[2]),
            out[3].max(glyph.bbox[3]),
        ];
    }
    out
}

fn fill_operator(operation: &Operation) -> bool {
    matches!(operation.operator.as_str(), "g" | "rg" | "k" | "sc" | "scn" | "cs")
}

/// Set `color` as the fill colour of every text run drawn with one of
/// `resources`, restoring the colour in force when another font is selected.
fn colour_runs(input: &[u8], page: u32, resources: &[String], color: &Rgb) -> Result<Vec<u8>, EditorError> {
    let mut document = overlay::load(input)?;
    let page_object = overlay::page_id(&document, page)?;
    let set = match color {
        Rgb::Rgb { r, g, b } => Operation::new("rg", [r, g, b].iter().map(|v| Object::Real(**v as f32)).collect()),
        Rgb::Cmyk { c, m, y, k } => Operation::new("k", [c, m, y, k].iter().map(|v| Object::Real(**v as f32)).collect()),
        Rgb::Spot { tint, c, m, y, k, .. } => Operation::new(
            "k",
            [c, m, y, k].iter().map(|v| Object::Real((**v * tint) as f32)).collect(),
        ),
    };
    let mut changed = false;
    for id in document.get_page_contents(page_object) {
        let Ok(stream) = document.get_object(id).and_then(Object::as_stream) else { continue };
        let plain = stream.decompressed_content().unwrap_or_else(|_| stream.content.clone());
        let Ok(content) = Content::decode(&plain) else { continue };
        let mut operations = Vec::with_capacity(content.operations.len() + 8);
        // The fill colour last set outside the restyled runs, to restore.
        let mut current_fill: Vec<Operation> = Vec::new();
        let mut inside = false;
        for operation in content.operations {
            if operation.operator == "Tf" {
                let selects_restyled = operation
                    .operands
                    .first()
                    .and_then(|o| o.as_name().ok())
                    .is_some_and(|name| resources.iter().any(|r| r.as_bytes() == name));
                if selects_restyled && !inside {
                    operations.push(operation);
                    operations.push(set.clone());
                    inside = true;
                    changed = true;
                    continue;
                }
                if !selects_restyled && inside {
                    inside = false;
                    operations.push(operation);
                    operations.extend(current_fill.iter().cloned());
                    if current_fill.is_empty() {
                        operations.push(Operation::new("g", vec![Object::Integer(0)]));
                    }
                    continue;
                }
            }
            if operation.operator == "ET" && inside {
                inside = false;
                operations.push(operation);
                operations.extend(current_fill.iter().cloned());
                continue;
            }
            if fill_operator(&operation) && !inside {
                if operation.operator == "cs" {
                    current_fill.clear();
                }
                if operation.operator != "cs" {
                    current_fill.retain(|o| o.operator == "cs");
                }
                current_fill.push(operation.clone());
            }
            operations.push(operation);
        }
        if changed {
            let encoded = Content { operations }
                .encode()
                .map_err(|e| EditorError::new("EDITOR_WRITE_FAILED", e.to_string()))?;
            if let Ok(stream) = document.get_object_mut(id).and_then(Object::as_stream_mut) {
                stream.set_plain_content(encoded);
                let _ = stream.compress();
            }
        }
    }
    if !changed {
        return Err(EditorError::new(
            "EDITOR_RESTYLE_UNLOCATED",
            "the restyled run could not be located to colour it",
        ));
    }
    overlay::save(&mut document)
}
