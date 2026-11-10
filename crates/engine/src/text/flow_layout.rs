//! Poppler-style text layout: glyphs → words → lines → blocks → flows, read
//! in the order `pdftotext` uses by default.
//!
//! This is an independent implementation of the algorithm Poppler's
//! `TextOutputDev` uses (same rules and thresholds, no shared code), working
//! on the per-glyph geometry the collector records
//! ([`TextChunk::glyph_run`]):
//!
//! * **Characters.** Each glyph contributes its own origin and advance with
//!   character/word spacing removed. Glyphs off the page or with non-finite
//!   geometry are dropped; whitespace ends the current word; a glyph mapping to
//!   several characters (a ligature) is split into equal-width characters; a
//!   spacing accent centred on the previous letter (or a letter centred on the
//!   previous accent) is merged into one accented character.
//! * **Words.** A new word starts on a gap above 0.1 × font size, an overlap of
//!   more than 0.2 × font size, an exact over-print of the previous glyph
//!   (fake bold), a baseline change above 0.5 pt, or a change of font size,
//!   writing mode or rotation.
//! * **Rotation pools.** Every word gets one of four rotations (0°, 90°, 180°,
//!   270°, relative to the displayed page) from its text matrix; text that is
//!   noticeably off-axis is flagged as diagonal (and can be discarded). Each
//!   rotation is laid out separately in its own reading frame.
//! * **Blocks.** Blocks grow from a seed word by adding words on adjacent lines
//!   that overlap them, words on the same lines within the column gap, and a
//!   few outlying words. Inside a block, duplicated words (fake bold, drop
//!   shadows) are removed, lines are built and the inter-word space threshold
//!   is chosen per line.
//! * **Reading order.** Blocks are ordered by a topological sort following
//!   T. Breuel (2003): a block comes first when it overlaps another
//!   horizontally and lies above it (rule 1), or lies entirely to its left (to
//!   its right on a right-to-left page) with no block in between (rule 2).
//!   Cells of a detected table are read row by row. Consecutive blocks are
//!   joined into flows (stories) when the next one sits below the previous one
//!   in its column with a font no larger.
//! * **Output.** Flows, blocks, lines; an end-of-line hyphen before a
//!   lower-case continuation is removed and the halves joined; right-to-left
//!   lines are put in logical order with the Unicode bidirectional algorithm;
//!   flows are separated by a blank line.
//!
//! Coordinates follow Poppler's device space: the displayed page (page box
//! with `/Rotate` applied), origin at the top left, y downwards, in points.
//! Each rotation pool is processed in a *canonical frame* in which its text
//! reads left to right and lines advance downwards, so the algorithms only
//! need the 0° case.

use std::collections::HashMap;

use unicode_normalization::UnicodeNormalization;

use super::collector::{is_rtl_char, TextChunk};

const POOL_STEP: f64 = 4.0;
const MIN_WORD_BREAK_SPACE: f64 = 0.1;
const MIN_DUP_BREAK_OVERLAP: f64 = 0.2;
const MAX_LINE_SPACING_DELTA: f64 = 1.5;
const MAX_BLOCK_FONT_SIZE_DELTA1: f64 = 0.05;
const MAX_BLOCK_FONT_SIZE_DELTA2: f64 = 0.6;
const MAX_BLOCK_FONT_SIZE_DELTA3: f64 = 0.2;
const MAX_WORD_FONT_SIZE_DELTA: f64 = 0.05;
const MAX_INTRA_LINE_DELTA: f64 = 0.5;
const MAX_WORD_SPACING: f64 = 1.5;
const MIN_COL_SPACING1: f64 = 0.7;
const MIN_COL_SPACING2: f64 = 1.0;
const MAX_BLOCK_SPACING: f64 = 2.5;
const MIN_CHAR_SPACING: f64 = -0.5;
const MAX_CHAR_SPACING: f64 = 0.03;
const MAX_WIDE_CHAR_SPACING_MUL: f64 = 1.3;
const MAX_WIDE_CHAR_SPACING: f64 = 0.4;
const DUP_MAX_PRI_DELTA: f64 = 0.1;
const DUP_MAX_SEC_DELTA: f64 = 0.2;
const COMB_MAX_MID_DELTA: f64 = 0.3;
const COMB_MAX_BASE_DELTA: f64 = 0.4;
const DIAGONAL_THRESHOLD: f64 = 0.1;
/// Baseline change, in points, that always starts a new word.
const MAX_WORD_BASE_DELTA: f64 = 0.5;
/// After this many glyphs smaller than 3 × 3 pt, further tiny glyphs are
/// ignored (protection against noise layers).
const MAX_TINY_CHARS: usize = 50_000;
/// Above this many blocks the rule-2 "no block in between" check (two n × n
/// bit matrices) is skipped so pathological pages stay bounded.
const MAX_BLOCKS_FOR_INTERVENING_CHECK: usize = 6_000;
/// Stream positions given to glyphs synthesised for chunks without recorded
/// geometry; they never claim adjacency with real glyphs.
const SYNTHETIC_STREAM_POS: u32 = 0x8000_0000;

/// Options of [`analyze_flows`].
#[derive(Debug, Clone, PartialEq)]
pub struct FlowLayoutOptions {
    /// Drop text whose baseline is noticeably diagonal (`pdftotext -nodiag`).
    pub discard_diagonal: bool,
    /// Join a word broken by a hyphen at the end of a line with its
    /// continuation on the next line (`pdftotext` does this by default).
    pub dehyphenate: bool,
    /// Merge spacing accents with the letter they are centred on.
    pub merge_combining: bool,
    /// Remove over-printed duplicate words (fake bold, drop shadows).
    pub remove_duplicates: bool,
}

impl Default for FlowLayoutOptions {
    fn default() -> Self {
        Self {
            discard_diagonal: false,
            dehyphenate: true,
            merge_combining: true,
            remove_duplicates: true,
        }
    }
}

/// Mapping from PDF user space to the displayed page (Poppler's device
/// space at 72 dpi): origin top left, y down, `/Rotate` applied.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageFrame {
    /// `X = a·x + c·y + e`, `Y = b·x + d·y + f` as `[a, b, c, d, e, f]`.
    pub matrix: [f64; 6],
    pub width: f64,
    pub height: f64,
}

impl PageFrame {
    /// The frame of a page box `[x0, y0, x1, y1]` shown with a `/Rotate` of
    /// `rotate` degrees (clockwise, a multiple of 90).
    pub fn new(page_box: [f64; 4], rotate: i32) -> Self {
        let x0 = page_box[0].min(page_box[2]);
        let x1 = page_box[0].max(page_box[2]);
        let y0 = page_box[1].min(page_box[3]);
        let y1 = page_box[1].max(page_box[3]);
        let (w, h) = (x1 - x0, y1 - y0);
        match rotate.rem_euclid(360) {
            90 => Self {
                matrix: [0.0, 1.0, 1.0, 0.0, -y0, -x0],
                width: h,
                height: w,
            },
            180 => Self {
                matrix: [-1.0, 0.0, 0.0, 1.0, x1, -y0],
                width: w,
                height: h,
            },
            270 => Self {
                matrix: [0.0, -1.0, -1.0, 0.0, y1, x1],
                width: h,
                height: w,
            },
            _ => Self {
                matrix: [1.0, 0.0, 0.0, -1.0, -x0, y1],
                width: w,
                height: h,
            },
        }
    }

    fn point(&self, x: f64, y: f64) -> (f64, f64) {
        let m = &self.matrix;
        (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
    }

    fn delta(&self, dx: f64, dy: f64) -> (f64, f64) {
        let m = &self.matrix;
        (m[0] * dx + m[2] * dy, m[1] * dx + m[3] * dy)
    }

    /// Device point back to PDF user space.
    fn user_point(&self, x: f64, y: f64) -> (f64, f64) {
        let m = &self.matrix;
        let det = m[0] * m[3] - m[1] * m[2];
        if det == 0.0 || !det.is_finite() {
            return (x, y);
        }
        let (tx, ty) = (x - m[4], y - m[5]);
        (
            (m[3] * tx - m[2] * ty) / det,
            (-m[1] * tx + m[0] * ty) / det,
        )
    }
}

/// A word of the laid-out page.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowWord {
    pub text: String,
    /// `[x0, y0, x1, y1]` in PDF user space (y up).
    pub bbox: [f64; 4],
    pub font_size: f64,
    /// Rotation relative to the displayed page: 0, 90, 180 or 270 degrees
    /// clockwise.
    pub rotation: u16,
    /// The word's baseline is noticeably off-axis.
    pub diagonal: bool,
    /// A space follows this word on its line.
    pub space_after: bool,
}

/// A line of a block: words on one baseline in reading order.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowLine {
    pub words: Vec<FlowWord>,
    /// The line's text in logical order (spaces between words included, the
    /// end-of-line hyphen kept).
    pub text: String,
    /// The line ends with a hyphen.
    pub hyphenated: bool,
    pub bbox: [f64; 4],
}

/// A block of lines (a paragraph, a table cell, a caption).
#[derive(Debug, Clone, PartialEq)]
pub struct FlowBlock {
    pub lines: Vec<FlowLine>,
    pub bbox: [f64; 4],
    pub rotation: u16,
    /// Index of the table this block was recognised as a cell of.
    pub table: Option<usize>,
}

/// A flow: consecutive blocks of one story or column, in reading order.
#[derive(Debug, Clone, PartialEq)]
pub struct TextFlow {
    pub blocks: Vec<FlowBlock>,
}

/// The result of [`analyze_flows`]: flows in reading order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FlowPage {
    pub flows: Vec<TextFlow>,
    /// Rotation holding most characters (0, 90, 180 or 270).
    pub primary_rotation: u16,
    /// Left-to-right characters outnumber right-to-left ones.
    pub primary_lr: bool,
    /// Number of words dropped as over-printed duplicates.
    pub duplicates_removed: usize,
    /// Number of glyphs dropped as diagonal (only with `discard_diagonal`).
    pub diagonal_discarded: usize,
}

impl FlowPage {
    /// The page text as `pdftotext` writes it by default: lines of each block
    /// on their own line, a hyphenated line joined with its continuation, and
    /// a blank line after each flow (trailing blank lines trimmed).
    pub fn text(&self) -> String {
        self.text_with(true)
    }

    /// [`text`](Self::text) without joining hyphenated lines.
    pub fn text_with(&self, dehyphenate: bool) -> String {
        let mut out = String::new();
        for flow in &self.flows {
            let lines: Vec<&FlowLine> = flow.blocks.iter().flat_map(|b| b.lines.iter()).collect();
            for (index, line) in lines.iter().enumerate() {
                let next = lines.get(index + 1);
                if dehyphenate && line.hyphenated && next.is_some_and(|n| joins_hyphen(line, n)) {
                    let mut text = line.text.clone();
                    text.pop();
                    out.push_str(&text);
                } else {
                    out.push_str(&line.text);
                    out.push('\n');
                }
            }
            out.push('\n');
        }
        while out.ends_with('\n') {
            out.pop();
        }
        out
    }
}

/// A hyphen ending `line` is a word break when a letter precedes it and the
/// next line starts with a lower-case letter. (Poppler joins every hyphenated
/// line; keeping "anti-" + "American" or "Jean-" + "Paul" apart avoids
/// gluing compounds that really are hyphenated.)
fn joins_hyphen(line: &FlowLine, next: &FlowLine) -> bool {
    let mut chars = line.text.chars().rev();
    if !chars.next().is_some_and(is_hyphen) {
        return false;
    }
    let before = chars.next();
    let first = next.text.chars().next();
    before.is_some_and(char::is_alphabetic) && first.is_some_and(char::is_lowercase)
}

fn is_hyphen(ch: char) -> bool {
    matches!(ch, '-' | '\u{00AD}' | '\u{2010}')
}

/// Lay a page's text out into flows, blocks, lines and words, in reading
/// order. `frame` is the displayed page; without it the page is assumed
/// upright and nothing is dropped as off-page.
pub fn analyze_flows(
    chunks: &[TextChunk],
    frame: Option<&PageFrame>,
    options: &FlowLayoutOptions,
) -> FlowPage {
    let mut builder = WordBuilder::new(frame, options);
    for chunk in chunks {
        builder.add_chunk(chunk);
    }
    builder.end_word();
    let WordBuilder {
        mut words,
        pools,
        frame,
        diagonal_discarded,
        ..
    } = builder;

    let mut blocks: Vec<Block> = Vec::new();
    let mut counts = [0usize; 4];
    let mut duplicates_removed = 0usize;
    for (rot, pool_words) in pools.iter().enumerate() {
        let built = build_blocks(&mut words, pool_words, rot as u8, options);
        for block in built {
            counts[rot] += block.char_count;
            duplicates_removed += block.duplicates;
            blocks.push(block);
        }
    }
    let mut primary_rot = 0usize;
    for rot in 1..4 {
        if counts[rot] > counts[primary_rot] {
            primary_rot = rot;
        }
    }
    let mut lr_count: i64 = 0;
    for block in &blocks {
        for line in &block.lines {
            for ch in line.chars.iter() {
                if is_rtl_char(*ch) {
                    lr_count -= 1;
                } else if ch.is_alphabetic() {
                    lr_count += 1;
                }
            }
        }
    }
    let primary_lr = lr_count >= 0;

    let (order, page_blocks) = reading_order(&blocks, primary_rot as u8, primary_lr, &frame);
    for (block, page_block) in blocks.iter_mut().zip(&page_blocks) {
        block.table = page_block.table;
    }
    let flows = build_flows(&blocks, &page_blocks, &order);

    let user_bbox = |rot: u8, r: Rect| -> [f64; 4] {
        let d = r.to_device(rot);
        let (ax, ay) = frame.user_point(d.p0, d.s0);
        let (bx, by) = frame.user_point(d.p1, d.s1);
        [ax.min(bx), ay.min(by), ax.max(bx), ay.max(by)]
    };
    let mut out_flows = Vec::with_capacity(flows.len());
    for flow in flows {
        let mut out_blocks = Vec::with_capacity(flow.len());
        for index in flow {
            let block = &blocks[index];
            let lines = block
                .lines
                .iter()
                .map(|line| FlowLine {
                    words: line
                        .words
                        .iter()
                        .map(|word| FlowWord {
                            text: word.text.clone(),
                            bbox: user_bbox(block.rot, word.rect),
                            font_size: word.font_size,
                            rotation: u16::from(block.rot) * 90,
                            diagonal: word.diagonal,
                            space_after: word.space_after,
                        })
                        .collect(),
                    text: line.text.clone(),
                    hyphenated: line.hyphenated,
                    bbox: user_bbox(block.rot, line.rect),
                })
                .collect();
            out_blocks.push(FlowBlock {
                lines,
                bbox: user_bbox(block.rot, block.rect),
                rotation: u16::from(block.rot) * 90,
                table: block.table,
            });
        }
        out_flows.push(TextFlow { blocks: out_blocks });
    }
    FlowPage {
        flows: out_flows,
        primary_rotation: primary_rot as u16 * 90,
        primary_lr,
        duplicates_removed,
        diagonal_discarded,
    }
}

// ---------------------------------------------------------------------------
// Geometry helpers

/// A box in a canonical frame: `p` along the reading direction, `s` across
/// lines (downwards).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Rect {
    p0: f64,
    s0: f64,
    p1: f64,
    s1: f64,
}

impl Rect {
    fn empty() -> Self {
        Self {
            p0: 0.0,
            s0: 0.0,
            p1: -1.0,
            s1: -1.0,
        }
    }

    fn is_empty(&self) -> bool {
        self.p0 > self.p1
    }

    fn union(&mut self, other: &Rect) {
        if self.is_empty() {
            *self = *other;
            return;
        }
        self.p0 = self.p0.min(other.p0);
        self.s0 = self.s0.min(other.s0);
        self.p1 = self.p1.max(other.p1);
        self.s1 = self.s1.max(other.s1);
    }

    /// This canonical-frame box of rotation `rot` in device coordinates
    /// (`p` = x, `s` = y).
    fn to_device(self, rot: u8) -> Rect {
        let (ax, ay) = from_canon(rot, self.p0, self.s0);
        let (bx, by) = from_canon(rot, self.p1, self.s1);
        Rect {
            p0: ax.min(bx),
            s0: ay.min(by),
            p1: ax.max(bx),
            s1: ay.max(by),
        }
    }

    /// A device box in the canonical frame of rotation `rot`.
    fn device_to_canon(self, rot: u8) -> Rect {
        let (ap, as_) = to_canon(rot, self.p0, self.s0);
        let (bp, bs) = to_canon(rot, self.p1, self.s1);
        Rect {
            p0: ap.min(bp),
            s0: as_.min(bs),
            p1: ap.max(bp),
            s1: as_.max(bs),
        }
    }
}

/// Device → canonical frame of rotation `rot`.
fn to_canon(rot: u8, x: f64, y: f64) -> (f64, f64) {
    match rot {
        0 => (x, y),
        1 => (y, -x),
        2 => (-x, -y),
        _ => (-y, x),
    }
}

/// Canonical frame of rotation `rot` → device.
fn from_canon(rot: u8, p: f64, s: f64) -> (f64, f64) {
    match rot {
        0 => (p, s),
        1 => (-s, p),
        2 => (-p, -s),
        _ => (s, -p),
    }
}

// ---------------------------------------------------------------------------
// Characters and words

#[derive(Debug, Clone, Copy)]
struct Char {
    ch: char,
    /// Leading edge along the reading direction.
    edge: f64,
    font: u32,
    pos: u32,
}

#[derive(Debug, Clone)]
struct Word {
    rot: u8,
    diagonal: bool,
    vertical: bool,
    chars: Vec<Char>,
    rect: Rect,
    base: f64,
    font_size: f64,
    edge_end: f64,
    pos_end: u32,
}

impl Word {
    fn new(rot: u8, diagonal: bool, vertical: bool, font_size: f64) -> Self {
        Self {
            rot,
            diagonal,
            vertical,
            chars: Vec::new(),
            rect: Rect::empty(),
            base: 0.0,
            font_size,
            edge_end: 0.0,
            pos_end: 0,
        }
    }

    fn set_initial_bounds(&mut self, p: f64, s: f64, ascent: f64, descent: f64) {
        let fs = self.font_size;
        if self.vertical {
            self.rect = Rect {
                p0: p,
                s0: s - fs / 2.0,
                p1: p,
                s1: s + fs / 2.0,
            };
        } else {
            let mut s0 = s - ascent * fs;
            let mut s1 = s - descent * fs;
            if s0 >= s1 {
                s0 = s;
                s1 = s + 1.0;
            }
            self.rect = Rect {
                p0: p,
                s0,
                p1: p,
                s1,
            };
        }
        self.base = s;
    }

    #[allow(clippy::too_many_arguments)]
    fn add_char(
        &mut self,
        font: u32,
        p: f64,
        s: f64,
        dp: f64,
        pos: u32,
        len: u32,
        ch: char,
        ascent: f64,
        descent: f64,
    ) {
        self.chars.push(Char {
            ch,
            edge: p,
            font,
            pos,
        });
        self.pos_end = pos.saturating_add(len);
        if self.chars.len() == 1 {
            self.set_initial_bounds(p, s, ascent, descent);
        }
        self.edge_end = p + dp;
        self.rect.p1 = self.edge_end;
    }

    /// Merge a spacing accent with the letter it is centred on (either order
    /// in the content stream). Returns false when `ch` is not merged.
    #[allow(clippy::too_many_arguments)]
    fn add_combining(
        &mut self,
        font: u32,
        font_size: f64,
        p: f64,
        s: f64,
        dp: f64,
        pos: u32,
        len: u32,
        ch: char,
        ascent: f64,
        descent: f64,
    ) -> bool {
        let Some(last) = self.chars.last().copied() else {
            return false;
        };
        if self.vertical {
            return false;
        }
        if let Some(combining) = combining_mark(ch) {
            if !last.ch.is_alphanumeric() {
                return false;
            }
            let max_mid = (self.edge_end - last.edge).abs() * COMB_MAX_MID_DELTA;
            let max_base = (self.rect.s1 - self.rect.s0) * COMB_MAX_BASE_DELTA;
            let char_mid = p + dp / 2.0;
            let edge_mid = (last.edge + self.edge_end) / 2.0;
            if (char_mid - edge_mid).abs() >= max_mid || (s - self.base).abs() >= max_base {
                return false;
            }
            // Keep the base letter's box: an accent's own placement is often odd.
            self.chars.push(Char {
                ch: combining,
                edge: edge_mid,
                font,
                pos,
            });
            self.pos_end = pos.saturating_add(len);
            return true;
        }
        if let Some(combining) = combining_mark(last.ch) {
            if !ch.is_alphanumeric() {
                return false;
            }
            let max_base = (ascent - descent) * font_size * COMB_MAX_BASE_DELTA;
            let max_mid = (dp * COMB_MAX_MID_DELTA).abs();
            let char_mid = p + dp / 2.0;
            let edge_mid = (last.edge + self.edge_end) / 2.0;
            if (char_mid - edge_mid).abs() >= max_mid || (s - self.base).abs() >= max_base {
                return false;
            }
            self.font_size = font_size;
            // The accent moves after its base letter.
            let n = self.chars.len();
            self.chars[n - 1] = Char {
                ch,
                edge: p,
                font,
                pos: last.pos,
            };
            self.chars.push(Char {
                ch: combining,
                edge: edge_mid,
                font: last.font,
                pos,
            });
            self.pos_end = pos.saturating_add(len);
            if self.chars.len() == 2 {
                self.set_initial_bounds(p, s, ascent, descent);
            }
            self.edge_end = p + dp;
            self.rect.p1 = self.edge_end;
            let n = self.chars.len();
            self.chars[n - 1].edge = (self.edge_end + self.chars[n - 2].edge) / 2.0;
            return true;
        }
        false
    }

    fn merge(&mut self, other: Word) {
        self.rect.union(&other.rect);
        self.chars.extend(other.chars);
        self.edge_end = other.edge_end;
        self.pos_end = other.pos_end;
    }

    fn len(&self) -> usize {
        self.chars.len()
    }
}

/// The combining mark a spacing accent stands for.
fn combining_mark(ch: char) -> Option<char> {
    Some(match ch {
        '\u{0060}' => '\u{0300}', // grave
        '\u{00A8}' => '\u{0308}', // dieresis
        '\u{00AF}' => '\u{0304}', // macron
        '\u{00B4}' => '\u{0301}', // acute
        '\u{00B8}' => '\u{0327}', // cedilla
        '\u{02C6}' => '\u{0302}', // circumflex
        '\u{02C7}' => '\u{030C}', // caron
        '\u{02D8}' => '\u{0306}', // breve
        '\u{02D9}' => '\u{0307}', // dot above
        '\u{02DA}' => '\u{030A}', // ring
        '\u{02DC}' => '\u{0303}', // tilde
        '\u{02DD}' => '\u{030B}', // double acute
        _ => return None,
    })
}

/// Text of a run of characters: dotless i/j carrying an accent above become
/// i/j (the TeX convention for accented i), then NFC.
fn compose_text(chars: impl Iterator<Item = char>) -> String {
    let raw: Vec<char> = chars.collect();
    let mut fixed = String::with_capacity(raw.len());
    for (index, &ch) in raw.iter().enumerate() {
        let accent_above = raw
            .get(index + 1)
            .is_some_and(|next| matches!(*next, '\u{0300}'..='\u{0315}'));
        match ch {
            '\u{0131}' if accent_above => fixed.push('i'),
            '\u{0237}' if accent_above => fixed.push('j'),
            _ => fixed.push(ch),
        }
    }
    if fixed.is_ascii() {
        fixed
    } else {
        fixed.nfc().collect()
    }
}

struct WordBuilder<'o> {
    options: &'o FlowLayoutOptions,
    frame: PageFrame,
    clip: bool,
    words: Vec<Word>,
    pools: [Vec<usize>; 4],
    current: Option<Word>,
    last_char_overlap: bool,
    tiny_chars: usize,
    fonts: HashMap<String, u32>,
    synthetic_pos: u32,
    diagonal_discarded: usize,
}

impl<'o> WordBuilder<'o> {
    fn new(frame: Option<&PageFrame>, options: &'o FlowLayoutOptions) -> Self {
        Self {
            options,
            frame: frame.copied().unwrap_or(PageFrame {
                matrix: [1.0, 0.0, 0.0, -1.0, 0.0, 0.0],
                width: f64::INFINITY,
                height: f64::INFINITY,
            }),
            clip: frame.is_some(),
            words: Vec::new(),
            pools: Default::default(),
            current: None,
            last_char_overlap: false,
            tiny_chars: 0,
            fonts: HashMap::new(),
            synthetic_pos: SYNTHETIC_STREAM_POS,
            diagonal_discarded: 0,
        }
    }

    fn end_word(&mut self) {
        if let Some(word) = self.current.take() {
            if word.len() > 0 && word.base.is_finite() {
                let id = self.words.len();
                self.pools[usize::from(word.rot)].push(id);
                self.words.push(word);
            }
        }
    }

    fn begin_word(&mut self, rot: u8, diagonal: bool, vertical: bool, font_size: f64) {
        self.current = Some(Word::new(rot, diagonal, vertical, font_size));
    }

    fn font_id(&mut self, name: &str) -> u32 {
        let next = self.fonts.len() as u32;
        *self.fonts.entry(name.to_string()).or_insert(next)
    }

    fn add_chunk(&mut self, chunk: &TextChunk) {
        if chunk.text.is_empty() {
            return;
        }
        let font_size = if chunk.font_size.is_finite() {
            chunk.font_size.abs()
        } else {
            return;
        };
        let font = self.font_id(&chunk.font_name);
        let vertical = chunk.is_vertical;
        let synthetic;
        let (font_matrix, ascent, descent, glyphs) = match chunk.glyph_run.as_deref() {
            Some(run) if !run.glyphs.is_empty() => {
                (run.font_matrix, run.ascent, run.descent, &run.glyphs[..])
            }
            _ => {
                synthetic = self.synthesize_glyphs(chunk);
                (
                    [font_size, 0.0, 0.0, font_size],
                    0.95,
                    -0.35,
                    &synthetic[..],
                )
            }
        };

        // Rotation and diagonality from the font matrix on the displayed page.
        let (m0, m1) = self.frame.delta(font_matrix[0], font_matrix[1]);
        let (m2, m3) = self.frame.delta(font_matrix[2], font_matrix[3]);
        let mut rot: u8 = if (m0 * m3).abs() > (m1 * m2).abs() {
            if m0 > 0.0 || m3 < 0.0 {
                0
            } else {
                2
            }
        } else if m2 > 0.0 {
            1
        } else {
            3
        };
        let diagonal = if m0.abs() >= m1.abs() {
            m1.abs() > DIAGONAL_THRESHOLD * m0.abs()
        } else {
            m0.abs() > DIAGONAL_THRESHOLD * m1.abs()
        };
        if vertical {
            // Vertical writing: lines are effectively rotated by 90 degrees.
            rot = (rot + 1) & 3;
        }

        for glyph in glyphs {
            let text = chunk
                .text
                .get(glyph.text_start as usize..glyph.text_end as usize)
                .unwrap_or("");
            self.add_glyph(
                glyph, text, font, font_size, rot, diagonal, vertical, ascent, descent,
            );
        }
    }

    /// Equal-width glyphs for a chunk without recorded geometry.
    fn synthesize_glyphs(&mut self, chunk: &TextChunk) -> Vec<super::collector::TextGlyph> {
        let count = chunk.text.chars().count().max(1) as f64;
        let width = if chunk.width.is_finite() {
            chunk.width.max(0.0)
        } else {
            0.0
        };
        let (ux, uy) = if chunk.is_vertical {
            (0.0, -1.0)
        } else {
            (1.0, 0.0)
        };
        let step = width / count;
        let mut out = Vec::with_capacity(count as usize);
        for (index, (offset, ch)) in chunk.text.char_indices().enumerate() {
            let along = step * index as f64;
            out.push(super::collector::TextGlyph {
                x: chunk.x + ux * along,
                y: chunk.y + uy * along,
                dx: ux * step,
                dy: uy * step,
                text_start: offset as u32,
                text_end: (offset + ch.len_utf8()) as u32,
                stream_pos: self.synthetic_pos,
                code_len: 1,
            });
            self.synthetic_pos = self.synthetic_pos.saturating_add(1);
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn add_glyph(
        &mut self,
        glyph: &super::collector::TextGlyph,
        text: &str,
        font: u32,
        font_size: f64,
        rot: u8,
        diagonal: bool,
        vertical: bool,
        ascent: f64,
        descent: f64,
    ) {
        let (x1, y1) = self.frame.point(glyph.x, glyph.y);
        let (w1, h1) = self.frame.delta(glyph.dx, glyph.dy);
        if !(x1.is_finite() && y1.is_finite() && w1.is_finite() && h1.is_finite()) {
            return;
        }
        if self.clip
            && (x1 + w1 < 0.0 || x1 > self.frame.width || y1 + h1 < 0.0 || y1 > self.frame.height)
        {
            return;
        }
        if w1.abs() < 3.0 && h1.abs() < 3.0 {
            self.tiny_chars += 1;
            if self.tiny_chars > MAX_TINY_CHARS {
                return;
            }
        }
        let mut chars = text.chars();
        let first = chars.next();
        let single = first.is_some() && chars.next().is_none();
        if single {
            let ch = first.unwrap_or(' ');
            if ch.is_whitespace() {
                self.end_word();
                return;
            }
            if ch == '\0' {
                return;
            }
        }
        let pos = glyph.stream_pos;
        let code_len = u32::from(glyph.code_len.max(1));

        if self.options.merge_combining && single {
            if let Some(word) = self.current.as_mut() {
                if word.rot == rot && !vertical {
                    let (p, s) = to_canon(rot, x1, y1);
                    let (dp, _) = to_canon(rot, w1, h1);
                    if word.add_combining(
                        font,
                        font_size,
                        p,
                        s,
                        dp,
                        pos,
                        code_len,
                        first.unwrap_or(' '),
                        ascent,
                        descent,
                    ) {
                        return;
                    }
                }
            }
        }

        // Start a new word when this glyph does not continue the current one.
        let mut break_word = false;
        if let Some(word) = self.current.as_ref().filter(|word| word.len() > 0) {
            let (p, base) = to_canon(word.rot, x1, y1);
            let wfs = word.font_size;
            let last_edge = word.chars.last().map_or(word.rect.p1, |c| c.edge);
            let sp = p - word.rect.p1;
            let delta = p - last_edge;
            let overlap = delta.abs() < DUP_MAX_PRI_DELTA * wfs
                && (base - word.base).abs() < DUP_MAX_SEC_DELTA * wfs;
            let size_changed = (font_size - wfs).abs() > font_size.max(wfs) * 1e-4;
            break_word = overlap
                || self.last_char_overlap
                || sp < -MIN_DUP_BREAK_OVERLAP * wfs
                || sp > MIN_WORD_BREAK_SPACE * wfs
                || (base - word.base).abs() > MAX_WORD_BASE_DELTA
                || size_changed
                || vertical != word.vertical
                || rot != word.rot;
            self.last_char_overlap = overlap;
        } else {
            self.last_char_overlap = false;
        }
        if break_word {
            self.end_word();
        }

        let Some(first) = first else {
            return;
        };
        if self.current.is_none() {
            self.begin_word(rot, diagonal, vertical, font_size);
        }
        if self.options.discard_diagonal && diagonal {
            self.diagonal_discarded += 1;
            return;
        }
        let (mut p, s) = to_canon(rot, x1, y1);
        let (mut dp, ds) = to_canon(rot, w1, h1);
        if dp < 0.0 {
            // Drawn against the reading direction (mirrored or reversed
            // text): every such glyph becomes its own word, boxes swapped.
            self.end_word();
            self.begin_word(rot, diagonal, vertical, font_size);
            p += dp;
            dp = -dp;
        }
        let n = if single {
            1
        } else {
            text.chars().count().max(1)
        };
        let step_p = dp / n as f64;
        let step_s = ds / n as f64;
        let word = self.current.as_mut().expect("word begun above");
        if single {
            word.add_char(font, p, s, dp, pos, code_len, first, ascent, descent);
        } else {
            for (index, ch) in text.chars().enumerate() {
                word.add_char(
                    font,
                    p + step_p * index as f64,
                    s + step_s * index as f64,
                    step_p,
                    pos,
                    code_len,
                    ch,
                    ascent,
                    descent,
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Pools

/// Word ids bucketed by baseline in 4-point steps.
#[derive(Debug, Default)]
struct Pool {
    min_idx: i64,
    buckets: Vec<Vec<usize>>,
}

impl Pool {
    fn index_of(base: f64) -> i64 {
        let value = (base / POOL_STEP).floor();
        if value.is_finite() {
            value.clamp(-(1i64 << 40) as f64, (1i64 << 40) as f64) as i64
        } else {
            0
        }
    }

    fn max_idx(&self) -> i64 {
        self.min_idx + self.buckets.len() as i64 - 1
    }

    /// Bucket index of `base`, clamped to the pool's range.
    fn base_idx(&self, base: f64) -> i64 {
        if base.is_nan() {
            return self.min_idx;
        }
        Self::index_of(base).clamp(self.min_idx, self.max_idx().max(self.min_idx))
    }

    fn add(&mut self, id: usize, base: f64) {
        let idx = Self::index_of(base);
        if self.buckets.is_empty() {
            self.min_idx = idx;
            self.buckets.push(Vec::new());
        }
        if idx < self.min_idx {
            let grow = (self.min_idx - idx) as usize;
            let mut front: Vec<Vec<usize>> = (0..grow).map(|_| Vec::new()).collect();
            front.append(&mut self.buckets);
            self.buckets = front;
            self.min_idx = idx;
        }
        while idx > self.max_idx() {
            self.buckets.push(Vec::new());
        }
        self.buckets[(idx - self.min_idx) as usize].push(id);
    }

    fn bucket(&self, idx: i64) -> &[usize] {
        if idx < self.min_idx || idx > self.max_idx() {
            return &[];
        }
        &self.buckets[(idx - self.min_idx) as usize]
    }

    fn bucket_mut(&mut self, idx: i64) -> Option<&mut Vec<usize>> {
        if idx < self.min_idx || idx > self.max_idx() {
            return None;
        }
        Some(&mut self.buckets[(idx - self.min_idx) as usize])
    }
}

// ---------------------------------------------------------------------------
// Blocks

#[derive(Debug, Clone)]
struct OutWord {
    text: String,
    rect: Rect,
    font_size: f64,
    diagonal: bool,
    space_after: bool,
}

#[derive(Debug, Clone)]
struct Line {
    rect: Rect,
    words: Vec<OutWord>,
    /// Characters of the line in visual order, spaces included.
    chars: Vec<char>,
    text: String,
    hyphenated: bool,
}

#[derive(Debug, Clone)]
struct Block {
    rot: u8,
    rect: Rect,
    lines: Vec<Line>,
    char_count: usize,
    first_font_size: f64,
    duplicates: usize,
    table: Option<usize>,
}

/// Build the blocks of one rotation pool.
fn build_blocks(
    words: &mut [Word],
    pool_words: &[usize],
    rot: u8,
    options: &FlowLayoutOptions,
) -> Vec<Block> {
    let mut pool = Pool::default();
    for &id in pool_words {
        pool.add(id, words[id].base);
    }
    let mut blocks = Vec::new();
    if pool.buckets.is_empty() {
        return blocks;
    }
    let mut pool_min = pool.min_idx;
    loop {
        while pool_min <= pool.max_idx() && pool.bucket(pool_min).is_empty() {
            pool_min += 1;
        }
        if pool_min > pool.max_idx() {
            break;
        }
        // The left-most word in the first four lines (avoids starting with a
        // superscript).
        let mut start = pool_min;
        for idx in pool_min + 1..(pool_min + 4).min(pool.max_idx() + 1) {
            let (Some(&a), Some(&b)) = (pool.bucket(idx).first(), pool.bucket(start).first())
            else {
                continue;
            };
            if words[a].rect.p0 < words[b].rect.p0 {
                start = idx;
            }
        }
        let seed = pool.bucket_mut(start).expect("bucket in range").remove(0);
        let mut members = vec![seed];
        let mut rect = words[seed].rect;
        let font_size = words[seed].font_size;
        let mut min_base = words[seed].base;
        let mut max_base = min_base;
        let col_space1 = MIN_COL_SPACING1 * font_size;
        let col_space2 = MIN_COL_SPACING2 * font_size;
        let line_space = MAX_LINE_SPACING_DELTA * font_size;
        let intra_line = MAX_INTRA_LINE_DELTA * font_size;

        loop {
            let mut found = false;

            // Words on the line above the block's top edge.
            let mut new_min = min_base;
            let mut idx = pool.base_idx(min_base);
            let lowest = pool.base_idx(min_base - line_space);
            while idx >= lowest {
                take_words(
                    &mut pool,
                    idx,
                    |w, rect| {
                        let take = w.base < min_base
                            && w.base >= min_base - line_space
                            && w.rect.p0 < rect.p1
                            && w.rect.p1 > rect.p0
                            && (w.font_size - font_size).abs()
                                < MAX_BLOCK_FONT_SIZE_DELTA1 * font_size;
                        if take {
                            new_min = w.base;
                        }
                        take
                    },
                    words,
                    &mut rect,
                    &mut members,
                    &mut found,
                    usize::MAX,
                );
                idx -= 1;
            }
            min_base = new_min;

            // Words on the line below the block's bottom edge.
            let mut new_max = max_base;
            let mut idx = pool.base_idx(max_base);
            let highest = pool.base_idx(max_base + line_space);
            while idx <= highest {
                take_words(
                    &mut pool,
                    idx,
                    |w, rect| {
                        let take = w.base > max_base
                            && w.base <= max_base + line_space
                            && w.rect.p0 < rect.p1
                            && w.rect.p1 > rect.p0
                            && (w.font_size - font_size).abs()
                                < MAX_BLOCK_FONT_SIZE_DELTA1 * font_size;
                        if take {
                            new_max = w.base;
                        }
                        take
                    },
                    words,
                    &mut rect,
                    &mut members,
                    &mut found,
                    usize::MAX,
                );
                idx += 1;
            }
            max_base = new_max;

            // Words on lines already in the block, overlapping it horizontally
            // within the column gap.
            let lo = pool.base_idx(min_base - intra_line);
            let hi = pool.base_idx(max_base + intra_line);
            for idx in lo..=hi {
                take_words(
                    &mut pool,
                    idx,
                    |w, rect| {
                        w.base >= min_base - intra_line
                            && w.base <= max_base + intra_line
                            && w.rect.p0 < rect.p1 + col_space1
                            && w.rect.p1 > rect.p0 - col_space1
                            && (w.font_size - font_size).abs()
                                < MAX_BLOCK_FONT_SIZE_DELTA2 * font_size
                    },
                    words,
                    &mut rect,
                    &mut members,
                    &mut found,
                    usize::MAX,
                );
            }
            if found {
                continue;
            }

            // Outliers: up to three lines' words just left, then just right,
            // of the block.
            for side in [false, true] {
                let lo = pool.base_idx(min_base - intra_line);
                let hi = pool.base_idx(max_base + intra_line);
                let matches = |w: &Word, rect: &Rect, min_base: f64, max_base: f64| {
                    let near = if side {
                        w.rect.p0 >= rect.p1 && w.rect.p0 < rect.p1 + col_space2
                    } else {
                        w.rect.p1 <= rect.p0 && w.rect.p1 > rect.p0 - col_space2
                    };
                    w.base >= min_base - intra_line
                        && w.base <= max_base + intra_line
                        && near
                        && (w.font_size - font_size).abs() < MAX_BLOCK_FONT_SIZE_DELTA3 * font_size
                };
                let count = (lo..=hi)
                    .filter(|&idx| {
                        pool.bucket(idx)
                            .iter()
                            .any(|&id| matches(&words[id], &rect, min_base, max_base))
                    })
                    .count();
                if count == 0 || count > 3 {
                    continue;
                }
                for idx in lo..=hi {
                    let (lo_base, hi_base) = (min_base, max_base);
                    let mut taken_base = None;
                    take_words(
                        &mut pool,
                        idx,
                        |w, rect| {
                            let take = matches(w, rect, lo_base, hi_base);
                            if take {
                                taken_base = Some(w.base);
                            }
                            take
                        },
                        words,
                        &mut rect,
                        &mut members,
                        &mut found,
                        1,
                    );
                    if let Some(base) = taken_base {
                        if base < min_base {
                            min_base = base;
                        } else if base > max_base {
                            max_base = base;
                        }
                    }
                }
            }
            if !found {
                break;
            }
        }

        blocks.push(coalesce_block(words, &members, rot, rect, options));
    }
    blocks
}

/// Move the words of bucket `idx` accepted by `accept` into the block, at
/// most `limit` of them. The block box grows as words join, and later words
/// of the bucket are tested against the grown box.
#[allow(clippy::too_many_arguments)]
fn take_words(
    pool: &mut Pool,
    idx: i64,
    mut accept: impl FnMut(&Word, &Rect) -> bool,
    words: &[Word],
    rect: &mut Rect,
    members: &mut Vec<usize>,
    found: &mut bool,
    limit: usize,
) {
    let Some(bucket) = pool.bucket_mut(idx) else {
        return;
    };
    let mut taken = 0;
    let mut k = 0;
    while k < bucket.len() && taken < limit {
        let id = bucket[k];
        if accept(&words[id], rect) {
            bucket.remove(k);
            rect.union(&words[id].rect);
            members.push(id);
            *found = true;
            taken += 1;
        } else {
            k += 1;
        }
    }
}

/// Inside a block: drop duplicated words, build lines and their text.
fn coalesce_block(
    words: &mut [Word],
    members: &[usize],
    rot: u8,
    rect: Rect,
    options: &FlowLayoutOptions,
) -> Block {
    let mut pool = Pool::default();
    for &id in members {
        pool.add(id, words[id].base);
    }
    for bucket in &mut pool.buckets {
        bucket.sort_by(|&a, &b| {
            let ca = words[a].rect.p0 + words[a].rect.p1;
            let cb = words[b].rect.p0 + words[b].rect.p1;
            ca.partial_cmp(&cb).unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    let mut duplicates = 0;
    if options.remove_duplicates {
        duplicates = remove_duplicates(words, &mut pool);
    }

    // Build the lines.
    let mut lines: Vec<(Rect, f64, Vec<usize>)> = Vec::new();
    let mut pool_min = pool.min_idx;
    loop {
        while pool_min <= pool.max_idx() && pool.bucket(pool_min).is_empty() {
            pool_min += 1;
        }
        if pool_min > pool.max_idx() {
            break;
        }
        let mut start = pool_min;
        for idx in pool_min + 1..(pool_min + 4).min(pool.max_idx() + 1) {
            let (Some(&a), Some(&b)) = (pool.bucket(idx).first(), pool.bucket(start).first())
            else {
                continue;
            };
            if words[a].rect.p0 < words[b].rect.p0 {
                start = idx;
            }
        }
        let first = pool.bucket_mut(start).expect("bucket in range").remove(0);
        let base = words[first].base;
        let font_size = words[first].font_size;
        let mut line_rect = words[first].rect;
        let mut line_words = vec![first];
        let mut last = first;
        let min_base = base - MAX_INTRA_LINE_DELTA * font_size;
        let max_base = base + MAX_INTRA_LINE_DELTA * font_size;
        let lo = pool.base_idx(min_base);
        let hi = pool.base_idx(max_base);
        let word_spacing = MAX_WORD_SPACING * font_size;
        loop {
            let mut best: Option<(i64, usize)> = None;
            let mut overlap = false;
            'buckets: for idx in lo..=hi {
                for (k, &id) in pool.bucket(idx).iter().enumerate() {
                    let w = &words[id];
                    if w.base >= min_base && w.base <= max_base {
                        let delta = w.rect.p0 - words[last].rect.p1;
                        if delta < MIN_CHAR_SPACING * font_size {
                            overlap = true;
                            break 'buckets;
                        }
                        let better = best
                            .is_none_or(|(bi, bk)| w.rect.p0 < words[pool.bucket(bi)[bk]].rect.p0);
                        if delta < word_spacing && better {
                            best = Some((idx, k));
                        }
                        break;
                    }
                }
            }
            let Some((idx, k)) = best.filter(|_| !overlap) else {
                break;
            };
            let id = pool.bucket_mut(idx).expect("bucket in range").remove(k);
            line_rect.union(&words[id].rect);
            line_words.push(id);
            last = id;
        }
        // Insert in (baseline, left edge) order.
        let key = (base, line_rect.p0);
        let at = lines
            .iter()
            .position(|(r, b, _)| {
                let cmp = key
                    .0
                    .partial_cmp(b)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(
                        key.1
                            .partial_cmp(&r.p0)
                            .unwrap_or(std::cmp::Ordering::Equal),
                    );
                cmp != std::cmp::Ordering::Greater
            })
            .unwrap_or(lines.len());
        lines.insert(at, (line_rect, base, line_words));
    }

    let mut first_font_size = 0.0;
    let mut char_count = 0;
    let built: Vec<Line> = lines
        .into_iter()
        .enumerate()
        .map(|(index, (line_rect, _base, ids))| {
            if index == 0 {
                first_font_size = words[ids[0]].font_size;
            }
            let line = coalesce_line(words, &ids, line_rect);
            char_count += line.chars.len();
            line
        })
        .collect();
    Block {
        rot,
        rect,
        lines: built,
        char_count,
        first_font_size,
        duplicates,
        table: None,
    }
}

/// Discard duplicated text (fake bold, drop shadows): identical words at
/// almost the same position, a word that is a prefix of another at its
/// position, or the overlapping head of a word equal to another's tail.
fn remove_duplicates(words: &mut [Word], pool: &mut Pool) -> usize {
    let mut removed = 0;
    if pool.buckets.is_empty() {
        return 0;
    }
    let max = pool.max_idx();
    let mut idx0 = pool.min_idx;
    while idx0 <= max {
        let mut k0 = 0;
        while k0 < pool.bucket(idx0).len() {
            let id0 = pool.bucket(idx0)[k0];
            let font_size = words[id0].font_size;
            let pri = DUP_MAX_PRI_DELTA * font_size;
            let sec = DUP_MAX_SEC_DELTA * font_size;
            let last_idx = pool.base_idx(words[id0].base + sec);
            for idx1 in idx0..=last_idx {
                let mut k1 = if idx1 == idx0 { k0 + 1 } else { 0 };
                while k1 < pool.bucket(idx1).len() {
                    let id1 = pool.bucket(idx1)[k1];
                    let (drop, keep) = duplicate_overlap(&words[id0], &words[id1], pri, sec);
                    if drop == keep {
                        pool.bucket_mut(idx1).expect("bucket in range").remove(k1);
                        removed += 1;
                        continue;
                    }
                    if drop != 0 {
                        let end = words[id0].rect.p1;
                        let w1 = &mut words[id1];
                        w1.chars.drain(..drop);
                        w1.rect.p0 = end;
                    }
                    k1 += 1;
                }
            }
            k0 += 1;
        }
        idx0 += 1;
    }
    removed
}

/// How much of `w1` duplicates `w0`: `(n, len)` drops the first `n`
/// characters of `w1` (all of it when `n == len`); `(0, len)` keeps it.
fn duplicate_overlap(w0: &Word, w1: &Word, pri: f64, sec: f64) -> (usize, usize) {
    let (a, b) = (&w0.rect, &w1.rect);
    let l0 = w0.len();
    let l1 = w1.len();
    if a.p0 > b.p1 || a.p1 < b.p0 || a.s0 > b.s1 || a.s1 < b.s0 {
        return (0, l1);
    }
    let equal_text =
        |len: usize, offset: usize| (0..len).all(|i| w0.chars[offset + i].ch == w1.chars[i].ch);
    let match_edges = |len: usize, offset: usize| {
        (w0.chars[offset].edge - w1.chars[0].edge).abs() < pri
            && (w0.chars[offset + len - 1].edge - w1.chars[len - 1].edge).abs() < pri
    };
    let match_base_top = (a.s0 - b.s0).abs() < sec && (a.s1 - b.s1).abs() < sec;
    if l0 == 0 || l1 == 0 {
        return (0, l1);
    }
    if l0 == l1 && equal_text(l0, 0) {
        return if match_edges(l0, 0) && match_base_top {
            (0, 0)
        } else {
            (0, l1)
        };
    } else if l0 < l1 && equal_text(l0, 0) {
        // w0 is a prefix of w1 (a bold word followed by a regular colon).
        if match_edges(l0, 0) && match_base_top {
            return (l0, l1);
        }
    } else if l0 > l1 && equal_text(l1, 0) && match_edges(l1, 0) && match_base_top {
        return (0, 0);
    }
    // The tail of w0 is the head of w1 (a quoted bold word).
    for offset in 1..l0 {
        let len = (l0 - offset).min(l1);
        if len == 0 {
            break;
        }
        if equal_text(len, offset) && match_edges(len, offset) {
            return (len, l1);
        }
    }
    (0, l1)
}

/// Merge a line's words, choose where spaces go and build its text.
fn coalesce_line(words: &[Word], ids: &[usize], rect: Rect) -> Line {
    let mut line_words: Vec<Word> = ids.iter().map(|&id| words[id].clone()).collect();
    let mut space_after = vec![false; line_words.len()];
    if line_words.len() > 1 {
        let font_size = line_words[0].font_size;
        let gap = |a: &Word, b: &Word| b.rect.p0 - a.rect.p1;
        // Letter-spaced text (a line of single-character words) needs a
        // threshold relative to its smallest gap.
        let mut min_space = if line_words[0].len() > 1 || line_words[1].len() > 1 {
            0.0
        } else {
            gap(&line_words[0], &line_words[1])
        };
        let mut i = 1;
        while i + 1 < line_words.len() && min_space > 0.0 {
            if line_words[i + 1].len() > 1 {
                min_space = 0.0;
            }
            let delta = gap(&line_words[i], &line_words[i + 1]);
            if delta < min_space {
                min_space = delta;
            }
            i += 1;
        }
        let space = if min_space <= 0.0 {
            MAX_CHAR_SPACING * font_size
        } else {
            (MAX_WIDE_CHAR_SPACING_MUL * min_space).min(MAX_WIDE_CHAR_SPACING * font_size)
        };

        let mut merged: Vec<Word> = Vec::with_capacity(line_words.len());
        let mut spaces: Vec<bool> = Vec::with_capacity(line_words.len());
        let mut iter = line_words.into_iter();
        let mut current = iter.next().expect("line has a word");
        for next in iter {
            let delta = gap(&current, &next);
            let same_font =
                current.chars.last().map(|c| c.font) == next.chars.first().map(|c| c.font);
            let adjacent = next.chars.first().map(|c| c.pos) == Some(current.pos_end);
            if delta >= space {
                merged.push(std::mem::replace(&mut current, next));
                spaces.push(true);
            } else if same_font
                && (current.font_size - next.font_size).abs() < MAX_WORD_FONT_SIZE_DELTA * font_size
                && adjacent
            {
                current.merge(next);
            } else {
                merged.push(std::mem::replace(&mut current, next));
                spaces.push(false);
            }
        }
        merged.push(current);
        spaces.push(false);
        line_words = merged;
        space_after = spaces;
    }

    let mut chars: Vec<char> = Vec::new();
    let mut out_words = Vec::with_capacity(line_words.len());
    for (word, &space) in line_words.iter().zip(&space_after) {
        chars.extend(word.chars.iter().map(|c| c.ch));
        if space {
            chars.push(' ');
        }
        out_words.push(OutWord {
            text: compose_text(word.chars.iter().map(|c| c.ch)),
            rect: word.rect,
            font_size: word.font_size,
            diagonal: word.diagonal,
            space_after: space,
        });
    }
    let hyphenated = chars.last().copied().is_some_and(is_hyphen);
    let visual = compose_text(chars.iter().copied());
    let text = super::reading_order::visual_to_logical(visual);
    Line {
        rect,
        words: out_words,
        chars,
        text,
        hyphenated,
    }
}

// ---------------------------------------------------------------------------
// Reading order and flows

/// Block geometry for page-level ordering, in the primary rotation's frame.
#[derive(Debug, Clone)]
struct PageBlock {
    r: Rect,
    /// Extended box (tables: the whole table; others: widened towards
    /// neighbours below).
    e: Rect,
    pri_min: f64,
    pri_max: f64,
    table: Option<usize>,
}

fn page_blocks(blocks: &[Block], primary: u8, frame: &PageFrame) -> Vec<PageBlock> {
    // The page's extent along the primary reading direction.
    let page = Rect {
        p0: 0.0,
        s0: 0.0,
        p1: frame.width,
        s1: frame.height,
    };
    let (mut pri_min, mut pri_max) = if page.p1.is_finite() && page.s1.is_finite() {
        let r = page.device_to_canon(primary);
        (r.p0, r.p1)
    } else {
        (f64::NEG_INFINITY, f64::INFINITY)
    };
    let rects: Vec<Rect> = blocks
        .iter()
        .map(|b| b.rect.to_device(b.rot).device_to_canon(primary))
        .collect();
    if !pri_min.is_finite() || !pri_max.is_finite() {
        pri_min = rects.iter().map(|r| r.p0).fold(f64::INFINITY, f64::min);
        pri_max = rects.iter().map(|r| r.p1).fold(f64::NEG_INFINITY, f64::max);
    }
    rects
        .into_iter()
        .map(|r| PageBlock {
            r,
            e: r,
            pri_min,
            pri_max,
            table: None,
        })
        .collect()
}

/// Narrow a block's free horizontal range by a side-by-side neighbour.
fn update_pri_min_max(block: &mut PageBlock, other: &Rect) {
    let r = block.r;
    if other.s0 < r.s1 && other.s1 > r.s0 {
        if other.p0 < r.p0 {
            let new_min = other.p1.min(r.p0);
            if new_min > block.pri_min {
                block.pri_min = new_min;
            }
        }
        if other.p1 > r.p1 {
            let new_max = other.p0.max(r.p1);
            if new_max < block.pri_max {
                block.pri_max = new_max;
            }
        }
    }
}

fn overlaps(a0: f64, a1: f64, b0: f64, b1: f64) -> bool {
    a0 <= b1 && a1 >= b0
}

/// Recognise tables: a block with a right neighbour, a neighbour below and a
/// diagonal neighbour, all aligned. Returns the number of tables.
fn detect_tables(pb: &mut [PageBlock], first_sizes: &[f64]) -> usize {
    let n = pb.len();
    let mut tables = 0usize;
    for i in 0..n {
        let b1 = pb[i].r;
        let (mut bx_min0, mut by_min0, mut bx_min1, mut by_min1) =
            (f64::MAX, f64::MAX, f64::MAX, f64::MAX);
        let (mut f2, mut f3, mut f4): (Option<usize>, Option<usize>, Option<usize>) =
            (None, None, None);
        for (j, block) in pb.iter().enumerate() {
            if j == i {
                continue;
            }
            let b2 = block.r;
            if b2.s0 <= b1.s1 && b2.s1 >= b1.s0 && b2.p0 > b1.p1 && b2.p0 < bx_min0 {
                bx_min0 = b2.p0;
                f2 = Some(j);
            } else if b2.p0 <= b1.p1 && b2.p1 >= b1.p0 && b2.s0 > b1.s1 && b2.s0 < by_min0 {
                by_min0 = b2.s0;
                f3 = Some(j);
            } else if b2.p0 > b1.p1 && b2.p0 < bx_min1 && b2.s0 > b1.s1 && b2.s0 < by_min1 {
                bx_min1 = b2.p0;
                by_min1 = b2.s0;
                f4 = Some(j);
            }
        }
        let (Some(j2), Some(j3), Some(j4)) = (f2, f3, f4) else {
            continue;
        };
        let (r2, r3, r4) = (pb[j2].r, pb[j3].r, pb[j4].r);
        if overlaps(r3.p0, r3.p1, r4.p0, r4.p1)
            || overlaps(r2.s0, r2.s1, r4.s0, r4.s1)
            || overlaps(r2.p0, r2.p1, r3.p0, r3.p1)
            || overlaps(r2.s0, r2.s1, r3.s0, r3.s1)
            || !(overlaps(r4.p0, r4.p1, r2.p0, r2.p1) && overlaps(r4.s0, r4.s1, r3.s0, r3.s1))
        {
            continue;
        }
        let size = [i, j2, j3, j4]
            .iter()
            .map(|&k| first_sizes[k])
            .fold(f64::INFINITY, f64::min);
        let delta_x = size * MIN_COL_SPACING1;
        let delta_y = size * MAX_INTRA_LINE_DELTA;
        let centre = |r: &Rect| ((r.p0 + r.p1) / 2.0, (r.s0 + r.s1) / 2.0);
        let (x1, y1) = centre(&b1);
        let (x2, y2) = centre(&r2);
        let (x3, y3) = centre(&r3);
        let (x4, y4) = centre(&r4);
        let mut corr_x = 0;
        let mut corr_y = 0;
        if (x1 - x3).abs() <= delta_x && (x2 - x4).abs() <= delta_x {
            corr_x += 1;
        }
        if (y1 - y2).abs() <= delta_y && (y3 - y4).abs() <= delta_y {
            corr_y += 1;
        }
        if (b1.p0 - r3.p0).abs() <= delta_x && (r2.p0 - r4.p0).abs() <= delta_x {
            corr_x += 1;
        }
        if (b1.p1 - r3.p1).abs() <= delta_x && (r2.p1 - r4.p1).abs() <= delta_x {
            corr_x += 1;
        }
        if (b1.s0 - r2.s0).abs() <= delta_y && (r3.s0 - r4.s0).abs() <= delta_y {
            corr_y += 1;
        }
        if (b1.s1 - r2.s1).abs() <= delta_y && (r3.s1 - r4.s1).abs() <= delta_y {
            corr_y += 1;
        }
        if corr_x > 0 && corr_y > 0 {
            let existing = [i, j2, j3, j4].iter().filter_map(|&k| pb[k].table).max();
            let id = existing.unwrap_or_else(|| {
                tables += 1;
                tables - 1
            });
            for k in [i, j2, j3, j4] {
                pb[k].table = Some(id);
            }
        }
    }
    tables
}

/// Blocks in reading order, with their page-level geometry.
fn reading_order(
    blocks: &[Block],
    primary: u8,
    primary_lr: bool,
    frame: &PageFrame,
) -> (Vec<usize>, Vec<PageBlock>) {
    let n = blocks.len();
    if n == 0 {
        return (Vec::new(), Vec::new());
    }
    let mut pb = page_blocks(blocks, primary, frame);
    for i in 0..n {
        for j in 0..n {
            if i != j {
                let other = pb[j].r;
                update_pri_min_max(&mut pb[i], &other);
            }
        }
    }
    let first_sizes: Vec<f64> = blocks.iter().map(|b| b.first_font_size).collect();
    let tables = detect_tables(&mut pb, &first_sizes);

    // Tables: every cell's extended box is the whole table.
    let mut envelopes = vec![Rect::empty(); tables];
    for block in &pb {
        if let Some(t) = block.table {
            envelopes[t].union(&block.r);
        }
    }
    for block in &mut pb {
        if let Some(t) = block.table {
            block.e = envelopes[t];
        }
    }
    // Other blocks extend horizontally towards blocks below them, up to the
    // nearest side-by-side neighbours.
    for i in 0..n {
        if pb[i].table.is_some() {
            continue;
        }
        let r1 = pb[i].r;
        let mut x_max = f64::MAX;
        let mut x_min = f64::MIN;
        for (j, other) in pb.iter().enumerate() {
            if j == i {
                continue;
            }
            let r2 = other.r;
            if r1.s0 <= r2.s1 && r1.s1 >= r2.s0 {
                if r2.p0 < x_max && r2.p0 > r1.p1 {
                    x_max = r2.p0;
                }
                if r2.p1 > x_min && r2.p1 < r1.p0 {
                    x_min = r2.p1;
                }
            }
        }
        let mut e = pb[i].e;
        for (j, other) in pb.iter().enumerate() {
            if j == i {
                continue;
            }
            let r2 = other.r;
            if r2.p1 > e.p1 && r2.p1 <= x_max && r2.s0 >= r1.s1 {
                e.p1 = r2.p1;
            }
            if r2.p0 < e.p0 && r2.p0 >= x_min && r2.s0 >= r1.s1 {
                e.p0 = r2.p0;
            }
        }
        pb[i].e = e;
    }
    // Rule 1: `a` overlaps `b` horizontally and lies above it.
    let rule1 = |a: &PageBlock, b: &PageBlock| -> bool {
        let overlap =
            (a.e.p0 <= b.e.p0 && b.e.p0 <= a.e.p1) || (b.e.p0 <= a.e.p0 && a.e.p0 <= b.e.p1);
        overlap && a.e.s0 < b.e.s0
    };
    // Rule 2: `a` lies entirely left of `b` (right of it on an RTL page).
    let rule2 = |a: &PageBlock, b: &PageBlock| -> bool {
        if primary_lr {
            a.e.p1 <= b.e.p0
        } else {
            b.e.p1 <= a.e.p0
        }
    };
    // Rule-1 relations as bit sets, so "is there a block c with b1 before c
    // and c before b2" is a word-wise intersection rather than a scan.
    let words_per_row = n.div_ceil(64);
    let check_intervening = n <= MAX_BLOCKS_FOR_INTERVENING_CHECK;
    let (mut below, mut above) = (Vec::new(), Vec::new());
    if check_intervening {
        below = vec![0u64; n * words_per_row];
        above = vec![0u64; n * words_per_row];
        for a in 0..n {
            for c in 0..n {
                if a != c && rule1(&pb[a], &pb[c]) {
                    // a is before c: c is below a, a is above c.
                    below[a * words_per_row + c / 64] |= 1 << (c % 64);
                    above[c * words_per_row + a / 64] |= 1 << (a % 64);
                }
            }
        }
    }
    let intervening = |b1: usize, b2: usize| -> bool {
        let row1 = &below[b1 * words_per_row..(b1 + 1) * words_per_row];
        let row2 = &above[b2 * words_per_row..(b2 + 1) * words_per_row];
        row1.iter().zip(row2).any(|(x, y)| x & y != 0)
    };
    // Is `b2` before `b1`?
    let before = |b1: usize, b2: usize| -> bool {
        let (a, b) = (&pb[b1], &pb[b2]);
        if let (Some(t1), Some(t2)) = (a.table, b.table) {
            if t1 == t2 {
                // Cells of one table: row by row.
                let side = if primary_lr {
                    b.r.p1 <= a.r.p0
                } else {
                    b.r.p0 >= a.r.p1
                };
                return (side && b.r.s0 <= a.r.s1 && b.r.s1 >= a.r.s0) || b.r.s1 <= a.r.s0;
            }
        }
        if rule1(b, a) {
            return true;
        }
        // Rule 2, unless some block lies below b1 and above b2.
        rule2(b, a) && !(check_intervening && intervening(b1, b2))
    };

    // Depth-first topological sort, iterative.
    let mut visited = vec![false; n];
    let mut sorted = Vec::with_capacity(n);
    for start in 0..n {
        if visited[start] {
            continue;
        }
        visited[start] = true;
        let mut stack: Vec<(usize, usize)> = vec![(start, 0)];
        while let Some(&mut (node, ref mut next)) = stack.last_mut() {
            let mut descended = false;
            while *next < n {
                let candidate = *next;
                *next += 1;
                if visited[candidate] {
                    continue;
                }
                if before(node, candidate) {
                    visited[candidate] = true;
                    stack.push((candidate, 0));
                    descended = true;
                    break;
                }
            }
            if !descended {
                stack.pop();
                sorted.push(node);
            }
        }
    }
    (sorted, pb)
}

/// Join consecutive blocks into flows.
fn build_flows(blocks: &[Block], pb: &[PageBlock], order: &[usize]) -> Vec<Vec<usize>> {
    let mut flows: Vec<Vec<usize>> = Vec::new();
    // (pri_min, pri_max, last block) of the open flow.
    let mut open: Option<(f64, f64, usize)> = None;
    for (position, &index) in order.iter().enumerate() {
        if let (Some((pri_min, pri_max, last)), true) = (open, position > 0) {
            let prev = &pb[last];
            let cur = &pb[index];
            let spacing = MAX_BLOCK_SPACING * blocks[last].first_font_size;
            let secondary_delta = cur.r.s0 - prev.r.s1;
            let below =
                cur.r.p0 >= prev.pri_min && cur.r.p1 <= prev.pri_max && cur.r.s0 > prev.r.s0;
            let fits = blocks[index].first_font_size <= blocks[last].first_font_size
                && cur.r.p0 >= pri_min
                && cur.r.p1 <= pri_max;
            if secondary_delta <= spacing && below && fits {
                flows.last_mut().expect("open flow").push(index);
                open = Some((pri_min, pri_max, index));
                continue;
            }
        }
        flows.push(vec![index]);
        open = Some((pb[index].pri_min, pb[index].pri_max, index));
    }
    flows
}

#[cfg(test)]
#[path = "flow_layout_tests.rs"]
mod tests;
