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
//! down (see [`crate::flow`]). When the page runs out of room, the
//! operation's overflow policy decides (see [`crate::overflow`]): it is
//! refused by default, and no policy leaves words out.

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
use crate::ops::{Align, EditorOperation, Rect, Rgb, TextOverflow};
use crate::overflow::{self, Fit, TextBox};
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
    /// `(start, end, text)`: page offsets inside `[start, end)` replaced by
    /// `text` as the paragraphs are re-set (a `replaceText` that no longer
    /// fits in place and is shrunk or continued on the next page).
    pub replacement: Option<(usize, usize, &'a str)>,
    /// What to do when the restyled text does not fit.
    pub fit: Fit,
}

impl Restyle<'_> {
    /// The paragraphs' raw text after the replacement.
    fn new_text(&self) -> String {
        match self.replacement {
            None => self.expected.to_owned(),
            Some((start, end, text)) => {
                let chars: Vec<char> = self.expected.chars().collect();
                let (a, b) = (start - self.start, end - self.start);
                let mut out: String = chars[..a].iter().collect();
                out.push_str(text);
                out.extend(chars[b..].iter());
                out
            }
        }
    }
}

/// The size of the text in `[start, end)` (of the paragraph at `start` for
/// an insertion): what a `shrink` starts from.
pub(crate) fn text_size(input: &[u8], page: u32, start: usize, end: usize) -> Result<f64, EditorError> {
    let layout = PageTextLayout::from_bytes(input, page as usize)?;
    let (from, to) = if start < end {
        (start, end)
    } else {
        layout
            .paragraphs
            .iter()
            .find(|p| p.range[0] <= start && start <= p.range[1])
            .map_or((start, end), |p| (p.range[0], p.range[1]))
    };
    Ok(source_size(&layout, from, to).unwrap_or(12.0))
}

/// A `replaceText` whose paragraph no longer fits in place, re-set as new
/// text (in its library match, or `fallback_font`) so it can be shrunk or
/// continued on the next page.
#[allow(clippy::too_many_arguments)]
pub(crate) fn reset_replacement(
    input: &[u8],
    before: &PageTextLayout,
    page: u32,
    start: usize,
    end: usize,
    text: &str,
    fallback_font: Option<&str>,
    fonts: &dyn FontSource,
    fit: Fit,
    report: &mut OperationReport,
) -> Result<Vec<u8>, EditorError> {
    let covering: Vec<_> = before
        .paragraphs
        .iter()
        .filter(|p| (p.range[0] < end && p.range[1] > start) || (p.range[0] <= start && start < p.range[1]))
        .collect();
    let (Some(first), Some(last)) = (covering.first(), covering.last()) else {
        return Err(EditorError::new(overflow::OVERFLOW, "the edited text is not in a paragraph that can be re-set"));
    };
    let (from, to) = (first.range[0], last.range[1]);
    let resource = before
        .glyphs
        .iter()
        .find(|g| g.range[0] >= from && g.range[1] <= to && !g.text.trim().is_empty())
        .map(|g| g.font_resource.clone());
    let library = fallback_font.map(str::to_owned).or_else(|| {
        let document = overlay::load(input).ok()?;
        page_fonts(&document, page)
            .into_iter()
            .find(|f| Some(&f.resource) == resource.as_ref())
            .and_then(|f| f.library_match)
            .map(|m| m.font)
    });
    let Some(font) = library else {
        return Err(EditorError::new(
            overflow::OVERFLOW,
            "the paragraph's font has no library match to re-set it in",
        ));
    };
    let expected = before.text_in([from, to]);
    let restyle = Restyle {
        page,
        start: from,
        end: to,
        expected: &expected,
        font: &font,
        size: None,
        color: None,
        align: None,
        line_height: None,
        replacement: Some((start, end, text)),
        fit,
    };
    report.notes.clear();
    report.fonts_added.clear();
    restyle_text(input, &restyle, fonts, report)
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
                replacement: restyle.replacement.map(|(start, end, text)| (start - base, end - base, text)),
                // A form's text has no next page to continue on.
                fit: restyle.fit.without_next_page(),
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
    // A `shrink` attempt: the text at that size, its line spacing scaled
    // with it (the spacing stays the same multiple of the size).
    if let Some(size) = restyle.fit.size {
        let line_height = restyle.line_height.or_else(|| Some(spacing(&before, restyle, &covering)));
        let shrunk = Restyle {
            size: Some(size),
            line_height,
            color: restyle.color.clone(),
            fit: Fit::new(restyle.fit.mode),
            ..*restyle
        };
        return restyle_text(input, &shrunk, fonts, report);
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
            let (out, added) = reset
                .draw(&reset.text)
                .map_err(|error| if error.code == overflow::OVERFLOW { error } else { first })?;
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
    make_room(input, out, &before, &indexes, Some((restyle, &covering, fonts)), restyle.fit, report)
}

/// 3. A paragraph that grew moves the text below it in its column down by
///    the growth, so nothing is drawn over. When the page has too little
///    room left (see [`overflow::bottom_limit`]), `fit` decides: refuse
///    (the default) and `continue` past the room there is fail with
///    `EDITOR_TEXT_OVERFLOW`; `nextPage` keeps the words that fit (the
///    paragraph is re-set, so it needs `refit`) and continues the rest at
///    the top of its column on the next page. Words are never left out.
pub(crate) fn make_room(
    input: &[u8],
    out: Vec<u8>,
    before: &PageTextLayout,
    paragraphs: &[usize],
    refit: Option<(&Restyle<'_>, &[&wellfriendpdf_engine::EditorParagraph], &dyn FontSource)>,
    fit: Fit,
    report: &mut OperationReport,
) -> Result<Vec<u8>, EditorError> {
    let page = before.page;
    let Some(source) = flow::source_block(before, paragraphs) else {
        return Ok(out);
    };
    let size = paragraphs
        .iter()
        .flat_map(|&p| before.glyphs.iter().filter(move |g| g.range[0] >= before.paragraphs[p].range[0] && g.range[1] <= before.paragraphs[p].range[1]))
        .map(|g| g.font_size)
        .fold(0.0_f64, f64::max)
        .max(1.0);
    let limit = overflow::bottom_limit(before, fit.mode, size);
    // Only a paragraph that now ends lower needs the content scan.
    let room_for = |out: &[u8], after: &PageTextLayout, edited: &[bool]| -> Result<Option<flow::Room>, EditorError> {
        match flow::edited_box(after, edited) {
            Some(grown) if source.0[1] - grown[1] > 0.25 => {
                let clips = flow::clips(out, page as u32, after)?;
                Ok(flow::plan(source, after, edited, &clips, limit))
            }
            _ => Ok(None),
        }
    };
    let short = |after: &PageTextLayout, edited: &[bool], room: &flow::Room| {
        let grown = flow::edited_box(after, edited).map_or(source.0[1], |b| b[1]);
        overflow::shortfall(after, edited, grown + room.overflow, page as u32)
    };
    let mut out = out;
    let mut after = PageTextLayout::from_bytes(&out, page)?;
    let mut edited = flow::edited_glyphs(before, &after);
    let mut room = room_for(&out, &after, &edited)?;
    let reset_holder;
    let mut continued: Option<(TextBox<'_>, &dyn FontSource, String, String)> = None;
    if let Some(over) = room.clone().filter(|r| r.overflow > 0.25) {
        let missing = short(&after, &edited, &over);
        match (fit.mode, refit) {
            (TextOverflow::NextPage, Some((restyle, covering, fonts))) => {
                // The words that fit stay; the rest continues overleaf.
                overflow::require_next_page(input, page as u32)?;
                let lowest = source.0[1] - (over.growth - over.overflow);
                reset_holder = Reset::prepare(input, before, restyle, covering, fonts)?;
                let reset = &reset_holder;
                let (fitted, kept, total) =
                    reset.fit(lowest).map_err(|_| overflow::overflow_error(&missing, fit.mode))?;
                out = fitted.0;
                report.fonts_added = fitted.1;
                let (kept_text, rest) = overflow::split_words(&reset.text, kept);
                if !rest.is_empty() {
                    overflow::note_continued(report, page as u32 + 1, total - kept, total);
                    continued = Some((reset.item(""), fonts, kept_text, rest));
                }
                after = PageTextLayout::from_bytes(&out, page)?;
                edited = flow::edited_glyphs(before, &after);
                room = room_for(&out, &after, &edited)?;
            }
            _ => return Err(overflow::overflow_error(&missing, fit.mode)),
        }
    }
    if let Some(room) = room {
        if room.overflow > 0.25 {
            return Err(overflow::overflow_error(&short(&after, &edited, &room), fit.mode));
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
    if fit.mode == TextOverflow::Continue {
        let margin = overflow::bottom_limit(before, TextOverflow::Refuse, size);
        let lowest = after
            .glyphs
            .iter()
            .filter(|g| flow::visible(g))
            .map(|g| g.bbox[1])
            .fold(f64::INFINITY, f64::min);
        if lowest < margin - 0.25 {
            report.notes.push("The text keeps its size; it and the text below it run past the bottom margin.".into());
            report.overflow = Some(overflow::OverflowOutcome { policy: TextOverflow::Continue, ..Default::default() });
        }
    }
    if let Some((item, fonts, kept, rest)) = continued {
        let next = TextBox { page: page as u32 + 1, text: &rest, ..item };
        out = overflow::continue_on_next_page(&out, before, &next, fonts)?;
        report.continued = Some(overflow::Continued { kept, rest });
    }
    Ok(out)
}

/// The paragraphs' line spacing as a multiple of their size.
fn spacing(before: &PageTextLayout, restyle: &Restyle<'_>, covering: &[&wellfriendpdf_engine::EditorParagraph]) -> f64 {
    let (bbox, lines) = block(covering);
    let source_size = source_size(before, restyle.start, restyle.end).unwrap_or(12.0);
    let pitch = if lines > 1 { (bbox[3] - bbox[1]) / lines as f64 } else { source_size * 1.2 };
    (pitch / source_size).clamp(0.8, 3.0)
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
    let new_text = restyle.new_text();
    let new_end = restyle.start + new_text.chars().count();
    let mut request: GeometricReflowRequest = serde_json::from_value(json!({
        "page": page,
        "source_text": restyle.expected,
        "replacement_text": new_text,
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
    let expected_page = format!(
        "{}{new_text}{}",
        before.text_in([0, restyle.start]),
        before.text_in([restyle.end, before.len()])
    );
    if after.text != expected_page {
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
        .filter(|g| g.range[0] >= restyle.start && g.range[1] <= new_end && !g.text.trim().is_empty())
        .any(|g| source_fonts.contains(&g.font_resource));
    if kept_source_font {
        return Err(EditorError::new("EDITOR_TEXT_LAYOUT_FAILED", "the engine kept the source font"));
    }
    if let Some(size) = restyle.size {
        let kept_source_size = after
            .glyphs
            .iter()
            .filter(|g| g.range[0] >= restyle.start && g.range[1] <= new_end && !g.text.trim().is_empty())
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
/// line-end hyphen joined to the next word, paragraphs on new lines; with
/// `replacement`, `(start, end, text)` replaces those page offsets.
fn paragraph_text(
    layout: &PageTextLayout,
    covering: &[&wellfriendpdf_engine::EditorParagraph],
    replacement: Option<(usize, usize, &str)>,
) -> String {
    let mut out = String::new();
    let mut inserted = false;
    for (index, paragraph) in covering.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        for (line_index, line) in layout.lines[paragraph.lines[0]..paragraph.lines[1]].iter().enumerate() {
            let mut text = String::new();
            for (offset, c) in (line.range[0]..).zip(layout.text_in(line.range).chars()) {
                if let Some((start, end, new)) = replacement {
                    if offset >= start && !inserted {
                        text.push_str(new);
                        inserted = true;
                    }
                    if offset >= start && offset < end {
                        continue;
                    }
                }
                text.push(c);
            }
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
    if let Some((_, _, new)) = replacement.filter(|_| !inserted) {
        out.push_str(new);
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
    /// The page's bottom edge: the drawn box reaches it, so drawing never
    /// clips the text (the room it may take is decided by `make_room`).
    floor: f64,
    width: f64,
    size: f64,
    ratio: f64,
    line_height: f64,
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
            overflow: TextOverflow::Refuse,
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
            text: paragraph_text(before, covering, restyle.replacement),
            page: restyle.page,
            fonts,
            font: restyle.font.to_owned(),
            bbox,
            // The first baseline lands where the source's did: the box top is
            // the paragraph top plus the growth of the first line.
            top: bbox[3] + (size - source_size).max(0.0) * 0.25,
            floor: before.page_box[1],
            width,
            size,
            ratio,
            line_height,
            color: restyle.color.clone().unwrap_or(Rgb::BLACK),
            align: restyle.align.unwrap_or_default(),
        })
    }

    /// Draw `text`; returns the bytes and the font resources added. Text
    /// that would run off the page is refused as an overflow.
    fn draw(&self, text: &str) -> Result<(Vec<u8>, Vec<String>), EditorError> {
        let item = self.item(text);
        let add = EditorOperation::AddText {
            page: item.page,
            rect: item.rect,
            text: text.to_owned(),
            font: self.font.clone(),
            size: self.size,
            color: self.color.clone(),
            align: self.align,
            line_height: self.line_height,
            overflow: TextOverflow::Continue,
        };
        let (out, added) = apply_operation(&self.removed, &add, self.fonts, true)?;
        Ok((out, added.fonts_added))
    }

    /// `text` in the paragraph's style, in its column from its top down to
    /// the page edge.
    fn item<'s>(&'s self, text: &'s str) -> TextBox<'s> {
        TextBox {
            page: self.page,
            rect: Rect { x: self.bbox[0], y: self.floor, width: self.width, height: (self.top - self.floor).max(1.0) },
            text,
            font: &self.font,
            size: self.size,
            color: &self.color,
            align: self.align,
            line_height: self.line_height,
        }
    }

    fn region(&self) -> [f64; 4] {
        let b = self.bbox;
        [b[0], b[1] - (self.ratio - 1.0) * (b[3] - b[1]), b[0] + self.width, self.top]
    }

    /// The longest run of whole words whose glyphs stay above `lowest`:
    /// `((bytes, fonts added), words kept, words in the paragraph)`.
    #[allow(clippy::type_complexity)]
    fn fit(&self, lowest: f64) -> Result<((Vec<u8>, Vec<String>), usize, usize), EditorError> {
        let before = PageTextLayout::from_bytes(&self.removed, self.page as usize)?;
        overflow::fit_words(&self.text, lowest, &before, |text| self.draw(text), |drawn| &drawn.0)
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
