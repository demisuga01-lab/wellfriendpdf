//! Restyling existing text: font face (family, bold, italic), size,
//! alignment, line spacing and colour of a `[start, end)` range.
//!
//! Font, size, alignment and spacing go through the engine's reflow
//! transaction with the chosen library face as the approved font, so the text
//! is re-set and re-flowed inside its own block (a table cell or column keeps
//! its box). Colour is then applied to exactly the text-showing operators that
//! draw the restyled run: they are the ones that select the font resource the
//! transaction added.
//!
//! A restyled paragraph that grows moves the text below it in its column
//! down (see [`crate::flow`]); when the page runs out of room the paragraph
//! keeps its style, is set within the height left and the report carries an
//! overflow note.

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
use crate::{flow, overlay};

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
    if restyle.start >= restyle.end
        || restyle.end > before.len()
        || before.text_in([restyle.start, restyle.end]) != restyle.expected
    {
        return Err(EditorError::new(
            "EDITOR_TEXT_STALE",
            "the selected text no longer matches the page; reopen the current version",
        ));
    }
    // Text inside a placed form is restyled in the form's own content.
    if let Some(form) = crate::forms::target(&before, restyle.start, restyle.end)? {
        let base = form.range[0];
        return crate::forms::edit_in_form(input, restyle.page, &form, report, |bytes, scratch, report| {
            let local = Restyle {
                page: scratch,
                start: restyle.start - base,
                end: restyle.end - base,
                color: restyle.color.clone(),
                ..*restyle
            };
            restyle_text(bytes, &local, fonts, report)
        });
    }
    if !before.editable {
        return Err(EditorError::new(
            "EDITOR_TEXT_NOT_EDITABLE",
            "this page's text cannot be edited in place (its fonts do not decode to exact text)",
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
    let out = match transaction(input, &before, restyle, &covering, bytes) {
        Ok((out, added)) => {
            report.fonts_added = added;
            report.notes.push(format!("The text is set in {} from the font library.", restyle.font));
            report.changed_region = Some(region(&before, restyle.start, restyle.end));
            out
        }
        // 2. Tagged or clipped source text must stay inside its marked-content
        //    scopes, where the engine keeps each run's own size. Whole
        //    paragraphs are then re-set as new text: the old glyphs are
        //    removed and the paragraph is written again in the chosen style.
        Err(first) => {
            let reset = Reset::prepare(input, &before, restyle, &covering, fonts).map_err(|_| first.clone())?;
            let (out, added) = reset.draw(&reset.text).map_err(|_| first)?;
            report.fonts_added = added;
            report.notes.push(format!(
                "The paragraph is tagged or clipped, so it was re-set as new text in {} (its old glyphs were removed).",
                restyle.font
            ));
            report.changed_region = Some(reset.region());
            out
        }
    };
    let indexes: Vec<usize> = before
        .paragraphs
        .iter()
        .enumerate()
        .filter(|(_, p)| p.range[0] < restyle.end && p.range[1] > restyle.start)
        .map(|(i, _)| i)
        .collect();
    make_room(input, out, &before, &indexes, Some((restyle, &covering, fonts)), report)
}

/// 3. A paragraph that grew moves the text below it in its column down by
///    the growth, so nothing is drawn over. When the page has too little
///    room left, the restyled paragraph keeps its style and is set within
///    the height there is; the words that do not fit are left out and the
///    report says so (never silently, and never by shrinking the text).
pub(crate) fn make_room(
    input: &[u8],
    out: Vec<u8>,
    before: &PageTextLayout,
    paragraphs: &[usize],
    refit: Option<(&Restyle<'_>, &[&wellfriendpdf_engine::EditorParagraph], &dyn FontSource)>,
    report: &mut OperationReport,
) -> Result<Vec<u8>, EditorError> {
    let page = before.page;
    let Some(source) = flow::source_block(before, paragraphs) else {
        return Ok(out);
    };
    // Only a paragraph that now ends lower needs the content scan.
    let room_for = |out: &[u8], after: &PageTextLayout, edited: &[bool]| -> Result<Option<flow::Room>, EditorError> {
        match flow::edited_box(after, edited) {
            Some(grown) if source.0[1] - grown[1] > 0.25 => {
                let clips = flow::clips(out, page as u32, after)?;
                Ok(flow::plan(source, after, edited, &clips))
            }
            _ => Ok(None),
        }
    };
    let mut out = out;
    let mut after = PageTextLayout::from_bytes(&out, page)?;
    let mut edited = flow::edited_glyphs(before, &after);
    let mut room = room_for(&out, &after, &edited)?;
    if let Some(over) = room.clone().filter(|r| r.overflow > 0.25) {
        match refit {
            Some((restyle, covering, fonts)) => {
                let lowest = source.0[1] - (over.growth - over.overflow);
                let reset = Reset::prepare(input, before, restyle, covering, fonts)?;
                let (fitted, kept, total) = reset.fit(lowest)?;
                out = fitted.0;
                report.fonts_added = fitted.1;
                report.notes.push(format!(
                    "Overflow: the restyled paragraph does not fit on the page. The text below it was moved down as far as the page allows, and the last {} of its {} words do not fit and are not shown. Shorten the text, choose a smaller size, or undo.",
                    total - kept,
                    total
                ));
                after = PageTextLayout::from_bytes(&out, page)?;
                edited = flow::edited_glyphs(before, &after);
                room = room_for(&out, &after, &edited)?;
            }
            None => report.notes.push(
                "Overflow: the edited paragraph does not fit on the page; the text below it was moved down as far as the page allows.".into(),
            ),
        }
    }
    if let Some(mut room) = room {
        if room.overflow > 0.25 {
            // Keep everything on the page: move by the room there is.
            room.reduce(room.overflow);
        }
        if !room.groups.is_empty() {
            let pushed = flow::push(&out, page as u32, &after, &room)?;
            let moved = PageTextLayout::from_bytes(&pushed, page)?;
            if moved.text != after.text || moved.glyphs.len() != after.glyphs.len() {
                return Err(EditorError::new(
                    "EDITOR_TEXT_LAYOUT_FAILED",
                    "moving the text below the edited paragraph changed it",
                ));
            }
            let (lines, shift) = (room.lines(), room.largest_shift());
            report.notes.push(format!(
                "{lines} following line{} in the same column moved down by {shift:.1} pt to make room.",
                if lines == 1 { "" } else { "s" }
            ));
            if let Some(region) = report.changed_region.as_mut() {
                for group in &room.groups {
                    let b = group.bbox;
                    *region = flow::union(*region, [b[0], b[1] - group.shift, b[2], b[3]]);
                }
            }
            out = pushed;
            after = moved;
        }
    }
    // The edited glyphs may touch other text no more than the source
    // paragraph did (glyph boxes taken from a font's ascent and descent
    // often meet the next paragraph's already).
    let source_glyphs: Vec<bool> = before
        .glyphs
        .iter()
        .map(|g| {
            paragraphs.iter().any(|&p| {
                let r = before.paragraphs[p].range;
                g.range[0] >= r[0] && g.range[1] <= r[1]
            })
        })
        .collect();
    let (was, _) = flow::overlaps(before, &source_glyphs, &source_glyphs);
    let own: Vec<bool> = flow::unchanged_glyphs(before, &after)
        .into_iter()
        .map(|matched| matched.is_some_and(|index| source_glyphs[index]))
        .collect();
    let (now, pair) = flow::overlaps(&after, &edited, &own);
    if let Some((mine, other)) = pair.filter(|_| now > was) {
        return Err(EditorError::new(
            "EDITOR_TEXT_LAYOUT_FAILED",
            format!("the edited paragraph would draw over other text ({mine:?} over {other:?})"),
        ));
    }
    Ok(out)
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

/// A paragraph re-set as new text: its old glyphs removed once, then the
/// text drawn in the requested style in the paragraph's box.
struct Reset<'a> {
    removed: Vec<u8>,
    text: String,
    page: u32,
    fonts: &'a dyn FontSource,
    font: String,
    bbox: [f64; 4],
    top: f64,
    width: f64,
    size: f64,
    ratio: f64,
    line_height: f64,
    rows: f64,
    color: Rgb,
    align: Align,
}

impl<'a> Reset<'a> {
    fn prepare(
        input: &[u8],
        before: &PageTextLayout,
        restyle: &Restyle<'_>,
        covering: &[&wellfriendpdf_engine::EditorParagraph],
        fonts: &'a dyn FontSource,
    ) -> Result<Self, EditorError> {
        let (bbox, lines) = block(covering);
        let source_size = source_size(before, restyle.start, restyle.end).unwrap_or(12.0);
        let size = restyle.size.unwrap_or(source_size);
        let ratio = (size / source_size).max(1.0);
        let pitch = if lines > 1 { (bbox[3] - bbox[1]) / lines as f64 } else { source_size * 1.2 };
        let line_height = restyle.line_height.unwrap_or((pitch / source_size).clamp(0.8, 3.0));
        let width = if lines == 1 { (bbox[2] - bbox[0]) * ratio + size } else { bbox[2] - bbox[0] + 1.0 };
        let remove = EditorOperation::ReplaceText {
            page: restyle.page,
            start: restyle.start,
            end: restyle.end,
            expected: restyle.expected.to_owned(),
            text: String::new(),
            fallback_font: None,
        };
        let removed_by = |bytes: Vec<u8>| -> Result<Vec<u8>, EditorError> {
            let left = PageTextLayout::from_bytes(&bytes, restyle.page as usize)?;
            if restyle.expected.chars().count() > 3 && left.text.contains(restyle.expected) {
                return Err(EditorError::new("EDITOR_TEXT_LAYOUT_FAILED", "the paragraph could not be removed"));
            }
            Ok(bytes)
        };
        // The reflow transaction deletes the text; when it cannot (all of
        // the page's text, with nothing to anchor the deletion on), the
        // paragraph's text runs are taken out of the content directly.
        let removed = apply_operation(input, &remove, fonts, true)
            .and_then(|(bytes, _)| removed_by(bytes))
            .or_else(|first| {
                let lines: Vec<usize> = covering.iter().flat_map(|p| p.lines[0]..p.lines[1]).collect();
                flow::remove_lines(input, restyle.page, before, &lines)
                    .and_then(removed_by)
                    .map_err(|_| first)
            })?;
        // Link underlines of the removed lines would cross the re-set text.
        let baselines: Vec<f64> = covering
            .iter()
            .flat_map(|p| before.lines[p.lines[0]..p.lines[1]].iter())
            .map(|line| line.baseline[0][1])
            .collect();
        let removed = flow::remove_underlines(&removed, restyle.page, bbox, &baselines, source_size)?;
        Ok(Self {
            removed,
            text: paragraph_text(before, covering),
            page: restyle.page,
            fonts,
            font: restyle.font.to_owned(),
            bbox,
            // The first baseline lands where the source's did: the box top is
            // the paragraph top plus the growth of the first line.
            top: bbox[3] + (size - source_size).max(0.0) * 0.25,
            width,
            size,
            ratio,
            line_height,
            rows: (lines as f64 * ratio).ceil() + 2.0,
            color: restyle.color.clone().unwrap_or(Rgb::BLACK),
            align: restyle.align.unwrap_or_default(),
        })
    }

    /// Draw `text`; returns the bytes and the font resources added.
    fn draw(&self, text: &str) -> Result<(Vec<u8>, Vec<String>), EditorError> {
        let height = self.size * self.line_height * self.rows;
        let add = EditorOperation::AddText {
            page: self.page,
            rect: Rect { x: self.bbox[0], y: self.top - height, width: self.width, height },
            text: text.to_owned(),
            font: self.font.clone(),
            size: self.size,
            color: self.color.clone(),
            align: self.align,
            line_height: self.line_height,
        };
        let (out, added) = apply_operation(&self.removed, &add, self.fonts, true)?;
        Ok((out, added.fonts_added))
    }

    fn region(&self) -> [f64; 4] {
        let b = self.bbox;
        [b[0], b[1] - (self.ratio - 1.0) * (b[3] - b[1]), b[0] + self.width, self.top]
    }

    /// The longest run of whole words whose glyphs stay above `lowest`:
    /// `((bytes, fonts added), words kept, words in the paragraph)`.
    #[allow(clippy::type_complexity)]
    fn fit(&self, lowest: f64) -> Result<((Vec<u8>, Vec<String>), usize, usize), EditorError> {
        let ends: Vec<usize> = self
            .text
            .char_indices()
            .zip(self.text.chars().skip(1).chain(std::iter::once(' ')))
            .filter(|((_, c), next)| !c.is_whitespace() && next.is_whitespace())
            .map(|((i, c), _)| i + c.len_utf8())
            .collect();
        let before = PageTextLayout::from_bytes(&self.removed, self.page as usize)?;
        let fits = |words: usize| -> Result<Option<(Vec<u8>, Vec<String>)>, EditorError> {
            let drawn = self.draw(&self.text[..ends[words - 1]])?;
            let after = PageTextLayout::from_bytes(&drawn.0, self.page as usize)?;
            let edited = flow::edited_glyphs(&before, &after);
            let bottom = flow::edited_box(&after, &edited).map_or(f64::INFINITY, |b| b[1]);
            Ok((bottom >= lowest - 0.25).then_some(drawn))
        };
        // Binary search over the word count: fitting is monotone in it.
        let (mut low, mut high) = (0usize, ends.len());
        let mut best = None;
        while low < high {
            let middle = (low + high).div_ceil(2);
            match fits(middle)? {
                Some(drawn) => {
                    best = Some((drawn, middle));
                    low = middle;
                }
                None => high = middle - 1,
            }
        }
        match best {
            Some((drawn, kept)) => Ok((drawn, kept, ends.len())),
            None => Err(EditorError::new(
                "EDITOR_TEXT_OVERFLOW",
                "the restyled paragraph does not fit on the page at the requested size; choose a smaller size",
            )),
        }
    }
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
