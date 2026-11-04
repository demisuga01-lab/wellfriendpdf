//! Making room for a paragraph that grew.
//!
//! When a restyled or edited paragraph ends lower than it did, the lines that
//! follow it in its column are moved down by the growth, as part of the same
//! operation, so the edited text never draws over them. What belongs to those
//! lines moves with them: link underlines and rules (paths), the clip a
//! browser draws a heading inside, and small images or forms set in a line.
//! Everything else (backgrounds, other columns, page-wide clips) stays.
//!
//! A column ends where the following text is much wider than the edited
//! paragraph (a full-width block below a two-column section), and a large
//! gap in the column (the end of a section, a footer) absorbs the growth
//! instead of moving what lies under it. Moved text never passes the bottom
//! of the page or of a clip that contains it (a browser's page margins).

use std::collections::HashMap;

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, Stream};
use wellfriendpdf_engine::{EditorGlyph, PageTextLayout};

use crate::error::EditorError;
use crate::overlay;

type Matrix = [f64; 6];
const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// Lines that move down together (one paragraph of the column).
#[derive(Debug, Clone)]
pub(crate) struct Group {
    pub lines: Vec<usize>,
    pub bbox: [f64; 4],
    /// Largest glyph size on the group's lines.
    pub size: f64,
    pub shift: f64,
}

/// The lines to move and how far.
#[derive(Debug, Clone)]
pub(crate) struct Room {
    /// How far the edited paragraph's lowest glyph moved down.
    pub growth: f64,
    /// How far moved text would pass the bottom of the page or of its clip
    /// (0 when it fits).
    pub overflow: f64,
    pub groups: Vec<Group>,
    /// The column's x-range and the source block's bottom: decorations move
    /// only from inside the column, below the source block.
    column: [f64; 2],
    source_bottom: f64,
}

impl Room {
    pub fn lines(&self) -> usize {
        self.groups.iter().map(|g| g.lines.len()).sum()
    }

    pub fn largest_shift(&self) -> f64 {
        self.groups.iter().map(|g| g.shift).fold(0.0, f64::max)
    }

    /// The group a decoration with page bounds `bbox` belongs to: inside the
    /// column, below the source block, within three quarters of a line of
    /// the group vertically; the nearest such group.
    fn group_of(&self, bbox: [f64; 4]) -> Option<&Group> {
        let [x0, x1] = self.column;
        if bbox[0] < x0 - 2.0 || bbox[2] > x1 + 2.0 || bbox[3] > self.source_bottom + 0.5 {
            return None;
        }
        let centre = (bbox[1] + bbox[3]) / 2.0;
        self.groups
            .iter()
            .filter(|g| {
                let t = 0.75 * g.size;
                bbox[3] <= g.bbox[3] + t && bbox[1] >= g.bbox[1] - t
            })
            .min_by(|a, b| {
                let da = (centre - (a.bbox[1] + a.bbox[3]) / 2.0).abs();
                let db = (centre - (b.bbox[1] + b.bbox[3]) / 2.0).abs();
                da.total_cmp(&db)
            })
    }
}

pub(crate) fn visible(glyph: &EditorGlyph) -> bool {
    !glyph.text.trim().is_empty() && !glyph.invisible
}

/// For each glyph of `after`: `true` when it is not an unchanged glyph of
/// `before` (same text in the same box), i.e. the edit drew it.
///
/// Boxes match within 0.05 units: rewriting a content stream stores its
/// numbers at single precision, which moves untouched glyphs by far less.
pub(crate) fn edited_glyphs(before: &PageTextLayout, after: &PageTextLayout) -> Vec<bool> {
    unchanged_glyphs(before, after).iter().map(Option::is_none).collect()
}

/// Unmatched glyphs of `before` by text: their box and index, taken once.
type GlyphSlots<'a> = HashMap<&'a str, Vec<Option<([f64; 4], usize)>>>;

/// For each glyph of `after`, the glyph of `before` it is (same text, same
/// box within 0.05 units), or `None` when the edit drew it.
pub(crate) fn unchanged_glyphs(before: &PageTextLayout, after: &PageTextLayout) -> Vec<Option<usize>> {
    let mut unchanged: GlyphSlots = HashMap::new();
    for (index, glyph) in before.glyphs.iter().enumerate() {
        unchanged.entry(glyph.text.as_str()).or_default().push(Some((glyph.bbox, index)));
    }
    after
        .glyphs
        .iter()
        .map(|glyph| {
            let candidates = unchanged.get_mut(glyph.text.as_str())?;
            let slot = candidates.iter_mut().find(|candidate| {
                candidate.is_some_and(|(b, _)| b.iter().zip(glyph.bbox).all(|(x, y)| (x - y).abs() < 0.05))
            })?;
            slot.take().map(|(_, index)| index)
        })
        .collect()
}

/// Bounds of the visible glyphs flagged in `edited`.
pub(crate) fn edited_box(after: &PageTextLayout, edited: &[bool]) -> Option<[f64; 4]> {
    after
        .glyphs
        .iter()
        .zip(edited)
        .filter(|(glyph, edited)| **edited && visible(glyph))
        .map(|(glyph, _)| glyph.bbox)
        .reduce(union)
}

/// The source block of `paragraphs` (indexes into `before`): its bounds and
/// its lowest baseline.
pub(crate) fn source_block(before: &PageTextLayout, paragraphs: &[usize]) -> Option<([f64; 4], f64)> {
    let bbox = paragraphs.iter().map(|&p| before.paragraphs[p].bbox).reduce(union)?;
    let lowest = paragraphs
        .iter()
        .flat_map(|&p| before.lines[before.paragraphs[p].lines[0]..before.paragraphs[p].lines[1]].iter())
        .map(|line| line.baseline[0][1].min(line.baseline[1][1]))
        .fold(f64::INFINITY, f64::min);
    Some((bbox, if lowest.is_finite() { lowest } else { bbox[1] }))
}

fn horizontal(layout: &PageTextLayout, line: usize) -> bool {
    let line = &layout.lines[line];
    let [[x0, y0], [x1, y1]] = line.baseline;
    let glyphs = &layout.glyphs[line.glyphs[0]..line.glyphs[1]];
    x1 >= x0 && (y1 - y0).abs() <= 0.02 * (x1 - x0) + 0.05 && glyphs.iter().all(|g| !g.vertical)
}

fn line_size(layout: &PageTextLayout, line: usize) -> f64 {
    let line = &layout.lines[line];
    layout.glyphs[line.glyphs[0]..line.glyphs[1]]
        .iter()
        .map(|g| g.font_size)
        .fold(0.0_f64, f64::max)
        .max(1.0)
}

/// The clips in force where each line of `layout` is drawn (page-space
/// bounds of every clipping path, outermost first).
pub(crate) fn clips(bytes: &[u8], page: u32, layout: &PageTextLayout) -> Result<Vec<Vec<[f64; 4]>>, EditorError> {
    let document = overlay::load(bytes)?;
    let page_id = overlay::page_id(&document, page)?;
    let mut tracker = Tracker::new(layout, None, HashMap::new());
    for id in document.get_page_contents(page_id) {
        tracker.rewrite(decode(&document, id)?);
    }
    Ok(tracker.line_clips)
}

/// Which lines of `after` follow the source block in its column, and how far
/// each must move for the edited glyphs (`edited`) to clear them.
///
/// Nothing may pass `limit` (the page's bottom margin, or its edge for
/// `continue`, see `crate::overflow::bottom_limit`) or a clip it is drawn
/// in; `Room::overflow` is by how much the plan would.
pub(crate) fn plan(
    source: ([f64; 4], f64),
    after: &PageTextLayout,
    edited: &[bool],
    line_clips: &[Vec<[f64; 4]>],
    limit: f64,
) -> Option<Room> {
    let (original, lowest_baseline) = source;
    let grown = edited_box(after, edited)?;
    let growth = original[1] - grown[1];
    if growth <= 0.25 {
        return None;
    }
    let (x0, x1) = (original[0].min(grown[0]), original[2].max(grown[2]));
    let width = (x1 - x0).max(1.0);
    // Unchanged horizontal lines below the block that share its x-range,
    // grouped by paragraph, top first.
    let mut found: Vec<(usize, Group)> = Vec::new();
    for (index, line) in after.lines.iter().enumerate() {
        let glyphs = line.glyphs[0]..line.glyphs[1];
        if edited[glyphs.clone()].iter().any(|e| *e)
            || !after.glyphs[glyphs].iter().any(visible)
            || !horizontal(after, index)
        {
            continue;
        }
        let size = line_size(after, index);
        let baseline = line.baseline[0][1];
        let below = baseline < lowest_baseline - 0.5 * size && line.bbox[3] <= original[1] + 0.75 * size;
        let shares_column = line.bbox[2].min(x1) - line.bbox[0].max(x0) > 1.0;
        if !(below && shares_column) {
            continue;
        }
        match found.iter_mut().find(|(paragraph, _)| *paragraph == line.paragraph) {
            Some((_, group)) => {
                group.lines.push(index);
                group.bbox = union(group.bbox, line.bbox);
                group.size = group.size.max(size);
            }
            None => found.push((line.paragraph, Group { lines: vec![index], bbox: line.bbox, size, shift: 0.0 })),
        }
    }
    found.sort_by(|a, b| b.1.bbox[3].total_cmp(&a.1.bbox[3]));
    let mut groups = Vec::new();
    let mut need = growth;
    let mut previous_bottom = original[1];
    let mut normal_gap: Option<f64> = None;
    // The edited lines themselves: above the limit and inside their clips
    // (what a clip hides is lost).
    let edited_floor = after
        .lines
        .iter()
        .enumerate()
        .filter(|(_, line)| edited[line.glyphs[0]..line.glyphs[1]].iter().any(|e| *e))
        .flat_map(|(index, _)| line_clips.get(index).into_iter().flatten())
        .map(|clip| clip[1])
        .fold(limit, f64::max);
    let mut overflow = (edited_floor - grown[1]).max(0.0);
    for (_, mut group) in found {
        let bbox = group.bbox;
        // Much wider than the column: a block spanning columns, below which
        // the column has ended.
        if bbox[0] < x0 - 0.25 * width - 2.0 || bbox[2] > x1 + 0.25 * width + 2.0 {
            break;
        }
        let gap = previous_bottom - bbox[3];
        let normal = *normal_gap.get_or_insert(gap.max(0.0));
        // White space well beyond the column's usual paragraph gap takes up
        // the growth before anything under it moves.
        need -= (gap - 2.0 * normal.max(6.0)).max(0.0);
        if need <= 0.25 {
            break;
        }
        // The lowest the group may go: the page, or a clip around it that
        // does not move with it (a clip local to the group moves too).
        let t = 0.75 * group.size;
        let floor = group
            .lines
            .iter()
            .flat_map(|&line| line_clips.get(line).into_iter().flatten())
            .filter(|clip| !(clip[3] <= bbox[3] + t && clip[1] >= bbox[1] - t))
            .map(|clip| clip[1])
            .fold(limit, f64::max);
        overflow = overflow.max(floor - (bbox[1] - need));
        group.shift = need;
        groups.push(group);
        previous_bottom = bbox[1];
    }
    Some(Room { growth, overflow: overflow.max(0.0), groups, column: [x0, x1], source_bottom: original[1] })
}

/// Move `room`'s groups (lines of `layout`, the layout of `bytes`) down, with
/// their decorations. Each text-showing operator that starts a line is
/// attributed to the layout line at its start point; the text matrix of
/// moved runs is translated, and restored where an unmoved run follows in
/// the same text object. Paths (clips included) are moved by rewriting their
/// coordinates, XObjects by a translation around their `Do`. Content streams
/// are written as new objects, so a stream shared with another page is left
/// unchanged.
pub(crate) fn push(bytes: &[u8], page: u32, layout: &PageTextLayout, room: &Room) -> Result<Vec<u8>, EditorError> {
    if room.groups.is_empty() {
        return Ok(bytes.to_vec());
    }
    let mut document = overlay::load(bytes)?;
    let page_id = overlay::page_id(&document, page)?;
    let xobjects = xobject_frames(&document, page_id);
    let mut tracker = Tracker::new(layout, Some(room), xobjects);
    let mut streams = Vec::new();
    for id in document.get_page_contents(page_id) {
        let operations = tracker.rewrite(decode(&document, id)?);
        let encoded = Content { operations }
            .encode()
            .map_err(|e| EditorError::new("EDITOR_WRITE_FAILED", e.to_string()))?;
        let mut stream = Stream::new(Dictionary::new(), encoded);
        let _ = stream.compress();
        streams.push(Object::Reference(document.add_object(stream)));
    }
    if tracker.moved == 0 {
        return Err(EditorError::new(
            "EDITOR_TEXT_LAYOUT_FAILED",
            "the text below the edited paragraph could not be located to move it",
        ));
    }
    document
        .get_object_mut(page_id)
        .and_then(Object::as_dict_mut)
        .map_err(|e| EditorError::new("EDITOR_WRITE_FAILED", e.to_string()))?
        .set("Contents", Object::Array(streams));
    overlay::save(&mut document)
}

/// Remove the text runs that draw `lines` of `layout` (the layout of
/// `bytes`), keeping every other operator: the paragraph is re-set as new
/// text. Used where the reflow transaction cannot delete the text (all of a
/// page's or a form's text, with nothing around it to anchor the deletion).
pub(crate) fn remove_lines(bytes: &[u8], page: u32, layout: &PageTextLayout, lines: &[usize]) -> Result<Vec<u8>, EditorError> {
    let mut document = overlay::load(bytes)?;
    let page_id = overlay::page_id(&document, page)?;
    let mut tracker = Tracker::new(layout, None, HashMap::new());
    tracker.drop_lines = vec![false; layout.lines.len()];
    for &line in lines {
        tracker.drop_lines[line] = true;
    }
    let mut streams = Vec::new();
    for id in document.get_page_contents(page_id) {
        let operations = tracker.rewrite(decode(&document, id)?);
        let encoded = Content { operations }
            .encode()
            .map_err(|e| EditorError::new("EDITOR_WRITE_FAILED", e.to_string()))?;
        let mut stream = Stream::new(Dictionary::new(), encoded);
        let _ = stream.compress();
        streams.push(Object::Reference(document.add_object(stream)));
    }
    if tracker.dropped == 0 {
        return Err(EditorError::new("EDITOR_TEXT_LAYOUT_FAILED", "the paragraph's text runs could not be located"));
    }
    document
        .get_object_mut(page_id)
        .and_then(Object::as_dict_mut)
        .map_err(|e| EditorError::new("EDITOR_WRITE_FAILED", e.to_string()))?
        .set("Contents", Object::Array(streams));
    overlay::save(&mut document)
}

/// Remove the thin filled rectangles just under `baselines` inside `area`
/// (link underlines drawn as paths): the text they decorated was removed
/// and re-set elsewhere, so they would cross it.
pub(crate) fn remove_underlines(
    bytes: &[u8],
    page: u32,
    area: [f64; 4],
    baselines: &[f64],
    size: f64,
) -> Result<Vec<u8>, EditorError> {
    let mut document = overlay::load(bytes)?;
    let page_id = overlay::page_id(&document, page)?;
    let mut removed = 0usize;
    let mut streams = Vec::new();
    let mut ctm = IDENTITY;
    let mut stack = Vec::new();
    for id in document.get_page_contents(page_id) {
        let mut out = Vec::new();
        let mut path: Vec<Operation> = Vec::new();
        for operation in decode(&document, id)? {
            match operation.operator.as_str() {
                "q" => {
                    stack.push(ctm);
                    out.push(operation);
                }
                "Q" => {
                    ctm = stack.pop().unwrap_or(IDENTITY);
                    out.push(operation);
                }
                "cm" => {
                    if let Some(m) = matrix(&operation.operands) {
                        ctm = multiply(m, ctm);
                    }
                    out.push(operation);
                }
                "m" | "l" | "c" | "v" | "y" | "h" | "re" | "W" | "W*" => path.push(operation),
                "f" | "F" | "f*" if path.len() == 1 && path[0].operator == "re" => {
                    let values: Vec<f64> = path[0].operands.iter().filter_map(|o| number(Some(o))).collect();
                    let underline = values.len() == 4 && {
                        let [x, y, w, h] = [values[0], values[1], values[2], values[3]];
                        let corners = [[x, y], [x + w, y], [x, y + h], [x + w, y + h]].map(|p| apply(ctm, p));
                        let b = corners.iter().fold(
                            [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY],
                            |a, p| [a[0].min(p[0]), a[1].min(p[1]), a[2].max(p[0]), a[3].max(p[1])],
                        );
                        b[3] - b[1] <= 0.12 * size
                            && b[2] > b[0]
                            && b[0] >= area[0] - 1.0
                            && b[2] <= area[2] + 1.0
                            && baselines.iter().any(|y| b[3] <= y + 0.05 * size && b[1] >= y - 0.35 * size)
                    };
                    if underline {
                        removed += 1;
                        path.clear();
                    } else {
                        out.append(&mut path);
                        out.push(operation);
                    }
                }
                _ => {
                    out.append(&mut path);
                    out.push(operation);
                }
            }
        }
        out.append(&mut path);
        streams.push(out);
    }
    if removed == 0 {
        return Ok(bytes.to_vec());
    }
    let mut references = Vec::new();
    for operations in streams {
        let encoded = Content { operations }
            .encode()
            .map_err(|e| EditorError::new("EDITOR_WRITE_FAILED", e.to_string()))?;
        let mut stream = Stream::new(Dictionary::new(), encoded);
        let _ = stream.compress();
        references.push(Object::Reference(document.add_object(stream)));
    }
    document
        .get_object_mut(page_id)
        .and_then(Object::as_dict_mut)
        .map_err(|e| EditorError::new("EDITOR_WRITE_FAILED", e.to_string()))?
        .set("Contents", Object::Array(references));
    overlay::save(&mut document)
}

fn decode(document: &Document, id: lopdf::ObjectId) -> Result<Vec<Operation>, EditorError> {
    let stream = document
        .get_object(id)
        .and_then(Object::as_stream)
        .map_err(|e| EditorError::new("EDITOR_TEXT_LAYOUT_FAILED", e.to_string()))?;
    let plain = stream.decompressed_content().unwrap_or_else(|_| stream.content.clone());
    Content::decode(&plain).map(|c| c.operations).map_err(|e| {
        EditorError::new("EDITOR_TEXT_LAYOUT_FAILED", format!("the page content could not be read to move text: {e}"))
    })
}

/// Page XObjects by resource name: the unit square for an image, the
/// `/BBox` under `/Matrix` for a form, as `(matrix, bbox)`.
fn xobject_frames(document: &Document, page: lopdf::ObjectId) -> HashMap<Vec<u8>, (Matrix, [f64; 4])> {
    let mut out = HashMap::new();
    let Some(resources) = crate::fonts::page_resources(document, page) else {
        return out;
    };
    let Some(xobjects) = resources.get(b"XObject").ok().and_then(|o| resolve_dict(document, o)) else {
        return out;
    };
    for (name, value) in xobjects.iter() {
        let Ok(stream) = value.as_reference().and_then(|id| document.get_object(id)).and_then(Object::as_stream) else {
            continue;
        };
        let dict = &stream.dict;
        let frame = match dict.get(b"Subtype").and_then(Object::as_name) {
            Ok(b"Image") => (IDENTITY, [0.0, 0.0, 1.0, 1.0]),
            Ok(b"Form") => {
                let numbers = |key: &[u8]| -> Option<Vec<f64>> {
                    let array = dict.get(key).ok()?.as_array().ok()?;
                    array.iter().map(|o| number(Some(o))).collect()
                };
                let Some(b) = numbers(b"BBox").filter(|b| b.len() == 4) else { continue };
                let m = numbers(b"Matrix").filter(|m| m.len() == 6).map_or(IDENTITY, |m| [m[0], m[1], m[2], m[3], m[4], m[5]]);
                (m, [b[0].min(b[2]), b[1].min(b[3]), b[0].max(b[2]), b[1].max(b[3])])
            }
            _ => continue,
        };
        out.insert(name.clone(), frame);
    }
    out
}

fn resolve_dict<'a>(document: &'a Document, object: &'a Object) -> Option<&'a Dictionary> {
    match object {
        Object::Reference(id) => document.get_object(*id).ok()?.as_dict().ok(),
        Object::Dictionary(dict) => Some(dict),
        _ => None,
    }
}

/// Text and graphics state needed to know where each text run, path and
/// XObject is drawn.
struct Tracker<'a> {
    layout: &'a PageTextLayout,
    /// `None` only scans (records each line's clips).
    room: Option<&'a Room>,
    shift_of: Vec<Option<f64>>,
    xobjects: HashMap<Vec<u8>, (Matrix, [f64; 4])>,
    ctm: Matrix,
    stack: Vec<(Matrix, f64, usize)>,
    clips: Vec<[f64; 4]>,
    line_clips: Vec<Vec<[f64; 4]>>,
    tm: Matrix,
    tlm: Matrix,
    leading: f64,
    /// A line-positioning operator ran since the last text run.
    pending: bool,
    /// The translation (text-space units before the CTM) currently added to
    /// the emitted text matrix.
    emitted: Option<[f64; 2]>,
    /// Path construction operators not yet painted, with their bounds.
    path: Vec<Operation>,
    path_box: Option<[f64; 4]>,
    clipping: bool,
    moved: usize,
    /// Lines whose runs are removed (re-set elsewhere), and the line of the
    /// run in progress.
    drop_lines: Vec<bool>,
    current_line: Option<usize>,
    dropped: usize,
}

impl<'a> Tracker<'a> {
    fn new(layout: &'a PageTextLayout, room: Option<&'a Room>, xobjects: HashMap<Vec<u8>, (Matrix, [f64; 4])>) -> Self {
        let mut shift_of = vec![None; layout.lines.len()];
        for group in room.map(|r| r.groups.as_slice()).unwrap_or_default() {
            for &line in &group.lines {
                shift_of[line] = Some(group.shift);
            }
        }
        Self {
            layout,
            room,
            shift_of,
            xobjects,
            ctm: IDENTITY,
            stack: Vec::new(),
            clips: Vec::new(),
            line_clips: vec![Vec::new(); layout.lines.len()],
            tm: IDENTITY,
            tlm: IDENTITY,
            leading: 0.0,
            pending: true,
            emitted: None,
            path: Vec::new(),
            path_box: None,
            clipping: false,
            moved: 0,
            drop_lines: Vec::new(),
            current_line: None,
            dropped: 0,
        }
    }

    fn rewrite(&mut self, operations: Vec<Operation>) -> Vec<Operation> {
        let mut out = Vec::with_capacity(operations.len() + 16);
        for operation in operations {
            match operation.operator.as_str() {
                "q" => {
                    self.stack.push((self.ctm, self.leading, self.clips.len()));
                    out.push(operation);
                }
                "Q" => {
                    if let Some((ctm, leading, clips)) = self.stack.pop() {
                        self.ctm = ctm;
                        self.leading = leading;
                        self.clips.truncate(clips);
                    }
                    out.push(operation);
                }
                "cm" => {
                    if let Some(m) = matrix(&operation.operands) {
                        self.ctm = multiply(m, self.ctm);
                    }
                    out.push(operation);
                }
                "m" | "l" | "c" | "v" | "y" | "h" | "re" => {
                    self.extend_path(&operation);
                    self.path.push(operation);
                }
                "W" | "W*" => {
                    self.clipping = true;
                    self.path.push(operation);
                }
                "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" | "n" => {
                    self.paint(&mut out);
                    out.push(operation);
                }
                "Do" => self.place(operation, &mut out),
                "BT" => {
                    self.tm = IDENTITY;
                    self.tlm = IDENTITY;
                    self.pending = true;
                    self.emitted = None;
                    out.push(operation);
                }
                "Tm" => match matrix(&operation.operands) {
                    Some(m) => {
                        self.tm = m;
                        self.tlm = m;
                        self.pending = true;
                        out.push(match self.emitted {
                            Some(u) => tm_operation(translated(m, u)),
                            None => operation,
                        });
                    }
                    None => out.push(operation),
                },
                "Td" | "TD" => {
                    if let (Some(tx), Some(ty)) = (number(operation.operands.first()), number(operation.operands.get(1))) {
                        if operation.operator == "TD" {
                            self.leading = -ty;
                        }
                        self.next_line(tx, ty);
                    }
                    out.push(operation);
                }
                "T*" => {
                    self.next_line(0.0, -self.leading);
                    out.push(operation);
                }
                "TL" => {
                    if let Some(value) = number(operation.operands.first()) {
                        self.leading = value;
                    }
                    out.push(operation);
                }
                "Tj" | "TJ" => {
                    if let Some(change) = self.decide() {
                        out.push(change);
                    }
                    if self.dropping() {
                        continue;
                    }
                    out.push(operation);
                }
                "'" | "\"" => {
                    self.next_line(0.0, -self.leading);
                    let change = self.decide();
                    if self.dropping() {
                        // Keep the line move and spacing, drop the run.
                        let mut operands = operation.operands;
                        if operation.operator == "\"" && operands.len() == 3 {
                            out.push(Operation::new("Tw", vec![operands.remove(0)]));
                            out.push(Operation::new("Tc", vec![operands.remove(0)]));
                        }
                        out.push(Operation::new("T*", vec![]));
                        continue;
                    }
                    match change {
                        None => out.push(operation),
                        Some(change) => {
                            // Split into its parts so the new text matrix
                            // lands between the line move and the run.
                            let mut operands = operation.operands;
                            if operation.operator == "\"" && operands.len() == 3 {
                                out.push(Operation::new("Tw", vec![operands.remove(0)]));
                                out.push(Operation::new("Tc", vec![operands.remove(0)]));
                            }
                            out.push(Operation::new("T*", vec![]));
                            out.push(change);
                            out.push(Operation::new("Tj", operands));
                        }
                    }
                }
                _ => out.push(operation),
            }
        }
        out
    }

    fn extend_path(&mut self, operation: &Operation) {
        let values: Vec<f64> = operation.operands.iter().filter_map(|o| number(Some(o))).collect();
        let points: Vec<[f64; 2]> = if operation.operator == "re" && values.len() == 4 {
            let [x, y, w, h] = [values[0], values[1], values[2], values[3]];
            vec![[x, y], [x + w, y], [x, y + h], [x + w, y + h]]
        } else {
            values.chunks_exact(2).map(|p| [p[0], p[1]]).collect()
        };
        for point in points {
            let [x, y] = apply(self.ctm, point);
            self.path_box = Some(match self.path_box {
                None => [x, y, x, y],
                Some(b) => [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)],
            });
        }
    }

    /// Emit the pending path, moved with its group when it belongs to one.
    fn paint(&mut self, out: &mut Vec<Operation>) {
        let path = std::mem::take(&mut self.path);
        let bbox = self.path_box.take();
        let clipping = std::mem::replace(&mut self.clipping, false);
        let shift = bbox.and_then(|b| self.room?.group_of(b)).map(|g| g.shift);
        let bbox = bbox.map(|b| match shift {
            Some(s) => [b[0], b[1] - s, b[2], b[3] - s],
            None => b,
        });
        if clipping {
            if let Some(b) = bbox {
                self.clips.push(b);
            }
        }
        match shift {
            None => out.extend(path),
            Some(s) => {
                self.moved += 1;
                let u = down_in_text_space(self.ctm, s);
                out.extend(path.into_iter().map(|mut operation| {
                    if operation.operator == "re" {
                        let coordinates = operation.operands.len().min(2);
                        translate_operands(&mut operation.operands[..coordinates], u);
                    } else if operation.operator != "h" && !operation.operator.starts_with('W') {
                        translate_operands(&mut operation.operands, u);
                    }
                    operation
                }));
            }
        }
    }

    /// An XObject inside a moved group's lines moves with them.
    fn place(&mut self, operation: Operation, out: &mut Vec<Operation>) {
        let frame = operation
            .operands
            .first()
            .and_then(|o| o.as_name().ok())
            .and_then(|name| self.xobjects.get(name))
            .copied();
        let shift = frame.and_then(|(m, b)| {
            let to_page = multiply(m, self.ctm);
            let corners = [[b[0], b[1]], [b[2], b[1]], [b[0], b[3]], [b[2], b[3]]].map(|p| apply(to_page, p));
            let bbox = corners.iter().fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |a, p| {
                [a[0].min(p[0]), a[1].min(p[1]), a[2].max(p[0]), a[3].max(p[1])]
            });
            self.room?.group_of(bbox).map(|g| g.shift)
        });
        match shift {
            None => out.push(operation),
            Some(s) => {
                self.moved += 1;
                let u = down_in_text_space(self.ctm, s);
                out.push(Operation::new("q", vec![]));
                out.push(Operation::new(
                    "cm",
                    [1.0, 0.0, 0.0, 1.0, u[0], u[1]].iter().map(|v| Object::Real(*v as f32)).collect(),
                ));
                out.push(operation);
                out.push(Operation::new("Q", vec![]));
            }
        }
    }

    fn next_line(&mut self, tx: f64, ty: f64) {
        self.tlm = multiply([1.0, 0.0, 0.0, 1.0, tx, ty], self.tlm);
        self.tm = self.tlm;
        self.pending = true;
    }

    /// At the first run after a line move: the run's line decides whether
    /// it moves; returns the `Tm` to emit when that differs from the text
    /// matrix in force.
    fn decide(&mut self) -> Option<Operation> {
        if !self.pending {
            return None;
        }
        self.pending = false;
        let origin = apply(self.ctm, [self.tm[4], self.tm[5]]);
        let line = self.line_at(origin);
        self.current_line = line;
        if let Some(line) = line {
            if self.room.is_none() {
                self.line_clips[line].extend(self.clips.iter().copied());
            }
        }
        let want = line.and_then(|line| self.shift_of[line]).map(|shift| {
            self.moved += 1;
            down_in_text_space(self.ctm, shift)
        });
        if want == self.emitted {
            return None;
        }
        self.emitted = want;
        Some(tm_operation(translated(self.tm, want.unwrap_or([0.0, 0.0]))))
    }

    /// Whether the run in progress is on a line being removed.
    fn dropping(&mut self) -> bool {
        let drop = self.current_line.is_some_and(|line| self.drop_lines.get(line).copied().unwrap_or(false));
        self.dropped += usize::from(drop);
        drop
    }

    /// The layout line a run starting at `point` (page space) draws: the
    /// line of the glyph whose pen start is nearest, on the run's baseline
    /// and at or just after the point (a leading kerning offset). Glyphs, not
    /// lines, are compared because a superscript joins its line's glyphs
    /// while sitting on its own baseline.
    fn line_at(&self, point: [f64; 2]) -> Option<usize> {
        let mut best: Option<(f64, usize)> = None;
        for glyph in &self.layout.glyphs {
            if glyph.vertical {
                continue;
            }
            let size = glyph.font_size.max(1.0);
            let [x, y] = glyph.baseline[0];
            let (dx, dy) = (x - point[0], (y - point[1]).abs());
            if dy > 0.35 * size || dx < -0.05 * size - 0.1 || dx > 1.5 * size {
                continue;
            }
            let score = dy + 0.05 * dx.abs();
            if best.is_none_or(|(s, _)| score < s) {
                best = Some((score, glyph.line));
            }
        }
        best.map(|(_, line)| line)
    }
}

/// Add `u` to each coordinate pair of `operands` (path coordinates in user
/// space).
fn translate_operands(operands: &mut [Object], u: [f64; 2]) {
    for (index, operand) in operands.iter_mut().enumerate() {
        if let Some(value) = number(Some(operand)) {
            *operand = Object::Real((value + u[index % 2]) as f32);
        }
    }
}

/// The user-space (or text-space, before the CTM) translation that moves
/// what is drawn `shift` units down the page under `ctm`.
fn down_in_text_space(ctm: Matrix, shift: f64) -> [f64; 2] {
    let [a, b, c, d, _, _] = ctm;
    let det = a * d - b * c;
    if det.abs() < 1e-12 {
        return [0.0, -shift];
    }
    [shift * c / det, -shift * a / det]
}

fn translated(m: Matrix, u: [f64; 2]) -> Matrix {
    [m[0], m[1], m[2], m[3], m[4] + u[0], m[5] + u[1]]
}

fn tm_operation(m: Matrix) -> Operation {
    Operation::new("Tm", m.iter().map(|v| Object::Real(*v as f32)).collect())
}

fn multiply(a: Matrix, b: Matrix) -> Matrix {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}

fn apply(m: Matrix, p: [f64; 2]) -> [f64; 2] {
    [p[0] * m[0] + p[1] * m[2] + m[4], p[0] * m[1] + p[1] * m[3] + m[5]]
}

fn number(object: Option<&Object>) -> Option<f64> {
    match object? {
        Object::Integer(i) => Some(*i as f64),
        Object::Real(r) => Some(*r as f64),
        _ => None,
    }
}

fn matrix(operands: &[Object]) -> Option<Matrix> {
    if operands.len() != 6 {
        return None;
    }
    let mut m = [0.0; 6];
    for (slot, operand) in m.iter_mut().zip(operands) {
        *slot = number(Some(operand))?;
    }
    Some(m)
}

pub(crate) fn union(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

fn intersects(a: [f64; 4], b: [f64; 4], tolerance: f64) -> bool {
    a[0] < b[2] - tolerance && b[0] < a[2] - tolerance && a[1] < b[3] - tolerance && b[1] < a[3] - tolerance
}

/// Intersecting pairs of a visible glyph flagged in `mine` and a visible,
/// horizontal glyph flagged in neither `mine` nor `own` (the edited
/// paragraph's own unchanged glyphs, whose spacing the line check owns),
/// with the first pair found.
pub(crate) fn overlaps(layout: &PageTextLayout, mine: &[bool], own: &[bool]) -> (usize, Option<(String, String)>) {
    let flagged: Vec<&EditorGlyph> =
        layout.glyphs.iter().zip(mine).filter(|(g, m)| **m && visible(g)).map(|(g, _)| g).collect();
    let Some(area) = flagged.iter().map(|g| g.bbox).reduce(union) else {
        return (0, None);
    };
    let mut count = 0;
    let mut first = None;
    for (index, other) in layout.glyphs.iter().enumerate() {
        if mine[index] || own[index] || !visible(other) || other.vertical || !intersects(other.bbox, area, 0.01) {
            continue;
        }
        for glyph in flagged.iter().filter(|g| intersects(g.bbox, other.bbox, 0.01)) {
            count += 1;
            first.get_or_insert_with(|| (glyph.text.clone(), other.text.clone()));
        }
    }
    (count, first)
}
