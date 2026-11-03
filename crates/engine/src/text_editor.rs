//! Engine-level building blocks for click-to-edit text editors.
//!
//! [`PageTextLayout`] maps a page's editable text to geometry: per-glyph
//! quads, lines and paragraphs, hit-testing a click to a caret offset, caret
//! geometry, selection quads, word/line/paragraph boundaries and caret
//! movement, in PDF user space and in rendered pixels at any DPI.
//! [`TextEditSession`] applies replace/insert/delete to those offsets through
//! the engine's style-preserving reflow transaction, reports the region that
//! needs repainting, and keeps undo/redo history.
//!
//! The UI owns presentation and input; the engine owns text, geometry and
//! the PDF. Offsets are Unicode-scalar positions in [`PageTextLayout::text`],
//! the same logical text the editing transactions verify against, so a
//! selection made here is applied to exactly the selected source glyphs.

use serde::Serialize;

use crate::error::{Result, WellfriendError};
use crate::{ContentEngine, PdfObject};

/// One painted glyph of the page's editable text.
#[derive(Debug, Clone, Serialize)]
pub struct EditorGlyph {
    /// Text this glyph contributes to [`PageTextLayout::text`].
    pub text: String,
    /// `[start, end)` scalar offsets in [`PageTextLayout::text`].
    pub range: [usize; 2],
    /// Corners in PDF user space: bottom-start, bottom-end, top-end,
    /// top-start (relative to the writing direction).
    pub quad: [[f64; 2]; 4],
    /// Axis-aligned bounds of `quad`: `[x0, y0, x1, y1]`.
    pub bbox: [f64; 4],
    /// Pen position before and after the glyph on its baseline.
    pub baseline: [[f64; 2]; 2],
    /// Page font resource name (`/F1`...).
    pub font_resource: String,
    /// Font size in user-space units.
    pub font_size: f64,
    pub vertical: bool,
    /// Rendering mode 3 (an invisible OCR layer).
    pub invisible: bool,
    /// Index into [`PageTextLayout::lines`].
    pub line: usize,
}

/// A run of glyphs sharing one baseline.
#[derive(Debug, Clone, Serialize)]
pub struct EditorLine {
    /// `[first, end)` indexes into [`PageTextLayout::glyphs`].
    pub glyphs: [usize; 2],
    /// `[start, end)` scalar offsets covered by the line.
    pub range: [usize; 2],
    pub bbox: [f64; 4],
    /// Start of the first glyph's baseline and end of the last's.
    pub baseline: [[f64; 2]; 2],
    /// Index into [`PageTextLayout::paragraphs`].
    pub paragraph: usize,
}

/// Text drawn by a Form XObject that the page content places with `Do`.
///
/// Its glyphs follow the page's own glyphs in [`PageTextLayout::glyphs`],
/// already in page space, and its text follows the page's own text (after a
/// line break) at [`Self::range`]. An edit inside the range rewrites the
/// form's content stream; a form placed more than once is copied first so
/// its other placements are unchanged.
#[derive(Debug, Clone, Serialize)]
pub struct EditorFormText {
    /// `[start, end)` offsets of the form's text in [`PageTextLayout::text`].
    pub range: [usize; 2],
    /// XObject resource name the page content invokes (`Fm1`).
    pub name: String,
    /// Index of the invoking `Do` among all `Do` operators of the page
    /// content, in content order.
    pub placement: usize,
    pub object: u32,
    pub generation: u16,
    /// Form space to page user space: the form's `/Matrix` times the CTM at
    /// the `Do`.
    pub matrix: [f64; 6],
    /// The form's text is the editing model's exact text (see
    /// [`PageTextLayout::editable`]).
    pub editable: bool,
}

/// Consecutive lines with one alignment edge and a regular line pitch.
#[derive(Debug, Clone, Serialize)]
pub struct EditorParagraph {
    /// `[first, end)` indexes into [`PageTextLayout::lines`].
    pub lines: [usize; 2],
    pub range: [usize; 2],
    pub bbox: [f64; 4],
}

/// A caret: an offset and the segment to draw for it, in PDF user space.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Caret {
    pub offset: usize,
    pub top: [f64; 2],
    pub bottom: [f64; 2],
    pub line: usize,
}

/// Caret movements for keyboard navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CaretMove {
    Left,
    Right,
    WordLeft,
    WordRight,
    LineStart,
    LineEnd,
    LineUp,
    LineDown,
    ParagraphStart,
    ParagraphEnd,
    TextStart,
    TextEnd,
}

impl CaretMove {
    /// Parse `left`, `right`, `word_left`, `word_right`, `line_start`,
    /// `line_end`, `line_up`, `line_down`, `paragraph_start`,
    /// `paragraph_end`, `text_start` or `text_end` (`-` also accepted).
    pub fn parse(name: &str) -> Option<Self> {
        Some(
            match name.trim().to_ascii_lowercase().replace('-', "_").as_str() {
                "left" => Self::Left,
                "right" => Self::Right,
                "word_left" => Self::WordLeft,
                "word_right" => Self::WordRight,
                "line_start" | "home" => Self::LineStart,
                "line_end" | "end" => Self::LineEnd,
                "line_up" | "up" => Self::LineUp,
                "line_down" | "down" => Self::LineDown,
                "paragraph_start" => Self::ParagraphStart,
                "paragraph_end" => Self::ParagraphEnd,
                "text_start" => Self::TextStart,
                "text_end" => Self::TextEnd,
                _ => return None,
            },
        )
    }
}

/// Text geometry of one page for building an editor UI.
#[derive(Debug, Clone, Serialize)]
pub struct PageTextLayout {
    pub page: usize,
    /// The page's logical text. Offsets index its Unicode scalars.
    pub text: String,
    /// `true` when [`Self::text`] is the editing model's exact text, so its
    /// offsets can be edited through [`TextEditSession`]. `false` when the
    /// editing model refuses the page (an ambiguous font encoding, say); the
    /// layout then still supports selection and hit-testing.
    pub editable: bool,
    pub glyphs: Vec<EditorGlyph>,
    pub lines: Vec<EditorLine>,
    pub paragraphs: Vec<EditorParagraph>,
    /// Crop box, display rotation and `/UserUnit`, for device mapping.
    pub page_box: [f64; 4],
    pub rotation: u32,
    pub user_unit: f64,
    /// Text inside Form XObjects placed by the page content, in placement
    /// order; it follows the page's own text.
    pub forms: Vec<EditorFormText>,
    #[serde(skip)]
    chars: Vec<char>,
    /// Scalars of the page content's own text (before any form text).
    #[serde(skip)]
    page_chars: usize,
}

impl PageTextLayout {
    /// Build the layout of `page` (1-based) of an opened document.
    pub fn build(engine: &ContentEngine, page: usize) -> Result<Self> {
        let pdf_page = engine.document().get_page(page)?;
        let source = crate::advanced_editing::editor_layout::glyph_layout(engine, &pdf_page)?;
        let editable = source.editable;
        let mut logical_text = source.logical_text;
        let page_chars = logical_text.chars().count();
        let mut glyphs: Vec<EditorGlyph> = source
            .glyphs
            .into_iter()
            .map(|glyph| editor_glyph(glyph, None, 0))
            .collect();
        // Text inside forms the page places follows the page's own text; a
        // form starts a new line and paragraph.
        let mut forms = Vec::new();
        let mut breaks = Vec::new();
        let mut length = page_chars;
        for placement in form_placements(engine, &pdf_page).unwrap_or_default() {
            let mut form_page = pdf_page.clone();
            form_page.contents = vec![(placement.object, placement.generation)];
            form_page.resources = placement.resources;
            let Ok(form) = crate::advanced_editing::editor_layout::glyph_layout(engine, &form_page) else {
                continue;
            };
            if form.glyphs.is_empty() {
                continue;
            }
            if length > 0 && !logical_text.ends_with(char::is_whitespace) {
                logical_text.push('\n');
                length += 1;
            }
            let start = length;
            length += form.logical_text.chars().count();
            logical_text.push_str(&form.logical_text);
            breaks.push(glyphs.len());
            glyphs.extend(
                form.glyphs
                    .into_iter()
                    .map(|glyph| editor_glyph(glyph, Some(&placement.matrix), start)),
            );
            forms.push(EditorFormText {
                range: [start, length],
                name: placement.name,
                placement: placement.placement,
                object: placement.object,
                generation: placement.generation,
                matrix: placement.matrix,
                editable: form.editable,
            });
        }
        let lines = group_lines(&mut glyphs, &breaks);
        let (lines, paragraphs) = group_paragraphs(lines, &glyphs, &breaks);
        let mut layout = Self {
            page,
            chars: logical_text.chars().collect(),
            text: logical_text,
            editable,
            glyphs,
            lines,
            paragraphs,
            page_box: engine.page_box(page)?,
            rotation: (pdf_page.rotate.rem_euclid(360)) as u32,
            user_unit: pdf_page.user_unit,
            forms,
            page_chars,
        };
        for (index, line) in layout.lines.iter().enumerate() {
            for glyph in &mut layout.glyphs[line.glyphs[0]..line.glyphs[1]] {
                glyph.line = index;
            }
        }
        Ok(layout)
    }

    /// Open `bytes` and build the layout of `page` (1-based).
    pub fn from_bytes(bytes: &[u8], page: usize) -> Result<Self> {
        let engine = ContentEngine::open_bytes(bytes.to_vec())?;
        Self::build(&engine, page)
    }

    /// Number of scalars in [`Self::text`] (the largest caret offset).
    pub fn len(&self) -> usize {
        self.chars.len()
    }

    /// Number of scalars of the page content's own text: offsets below it
    /// are page text, the rest belongs to [`Self::forms`].
    pub fn page_text_len(&self) -> usize {
        self.page_chars
    }

    /// The form whose text holds all of `[start, end)`.
    pub fn form_at(&self, [start, end]: [usize; 2]) -> Option<&EditorFormText> {
        self.forms
            .iter()
            .find(|form| form.range[0] <= start && end <= form.range[1])
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    /// The text of `[start, end)`.
    pub fn text_in(&self, range: [usize; 2]) -> String {
        let [start, end] = self.clamp_range(range);
        self.chars[start..end].iter().collect()
    }

    /// Index of the glyph whose quad contains `point` (PDF user space).
    pub fn glyph_at(&self, point: [f64; 2]) -> Option<usize> {
        self.glyphs
            .iter()
            .position(|glyph| quad_contains(&glyph.quad, point))
    }

    /// The caret offset for a click at `point` (PDF user space): the nearest
    /// line, then the glyph edge nearest along that line's baseline.
    pub fn hit_test(&self, point: [f64; 2]) -> Option<usize> {
        let line = self.nearest_line(point)?;
        Some(self.offset_on_line(line, point))
    }

    /// [`Self::hit_test`] for a point in rendered pixels at `dpi`.
    pub fn hit_test_device(&self, pixel: [f64; 2], dpi: u32) -> Option<usize> {
        self.hit_test(self.from_device(pixel, dpi)?)
    }

    /// Caret geometry for `offset`. At a line break the caret sits at the end
    /// of the earlier line; elsewhere at the start edge of the glyph that
    /// begins at `offset`.
    pub fn caret(&self, offset: usize) -> Option<Caret> {
        let offset = offset.min(self.len());
        if let Some((index, glyph)) = self.glyphs.iter().enumerate().find(|(_, glyph)| {
            glyph.range[0] <= offset && offset < glyph.range[1].max(glyph.range[0] + 1)
        }) {
            let line = &self.lines[glyph.line];
            let at_line_start = index == line.glyphs[0] && offset == glyph.range[0];
            if at_line_start && glyph.line > 0 {
                let previous = &self.lines[glyph.line - 1];
                if previous.range[1] == offset {
                    let last = &self.glyphs[previous.glyphs[1] - 1];
                    return Some(Caret {
                        offset,
                        bottom: last.quad[1],
                        top: last.quad[2],
                        line: glyph.line - 1,
                    });
                }
            }
            if offset == glyph.range[0] {
                return Some(Caret {
                    offset,
                    bottom: glyph.quad[0],
                    top: glyph.quad[3],
                    line: glyph.line,
                });
            }
            let fraction =
                (offset - glyph.range[0]) as f64 / (glyph.range[1] - glyph.range[0]).max(1) as f64;
            return Some(Caret {
                offset,
                bottom: lerp(glyph.quad[0], glyph.quad[1], fraction),
                top: lerp(glyph.quad[3], glyph.quad[2], fraction),
                line: glyph.line,
            });
        }
        // Past the last glyph that starts before `offset`: its end edge.
        self.glyphs
            .iter()
            .rev()
            .find(|glyph| glyph.range[1] <= offset)
            .map(|glyph| Caret {
                offset,
                bottom: glyph.quad[1],
                top: glyph.quad[2],
                line: glyph.line,
            })
    }

    /// One quad per line covering the selected glyphs of `[start, end)`.
    pub fn selection_quads(&self, range: [usize; 2]) -> Vec<[[f64; 2]; 4]> {
        let [start, end] = self.clamp_range(range);
        if start == end {
            return Vec::new();
        }
        let mut quads = Vec::new();
        for line in &self.lines {
            if line.range[1] <= start || line.range[0] >= end {
                continue;
            }
            let selected = self.glyphs[line.glyphs[0]..line.glyphs[1]]
                .iter()
                .filter(|glyph| glyph.range[0] < end && glyph.range[1] > start)
                .collect::<Vec<_>>();
            if let (Some(first), Some(last)) = (selected.first(), selected.last()) {
                quads.push([first.quad[0], last.quad[1], last.quad[2], first.quad[3]]);
            }
        }
        quads
    }

    /// The word around `offset`: glyphs joined without a space glyph or a
    /// visible gap on the same line.
    pub fn word_range(&self, offset: usize) -> [usize; 2] {
        let Some(index) = self.glyph_index_at(offset) else {
            return [offset.min(self.len()); 2];
        };
        if is_space_text(&self.glyphs[index].text) {
            return self.glyphs[index].range;
        }
        let mut first = index;
        while first > 0 && self.joined(first - 1, first) {
            first -= 1;
        }
        let mut last = index;
        while last + 1 < self.glyphs.len() && self.joined(last, last + 1) {
            last += 1;
        }
        [self.glyphs[first].range[0], self.glyphs[last].range[1]]
    }

    /// The line around `offset`.
    pub fn line_range(&self, offset: usize) -> [usize; 2] {
        self.line_index_at(offset)
            .map_or([offset.min(self.len()); 2], |line| self.lines[line].range)
    }

    /// The paragraph around `offset`.
    pub fn paragraph_range(&self, offset: usize) -> [usize; 2] {
        self.line_index_at(offset)
            .map_or([offset.min(self.len()); 2], |line| {
                self.paragraphs[self.lines[line].paragraph].range
            })
    }

    /// The caret offset after a keyboard movement from `offset`.
    pub fn move_caret(&self, offset: usize, movement: CaretMove) -> usize {
        let offset = offset.min(self.len());
        match movement {
            CaretMove::Left => offset.saturating_sub(1),
            CaretMove::Right => (offset + 1).min(self.len()),
            CaretMove::TextStart => 0,
            CaretMove::TextEnd => self.len(),
            CaretMove::LineStart => self.line_range(offset)[0],
            CaretMove::LineEnd => self.line_range(offset)[1],
            CaretMove::ParagraphStart => self.paragraph_range(offset)[0],
            CaretMove::ParagraphEnd => self.paragraph_range(offset)[1],
            CaretMove::WordLeft => {
                let mut position = offset;
                // Skip spaces, then to the start of the word before them.
                while position > 0 && self.is_space_at(position - 1) {
                    position -= 1;
                }
                if position == 0 {
                    return 0;
                }
                let word = self.word_range(position - 1);
                word[0].min(position - 1)
            }
            CaretMove::WordRight => {
                let mut position = offset;
                if position < self.len() && !self.is_space_at(position) {
                    position = self.word_range(position)[1].max(position + 1);
                }
                while position < self.len() && self.is_space_at(position) {
                    position += 1;
                }
                position.min(self.len())
            }
            CaretMove::LineUp | CaretMove::LineDown => {
                let Some(caret) = self.caret(offset) else {
                    return offset;
                };
                let target_line = if movement == CaretMove::LineUp {
                    caret.line.checked_sub(1)
                } else {
                    Some(caret.line + 1).filter(|line| *line < self.lines.len())
                };
                let Some(target_line) = target_line else {
                    return if movement == CaretMove::LineUp {
                        0
                    } else {
                        self.len()
                    };
                };
                let point = midpoint(caret.bottom, caret.top);
                self.offset_on_line(target_line, point)
            }
        }
    }

    /// The page-to-pixel transform `[a, b, c, d, e, f]` of a render at `dpi`
    /// (crop box, display rotation and `/UserUnit` applied, as rendered).
    pub fn device_transform(&self, dpi: u32) -> [f64; 6] {
        crate::Viewport::new_rotated_with_user_unit(
            self.page_box,
            dpi,
            self.rotation,
            self.user_unit,
        )
        .to_transform()
        .to_array()
    }

    /// A PDF user-space point in rendered pixels at `dpi`.
    pub fn to_device(&self, point: [f64; 2], dpi: u32) -> [f64; 2] {
        let [a, b, c, d, e, f] = self.device_transform(dpi);
        [
            a * point[0] + c * point[1] + e,
            b * point[0] + d * point[1] + f,
        ]
    }

    /// A rendered pixel position at `dpi` in PDF user space.
    pub fn from_device(&self, pixel: [f64; 2], dpi: u32) -> Option<[f64; 2]> {
        let [a, b, c, d, e, f] = self.device_transform(dpi);
        let determinant = a * d - b * c;
        if determinant.abs() < 1e-12 {
            return None;
        }
        let (x, y) = (pixel[0] - e, pixel[1] - f);
        Some([(d * x - c * y) / determinant, (a * y - b * x) / determinant])
    }

    fn clamp_range(&self, [start, end]: [usize; 2]) -> [usize; 2] {
        let end = end.min(self.len());
        [start.min(end), end]
    }

    fn glyph_index_at(&self, offset: usize) -> Option<usize> {
        self.glyphs
            .iter()
            .position(|glyph| glyph.range[0] <= offset && offset < glyph.range[1])
            .or_else(|| {
                self.glyphs
                    .iter()
                    .rposition(|glyph| glyph.range[1] <= offset && glyph.range[1] > glyph.range[0])
            })
    }

    fn line_index_at(&self, offset: usize) -> Option<usize> {
        self.lines
            .iter()
            .position(|line| line.range[0] <= offset && offset < line.range[1])
            .or_else(|| self.lines.iter().rposition(|line| line.range[1] <= offset))
            .or((!self.lines.is_empty()).then_some(0))
    }

    fn is_space_at(&self, offset: usize) -> bool {
        self.chars.get(offset).is_some_and(|ch| ch.is_whitespace())
    }

    /// Glyphs `a` and `b` (adjacent indexes) belong to one word.
    fn joined(&self, a: usize, b: usize) -> bool {
        let (left, right) = (&self.glyphs[a], &self.glyphs[b]);
        if left.line != right.line || is_space_text(&left.text) || is_space_text(&right.text) {
            return false;
        }
        let direction = unit(left.baseline[0], left.baseline[1])
            .or_else(|| unit(right.baseline[0], right.baseline[1]))
            .unwrap_or([1.0, 0.0]);
        let gap = dot(sub(right.baseline[0], left.baseline[1]), direction);
        gap < crate::text::reading_order::WORD_GAP_EM * left.font_size.max(right.font_size)
    }

    fn nearest_line(&self, point: [f64; 2]) -> Option<usize> {
        self.lines
            .iter()
            .enumerate()
            .map(|(index, line)| (index, rect_distance(&line.bbox, point)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(index, _)| index)
    }

    fn offset_on_line(&self, line: usize, point: [f64; 2]) -> usize {
        let line = &self.lines[line];
        let glyphs = &self.glyphs[line.glyphs[0]..line.glyphs[1]];
        let direction = unit(line.baseline[0], line.baseline[1]).unwrap_or([1.0, 0.0]);
        let along = dot(point, direction);
        let mut best = (f64::INFINITY, line.range[0]);
        for glyph in glyphs {
            for (edge, offset) in [
                (glyph.baseline[0], glyph.range[0]),
                (glyph.baseline[1], glyph.range[1]),
            ] {
                let distance = (dot(edge, direction) - along).abs();
                if distance < best.0 {
                    best = (distance, offset);
                }
            }
        }
        best.1
    }
}

fn editor_glyph(
    glyph: crate::advanced_editing::editor_layout::SourceGlyph,
    matrix: Option<&[f64; 6]>,
    offset: usize,
) -> EditorGlyph {
    let place = |point: [f64; 2]| match matrix {
        Some(m) => {
            let (x, y) = crate::content::state::transform_point(m, point[0], point[1]);
            [x, y]
        }
        None => point,
    };
    let quad = glyph.quad.map(place);
    let scale = matrix.map_or(1.0, |m| m[2].hypot(m[3]));
    EditorGlyph {
        bbox: quad_bounds(&quad),
        text: glyph.text,
        range: [glyph.logical_range[0] + offset, glyph.logical_range[1] + offset],
        quad,
        baseline: glyph.baseline.map(place),
        font_resource: glyph.font_resource,
        font_size: glyph.font_size * scale,
        vertical: glyph.vertical,
        invisible: glyph.invisible,
        line: 0,
    }
}

/// A Form XObject the page content places with `Do`.
struct FormPlacement {
    name: String,
    placement: usize,
    object: u32,
    generation: u16,
    matrix: [f64; 6],
    resources: crate::PdfDictionary,
}

/// The most form placements whose text a page layout includes.
const MAX_FORM_PLACEMENTS: usize = 64;

/// Forms placed directly by the page content (not inside other forms), with
/// the form-to-page matrix of each placement.
fn form_placements(engine: &ContentEngine, page: &crate::document::PdfPage) -> Result<Vec<FormPlacement>> {
    use crate::content::state::{concat_matrix, IDENTITY_MATRIX};
    let reader = engine.document().reader();
    let Some(xobjects) = page
        .resources
        .get("XObject")
        .and_then(|value| reader.resolve(value.clone()).ok())
        .and_then(|value| value.as_dict().cloned())
    else {
        return Ok(Vec::new());
    };
    let mut content = Vec::new();
    for (number, generation) in &page.contents {
        let stream = reader.get_object(*number, *generation)?;
        content.extend(crate::filters::decode_stream(&stream, reader)?);
        content.push(b'\n');
    }
    let mut out = Vec::new();
    let mut ctm = IDENTITY_MATRIX;
    let mut stack = Vec::new();
    let mut invocations = 0usize;
    for operation in crate::content::parser::ContentParser::parse(&content)? {
        match operation.operator.as_str() {
            "q" => stack.push(ctm),
            "Q" => ctm = stack.pop().unwrap_or(IDENTITY_MATRIX),
            "cm" => {
                let values: Vec<f64> = (0..6).filter_map(|i| operation.number(i)).collect();
                if values.len() == 6 {
                    ctm = concat_matrix(&[values[0], values[1], values[2], values[3], values[4], values[5]], &ctm);
                }
            }
            "Do" => {
                let placement = invocations;
                invocations += 1;
                let Some(name) = operation.name(0) else { continue };
                let Some((object, generation)) = xobjects.get(name).and_then(PdfObject::as_reference) else {
                    continue;
                };
                let Ok(stream) = reader.get_object(object, generation) else { continue };
                let Some((dict, _)) = stream.as_stream() else { continue };
                if dict.get_name("Subtype") != Some("Form") {
                    continue;
                }
                let form_matrix = dict
                    .get("Matrix")
                    .and_then(PdfObject::as_array)
                    .map(|items| items.iter().filter_map(PdfObject::as_number).collect::<Vec<_>>())
                    .filter(|items| items.len() == 6)
                    .map_or(IDENTITY_MATRIX, |m| [m[0], m[1], m[2], m[3], m[4], m[5]]);
                let resources = dict
                    .get("Resources")
                    .and_then(|value| reader.resolve(value.clone()).ok())
                    .and_then(|value| value.as_dict().cloned())
                    .unwrap_or_else(|| page.resources.clone());
                out.push(FormPlacement {
                    name: name.to_string(),
                    placement,
                    object,
                    generation,
                    matrix: concat_matrix(&form_matrix, &ctm),
                    resources,
                });
                if out.len() == MAX_FORM_PLACEMENTS {
                    break;
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

fn is_space_text(text: &str) -> bool {
    !text.is_empty() && text.chars().all(char::is_whitespace)
}

/// Split glyphs (in content order) into lines: a new line starts when the
/// baseline moves off the current line or jumps backwards along it.
fn group_lines(glyphs: &mut [EditorGlyph], forms: &[usize]) -> Vec<EditorLine> {
    let mut lines: Vec<EditorLine> = Vec::new();
    let mut start = 0usize;
    for index in 0..=glyphs.len() {
        let breaks = index == glyphs.len()
            || (index > start
                && (forms.contains(&index) || starts_new_line(&glyphs[index - 1], &glyphs[index])));
        if !breaks {
            continue;
        }
        if index > start {
            let members = &glyphs[start..index];
            let bbox = members
                .iter()
                .map(|glyph| glyph.bbox)
                .reduce(union_rect)
                .unwrap_or([0.0; 4]);
            lines.push(EditorLine {
                glyphs: [start, index],
                range: [members[0].range[0], members[members.len() - 1].range[1]],
                bbox,
                baseline: [
                    members[0].baseline[0],
                    members[members.len() - 1].baseline[1],
                ],
                paragraph: 0,
            });
        }
        start = index;
    }
    lines
}

fn starts_new_line(previous: &EditorGlyph, next: &EditorGlyph) -> bool {
    if previous.vertical != next.vertical {
        return true;
    }
    let Some(direction) = unit(previous.baseline[0], previous.baseline[1])
        .or_else(|| unit(next.baseline[0], next.baseline[1]))
    else {
        return true;
    };
    let normal = [-direction[1], direction[0]];
    let size = previous.font_size.max(next.font_size).max(1e-6);
    let step = sub(next.baseline[0], previous.baseline[1]);
    let across = dot(step, normal).abs();
    let along = dot(step, direction);
    // A gap wider than 2.5 em is a column or table-cell boundary, not a word
    // space (justified text stays well below it).
    across > 0.5 * size || along < -0.5 * size || along > 2.5 * size
}

/// The size and font resource covering most visible glyphs of a line.
fn dominant_style(line: &EditorLine, glyphs: &[EditorGlyph]) -> (f64, String) {
    let mut sizes: Vec<(f64, String, usize)> = Vec::new();
    for glyph in &glyphs[line.glyphs[0]..line.glyphs[1]] {
        if glyph.text.trim().is_empty() || glyph.invisible {
            continue;
        }
        match sizes.iter_mut().find(|(size, resource, _)| {
            (*size - glyph.font_size).abs() < 0.01 && *resource == glyph.font_resource
        }) {
            Some(entry) => entry.2 += 1,
            None => sizes.push((glyph.font_size, glyph.font_resource.clone(), 1)),
        }
    }
    sizes
        .into_iter()
        .max_by_key(|(_, _, count)| *count)
        .map(|(size, resource, _)| (size, resource))
        .unwrap_or((0.0, String::new()))
}

/// Whether two lines are set in different styles: a size change over 15%
/// (a heading above body text) or a different font at a different size.
fn style_break(previous: &EditorLine, current: &EditorLine, glyphs: &[EditorGlyph]) -> bool {
    let (a_size, a_font) = dominant_style(previous, glyphs);
    let (b_size, b_font) = dominant_style(current, glyphs);
    if a_size <= 0.0 || b_size <= 0.0 {
        return false;
    }
    let ratio = a_size.max(b_size) / a_size.min(b_size);
    ratio > 1.15 || (a_font != b_font && ratio > 1.05)
}

/// Group consecutive lines into paragraphs: same direction, a line pitch
/// under 1.7 em and no larger than 1.4x the paragraph's first pitch, a
/// shared start edge (or the following line starting at the paragraph's),
/// and the same text style (a heading never joins the body below it).
fn group_paragraphs(
    mut lines: Vec<EditorLine>,
    glyphs: &[EditorGlyph],
    forms: &[usize],
) -> (Vec<EditorLine>, Vec<EditorParagraph>) {
    let mut paragraphs: Vec<EditorParagraph> = Vec::new();
    let mut first_pitch: Option<f64> = None;
    for index in 0..lines.len() {
        let continues = index > 0 && !forms.contains(&lines[index].glyphs[0]) && {
            let previous = &lines[index - 1];
            let current = &lines[index];
            let size = glyphs[previous.glyphs[0]..previous.glyphs[1]]
                .iter()
                .map(|glyph| glyph.font_size)
                .fold(0.0_f64, f64::max)
                .max(1e-6);
            let direction = unit(previous.baseline[0], previous.baseline[1]).unwrap_or([1.0, 0.0]);
            let normal = [-direction[1], direction[0]];
            let pitch = -dot(sub(current.baseline[0], previous.baseline[0]), normal);
            let indent = dot(sub(current.baseline[0], previous.baseline[0]), direction).abs();
            let paragraph_start = &lines[paragraphs.last().map_or(0, |p| p.lines[0])];
            let paragraph_indent = dot(
                sub(current.baseline[0], paragraph_start.baseline[0]),
                direction,
            )
            .abs();
            pitch > 0.0
                && pitch < 1.7 * size
                && first_pitch.is_none_or(|first| pitch <= 1.4 * first)
                && (indent < 1.5 * size || paragraph_indent < 1.5 * size)
                && !style_break(previous, current, glyphs)
        };
        if continues {
            let previous_baseline = lines[index - 1].baseline[0];
            let direction = unit(lines[index - 1].baseline[0], lines[index - 1].baseline[1])
                .unwrap_or([1.0, 0.0]);
            let normal = [-direction[1], direction[0]];
            first_pitch.get_or_insert(-dot(
                sub(lines[index].baseline[0], previous_baseline),
                normal,
            ));
            let paragraph = paragraphs.last_mut().expect("continuing paragraph exists");
            paragraph.lines[1] = index + 1;
            paragraph.range[1] = lines[index].range[1];
            paragraph.bbox = union_rect(paragraph.bbox, lines[index].bbox);
        } else {
            first_pitch = None;
            paragraphs.push(EditorParagraph {
                lines: [index, index + 1],
                range: lines[index].range,
                bbox: lines[index].bbox,
            });
        }
        lines[index].paragraph = paragraphs.len() - 1;
    }
    (lines, paragraphs)
}

/// An edit session over one document: geometry for the editor, edits through
/// the engine's style-preserving reflow transaction, and undo/redo.
pub struct TextEditSession {
    history: Vec<Vec<u8>>,
    current: usize,
    layouts: std::collections::HashMap<usize, PageTextLayout>,
}

/// What an edit changed.
#[derive(Debug, Clone, Serialize)]
pub struct TextEditOutcome {
    pub page: usize,
    /// Caret offset after the inserted text, in the edited page's text.
    pub caret: usize,
    /// User-space rectangle whose pixels may have changed (`[x0, y0, x1,
    /// y1]`); repaint it, or the whole page when `None`.
    pub dirty_region: Option<[f64; 4]>,
    /// The transaction's own report.
    pub report: serde_json::Value,
}

impl TextEditSession {
    /// Start a session on a document's bytes.
    pub fn open(bytes: Vec<u8>) -> Result<Self> {
        ContentEngine::open_bytes(bytes.clone())?;
        Ok(Self {
            history: vec![bytes],
            current: 0,
            layouts: std::collections::HashMap::new(),
        })
    }

    /// The current document bytes (save these to keep the edits).
    pub fn bytes(&self) -> &[u8] {
        &self.history[self.current]
    }

    /// The layout of `page` (1-based) in the current revision.
    pub fn layout(&mut self, page: usize) -> Result<&PageTextLayout> {
        if !self.layouts.contains_key(&page) {
            let layout = PageTextLayout::from_bytes(self.bytes(), page)?;
            self.layouts.insert(page, layout);
        }
        Ok(&self.layouts[&page])
    }

    /// Replace `[start, end)` of the page's text with `text`.
    pub fn replace(
        &mut self,
        page: usize,
        range: [usize; 2],
        text: &str,
    ) -> Result<TextEditOutcome> {
        let before = self.layout(page)?.clone();
        if !before.editable {
            return Err(WellfriendError::UnsupportedFeature(
                "this page's text cannot be edited in place (its fonts do not decode to an exact logical text); selection and hit-testing still work".into(),
            ));
        }
        let [start, end] = before.clamp_range(range);
        if end > before.page_text_len() {
            return Err(WellfriendError::UnsupportedFeature(
                "text inside a form XObject is edited through the editor operation list, not a text edit session".into(),
            ));
        }
        // The reflow transaction rewrites a non-empty source selection. A
        // pure insertion or deletion is widened by one neighbouring scalar
        // that is written back unchanged.
        let (selection, replacement, caret) = if start < end && !text.is_empty() {
            ([start, end], text.to_string(), start + text.chars().count())
        } else if start > 0 {
            let kept = before.text_in([start - 1, start]);
            (
                [start - 1, end],
                format!("{kept}{text}"),
                start + text.chars().count(),
            )
        } else if end < before.len() {
            let kept = before.text_in([end, end + 1]);
            (
                [start, end + 1],
                format!("{text}{kept}"),
                start + text.chars().count(),
            )
        } else {
            return Err(WellfriendError::invalid_input(
                "text edit needs page text to anchor the change",
            ));
        };
        let source_text = before.text_in(selection);
        let request: crate::text_reflow::GeometricReflowRequest =
            serde_json::from_value(serde_json::json!({
                "page": page,
                "source_text": source_text,
                "replacement_text": replacement,
                "target_logical_scalar_range": selection,
            }))
            .map_err(|error| WellfriendError::invalid_input(error.to_string()))?;
        let (edited, report) = crate::text_reflow::apply_reflow_region(self.bytes(), &request)?;
        let after = PageTextLayout::from_bytes(&edited, page)?;
        let dirty_region = changed_region(&before, &after);
        self.history.truncate(self.current + 1);
        self.history.push(edited);
        self.current += 1;
        self.layouts.clear();
        self.layouts.insert(page, after);
        Ok(TextEditOutcome {
            page,
            caret,
            dirty_region,
            report: serde_json::to_value(&report).unwrap_or(serde_json::Value::Null),
        })
    }

    /// Insert `text` at `offset`.
    pub fn insert(&mut self, page: usize, offset: usize, text: &str) -> Result<TextEditOutcome> {
        self.replace(page, [offset, offset], text)
    }

    /// Delete `[start, end)`.
    pub fn delete(&mut self, page: usize, range: [usize; 2]) -> Result<TextEditOutcome> {
        self.replace(page, range, "")
    }

    pub fn can_undo(&self) -> bool {
        self.current > 0
    }

    pub fn can_redo(&self) -> bool {
        self.current + 1 < self.history.len()
    }

    /// Step back one edit. Returns `false` when there is nothing to undo.
    pub fn undo(&mut self) -> bool {
        if !self.can_undo() {
            return false;
        }
        self.current -= 1;
        self.layouts.clear();
        true
    }

    /// Re-apply the last undone edit. Returns `false` when there is none.
    pub fn redo(&mut self) -> bool {
        if !self.can_redo() {
            return false;
        }
        self.current += 1;
        self.layouts.clear();
        true
    }
}

/// Bounds of the glyphs that differ between two layouts of one page,
/// comparing from both ends so unchanged text before and after is skipped.
fn changed_region(before: &PageTextLayout, after: &PageTextLayout) -> Option<[f64; 4]> {
    let same = |a: &EditorGlyph, b: &EditorGlyph| {
        a.text == b.text
            && a.quad
                .iter()
                .flatten()
                .zip(b.quad.iter().flatten())
                .all(|(x, y)| (x - y).abs() < 1e-6)
    };
    let prefix = before
        .glyphs
        .iter()
        .zip(&after.glyphs)
        .take_while(|(a, b)| same(a, b))
        .count();
    let suffix = before.glyphs[prefix..]
        .iter()
        .rev()
        .zip(after.glyphs[prefix..].iter().rev())
        .take_while(|(a, b)| same(a, b))
        .count();
    before.glyphs[prefix..before.glyphs.len() - suffix]
        .iter()
        .chain(&after.glyphs[prefix..after.glyphs.len() - suffix])
        .map(|glyph| glyph.bbox)
        .reduce(union_rect)
}

fn quad_bounds(quad: &[[f64; 2]; 4]) -> [f64; 4] {
    let xs = quad.map(|point| point[0]);
    let ys = quad.map(|point| point[1]);
    [
        xs.iter().copied().fold(f64::INFINITY, f64::min),
        ys.iter().copied().fold(f64::INFINITY, f64::min),
        xs.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        ys.iter().copied().fold(f64::NEG_INFINITY, f64::max),
    ]
}

fn union_rect(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

fn rect_distance(rect: &[f64; 4], point: [f64; 2]) -> f64 {
    let dx = (rect[0] - point[0]).max(point[0] - rect[2]).max(0.0);
    let dy = (rect[1] - point[1]).max(point[1] - rect[3]).max(0.0);
    // Vertical distance dominates so a click beside a line still picks it.
    dx * 0.25 + dy
}

/// Point inside a convex quad (either winding).
fn quad_contains(quad: &[[f64; 2]; 4], point: [f64; 2]) -> bool {
    let mut sign = 0.0_f64;
    for index in 0..4 {
        let a = quad[index];
        let b = quad[(index + 1) % 4];
        let cross = (b[0] - a[0]) * (point[1] - a[1]) - (b[1] - a[1]) * (point[0] - a[0]);
        if cross.abs() < 1e-12 {
            continue;
        }
        if sign == 0.0 {
            sign = cross.signum();
        } else if cross.signum() != sign {
            return false;
        }
    }
    true
}

fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn dot(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

fn midpoint(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]
}

fn lerp(a: [f64; 2], b: [f64; 2], t: f64) -> [f64; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

fn unit(from: [f64; 2], to: [f64; 2]) -> Option<[f64; 2]> {
    let delta = sub(to, from);
    let length = delta[0].hypot(delta[1]);
    (length > 1e-9).then(|| [delta[0] / length, delta[1] / length])
}

#[cfg(test)]
#[path = "text_editor_tests.rs"]
mod tests;
