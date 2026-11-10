//! Geometric layout analysis — recover document structure (columns, blocks,
//! reading order) from the flat set of positioned text chunks.
//!
//! # What this adds over the default extraction
//!
//! The default text path ([`crate::text`]) groups chunks into lines and does a
//! crude 2-column split. This module recovers a real hierarchical **page →
//! region → block → line** structure and a correct **reading order** across
//! arbitrary multi-column layouts, using the classic, ML-free **recursive
//! XY-cut** (projection-profile) algorithm with a Docstrum-style spacing
//! estimate to set document-relative thresholds.
//!
//! # Coordinate space
//!
//! Operates purely on [`TextChunk`] geometry, which is **PDF user space**
//! (origin bottom-left, y increases *upward*). A chunk's box is
//! `[x, x+width] × [y, y+font_size]` (the baseline is at `y`; the cap height is
//! approximated by `font_size`). "Higher on the page" therefore means *larger*
//! y, and reading top-to-bottom means *descending* y. All thresholds are
//! **document-relative** — scaled to the median font size / estimated line
//! height — so the analysis generalises across DPIs and font sizes.

use serde::Serialize;

use crate::text::TextChunk;

/// An axis-aligned bounding box in PDF user space (y-up).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct BBox {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl BBox {
    pub fn width(&self) -> f64 {
        (self.x1 - self.x0).max(0.0)
    }
    pub fn height(&self) -> f64 {
        (self.y1 - self.y0).max(0.0)
    }
    fn union(&self, other: &BBox) -> BBox {
        BBox {
            x0: self.x0.min(other.x0),
            y0: self.y0.min(other.y0),
            x1: self.x1.max(other.x1),
            y1: self.y1.max(other.y1),
        }
    }
    /// Union of a non-empty slice of boxes.
    fn bounding(boxes: &[BBox]) -> Option<BBox> {
        let mut iter = boxes.iter();
        let first = *iter.next()?;
        Some(iter.fold(first, |acc, b| acc.union(b)))
    }
}

/// A positioned word/run with its box (an internal working item — the chunk's
/// text plus its geometry).
#[derive(Debug, Clone)]
struct Item {
    bbox: BBox,
    text: String,
    font_size: f64,
    is_rtl: bool,
    /// Index into the page's vertical-text blocks when this item stands for
    /// a whole vertical block (so the XY-cut places it by position).
    vertical_block: Option<usize>,
}

impl Item {
    fn from_chunk(c: &TextChunk) -> Option<Self> {
        // Skip vertical runs (laid out separately by `vertical_blocks`) and
        // pure-whitespace/empty chunks (they carry no structure).
        if c.is_vertical || c.text.trim().is_empty() {
            return None;
        }
        let fs = if c.font_size > 0.0 { c.font_size } else { 1.0 };
        Some(Item {
            bbox: BBox {
                x0: c.x,
                y0: c.y,
                x1: c.x + c.width.max(0.0),
                y1: c.y + fs,
            },
            text: c.text.clone(),
            font_size: fs,
            is_rtl: c.is_rtl,
            vertical_block: None,
        })
    }
}

/// A reconstructed line within a block: the joined text and its box.
#[derive(Debug, Clone, Serialize)]
pub struct LayoutLine {
    pub text: String,
    pub bbox: BBox,
    /// True when the dominant script on the line is right-to-left.
    pub is_rtl: bool,
}

/// A logical block (≈ paragraph or a table region): an ordered set of lines.
#[derive(Debug, Clone, Serialize)]
pub struct LayoutBlock {
    pub bbox: BBox,
    pub lines: Vec<LayoutLine>,
    /// Representative font size (median of the block's lines).
    pub font_size: f64,
}

impl LayoutBlock {
    /// The block's text, lines joined by '\n', in reading order.
    pub fn text(&self) -> String {
        self.lines
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// The structured result of analysing one page: blocks in reading order.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PageLayout {
    /// Blocks in reading order (columns left-to-right — or right-to-left for
    /// RTL-dominant pages — and top-to-bottom within a column).
    pub blocks: Vec<LayoutBlock>,
    /// True when the page's text is right-to-left dominant (most chunks RTL).
    /// Exposed so downstream consumers (e.g. the document-model precedence
    /// ordering) use a single, consistent RTL source rather than re-deriving it
    /// from the per-block flags. Defaults to `false` for an empty page.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub page_is_rtl: bool,
}

impl PageLayout {
    /// The whole page's text in reading order: blocks separated by a blank line.
    pub fn text(&self) -> String {
        self.blocks
            .iter()
            .map(LayoutBlock::text)
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

/// Tunable thresholds for the XY-cut. All gap thresholds are expressed as
/// multiples of the document's estimated line height, so the analysis is
/// resolution- and font-size-independent.
#[derive(Debug, Clone)]
pub struct LayoutConfig {
    /// A vertical gutter (column gap) must be at least this multiple of the
    /// median line height to be cut. Column gutters are wider than word gaps.
    pub column_gap_factor: f64,
    /// A horizontal gap (between stacked blocks) must be at least this multiple
    /// of the median line height to be cut.
    pub block_gap_factor: f64,
    /// Two items share a line if their vertical baseline centres differ by less
    /// than this multiple of the line height.
    pub line_overlap_factor: f64,
    /// Maximum XY-cut recursion depth (guards pathological inputs).
    pub max_depth: u32,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            column_gap_factor: 1.2,
            block_gap_factor: 1.2,
            line_overlap_factor: 0.6,
            max_depth: 40,
        }
    }
}

/// Analyse a page's text chunks into a structured, reading-ordered layout.
///
/// Vertical (WMode 1) runs are not part of the XY-cut: they are grouped into
/// columns read top to bottom, columns right to left, and the resulting blocks
/// are merged into the horizontal reading order by their position on the page.
pub fn analyze_page(chunks: &[TextChunk], config: &LayoutConfig) -> PageLayout {
    let vertical: Vec<TextChunk> = chunks
        .iter()
        .filter(|c| c.is_vertical && !c.text.trim().is_empty())
        .cloned()
        .collect();
    let vertical = if vertical.is_empty() {
        Vec::new()
    } else {
        vertical_blocks(vertical)
    };
    analyze_items(chunks, vertical, config)
}

fn analyze_items(
    chunks: &[TextChunk],
    vertical: Vec<LayoutBlock>,
    config: &LayoutConfig,
) -> PageLayout {
    let items = remove_overprinted(chunks.iter().filter_map(Item::from_chunk).collect());
    if items.is_empty() {
        return PageLayout {
            blocks: vertical,
            page_is_rtl: false,
        };
    }

    // Estimate the characteristic line height = median font size. Robust to a
    // few outlier headings/footnotes.
    let line_height = median_font_size(&items).max(1.0);

    // RTL-dominant page? If most text is RTL, columns read right-to-left.
    let rtl_items = items.iter().filter(|i| i.is_rtl).count();
    let page_is_rtl = rtl_items * 2 > items.len();

    // Vertical blocks join the cut as one opaque item each, so whitespace
    // around them orders them among the horizontal regions.
    let mut items = items;
    items.extend(vertical.iter().enumerate().map(|(index, block)| Item {
        bbox: block.bbox,
        text: String::new(),
        font_size: block.font_size.max(1.0),
        is_rtl: false,
        vertical_block: Some(index),
    }));
    let mut vertical: Vec<Option<LayoutBlock>> = vertical.into_iter().map(Some).collect();

    // Estimate the typical inter-line PITCH (baseline-to-baseline of vertically
    // adjacent items). The horizontal (block-separating) XY cut must only fire
    // on vertical whitespace LARGER than normal line spacing, otherwise every
    // line's inter-line gap looks like a block boundary. The column-separating
    // vertical cut, by contrast, keys off the font-size-relative gutter width.
    // Vertical blocks take part, so the gaps around them count as breaks.
    let line_pitch = estimate_line_pitch(&items, line_height);

    // Recursive XY-cut produces leaf regions in reading order.
    let mut leaves: Vec<Vec<Item>> = Vec::new();
    xy_cut(
        items,
        line_height,
        line_pitch,
        config,
        page_is_rtl,
        0,
        &mut leaves,
    );

    // Each leaf region becomes one or more blocks (a region may still contain a
    // few stacked paragraphs that no whitespace cut separated).
    let mut blocks = Vec::new();
    for leaf in leaves {
        let (placeholders, leaf): (Vec<Item>, Vec<Item>) = leaf
            .into_iter()
            .partition(|item| item.vertical_block.is_some());
        let mut region = region_to_blocks(leaf, line_height, config, page_is_rtl);
        let region_vertical: Vec<LayoutBlock> = placeholders
            .iter()
            .filter_map(|item| item.vertical_block.and_then(|i| vertical[i].take()))
            .collect();
        merge_vertical_blocks(&mut region, region_vertical);
        blocks.extend(region);
    }

    PageLayout {
        blocks,
        page_is_rtl,
    }
}

/// Drop runs over-printed by an identical run (fake bold, fill-then-stroke,
/// drop shadows): same text and font size within 5%, origin within 0.1 ×
/// font size horizontally and 0.2 × font size vertically (Poppler's
/// duplicate tolerances). The first occurrence is kept; order is preserved.
fn remove_overprinted(items: Vec<Item>) -> Vec<Item> {
    if items.len() < 2 {
        return items;
    }
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|&a, &b| {
        items[a]
            .text
            .cmp(&items[b].text)
            .then(
                items[a]
                    .bbox
                    .y0
                    .partial_cmp(&items[b].bbox.y0)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
            .then(a.cmp(&b))
    });
    let mut dropped = vec![false; items.len()];
    for (k, &i) in order.iter().enumerate() {
        if dropped[i] {
            continue;
        }
        let first = &items[i];
        let size = first.font_size;
        for &j in &order[k + 1..] {
            let other = &items[j];
            if other.text != first.text || other.bbox.y0 - first.bbox.y0 > 0.2 * size {
                break;
            }
            if !dropped[j]
                && (other.bbox.x0 - first.bbox.x0).abs() < 0.1 * size
                && (other.font_size - size).abs() < 0.05 * size
            {
                dropped[j] = true;
            }
        }
    }
    items
        .into_iter()
        .zip(dropped)
        .filter_map(|(item, dropped)| (!dropped).then_some(item))
        .collect()
}

/// Lay vertical runs out as blocks: chunks are grouped into columns (read top
/// to bottom), and neighbouring columns whose vertical extents overlap form one
/// block whose lines are the columns from right to left.
fn vertical_blocks(chunks: Vec<TextChunk>) -> Vec<LayoutBlock> {
    let reader = crate::text::reading_order::ReadingOrderReconstructor::default();
    // `group_into_columns` returns columns right to left.
    let columns: Vec<(BBox, f64, LayoutLine)> = reader
        .group_into_columns(chunks)
        .into_iter()
        .enumerate()
        .filter_map(|(index, group)| {
            let top = group.iter().map(|c| c.y).fold(f64::NEG_INFINITY, f64::max);
            let bottom = group
                .iter()
                .map(|c| c.y - c.width.max(c.font_size).max(0.0))
                .fold(f64::INFINITY, f64::min);
            let line = reader.vertical_column_line(group, index);
            if line.text.trim().is_empty() || !top.is_finite() || !bottom.is_finite() {
                return None;
            }
            let bbox = BBox {
                x0: line.x_min,
                y0: bottom.min(top),
                x1: line.x_max.max(line.x_min),
                y1: top.max(bottom),
            };
            let font_size = line.font_size;
            Some((
                bbox,
                font_size,
                LayoutLine {
                    text: line.text.trim().to_string(),
                    bbox,
                    is_rtl: false,
                },
            ))
        })
        .collect();

    let mut blocks: Vec<LayoutBlock> = Vec::new();
    let mut current: Vec<(BBox, f64, LayoutLine)> = Vec::new();
    for column in columns {
        if let Some(previous) = current.last() {
            let gap = previous.0.x0 - column.0.x1;
            let overlap = previous.0.y0 < column.0.y1 && column.0.y0 < previous.0.y1;
            if gap > 2.0 * previous.1.max(column.1) || !overlap {
                blocks.push(finish_vertical_block(std::mem::take(&mut current)));
            }
        }
        current.push(column);
    }
    if !current.is_empty() {
        blocks.push(finish_vertical_block(current));
    }
    blocks
}

fn finish_vertical_block(columns: Vec<(BBox, f64, LayoutLine)>) -> LayoutBlock {
    let mut sizes: Vec<f64> = columns.iter().map(|c| c.1).collect();
    sizes.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let font_size = sizes.get(sizes.len() / 2).copied().unwrap_or(0.0);
    let boxes: Vec<BBox> = columns.iter().map(|c| c.0).collect();
    LayoutBlock {
        bbox: BBox::bounding(&boxes).unwrap_or(BBox {
            x0: 0.0,
            y0: 0.0,
            x1: 0.0,
            y1: 0.0,
        }),
        lines: columns.into_iter().map(|c| c.2).collect(),
        font_size,
    }
}

/// Insert vertical blocks into the horizontal reading order by position: a
/// vertical block goes before the first horizontal block that starts lower on
/// the page than it does (so a vertical title above body text is read first,
/// and a vertical column beside a horizontal header is read after it).
fn merge_vertical_blocks(blocks: &mut Vec<LayoutBlock>, vertical: Vec<LayoutBlock>) {
    for block in vertical {
        let position = blocks
            .iter()
            .position(|existing| existing.bbox.y1 < block.bbox.y1)
            .unwrap_or(blocks.len());
        blocks.insert(position, block);
    }
}

/// Recursive XY-cut. Tries the *widest* significant gap in either projection;
/// cuts there and recurses. A vertical cut (gap in the X projection) separates
/// columns and is emitted left-to-right (right-to-left when `page_is_rtl`); a
/// horizontal cut (gap in the Y projection) separates stacked blocks and is
/// emitted top-to-bottom. When no significant gap remains the region is a leaf.
#[allow(clippy::too_many_arguments)]
fn xy_cut(
    items: Vec<Item>,
    line_height: f64,
    line_pitch: f64,
    config: &LayoutConfig,
    page_is_rtl: bool,
    depth: u32,
    out: &mut Vec<Vec<Item>>,
) {
    if items.is_empty() {
        return;
    }
    if items.len() == 1 || depth >= config.max_depth {
        out.push(items);
        return;
    }

    // Column gutter: a vertical band wider than ~1.2 line heights.
    let col_threshold = config.column_gap_factor * line_height;
    // Block separator: vertical whitespace LARGER than a normal inter-line gap.
    // The inter-line *gap* (not pitch) is `line_pitch - line_height`; a block
    // boundary is a gap exceeding `block_gap_factor` times the pitch, so single
    // line spacing never triggers a cut.
    let block_threshold = (config.block_gap_factor * line_pitch).max(line_pitch * 1.4);

    // Find the widest vertical gutter (gap along X) and widest horizontal gap
    // (gap along Y). Prefer the *larger* of the two qualifying gaps; on a tie
    // prefer the vertical cut (column gutters define reading order and should be
    // cut before stacking).
    let v_gap = widest_gap(&items, Axis::X, col_threshold);
    let h_gap = widest_gap(&items, Axis::Y, block_threshold);

    let cut = match (v_gap, h_gap) {
        (Some(v), Some(h)) => {
            if v.width >= h.width {
                Some((Axis::X, v.position))
            } else {
                Some((Axis::Y, h.position))
            }
        }
        (Some(v), None) => Some((Axis::X, v.position)),
        (None, Some(h)) => Some((Axis::Y, h.position)),
        (None, None) => None,
    };

    let Some((axis, position)) = cut else {
        out.push(items);
        return;
    };

    let (mut lo, mut hi): (Vec<Item>, Vec<Item>) = items.into_iter().partition(|it| match axis {
        Axis::X => center(it.bbox.x0, it.bbox.x1) < position,
        Axis::Y => center(it.bbox.y0, it.bbox.y1) < position,
    });

    match axis {
        Axis::X => {
            // `lo` = left column, `hi` = right column. Reading order: left then
            // right for LTR; right then left for an RTL-dominant page.
            if page_is_rtl {
                std::mem::swap(&mut lo, &mut hi);
            }
            xy_cut(
                lo,
                line_height,
                line_pitch,
                config,
                page_is_rtl,
                depth + 1,
                out,
            );
            xy_cut(
                hi,
                line_height,
                line_pitch,
                config,
                page_is_rtl,
                depth + 1,
                out,
            );
        }
        Axis::Y => {
            // `lo` = lower y (further down the page), `hi` = higher y (top).
            // Read top-to-bottom => higher y first.
            xy_cut(
                hi,
                line_height,
                line_pitch,
                config,
                page_is_rtl,
                depth + 1,
                out,
            );
            xy_cut(
                lo,
                line_height,
                line_pitch,
                config,
                page_is_rtl,
                depth + 1,
                out,
            );
        }
    }
}

#[derive(Clone, Copy)]
enum Axis {
    X,
    Y,
}

struct Gap {
    position: f64,
    width: f64,
}

/// Find the widest empty band (no item overlaps it) along `axis`, wider than
/// `threshold`. The band's centre is returned as the cut position. The page
/// margins are not considered gaps (only interior gaps split a region).
fn widest_gap(items: &[Item], axis: Axis, threshold: f64) -> Option<Gap> {
    // Build [start, end] intervals of every item along the axis, then sweep to
    // find the largest uncovered interior interval.
    let mut intervals: Vec<(f64, f64)> = items
        .iter()
        .map(|it| match axis {
            Axis::X => (it.bbox.x0, it.bbox.x1),
            Axis::Y => (it.bbox.y0, it.bbox.y1),
        })
        .collect();
    intervals.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    let mut best: Option<Gap> = None;
    // Coverage frontier: the furthest end seen so far.
    let mut frontier = intervals[0].1;
    for &(start, end) in &intervals[1..] {
        if start > frontier {
            let gap_w = start - frontier;
            if gap_w >= threshold {
                let pos = (frontier + start) / 2.0;
                if best.as_ref().map(|b| gap_w > b.width).unwrap_or(true) {
                    best = Some(Gap {
                        position: pos,
                        width: gap_w,
                    });
                }
            }
        }
        if end > frontier {
            frontier = end;
        }
    }
    best
}

/// Turn a leaf region's items into blocks: group into lines, then split lines
/// into paragraph blocks at vertical gaps larger than the typical line spacing.
fn region_to_blocks(
    items: Vec<Item>,
    line_height: f64,
    config: &LayoutConfig,
    page_is_rtl: bool,
) -> Vec<LayoutBlock> {
    if items.is_empty() {
        return Vec::new();
    }
    let lines = group_into_lines(items, line_height, config, page_is_rtl);
    if lines.is_empty() {
        return Vec::new();
    }

    // Estimate the typical line pitch (centre-to-centre of consecutive lines).
    let mut pitches: Vec<f64> = lines
        .windows(2)
        .map(|w| {
            let a = center(w[0].bbox.y0, w[0].bbox.y1);
            let b = center(w[1].bbox.y0, w[1].bbox.y1);
            (a - b).abs()
        })
        .filter(|p| *p > 0.0)
        .collect();
    pitches.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let typical_pitch = if pitches.is_empty() {
        line_height
    } else {
        pitches[pitches.len() / 2]
    };
    // A paragraph break is a vertical gap noticeably larger than the typical
    // pitch (1.5x), document-relative.
    let para_gap = typical_pitch * 1.5;

    let mut blocks: Vec<LayoutBlock> = Vec::new();
    let mut current: Vec<LayoutLine> = Vec::new();
    let mut prev_center: Option<f64> = None;
    for line in lines {
        let c = center(line.bbox.y0, line.bbox.y1);
        if let Some(pc) = prev_center {
            if (pc - c).abs() > para_gap && !current.is_empty() {
                blocks.push(finish_block(std::mem::take(&mut current)));
            }
        }
        prev_center = Some(c);
        current.push(line);
    }
    if !current.is_empty() {
        blocks.push(finish_block(current));
    }
    blocks
}

fn finish_block(lines: Vec<LayoutLine>) -> LayoutBlock {
    let boxes: Vec<BBox> = lines.iter().map(|l| l.bbox).collect();
    let bbox = BBox::bounding(&boxes).unwrap_or(BBox {
        x0: 0.0,
        y0: 0.0,
        x1: 0.0,
        y1: 0.0,
    });
    let mut sizes: Vec<f64> = lines.iter().map(|l| l.bbox.height()).collect();
    sizes.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let font_size = sizes.get(sizes.len() / 2).copied().unwrap_or(0.0);
    LayoutBlock {
        bbox,
        lines,
        font_size,
    }
}

/// Group a region's items into horizontal lines (by overlapping y), each line's
/// items ordered left-to-right (or right-to-left for an RTL line), joined with
/// spacing inferred from horizontal gaps.
fn group_into_lines(
    mut items: Vec<Item>,
    line_height: f64,
    config: &LayoutConfig,
    page_is_rtl: bool,
) -> Vec<LayoutLine> {
    // Sort by descending y (top of page first).
    items.sort_by(|a, b| {
        center(b.bbox.y0, b.bbox.y1)
            .partial_cmp(&center(a.bbox.y0, a.bbox.y1))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let tol = config.line_overlap_factor * line_height;
    let mut lines: Vec<Vec<Item>> = Vec::new();
    for item in items {
        let cy = center(item.bbox.y0, item.bbox.y1);
        // Find an existing line whose representative centre is within tolerance.
        let same_line = lines
            .last()
            .map(|last| {
                let lcy = center(last[0].bbox.y0, last[0].bbox.y1);
                (lcy - cy).abs() <= tol
            })
            .unwrap_or(false);
        if same_line {
            lines.last_mut().unwrap().push(item);
        } else {
            lines.push(vec![item]);
        }
    }

    lines
        .into_iter()
        .map(|line_items| build_line(line_items, line_height, page_is_rtl))
        .collect()
}

/// Order one line's items horizontally and join with inferred spaces.
fn build_line(mut items: Vec<Item>, line_height: f64, page_is_rtl: bool) -> LayoutLine {
    // Decide line directionality: RTL if the line is RTL-dominant.
    let rtl_items = items.iter().filter(|i| i.is_rtl).count();
    let line_is_rtl = rtl_items * 2 > items.len() || (page_is_rtl && rtl_items > 0);

    // Always order left-to-right by x for gap analysis; reverse the join for RTL.
    items.sort_by(|a, b| {
        a.bbox
            .x0
            .partial_cmp(&b.bbox.x0)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let boxes: Vec<BBox> = items.iter().map(|i| i.bbox).collect();
    let bbox = BBox::bounding(&boxes).unwrap_or(BBox {
        x0: 0.0,
        y0: 0.0,
        x1: 0.0,
        y1: 0.0,
    });

    // Word-gap threshold: a gap wider than WORD_GAP_EM of the line font size inserts a
    // space (document-relative, matching the default reader's word_gap_factor).
    let word_gap = crate::text::reading_order::WORD_GAP_EM * line_height;
    let mut ltr_text = String::new();
    let mut prev_right: Option<f64> = None;
    for it in &items {
        if let Some(pr) = prev_right {
            let gap = it.bbox.x0 - pr;
            if gap > word_gap && !ltr_text.ends_with(' ') {
                ltr_text.push(' ');
            }
        }
        ltr_text.push_str(&it.text);
        prev_right = Some(it.bbox.x1);
    }

    let text = if line_is_rtl {
        // Reverse the visual order of space-separated tokens for RTL display.
        ltr_text.split(' ').rev().collect::<Vec<_>>().join(" ")
    } else {
        ltr_text
    };

    LayoutLine {
        text: text.trim().to_string(),
        bbox,
        is_rtl: line_is_rtl,
    }
}

fn center(a: f64, b: f64) -> f64 {
    (a + b) / 2.0
}

fn median_font_size(items: &[Item]) -> f64 {
    let mut sizes: Vec<f64> = items.iter().map(|i| i.font_size).collect();
    sizes.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    sizes.get(sizes.len() / 2).copied().unwrap_or(1.0)
}

/// Estimate the typical inter-line pitch (centre-to-centre vertical distance
/// between vertically-adjacent text rows). Computed by first clustering items
/// into rough rows (by y-centre), then taking the median gap between
/// consecutive row centres. Falls back to ~1.2× line height when there are too
/// few rows to measure.
fn estimate_line_pitch(items: &[Item], line_height: f64) -> f64 {
    // Collect distinct row centres (quantised to avoid duplicates within a row).
    let mut centers: Vec<f64> = items.iter().map(|i| center(i.bbox.y0, i.bbox.y1)).collect();
    centers.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));

    let row_tol = 0.5 * line_height;
    let mut rows: Vec<f64> = Vec::new();
    for c in centers {
        if rows.last().map(|r| (r - c).abs() > row_tol).unwrap_or(true) {
            rows.push(c);
        }
    }
    if rows.len() < 2 {
        return line_height * 1.2;
    }
    let mut gaps: Vec<f64> = rows.windows(2).map(|w| (w[0] - w[1]).abs()).collect();
    gaps.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    // Use a LOW percentile (25th), not the median: the typical body line pitch
    // is the most common *small* gap, and a median is pulled upward by sparse
    // paragraph/section gaps (which we explicitly do NOT want to treat as
    // normal line spacing — those are the block boundaries we want to cut at).
    let idx = gaps.len() / 4;
    let pitch = gaps[idx];
    if pitch > 0.0 {
        pitch
    } else {
        line_height * 1.2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(text: &str, x: f64, y: f64, w: f64, fs: f64) -> TextChunk {
        TextChunk {
            text: text.to_string(),
            x,
            y,
            font_size: fs,
            font_name: "F".into(),
            width: w,
            is_rtl: false,
            is_vertical: false,
            is_invisible: false,
            is_actual_text: false,
            mapping_sources: Vec::new(),
            glyph_run: None,
        }
    }

    fn rtl_chunk(text: &str, x: f64, y: f64, w: f64, fs: f64) -> TextChunk {
        let mut c = chunk(text, x, y, w, fs);
        c.is_rtl = true;
        c
    }

    #[test]
    fn single_column_reads_top_to_bottom() {
        // Three lines stacked vertically (PDF y decreases down the page).
        let chunks = vec![
            chunk("First line", 50.0, 700.0, 100.0, 10.0),
            chunk("Second line", 50.0, 685.0, 100.0, 10.0),
            chunk("Third line", 50.0, 670.0, 100.0, 10.0),
        ];
        let layout = analyze_page(&chunks, &LayoutConfig::default());
        let text = layout.text();
        let first = text.find("First").unwrap();
        let second = text.find("Second").unwrap();
        let third = text.find("Third").unwrap();
        assert!(
            first < second && second < third,
            "top-to-bottom order: {text:?}"
        );
    }

    #[test]
    fn two_columns_read_in_column_order_not_interleaved() {
        // Two clear columns separated by a wide gutter. Left column at x≈50,
        // right column at x≈350 (gutter ~250pt >> line height). Each column has
        // two stacked lines. Correct reading order: L1, L2, then R1, R2 — NOT
        // L1,R1,L2,R2 (which is what a naive top-to-bottom dump produces).
        let chunks = vec![
            chunk("Left top", 50.0, 700.0, 120.0, 10.0),
            chunk("Right top", 350.0, 700.0, 120.0, 10.0),
            chunk("Left bottom", 50.0, 680.0, 120.0, 10.0),
            chunk("Right bottom", 350.0, 680.0, 120.0, 10.0),
        ];
        let layout = analyze_page(&chunks, &LayoutConfig::default());
        let text = layout.text();
        let lt = text.find("Left top").unwrap();
        let lb = text.find("Left bottom").unwrap();
        let rt = text.find("Right top").unwrap();
        let rb = text.find("Right bottom").unwrap();
        // Left column fully precedes the right column.
        assert!(lt < lb, "left column top-to-bottom: {text:?}");
        assert!(
            lb < rt,
            "left column before right column (not interleaved): {text:?}"
        );
        assert!(rt < rb, "right column top-to-bottom: {text:?}");
    }

    #[test]
    fn header_two_columns_footer_segment_correctly() {
        // A full-width header, two columns below it, then a full-width footer.
        // Reading order: header, left col, right col, footer.
        let chunks = vec![
            chunk("HEADER full width", 50.0, 750.0, 400.0, 12.0),
            chunk("Left A", 50.0, 700.0, 120.0, 10.0),
            chunk("Left B", 50.0, 685.0, 120.0, 10.0),
            chunk("Right A", 350.0, 700.0, 120.0, 10.0),
            chunk("Right B", 350.0, 685.0, 120.0, 10.0),
            chunk("FOOTER full width", 50.0, 600.0, 400.0, 10.0),
        ];
        let layout = analyze_page(&chunks, &LayoutConfig::default());
        let text = layout.text();
        let h = text.find("HEADER").unwrap();
        let la = text.find("Left A").unwrap();
        let lb = text.find("Left B").unwrap();
        let ra = text.find("Right A").unwrap();
        let f = text.find("FOOTER").unwrap();
        assert!(h < la, "header before columns");
        assert!(la < lb && lb < ra, "left col before right col");
        assert!(ra < f, "columns before footer: {text:?}");
    }

    #[test]
    fn rtl_two_columns_read_right_to_left() {
        // An RTL-dominant page with two columns: the RIGHT column is read first.
        let chunks = vec![
            rtl_chunk("\u{05D0}\u{05D1}", 50.0, 700.0, 120.0, 10.0), // left-col, top
            rtl_chunk("\u{05D2}\u{05D3}", 350.0, 700.0, 120.0, 10.0), // right-col, top
            rtl_chunk("\u{05D4}\u{05D5}", 50.0, 680.0, 120.0, 10.0), // left-col, bottom
            rtl_chunk("\u{05D6}\u{05D7}", 350.0, 680.0, 120.0, 10.0), // right-col, bottom
        ];
        let layout = analyze_page(&chunks, &LayoutConfig::default());
        // Two blocks (two columns); the first block in reading order is the
        // RIGHT column (larger x).
        assert!(layout.blocks.len() >= 2, "should find two columns");
        let first = &layout.blocks[0];
        let second = &layout.blocks[1];
        assert!(
            first.bbox.x0 > second.bbox.x0,
            "RTL page: right column (larger x) read first: {:.0} vs {:.0}",
            first.bbox.x0,
            second.bbox.x0
        );
    }

    #[test]
    fn paragraph_break_splits_blocks() {
        // Two paragraphs in one column separated by a wide vertical gap.
        let chunks = vec![
            chunk("Para one line one", 50.0, 700.0, 150.0, 10.0),
            chunk("Para one line two", 50.0, 688.0, 150.0, 10.0),
            // Large gap (~40pt) before the next paragraph.
            chunk("Para two line one", 50.0, 640.0, 150.0, 10.0),
            chunk("Para two line two", 50.0, 628.0, 150.0, 10.0),
        ];
        let layout = analyze_page(&chunks, &LayoutConfig::default());
        assert!(
            layout.blocks.len() >= 2,
            "a wide vertical gap should split paragraphs into separate blocks, got {}",
            layout.blocks.len()
        );
    }

    #[test]
    fn overprinted_fake_bold_runs_are_kept_once() {
        // "Title" painted three times 0.3pt apart (fake bold), then text.
        let chunks = vec![
            chunk("Title", 50.0, 700.0, 30.0, 10.0),
            chunk("Title", 50.3, 700.0, 30.0, 10.0),
            chunk("Title", 50.6, 700.1, 30.0, 10.0),
            chunk("body", 50.0, 686.0, 25.0, 10.0),
            // The same word elsewhere on the line is not a duplicate.
            chunk("Title", 85.0, 700.0, 30.0, 10.0),
        ];
        let layout = analyze_page(&chunks, &LayoutConfig::default());
        assert_eq!(layout.text(), "Title Title\nbody");
    }

    #[test]
    fn empty_input_yields_empty_layout() {
        let layout = analyze_page(&[], &LayoutConfig::default());
        assert!(layout.blocks.is_empty());
        assert_eq!(layout.text(), "");
    }

    #[test]
    fn words_join_with_spaces_within_a_line() {
        let chunks = vec![
            chunk("Hello", 50.0, 700.0, 30.0, 10.0),
            chunk("World", 90.0, 700.0, 30.0, 10.0), // ~10pt gap > word gap
        ];
        let layout = analyze_page(&chunks, &LayoutConfig::default());
        assert_eq!(layout.text(), "Hello World");
    }

    fn vertical_chunk(text: &str, x: f64, y: f64, fs: f64) -> TextChunk {
        TextChunk {
            is_vertical: true,
            ..chunk(text, x, y, fs, fs)
        }
    }

    #[test]
    fn vertical_runs_are_kept_on_mixed_pages() {
        // A horizontal heading on top, two vertical columns below it (read
        // right column first), and a horizontal footer at the bottom.
        let chunks = vec![
            chunk("Heading", 50.0, 760.0, 60.0, 12.0),
            vertical_chunk("\u{4E00}", 300.0, 700.0, 12.0),
            vertical_chunk("\u{4E8C}", 300.0, 688.0, 12.0),
            vertical_chunk("\u{4E09}", 284.0, 700.0, 12.0),
            vertical_chunk("\u{56DB}", 284.0, 688.0, 12.0),
            chunk("Footer", 50.0, 100.0, 50.0, 12.0),
        ];
        let layout = analyze_page(&chunks, &LayoutConfig::default());
        let text = layout.text();
        assert_eq!(
            text, "Heading\n\n\u{4E00}\u{4E8C}\n\u{4E09}\u{56DB}\n\nFooter",
            "vertical text must survive between the horizontal blocks: {text:?}"
        );
    }

    #[test]
    fn vertical_only_page_still_produces_text() {
        let chunks = vec![
            vertical_chunk("\u{4E00}", 300.0, 700.0, 12.0),
            vertical_chunk("\u{4E8C}", 300.0, 688.0, 12.0),
        ];
        let layout = analyze_page(&chunks, &LayoutConfig::default());
        assert_eq!(layout.text(), "\u{4E00}\u{4E8C}");
    }
}
