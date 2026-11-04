//! Text that does not fit: the overflow policies of `replaceText`,
//! `restyleText` and `addText`, and the check that no text operation loses
//! the user's words.
//!
//! A text operation has room down to a limit: for an edited or restyled
//! paragraph, the page's bottom margin (the lowest text already on the page,
//! or a margin as deep as the page's top margin, whichever is lower) and the
//! text below it in its column, which moves down only as far as that limit;
//! for an added text box, the bottom of its box. What happens when the text
//! needs more is the operation's [`TextOverflow`] policy:
//!
//! - `refuse` (the default): the operation fails with `EDITOR_TEXT_OVERFLOW`
//!   (how many words and characters do not fit, and whether `continue` and
//!   `nextPage` could place them) and the document is unchanged;
//! - `shrink`: the largest size, in quarter points down to
//!   [`MIN_SHRINK_SIZE`], at which everything fits (line spacing scales with
//!   it);
//! - `continue`: the text keeps its style and may run past the margin or its
//!   box down to the page edge, never over other text;
//! - `nextPage`: the words that fit stay, the rest continues at the top of
//!   the same column on the next page, whose column text moves down.
//!
//! [`ensure_kept`] runs after every text operation: the page must read back
//! every word the operation was asked to show (whitespace-normalised), on
//! the page, or the operation fails with `EDITOR_TEXT_LOST`.

use serde::Serialize;
use serde_json::{json, Value};
use wellfriendpdf_engine::{EditorLine, PageTextLayout};

use crate::apply::{apply_operation, OperationReport};
use crate::error::EditorError;
use crate::flow;
use crate::fonts::{covers, FontSource};
use crate::ops::{Align, EditorOperation, Rect, Rgb, TextOverflow};

pub const OVERFLOW: &str = "EDITOR_TEXT_OVERFLOW";
pub const TEXT_LOST: &str = "EDITOR_TEXT_LOST";
/// `shrink` never sets text smaller than this (or than the requested size).
pub const MIN_SHRINK_SIZE: f64 = 4.0;

/// How one attempt treats text that needs more room: `mode` is `Refuse`,
/// `Continue` or `NextPage` (a `shrink` is a search over `size` with
/// `Refuse`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    pub mode: TextOverflow,
    /// The size to set the text at instead of the requested one.
    pub size: Option<f64>,
}

impl Fit {
    pub const REFUSE: Fit = Fit { mode: TextOverflow::Refuse, size: None };

    pub const fn new(mode: TextOverflow) -> Self {
        Self { mode, size: None }
    }

    fn shrunk(size: f64) -> Self {
        Self { mode: TextOverflow::Refuse, size: Some(size) }
    }

    /// For an edit that cannot continue on another page (the in-place
    /// transaction, text inside a form): `nextPage` asks it to refuse, and
    /// the caller re-sets the text when it does.
    pub fn without_next_page(self) -> Self {
        match self.mode {
            TextOverflow::NextPage => Self { mode: TextOverflow::Refuse, ..self },
            _ => self,
        }
    }
}

/// What an overflow policy did, in the operation's report.
#[derive(Debug, Clone, Serialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OverflowOutcome {
    pub policy: TextOverflow,
    /// The size `shrink` set the text at.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    /// The page the rest continues on (`nextPage`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_page: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moved_words: Option<usize>,
}

/// The two parts of a text split by `nextPage`, for [`ensure_kept`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Continued {
    pub kept: String,
    pub rest: String,
}

/// A text box to draw (an `addText`, or a re-set paragraph).
#[derive(Debug, Clone, Copy)]
pub struct TextBox<'a> {
    pub page: u32,
    pub rect: Rect,
    pub text: &'a str,
    pub font: &'a str,
    pub size: f64,
    pub color: &'a Rgb,
    pub align: Align,
    pub line_height: f64,
}

/// How much of an edit's text does not fit.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Shortfall {
    pub page: u32,
    pub words: usize,
    pub characters: usize,
    pub total_words: usize,
}

fn policy_name(mode: TextOverflow) -> Value {
    serde_json::to_value(mode).unwrap_or(Value::Null)
}

pub(crate) fn overflow_error(shortfall: &Shortfall, mode: TextOverflow) -> EditorError {
    let Shortfall { page, words, characters, total_words } = *shortfall;
    let what = format!("{words} of {total_words} words ({characters} characters)");
    let message = match mode {
        TextOverflow::Continue => format!(
            "the text does not fit on page {page} even down to the page edge: {what} would run off the page or over other text"
        ),
        TextOverflow::NextPage => format!("the text does not fit on page {page}: {what} do not fit and cannot continue on the next page"),
        _ => format!(
            "the text does not fit on page {page}: {what} would pass the bottom margin or the text below it. Nothing was changed; choose shrink, continue or nextPage, or undo the edit"
        ),
    };
    let mut error = EditorError::new(OVERFLOW, message);
    error.details = Some(json!({
        "policy": policy_name(mode),
        "page": page,
        "words": words,
        "characters": characters,
        "totalWords": total_words,
    }));
    error
}

fn no_next_page(message: String) -> EditorError {
    let mut error = EditorError::new(OVERFLOW, message);
    error.details = Some(json!({ "policy": "nextPage", "nextPage": false }));
    error
}

/// The lowest a text operation's glyphs may reach under `mode` (see the
/// module documentation). `size` is the text's size: a last line's
/// descenders may reach a third of it below the margin.
pub(crate) fn bottom_limit(before: &PageTextLayout, mode: TextOverflow, size: f64) -> f64 {
    let floor = before.page_box[1];
    if mode == TextOverflow::Continue {
        return floor;
    }
    let (mut lowest, mut highest) = (f64::INFINITY, f64::NEG_INFINITY);
    for glyph in before.glyphs.iter().filter(|g| flow::visible(g) && !g.vertical) {
        lowest = lowest.min(glyph.bbox[1]);
        highest = highest.max(glyph.bbox[3]);
    }
    if !lowest.is_finite() {
        return floor;
    }
    let margin = floor + (before.page_box[3] - highest).max(0.0);
    (lowest.min(margin) - 0.3 * size).max(floor)
}

/// Words and characters on the lines with edited glyphs that pass `allowed`.
pub(crate) fn shortfall(after: &PageTextLayout, edited: &[bool], allowed: f64, page: u32) -> Shortfall {
    let mut out = Shortfall { page, ..Shortfall::default() };
    for line in &after.lines {
        let span = line.glyphs[0]..line.glyphs[1];
        let mine: Vec<_> = after.glyphs[span.clone()]
            .iter()
            .zip(&edited[span])
            .filter(|(glyph, edited)| **edited && flow::visible(glyph))
            .map(|(glyph, _)| glyph)
            .collect();
        if mine.is_empty() {
            continue;
        }
        let text = after.text_in(line.range);
        let words = text.split_whitespace().count();
        out.total_words += words;
        if mine.iter().any(|glyph| glyph.bbox[1] < allowed - 0.25) {
            out.words += words;
            out.characters += text.chars().filter(|c| !c.is_whitespace()).count();
        }
    }
    out
}

/// Run a text operation under `policy`. `run` applies it once under a
/// [`Fit`]; `requested_size` is the size a `shrink` starts from.
pub(crate) fn resolve(
    policy: TextOverflow,
    requested_size: impl FnOnce() -> Result<f64, EditorError>,
    mut run: impl FnMut(Fit) -> Result<(Vec<u8>, OperationReport), EditorError>,
) -> Result<(Vec<u8>, OperationReport), EditorError> {
    match policy {
        TextOverflow::Refuse => match run(Fit::REFUSE) {
            Err(mut error) if error.code == OVERFLOW => {
                // Say which policies could place the text.
                let can_continue = run(Fit::new(TextOverflow::Continue)).is_ok();
                let can_move = run(Fit::new(TextOverflow::NextPage)).is_ok();
                let mut details = match error.details.take() {
                    Some(Value::Object(map)) => map,
                    _ => serde_json::Map::new(),
                };
                details.insert("continue".into(), Value::Bool(can_continue));
                details.insert("nextPage".into(), Value::Bool(can_move));
                error.details = Some(Value::Object(details));
                error.message.push_str(if can_move {
                    ". The rest can continue on the next page."
                } else {
                    ". The rest cannot continue on the next page."
                });
                Err(error)
            }
            other => other,
        },
        TextOverflow::Continue | TextOverflow::NextPage => run(Fit::new(policy)),
        TextOverflow::Shrink => match run(Fit::REFUSE) {
            Err(error) if error.code == OVERFLOW => {
                let requested = requested_size()?;
                let smallest = MIN_SHRINK_SIZE.min(requested);
                let mut best = match run(Fit::shrunk(smallest)) {
                    Ok(result) => (smallest, result),
                    Err(mut error) => {
                        if error.code == OVERFLOW {
                            error.message = format!("even at {smallest} pt, {}", error.message);
                        }
                        return Err(error);
                    }
                };
                let (mut low, mut high) = (smallest, requested);
                loop {
                    let middle = ((low + high) * 2.0).floor() / 4.0;
                    if middle <= low + 1e-9 || middle >= high - 1e-9 {
                        break;
                    }
                    match run(Fit::shrunk(middle)) {
                        Ok(result) => {
                            low = middle;
                            best = (middle, result);
                        }
                        Err(_) => high = middle,
                    }
                }
                let (size, (bytes, mut report)) = best;
                report.notes.push(format!(
                    "Shrunk to {size} pt (from {requested:.1} pt) so every word fits."
                ));
                report.overflow = Some(OverflowOutcome {
                    policy: TextOverflow::Shrink,
                    size: Some(size),
                    ..OverflowOutcome::default()
                });
                Ok((bytes, report))
            }
            other => other,
        },
    }
}

fn word_ends(text: &str) -> Vec<usize> {
    text.char_indices()
        .zip(text.chars().skip(1).chain(std::iter::once(' ')))
        .filter(|((_, c), next)| !c.is_whitespace() && next.is_whitespace())
        .map(|((i, c), _)| i + c.len_utf8())
        .collect()
}

/// `text` split after its first `kept` words.
pub(crate) fn split_words(text: &str, kept: usize) -> (String, String) {
    let ends = word_ends(text);
    if kept == 0 {
        return (String::new(), text.trim().to_owned());
    }
    let at = ends[(kept - 1).min(ends.len() - 1)];
    (text[..at].to_owned(), text[at..].trim().to_owned())
}

/// The longest run of whole words of `text` that `draw` sets with every
/// new glyph at or above `lowest`: `(drawn, words kept, words in total)`.
/// A drawing refused as an overflow counts as not fitting.
pub(crate) fn fit_words<T>(
    text: &str,
    lowest: f64,
    before: &PageTextLayout,
    mut draw: impl FnMut(&str) -> Result<T, EditorError>,
    bytes: impl Fn(&T) -> &[u8],
) -> Result<(T, usize, usize), EditorError> {
    let ends = word_ends(text);
    let page = before.page;
    let mut fits = |words: usize| -> Result<Option<T>, EditorError> {
        let drawn = match draw(&text[..ends[words - 1]]) {
            Err(error) if error.code == OVERFLOW => return Ok(None),
            other => other?,
        };
        let after = PageTextLayout::from_bytes(bytes(&drawn), page)?;
        let edited = flow::edited_glyphs(before, &after);
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
        None => Err(EditorError::new(OVERFLOW, "not even the first word fits in the room there is")),
    }
}

/// `addText` under `fit`: the text is drawn from the top of its box; it
/// overflows when a glyph passes the bottom of the box (the page edge for
/// `continue`).
pub(crate) fn add_text(
    input: &[u8],
    item: &TextBox<'_>,
    fit: Fit,
    fonts: &dyn FontSource,
    report: &mut OperationReport,
) -> Result<Vec<u8>, EditorError> {
    let item = TextBox { size: fit.size.unwrap_or(item.size), ..*item };
    let page = item.page as usize;
    let before = PageTextLayout::from_bytes(input, page)?;
    let out = crate::apply::add_text(input, &item, fonts, report)?;
    let after = PageTextLayout::from_bytes(&out, page)?;
    let edited = flow::edited_glyphs(&before, &after);
    let Some(drawn) = flow::edited_box(&after, &edited) else {
        return Ok(out);
    };
    let floor = if fit.mode == TextOverflow::Continue { before.page_box[1] } else { item.rect.y };
    if drawn[1] >= floor - 0.25 {
        if drawn[1] < item.rect.y - 0.25 {
            report.notes.push("The text runs past the bottom of its box.".into());
            report.overflow = Some(OverflowOutcome { policy: TextOverflow::Continue, ..OverflowOutcome::default() });
            if let Some(region) = report.changed_region.as_mut() {
                *region = flow::union(*region, drawn);
            }
        }
        return Ok(out);
    }
    let missing = shortfall(&after, &edited, floor, item.page);
    if fit.mode != TextOverflow::NextPage {
        return Err(overflow_error(&missing, fit.mode));
    }
    // The words that fit stay in the box; the rest continues overleaf.
    require_next_page(input, item.page)?;
    let (fitted, kept, total) = fit_words(
        item.text,
        item.rect.y,
        &before,
        |text| {
            let mut scratch = report.clone();
            crate::apply::add_text(input, &TextBox { text, ..item }, fonts, &mut scratch)
        },
        |bytes| bytes,
    )
    .map_err(|_| overflow_error(&missing, fit.mode))?;
    let (kept_text, rest) = split_words(item.text, kept);
    let next = TextBox { page: item.page + 1, text: &rest, ..item };
    let out = continue_on_next_page(&fitted, &before, &next, fonts)?;
    note_continued(report, item.page + 1, total - kept, total);
    report.continued = Some(Continued { kept: kept_text, rest });
    Ok(out)
}

pub(crate) fn note_continued(report: &mut OperationReport, next_page: u32, moved: usize, total: usize) {
    report.notes.push(format!(
        "The last {moved} of {total} words did not fit and continue at the top of page {next_page}."
    ));
    report.overflow = Some(OverflowOutcome {
        policy: TextOverflow::NextPage,
        next_page: Some(next_page),
        moved_words: Some(moved),
        ..OverflowOutcome::default()
    });
}

/// `nextPage` needs a page after `page`.
pub(crate) fn require_next_page(bytes: &[u8], page: u32) -> Result<(), EditorError> {
    let pages = crate::overlay::load(bytes)?.get_pages().len() as u32;
    if page >= pages {
        return Err(no_next_page(format!(
            "page {page} is the last page: the rest of the text has no next page to continue on"
        )));
    }
    Ok(())
}

fn line_size(layout: &PageTextLayout, line: &EditorLine) -> f64 {
    layout.glyphs[line.glyphs[0]..line.glyphs[1]]
        .iter()
        .map(|g| g.font_size)
        .fold(0.0_f64, f64::max)
}

/// Set `item.text` at the top of the column `[item.rect.x, + width]` of page
/// `item.page`, moving that column's text down by its height (plus half a
/// line). Refused as an overflow when there is no such page, or the page has
/// no room for it.
pub(crate) fn continue_on_next_page(
    bytes: &[u8],
    source: &PageTextLayout,
    item: &TextBox<'_>,
    fonts: &dyn FontSource,
) -> Result<Vec<u8>, EditorError> {
    if item.text.trim().is_empty() {
        return Ok(bytes.to_vec());
    }
    require_next_page(bytes, item.page - 1)?;
    let page = item.page as usize;
    let before = PageTextLayout::from_bytes(bytes, page)?;
    let (x0, x1) = (item.rect.x, item.rect.x + item.rect.width);
    let mut column: Vec<&EditorLine> = before
        .lines
        .iter()
        .filter(|line| {
            line.bbox[2].min(x1) - line.bbox[0].max(x0) > 1.0
                && before.glyphs[line.glyphs[0]..line.glyphs[1]].iter().any(flow::visible)
        })
        .collect();
    column.sort_by(|a, b| b.bbox[3].total_cmp(&a.bbox[3]));
    // A running header (small type well above the column) stays on top.
    while column.len() >= 2 {
        let (first, second) = (column[0], column[1]);
        let height = first.bbox[3] - first.bbox[1];
        if line_size(&before, first) < 0.85 * item.size && first.bbox[1] - second.bbox[3] > 1.5 * height {
            column.remove(0);
        } else {
            break;
        }
    }
    let highest = source
        .glyphs
        .iter()
        .filter(|g| flow::visible(g))
        .map(|g| g.bbox[3])
        .fold(f64::NEG_INFINITY, f64::max);
    let top_margin = if highest.is_finite() { (source.page_box[3] - highest).max(18.0) } else { 72.0 };
    let top = column.first().map_or(before.page_box[3] - top_margin, |line| line.bbox[3]);
    let floor = before.page_box[1];
    let add = EditorOperation::AddText {
        page: item.page,
        rect: Rect { x: x0, y: floor, width: item.rect.width, height: (top - floor).max(1.0) },
        text: item.text.to_owned(),
        font: item.font.to_owned(),
        size: item.size,
        color: item.color.clone(),
        align: item.align,
        line_height: item.line_height,
        overflow: TextOverflow::Continue,
    };
    let unavailable = || no_next_page(format!("page {} has no room for the rest of the text", item.page));
    let (drawn, _) = apply_operation(bytes, &add, fonts, true)
        .map_err(|error| if error.code == OVERFLOW { unavailable() } else { error })?;
    let after = PageTextLayout::from_bytes(&drawn, page)?;
    let edited = flow::edited_glyphs(&before, &after);
    let gap = 0.5 * item.size * item.line_height;
    let block = ([x0, top + gap, x1, top + gap], top + gap);
    let limit = bottom_limit(&before, TextOverflow::Refuse, item.size);
    let clips = flow::clips(&drawn, item.page, &after)?;
    let moved = match flow::plan(block, &after, &edited, &clips, limit) {
        Some(room) if room.overflow > 0.25 => return Err(unavailable()),
        Some(room) if !room.groups.is_empty() => flow::push(&drawn, item.page, &after, &room)?,
        _ => drawn,
    };
    let layout = PageTextLayout::from_bytes(&moved, page)?;
    if layout.text != after.text {
        return Err(EditorError::new(
            "EDITOR_TEXT_LAYOUT_FAILED",
            "moving the next page's column text down changed it",
        ));
    }
    // The continued text may not draw over text it did not move.
    let mine: Vec<bool> = flow::unchanged_glyphs(&after, &layout)
        .into_iter()
        .map(|matched| matched.is_some_and(|index| edited[index]))
        .collect();
    let none = vec![false; layout.glyphs.len()];
    if let (count, Some((a, b))) = flow::overlaps(&layout, &mine, &none) {
        if count > 0 {
            return Err(no_next_page(format!(
                "the rest of the text would draw over text on page {} ({a:?} over {b:?})",
                item.page
            )));
        }
    }
    Ok(moved)
}

/// Text compared as a reader sees it: without whitespace (line breaks,
/// spaces a re-set may add or drop) and soft hyphens; `loose` also drops
/// hyphens (a line-end hyphen joined to the next word).
fn normalized(text: &str, loose: bool) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace() && *c != '\u{ad}' && !(loose && matches!(c, '-' | '\u{2010}' | '\u{2011}')))
        .collect()
}

fn contains(page: &str, wanted: &str) -> bool {
    normalized(page, false).contains(&normalized(wanted, false))
        || normalized(page, true).contains(&normalized(wanted, true))
}

fn lost(page: u32, what: &str) -> EditorError {
    let sample: String = what.split_whitespace().take(8).collect::<Vec<_>>().join(" ");
    EditorError::new(
        TEXT_LOST,
        format!("the edit would lose text on page {page} (\u{201c}{sample}\u{2026}\u{201d} is not on the page afterwards); nothing was changed"),
    )
}

/// Visible glyphs drawn outside the page (their baseline off the page box).
fn off_page(layout: &PageTextLayout) -> usize {
    let [x0, y0, x1, y1] = layout.page_box;
    layout
        .glyphs
        .iter()
        .filter(|g| flow::visible(g))
        .filter(|g| {
            let [x, y] = g.baseline[0];
            y < y0 - 0.01 || y > y1 + 0.01 || x > x1 + 0.01 || g.bbox[2] < x0 - 0.01
        })
        .count()
}

/// The invariant every text operation keeps: the page (and, for text that
/// continued, the next page) reads back every word the operation was asked
/// to show, and no more of its glyphs lie off the page than before.
pub(crate) fn ensure_kept(
    input: &[u8],
    output: &[u8],
    operation: &EditorOperation,
    fonts: &dyn FontSource,
    report: &OperationReport,
) -> Result<(), EditorError> {
    let page = match operation {
        EditorOperation::ReplaceText { page, .. }
        | EditorOperation::RestyleText { page, .. }
        | EditorOperation::AddText { page, .. } => *page,
        _ => return Ok(()),
    };
    let before = PageTextLayout::from_bytes(input, page as usize)?;
    let requested = match operation {
        EditorOperation::ReplaceText { start, end, text, .. } => {
            // The edited paragraphs' text after the replacement.
            let (start, end) = (*start, *end);
            let touched: Vec<_> = before
                .paragraphs
                .iter()
                .filter(|p| (p.range[0] < end && p.range[1] > start) || (p.range[0] <= start && start <= p.range[1]))
                .collect();
            let from = touched.iter().map(|p| p.range[0]).min().unwrap_or(start).min(start);
            let to = touched.iter().map(|p| p.range[1]).max().unwrap_or(end).max(end);
            format!("{}{}{}", before.text_in([from, start]), text, before.text_in([end, to]))
        }
        EditorOperation::RestyleText { expected, .. } => expected.clone(),
        EditorOperation::AddText { text, font, .. } => {
            // Characters no font can draw are reported by the operation.
            let faces: Vec<Vec<u8>> = [font.as_str(), "noto-sans-regular", "dejavu-sans-regular"]
                .iter()
                .filter_map(|id| fonts.font_bytes(id))
                .collect();
            let mut drawable = std::collections::BTreeMap::new();
            text.chars()
                .filter(|c| {
                    *drawable.entry(*c).or_insert_with(|| {
                        c.is_whitespace() || faces.iter().any(|face| covers(face, &c.to_string()))
                    })
                })
                .collect()
        }
        _ => return Ok(()),
    };
    let after = PageTextLayout::from_bytes(output, page as usize)?;
    match &report.continued {
        Some(continued) => {
            if !contains(&after.text, &continued.kept) {
                return Err(lost(page, &continued.kept));
            }
            let next = PageTextLayout::from_bytes(output, page as usize + 1)?;
            if !contains(&next.text, &continued.rest) {
                return Err(lost(page + 1, &continued.rest));
            }
            let joined = format!("{}{}", continued.kept, continued.rest);
            if normalized(&joined, true) != normalized(&requested, true) {
                return Err(lost(page, &requested));
            }
        }
        None => {
            if !contains(&after.text, &requested) {
                return Err(lost(page, &requested));
            }
        }
    }
    if off_page(&after) > off_page(&before) {
        return Err(EditorError::new(
            TEXT_LOST,
            format!("the edit would draw text outside page {page}, where it cannot be seen; nothing was changed"),
        ));
    }
    Ok(())
}
