//! Approved rectangular topology and PDF-native table pagination. Cell values,
//! source ownership and row rules are explicit input, never extraction guesses.
use super::*;
use crate::advanced_editing::StoryDecoration;
use crate::fonts::line_layout::{LineMetrics, MeasuredLine, PreparedParagraph};
use crate::typed_tables::TableValue;

#[path = "table_paint.rs"]
mod paint;
pub(crate) use paint::{detach_source_paint, validate_source};
#[path = "table_blocks.rs"]
mod blocks;
#[path = "table_fragmentation.rs"]
mod fragmentation;
use blocks::{CellBlock, FlowLine};

fn one() -> usize {
    1
}
fn padding() -> [f64; 4] {
    [3.0; 4]
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableRow {
    pub id: String,
    #[serde(default)]
    pub min_height: f64,
    #[serde(default)]
    pub allow_split: bool,
    #[serde(default)]
    pub break_before: bool,
    #[serde(default)]
    pub keep_with_next: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableCell {
    /// Stable cell identity. With no paragraph_ids, also its paragraph ID.
    pub id: String,
    /// Ordered, independently styled story paragraphs. Empty preserves the
    /// original one-paragraph-per-cell representation.
    #[serde(default)]
    pub paragraph_ids: Vec<String>,
    pub row: usize,
    pub column: usize,
    #[serde(default = "one")]
    pub row_span: usize,
    #[serde(default = "one")]
    pub column_span: usize,
    /// Left, bottom, right, top in PDF points.
    #[serde(default = "padding")]
    pub padding: [f64; 4],
    #[serde(default)]
    pub background: Option<[f64; 3]>,
    /// None uses the paragraph's text; Some is checked against exact evaluation.
    #[serde(default)]
    pub value: Option<TableValue>,
}
impl TableCell {
    pub fn block_ids(&self) -> impl Iterator<Item = &str> {
        self.paragraph_ids
            .iter()
            .map(String::as_str)
            .chain(self.paragraph_ids.is_empty().then_some(self.id.as_str()))
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableBorder {
    pub width: f64,
    pub rgb: [f64; 3],
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourcePaintAction {
    Keep,
    Remove,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourcePaintDecision {
    pub page: usize,
    pub stable_id: String,
    pub action: SourcePaintAction,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableLayout {
    pub column_weights: Vec<f64>,
    pub rows: Vec<TableRow>,
    pub cells: Vec<TableCell>,
    #[serde(default)]
    pub header_rows: usize,
    #[serde(default)]
    pub border: Option<TableBorder>,
    /// Original page-level path occurrences intersecting unowned table frames.
    /// Saved owned frames replace their grid together with their text instead.
    #[serde(default)]
    pub source_paint: Vec<SourcePaintDecision>,
    /// Full approved Table/TR/TH/TD ownership; distinct from paragraph tags.
    #[serde(default)]
    pub tagging: Option<crate::tagged_structure::story::tables::TableTagging>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableCellFragment {
    pub cell_id: String,
    pub row: usize,
    pub column: usize,
    pub row_span: usize,
    pub column_span: usize,
    pub rect: [f64; 4],
    /// Legacy cells use paragraph byte offsets. Explicit paragraph_ids use a
    /// cell-local cursor with one virtual separator after every block (including
    /// empty/final blocks). Separators are not inserted into PDF text. Use
    /// paragraph_fragments for actual source text ranges.
    pub logical_byte_range: [usize; 2],
    #[serde(default)]
    pub paragraph_fragments: Vec<TableParagraphFragment>,
    pub repeated_header: bool,
    pub continued: bool,
    pub line_count: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableParagraphFragment {
    pub paragraph_id: String,
    pub logical_byte_range: [usize; 2],
    pub line_count: usize,
}

pub fn evaluated_values(request: &LinkedStoryRequest) -> Result<BTreeMap<String, String>> {
    let table = request
        .table_layout
        .as_ref()
        .ok_or_else(|| fail("story has no table layout"))?;
    if table.cells.len() > 4096
        || request.paragraphs.len() > 16_384
        || request
            .paragraphs
            .iter()
            .fold(0usize, |n, p| n.saturating_add(p.text.len()))
            > 4_000_000
    {
        return Err(fail("table value input budget exceeded"));
    }
    let paragraphs = request
        .paragraphs
        .iter()
        .map(|p| (p.id.as_str(), p))
        .collect::<BTreeMap<_, _>>();
    if paragraphs.len() != request.paragraphs.len() {
        return Err(fail("duplicate table paragraph identity"));
    }
    let mut owners = BTreeSet::new();
    for cell in &table.cells {
        if cell.paragraph_ids.len() > 4096 {
            return Err(fail("table cell block budget exceeded"));
        }
        let mut length = 0usize;
        for id in cell.block_ids() {
            if !owners.insert(id) {
                return Err(fail("table paragraph belongs to multiple cell blocks"));
            }
            let p = paragraphs
                .get(id)
                .ok_or_else(|| fail("table block paragraph missing"))?;
            length = length.saturating_add(p.text.len()).saturating_add(1);
        }
        if length.saturating_sub(1) > 64_000 {
            return Err(fail("table cell text exceeds value budget"));
        }
    }
    if owners.len() != paragraphs.len() {
        return Err(fail("unowned paragraph in table values"));
    }
    let defaults = table
        .cells
        .iter()
        .map(|c| {
            Ok(TableValue::Text {
                text: c
                    .block_ids()
                    .map(|id| {
                        paragraphs
                            .get(id)
                            .map(|p| p.text.as_str())
                            .ok_or_else(|| fail("table cell paragraph is missing"))
                    })
                    .collect::<Result<Vec<_>>>()?
                    .join("\n"),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let values = table
        .cells
        .iter()
        .zip(&defaults)
        .map(|(c, fallback)| {
            (
                c.id.as_str(),
                c.row,
                c.column,
                c.value.as_ref().unwrap_or(fallback),
            )
        })
        .collect::<Vec<_>>();
    crate::typed_tables::evaluate_values(&request.story_id, &values)
}

/// Explicit draft operation: materialize approved typed values/formulas before
/// requesting a new preview receipt. Does not mutate or authorize a PDF.
pub fn synchronize_values(request: &mut LinkedStoryRequest) -> Result<()> {
    let values = evaluated_values(request)?;
    let mut replacements = BTreeMap::new();
    for cell in &request.table_layout.as_ref().unwrap().cells {
        if cell.value.is_some() {
            let ids = cell.block_ids().collect::<Vec<_>>();
            if ids.len() != 1 || replacements.insert(ids[0], &values[&cell.id]).is_some() {
                return Err(fail("a typed cell value requires one uniquely owned paragraph; mixed blocks must not be flattened"));
            }
        }
    }
    // A rejected draft must not leave partially materialized formula values.
    // Stage only text, not a clone of the potentially large approved font pool.
    let text = request
        .paragraphs
        .iter()
        .map(|p| {
            replacements
                .get(p.id.as_str())
                .map_or_else(|| p.text.clone(), |text| (**text).clone())
        })
        .collect::<Vec<_>>();
    let old = request
        .paragraphs
        .iter_mut()
        .zip(text)
        .map(|(p, text)| std::mem::replace(&mut p.text, text))
        .collect::<Vec<_>>();
    if let Err(error) = validate_topology(request) {
        for (paragraph, text) in request.paragraphs.iter_mut().zip(old) {
            paragraph.text = text;
        }
        return Err(error);
    }
    Ok(())
}

pub(crate) fn validate_topology(request: &LinkedStoryRequest) -> Result<()> {
    let Some(table) = &request.table_layout else {
        return Ok(());
    };
    if table.rows.is_empty()
        || table.rows.len() > 4096
        || table.column_weights.is_empty()
        || table.column_weights.len() > 256
        || table.cells.len() > 4096
        || table.rows.len().saturating_mul(table.column_weights.len()) > 1_048_576
        || table.header_rows > table.rows.len()
        || request.paragraphs.len() > 16_384
    {
        return Err(fail("invalid table topology dimensions/budget"));
    }
    if request.source_tags.is_some() {
        return Err(fail("table semantics need Table/TR/TH/TD subtree ownership, not paragraph-leaf tag migration"));
    }
    crate::tagged_structure::story::tables::validate_config(request)?;
    if request.frames.iter().any(|f| !f.exclusions.is_empty()) {
        return Err(fail("table frames must exclude pinned artwork geometrically; inline exclusion wrapping is not a cell grid"));
    }
    if table
        .column_weights
        .iter()
        .any(|w| !w.is_finite() || *w <= 0.0 || *w > 1e6)
    {
        return Err(fail("invalid table column weights"));
    }
    let colour = |rgb: &[f64; 3]| rgb.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v));
    if table.border.as_ref().is_some_and(|b| {
        !b.width.is_finite() || b.width <= 0.0 || b.width > 100.0 || !colour(&b.rgb)
    }) {
        return Err(fail("invalid table border"));
    }
    let mut row_ids = BTreeSet::new();
    for (i, row) in table.rows.iter().enumerate() {
        if row.id.is_empty()
            || !row_ids.insert(&row.id)
            || !row.min_height.is_finite()
            || row.min_height < 0.0
            || row.min_height > 1e6
            || i + 1 == table.rows.len() && row.keep_with_next
            || i > 0 && row.break_before && table.rows[i - 1].keep_with_next
            || i < table.header_rows && (row.break_before || row.allow_split)
            || i + 1 == table.header_rows && row.keep_with_next
        {
            return Err(fail("invalid/conflicting table row rules"));
        }
    }
    let mut positions = vec![false; table.rows.len() * table.column_weights.len()];
    let paragraphs = request
        .paragraphs
        .iter()
        .map(|p| (p.id.as_str(), p))
        .collect::<BTreeMap<_, _>>();
    let mut ids = BTreeSet::new();
    let mut block_ids = BTreeSet::new();
    if paragraphs.len() != request.paragraphs.len() {
        return Err(fail("duplicate table paragraph identity"));
    }
    for cell in &table.cells {
        if cell.paragraph_ids.len() > 4096 {
            return Err(fail("table cell block budget exceeded"));
        }
        let blocks = cell.block_ids().collect::<Vec<_>>();
        for (ordinal, id) in blocks.iter().enumerate() {
            let p = paragraphs
                .get(id)
                .ok_or_else(|| fail("table cell paragraph missing"))?;
            if !block_ids.insert(*id)
                || p.break_before
                || !p.page_break_before.is_none()
                || p.text.chars().any(crate::fonts::hard_break::is_form_feed)
                || p.keep_with_next && ordinal + 1 == blocks.len()
            {
                return Err(fail("table paragraphs require unique ownership; physical/forced page breaks use row rules; final block cannot keep with an absent successor"));
            }
        }
        if cell.id.is_empty()
            || !ids.insert(&cell.id)
            || cell.row_span == 0
            || cell.column_span == 0
            || cell
                .row
                .checked_add(cell.row_span)
                .is_none_or(|n| n > table.rows.len())
            || cell
                .column
                .checked_add(cell.column_span)
                .is_none_or(|n| n > table.column_weights.len())
            || cell
                .padding
                .iter()
                .any(|v| !v.is_finite() || *v < 0.0 || *v > 1e5)
            || cell.background.as_ref().is_some_and(|rgb| !colour(rgb))
            || cell.row < table.header_rows && cell.row + cell.row_span > table.header_rows
            || cell.value.is_some() && blocks.len() != 1
        {
            return Err(fail(
                "invalid table cell span/style or mixed-block typed value",
            ));
        }
        for r in cell.row..cell.row + cell.row_span {
            if r > cell.row && table.rows[r].break_before {
                return Err(fail("row break splits a merged cell"));
            }
            for c in cell.column..cell.column + cell.column_span {
                let position = r * table.column_weights.len() + c;
                if std::mem::replace(&mut positions[position], true) {
                    return Err(fail("overlapping merged table cells"));
                }
            }
        }
    }
    if block_ids.len() != paragraphs.len() {
        return Err(fail(
            "every table paragraph must belong to exactly one cell",
        ));
    }
    if positions.iter().any(|v| !v) {
        return Err(fail(
            "table topology has uncovered grid slots; include explicit empty cells",
        ));
    }
    let values = evaluated_values(request)?;
    if table
        .cells
        .iter()
        .filter(|c| c.value.is_some())
        .any(|c| values.get(&c.id) != Some(&paragraphs[c.block_ids().next().unwrap()].text))
    {
        return Err(fail("table paragraph text differs from typed values; synchronize values and request a new preview"));
    }
    Ok(())
}

#[derive(Clone)]
struct LineSlice<T> {
    data: std::sync::Arc<[T]>,
    start: usize,
}
impl<T> std::ops::Deref for LineSlice<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        &self.data[self.start..]
    }
}
#[derive(Clone)]
struct CachedCell {
    lines: std::sync::Arc<[MeasuredLine]>,
    metrics: std::sync::Arc<[LineMetrics]>,
    steps: std::sync::Arc<[f64]>,
    prefix: std::sync::Arc<[f64]>,
}
struct CellLines {
    cell: usize,
    lines: LineSlice<FlowLine>,
    from: usize,
    started: bool,
}
struct Group {
    range: std::ops::Range<usize>,
    cells: Vec<CellLines>,
    prefix: Vec<f64>,
}
struct Prepared<'a> {
    request: &'a LinkedStoryRequest,
    table: &'a TableLayout,
    fonts: &'a [ApprovedFontAsset],
    indices: &'a [usize],
    paragraphs: Vec<PreparedParagraph<'a>>,
    spans: Vec<Vec<crate::fonts::fallback::FontSpan>>,
    font_metrics: Vec<Option<crate::fonts::line_layout::PreparedFontMetrics<'a>>>,
    cell_blocks: Vec<Vec<CellBlock>>,
    cell_lengths: Vec<usize>,
    emitted_lines: std::cell::Cell<usize>,
    emitted_cells: std::cell::Cell<usize>,
    measured: std::cell::RefCell<BTreeMap<(usize, u64), CachedCell>>,
    cached_lines: std::cell::Cell<usize>,
    flows: std::cell::RefCell<BTreeMap<(usize, u64), blocks::CachedFlow>>,
    cached_flow_lines: std::cell::Cell<usize>,
}
impl<'a> Prepared<'a> {
    fn cell_lines(
        &self,
        paragraph: usize,
        from: usize,
        width: f64,
    ) -> Result<(
        LineSlice<MeasuredLine>,
        LineSlice<LineMetrics>,
        LineSlice<f64>,
        f64,
    )> {
        let p = &self.request.paragraphs[paragraph];
        let key = (paragraph, width.to_bits());
        let cached = self.measured.borrow().get(&key).and_then(|cached| {
            let start = if from == p.text.len() {
                Some(cached.lines.len())
            } else {
                cached
                    .lines
                    .binary_search_by_key(&from, |line| line.bytes.start)
                    .ok()
            }?;
            Some((cached.clone(), start))
        });
        let (cached, start) = if let Some(cached) = cached {
            cached
        } else {
            let lines = break_story_lines(
                &self.paragraphs[paragraph],
                &self.spans[paragraph],
                self.fonts,
                &self.font_metrics,
                p,
                self.request.writing_mode,
                from,
                width,
                100_000,
            )?;
            if lines.last().map_or(from, |l| l.bytes.end) != p.text.len() {
                return Err(fail("table cell line budget exceeded"));
            }
            let metrics = lines
                .iter()
                .map(|l| {
                    measure_story_line(
                        &self.paragraphs[paragraph],
                        &self.spans[paragraph],
                        self.fonts,
                        &self.font_metrics,
                        p,
                        self.request.writing_mode,
                        l.bytes.clone(),
                    )
                })
                .collect::<Result<Vec<_>>>()?;
            let steps = metrics
                .iter()
                .map(|m| p.line_height.max(m.ascent + m.descent))
                .collect::<Vec<_>>();
            let mut prefix = vec![0.0];
            for step in &steps {
                prefix.push(prefix.last().unwrap() + step);
            }
            let entry = CachedCell {
                lines: lines.into(),
                metrics: metrics.into(),
                steps: steps.into(),
                prefix: prefix.into(),
            };
            let mut cache = self.measured.borrow_mut();
            let old = cache.remove(&key).map_or(0, |v| v.lines.len());
            let retained = self.cached_lines.get().saturating_sub(old);
            if retained.saturating_add(entry.lines.len()) > 200_000 || cache.len() >= 8192 {
                cache.clear();
                self.cached_lines.set(0);
            } else {
                self.cached_lines.set(retained);
            }
            self.cached_lines
                .set(self.cached_lines.get() + entry.lines.len());
            cache.insert(key, entry.clone());
            (entry, 0)
        };
        let height = cached.prefix.last().unwrap() - cached.prefix[start];
        Ok((
            LineSlice {
                data: cached.lines,
                start,
            },
            LineSlice {
                data: cached.metrics,
                start,
            },
            LineSlice {
                data: cached.steps,
                start,
            },
            height,
        ))
    }
    fn columns(&self, frame: &StoryFrame) -> Result<Vec<f64>> {
        let inset = self.table.border.as_ref().map_or(0.0, |b| b.width / 2.0);
        let width = frame.rect[2] - frame.rect[0] - inset * 2.0;
        if width <= 0.0 {
            return Err(fail("table border exceeds frame width"));
        }
        let total = self.table.column_weights.iter().sum::<f64>();
        let mut x = vec![frame.rect[0] + inset];
        for weight in &self.table.column_weights {
            x.push(x.last().unwrap() + width * weight / total);
        }
        Ok(x)
    }
    fn group(
        &self,
        range: std::ops::Range<usize>,
        columns: &[f64],
        offsets: &BTreeMap<usize, usize>,
        height_used: f64,
    ) -> Result<Group> {
        let mut cells = Vec::new();
        let mut scratch_lines = 0usize;
        let mut endings: Vec<Vec<(usize, f64)>> = vec![Vec::new(); range.len() + 1];
        for (i, cell) in self
            .table
            .cells
            .iter()
            .enumerate()
            .filter(|(_, c)| c.row < range.end && c.row + c.row_span > range.start)
        {
            crate::cancel::check_current_cancel("table cell layout")?;
            let from = offsets.get(&i).copied().unwrap_or(0);
            let started = offsets.contains_key(&i);
            let width = columns[cell.column + cell.column_span]
                - columns[cell.column]
                - cell.padding[0]
                - cell.padding[2];
            if width <= 0.0 {
                return Err(fail("cell padding leaves no text width"));
            }
            let (lines, line_height) = self.flow_lines(i, from, started, width)?;
            scratch_lines = scratch_lines.saturating_add(lines.len());
            if scratch_lines > 200_000 {
                return Err(fail("table row-group shaping budget exceeded"));
            }
            let height = line_height + cell.padding[1] + cell.padding[3];
            endings[cell.row + cell.row_span - range.start]
                .push((cell.row.saturating_sub(range.start), height));
            cells.push(CellLines {
                cell: i,
                lines,
                from,
                started,
            });
        }
        // Forward difference constraints: prefix[j] >= prefix[i] + required.
        // Longest paths in this acyclic row-boundary graph give a deterministic
        // minimum total height satisfying every row and rowspan lower bound.
        let mut prefix: Vec<f64> = vec![0.0; range.len() + 1];
        for end in 1..prefix.len() {
            let minimum = (self.table.rows[range.start + end - 1].min_height
                - if end == 1 { height_used } else { 0.0 })
            .max(0.0);
            prefix[end] = prefix[end - 1] + minimum;
            for &(start, required) in &endings[end] {
                prefix[end] = prefix[end].max(prefix[start] + required);
            }
        }
        cells.sort_by_key(|c| {
            (
                self.table.cells[c.cell].row,
                self.table.cells[c.cell].column,
            )
        });
        Ok(Group {
            range,
            cells,
            prefix,
        })
    }
    fn paint(
        &self,
        frame: &mut StoryFrameLayout,
        group: &Group,
        columns: &[f64],
        top: f64,
        repeated: bool,
        takes: Option<&[usize]>,
        fragment_height: Option<f64>,
    ) -> Result<()> {
        for (ordinal, measured) in group.cells.iter().enumerate() {
            let cell = &self.table.cells[measured.cell];
            let first = cell.row.saturating_sub(group.range.start);
            let last = cell.row + cell.row_span - group.range.start;
            let end_height = group.prefix[last].min(fragment_height.unwrap_or(f64::INFINITY));
            if end_height <= group.prefix[first] + 1e-7 {
                continue;
            }
            let rect = [
                columns[cell.column],
                top - end_height,
                columns[cell.column + cell.column_span],
                top - group.prefix[first],
            ];
            let take = takes.map_or(measured.lines.len(), |t| t[ordinal]);
            self.emitted_lines
                .set(self.emitted_lines.get().saturating_add(take));
            self.emitted_cells
                .set(self.emitted_cells.get().saturating_add(1));
            if self.emitted_lines.get() > 200_000 || self.emitted_cells.get() > 100_000 {
                return Err(fail("table output line/fragment budget exceeded"));
            }
            let end = measured
                .lines
                .get(take.wrapping_sub(1))
                .map_or(measured.from, |l| l.cell_end);
            if rect[3] <= rect[1] {
                return Err(fail("zero-height table fragment"));
            }
            if let Some(rgb) = cell.background {
                frame.decorations.push(StoryDecoration::Fill { rect, rgb });
            }
            let mut y = rect[3] - cell.padding[3];
            let mut paragraph_fragments: Vec<TableParagraphFragment> = Vec::new();
            let mut painted_lines = 0;
            for i in 0..take {
                let line = &measured.lines[i];
                let p = &self.request.paragraphs[line.paragraph];
                let metric = line.metric;
                y -= line.before;
                if y - line.advance - line.after < rect[1] + cell.padding[1] - 1e-7 {
                    return Err(fail("table paragraph spacing exceeds cell fragment bounds"));
                }
                if let Some(part) = paragraph_fragments
                    .last_mut()
                    .filter(|part| part.paragraph_id == p.id)
                {
                    if part.logical_byte_range[1] != line.bytes.start {
                        return Err(fail("noncontiguous table paragraph fragment"));
                    }
                    part.logical_byte_range[1] = line.bytes.end;
                    part.line_count += usize::from(!line.empty);
                } else {
                    paragraph_fragments.push(TableParagraphFragment {
                        paragraph_id: p.id.clone(),
                        logical_byte_range: [line.bytes.start, line.bytes.end],
                        line_count: usize::from(!line.empty),
                    });
                }
                if line.empty {
                    y -= line.advance + line.after;
                    continue;
                }
                let baseline = y - metric.ascent;
                if baseline - metric.descent < rect[1] + cell.padding[1] - 1e-7 {
                    return Err(fail("table line exceeds cell fragment bounds"));
                }
                let tab_plan = crate::linked_stories::story_tab_plan(
                    &self.paragraphs[line.paragraph],
                    &self.spans[line.paragraph],
                    self.fonts,
                    &self.font_metrics,
                    p,
                    self.request.writing_mode,
                    line.bytes.clone(),
                )?;
                frame.lines.push(StoryPaintLine {
                    writing_mode: self.request.writing_mode,
                    text: p.text[line.bytes.clone()].to_owned(),
                    x: rect[0] + cell.padding[0] + metric.left_pad,
                    baseline,
                    width: rect[2]
                        - rect[0]
                        - cell.padding[0]
                        - cell.padding[2]
                        - metric.left_pad
                        - metric.right_pad,
                    font_size: p.font_size,
                    font_index: self.indices[line.paragraph],
                    font_spans: crate::fonts::fallback::slice_spans(
                        &self.spans[line.paragraph],
                        line.bytes.clone(),
                    ),
                    style_spans: line_paint_styles(
                        p,
                        line.bytes.start
                            ..line.bytes.start
                                + p.text[line.bytes.clone()]
                                    .trim_end_matches(crate::fonts::hard_break::is_hard_break)
                                    .len(),
                    ),
                    tab_segments: tab_plan.segments,
                    tab_decorations: tab_plan.decorations,
                    rgb: p.rgb,
                    rtl: p.rtl,
                    bidi: Some(
                        self.paragraphs[line.paragraph].bidi.line(
                            line.bytes.start
                                ..line.bytes.start
                                    + p.text[line.bytes.clone()]
                                        .trim_end_matches(crate::fonts::hard_break::is_hard_break)
                                        .len(),
                        )?,
                    ),
                    shaping: p.shaping.clone(),
                    tag_owner: None,
                    artifact: repeated,
                });
                frame.paragraph_ids.push(p.id.clone());
                painted_lines += 1;
                y -= line.advance + line.after;
            }
            frame.table_cells.push(TableCellFragment {
                cell_id: cell.id.clone(),
                row: cell.row,
                column: cell.column,
                row_span: cell.row_span,
                column_span: cell.column_span,
                rect,
                logical_byte_range: [measured.from, end],
                paragraph_fragments,
                repeated_header: repeated,
                continued: measured.started
                    || end_height < group.prefix[last] - 1e-7
                    || measured.from != 0
                    || end != self.cell_lengths[measured.cell],
                line_count: painted_lines,
            });
        }
        Ok(())
    }
}

fn groups(table: &TableLayout) -> Vec<std::ops::Range<usize>> {
    let mut cross = vec![false; table.rows.len()];
    for cell in &table.cells {
        for boundary in cell.row + 1..cell.row + cell.row_span {
            cross[boundary] = true;
        }
    }
    for i in table.header_rows..table.rows.len().saturating_sub(1) {
        cross[i + 1] |= table.rows[i].keep_with_next;
    }
    let mut out = Vec::new();
    let mut start = table.header_rows;
    for end in table.header_rows + 1..=table.rows.len() {
        if end == table.rows.len() || !cross[end] {
            out.push(start..end);
            start = end;
        }
    }
    out
}

fn add_borders(frame: &mut StoryFrameLayout, border: &TableBorder) {
    // Union collinear intervals so shared/merged cell edges paint only once,
    // including a long rowspan edge adjacent to several shorter cell edges.
    let mut edges: BTreeMap<(bool, u64), Vec<(f64, f64)>> = BTreeMap::new();
    for c in &frame.table_cells {
        let [left, bottom, right, top] = c.rect;
        for y in [bottom, top] {
            edges
                .entry((true, y.to_bits()))
                .or_default()
                .push((left, right));
        }
        for x in [left, right] {
            edges
                .entry((false, x.to_bits()))
                .or_default()
                .push((bottom, top));
        }
    }
    for ((horizontal, coordinate), mut intervals) in edges {
        intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut merged: Vec<(f64, f64)> = Vec::new();
        for (start, end) in intervals {
            if let Some(last) = merged.last_mut().filter(|last| start <= last.1 + 1e-7) {
                last.1 = last.1.max(end);
            } else {
                merged.push((start, end));
            }
        }
        let coordinate = f64::from_bits(coordinate);
        for (start, end) in merged {
            frame.decorations.push(StoryDecoration::Stroke {
                from: if horizontal {
                    [start, coordinate]
                } else {
                    [coordinate, start]
                },
                to: if horizontal {
                    [end, coordinate]
                } else {
                    [coordinate, end]
                },
                width: border.width,
                rgb: border.rgb,
            });
        }
    }
}

pub(crate) fn layout_table(
    request: &LinkedStoryRequest,
    fonts: &[ApprovedFontAsset],
    indices: &[usize],
    choices: Vec<StoryFontChoice>,
) -> Result<LinkedStoryPreview> {
    validate_topology(request)?;
    let table = request
        .table_layout
        .as_ref()
        .ok_or_else(|| fail("table layout missing"))?;
    let paragraphs = request
        .paragraphs
        .iter()
        .map(|p| {
            PreparedParagraph::with_break_settings(
                &p.text,
                ShapeOptions {
                    direction: Some(if p.rtl {
                        TextDirection::RightToLeft
                    } else {
                        TextDirection::LeftToRight
                    }),
                },
                &p.line_break,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let (cell_blocks, cell_lengths) = blocks::prepare_blocks(request)?;
    let spans = request
        .paragraphs
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let spans = choices
                .iter()
                .filter(|c| c.paragraph_id == p.id)
                .filter_map(|c| {
                    c.logical_byte_range
                        .map(|range| crate::fonts::fallback::FontSpan {
                            range,
                            font_index: c.font_index,
                        })
                })
                .collect::<Vec<_>>();
            if spans.is_empty() {
                vec![crate::fonts::fallback::FontSpan {
                    range: [0, p.text.len()],
                    font_index: indices[i],
                }]
            } else {
                spans
            }
        })
        .collect::<Vec<_>>();
    let used_font_indices = spans
        .iter()
        .flatten()
        .map(|span| span.font_index)
        .collect::<BTreeSet<_>>();
    let font_metrics = fonts
        .iter()
        .enumerate()
        .map(|(index, font)| {
            if !used_font_indices.contains(&index) {
                return Ok(None);
            }
            crate::cancel::check_current_cancel("table font metrics preparation")?;
            crate::fonts::line_layout::PreparedFontMetrics::new(&font.bytes).map(Some)
        })
        .collect::<Result<Vec<_>>>()?;
    let prepared = Prepared {
        request,
        table,
        fonts,
        indices,
        paragraphs,
        spans,
        font_metrics,
        cell_blocks,
        cell_lengths,
        emitted_lines: std::cell::Cell::new(0),
        emitted_cells: std::cell::Cell::new(0),
        measured: std::cell::RefCell::new(BTreeMap::new()),
        cached_lines: std::cell::Cell::new(0),
        flows: std::cell::RefCell::new(BTreeMap::new()),
        cached_flow_lines: std::cell::Cell::new(0),
    };
    let mut frames = request
        .frames
        .iter()
        .cloned()
        .map(|frame| StoryFrameLayout {
            frame,
            created: false,
            lines: Vec::new(),
            paragraph_ids: Vec::new(),
            decorations: Vec::new(),
            table_cells: Vec::new(),
            figures: Vec::new(),
        })
        .collect::<Vec<_>>();
    let blocks = groups(table);
    let mut block = 0;
    let mut row_start = table.header_rows;
    let mut frame_index = 0;
    let mut generated = 0;
    let mut offsets = BTreeMap::new();
    let mut row_height_used = 0.0;
    let mut headers_painted = 0usize;
    let inset = table.border.as_ref().map_or(0.0, |b| b.width / 2.0);
    loop {
        ensure_story_frame(request, &mut frames, &mut generated, frame_index)?;
        let columns = prepared.columns(&frames[frame_index].frame)?;
        let top = frames[frame_index].frame.rect[3] - inset;
        let bottom = frames[frame_index].frame.rect[1] + inset;
        let header = prepared.group(0..table.header_rows, &columns, &BTreeMap::new(), 0.0)?;
        let header_height = *header.prefix.last().unwrap();
        let mut y = top - header_height;
        let mut painted = false;
        let mut progressed = false;
        if blocks.is_empty() && header_height <= top - bottom + 1e-7 {
            prepared.paint(
                &mut frames[frame_index],
                &header,
                &columns,
                top,
                false,
                None,
                None,
            )?;
            break;
        }
        while block < blocks.len() && y > bottom + 1e-7 {
            crate::cancel::check_current_cancel("table row pagination")?;
            if progressed && table.rows[blocks[block].start].break_before && offsets.is_empty() {
                break;
            }
            let group = prepared.group(
                row_start..blocks[block].end,
                &columns,
                &offsets,
                row_height_used,
            )?;
            let height = *group.prefix.last().unwrap();
            let available = y - bottom;
            if height <= available + 1e-7 {
                if !painted {
                    prepared.paint(
                        &mut frames[frame_index],
                        &header,
                        &columns,
                        top,
                        headers_painted > 0,
                        None,
                        None,
                    )?;
                    headers_painted += 1;
                    painted = true;
                }
                prepared.paint(
                    &mut frames[frame_index],
                    &group,
                    &columns,
                    y,
                    false,
                    None,
                    None,
                )?;
                y -= height;
                block += 1;
                row_start = blocks
                    .get(block)
                    .map_or(table.rows.len(), |range| range.start);
                offsets.clear();
                row_height_used = 0.0;
                progressed = true;
                continue;
            }
            let next_frame = frames
                .get(frame_index + 1)
                .map(|f| &f.frame)
                .unwrap_or_else(|| request.frames.last().unwrap());
            let next_columns = prepared.columns(next_frame)?;
            if let Some(fragment) =
                prepared.fragment(&group, available, &next_columns, row_height_used)?
            {
                if !painted {
                    prepared.paint(
                        &mut frames[frame_index],
                        &header,
                        &columns,
                        top,
                        headers_painted > 0,
                        None,
                        None,
                    )?;
                    headers_painted += 1;
                }
                prepared.paint(
                    &mut frames[frame_index],
                    &group,
                    &columns,
                    y,
                    false,
                    Some(&fragment.takes),
                    Some(fragment.height),
                )?;
                for (c, take) in group.cells.iter().zip(fragment.takes) {
                    let cell = &table.cells[c.cell];
                    if group.prefix[cell.row.saturating_sub(group.range.start)]
                        >= fragment.height - 1e-7
                    {
                        continue;
                    }
                    offsets.insert(
                        c.cell,
                        c.lines
                            .get(take.wrapping_sub(1))
                            .map_or(c.from, |l| l.cell_end),
                    );
                }
                row_start = fragment.next_row;
                row_height_used = fragment.row_height_used;
                progressed = true;
            }
            break;
        }
        if !blocks.is_empty() && block == blocks.len() {
            break;
        }
        if !progressed && frame_index + 1 >= frames.len() {
            return Err(fail("table header and next safe row unit cannot fit the continuation geometry; change columns, row rules or frame geometry"));
        }
        frame_index += 1;
    }
    let mut covered = BTreeMap::<String, usize>::new();
    let mut paragraph_covered = BTreeMap::<String, usize>::new();
    let mut fragment_lines = BTreeMap::<String, Vec<usize>>::new();
    for frame in &mut frames {
        for c in &frame.table_cells {
            if c.repeated_header {
                continue;
            }
            let cursor = covered.entry(c.cell_id.clone()).or_default();
            if c.logical_byte_range[0] != *cursor || c.logical_byte_range[1] < *cursor {
                return Err(fail("table cell source coverage is not contiguous"));
            }
            *cursor = c.logical_byte_range[1];
            for part in &c.paragraph_fragments {
                let cursor = paragraph_covered
                    .entry(part.paragraph_id.clone())
                    .or_default();
                if part.logical_byte_range[0] != *cursor || part.logical_byte_range[1] < *cursor {
                    return Err(fail("table paragraph source coverage is not contiguous"));
                }
                *cursor = part.logical_byte_range[1];
                if part.line_count > 0 {
                    fragment_lines
                        .entry(part.paragraph_id.clone())
                        .or_default()
                        .push(part.line_count);
                }
            }
        }
        if let Some(border) = &table.border {
            add_borders(frame, border);
        }
    }
    if table
        .cells
        .iter()
        .enumerate()
        .any(|(i, c)| covered.get(&c.id).copied() != Some(prepared.cell_lengths[i]))
        || request
            .paragraphs
            .iter()
            .any(|p| paragraph_covered.get(&p.id).copied() != Some(p.text.len()))
    {
        return Err(fail("table pagination lost cell text"));
    }
    for p in &request.paragraphs {
        if let Some(parts) = fragment_lines.get(&p.id).filter(|parts| parts.len() > 1) {
            if parts[..parts.len() - 1].iter().any(|n| *n < p.orphans)
                || parts[1..].iter().any(|n| *n < p.widows)
            {
                return Err(fail("actual table continuation geometry violates cell widow/orphan rules; revise frame widths or row constraints"));
            }
        }
    }
    Ok(LinkedStoryPreview { input_sha256: request.input_sha256.clone(), story_id: request.story_id.clone(),
        changed_pages: frames.iter().map(|f| f.frame.page).collect::<BTreeSet<_>>().into_iter().collect(), frames,
        font_choices: choices, generated_pages: generated, page_breaks: Vec::new(), page_pruning: Default::default(), anchor_moves: Vec::new(), figure_removals: Vec::new(), figure_detachments: Vec::new(), full_page_invalidations: Vec::new(),
        qualification: "source_implementation_only; vps_corpus_gate_pending".into(),
        exact_limits: vec!["Approved table topology; no inferred numeric types/formulas or automatic semantic grouping".into(),
            "Repeated headers are artifacts; rowspans fragment at row/shaped-line boundaries; explicit keep chains stay atomic".into(),
            "Source grid decisions and current frame owners are explicit; table tagging requires approved complete text-cell ownership".into()],
        rebound_frames: Vec::new(), output_sha256: None, reused_paragraphs: 0, checkpoints: Vec::new() })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::authoring::{FontFace, GraphicsStyle, PageSize, PdfBuilder, TextStyle};
    use crate::content::Color;

    pub(crate) fn fixture(grid: bool) -> Vec<u8> {
        let mut builder = PdfBuilder::new();
        let page = builder.add_page(PageSize::custom(220.0, 160.0));
        page.draw_text(
            "ORIGINAL",
            12.0,
            138.0,
            &TextStyle::new(
                FontFace::Standard(crate::authoring::StandardFont::Helvetica),
                10.0,
            ),
        )
        .unwrap();
        if grid {
            page.draw_line(
                10.0,
                90.0,
                200.0,
                90.0,
                &GraphicsStyle::stroke(Color::black(), 1.0),
            );
        }
        builder.to_bytes().unwrap()
    }
    pub(crate) fn request(input: &[u8], long: bool) -> LinkedStoryRequest {
        let model = analyze_multi_run_text_range(input, 1).unwrap();
        let texts = [
            "Name".to_owned(),
            "Amount".to_owned(),
            if long {
                "A lengthy cell with wrapped words. ".repeat(35)
            } else {
                "Short".into()
            },
            "17".to_owned(),
        ];
        let paragraphs = texts.iter().enumerate().map(|(i,text)| serde_json::json!({
            "id":format!("c{i}"),"text":text,"preferred_font":"Helvetica","font_size":10.0,"line_height":12.0,"orphans":1,"widows":1
        })).collect::<Vec<_>>();
        serde_json::from_value(serde_json::json!({
            "story_id":"flowing-table","input_sha256":hash(input),
            "frames":[{"id":"table-source","page":1,"logical_range":[0,model.logical_text.chars().count()],"expected_text":model.logical_text,"rect":[10.0,10.0,200.0,150.0]}],
            "paragraphs":paragraphs,"fonts":[],"allow_font_substitution":true,"allow_page_creation":true,"max_new_pages":64,
            "table_layout":{"column_weights":[2.0,1.0],"header_rows":1,"border":{"width":0.75,"rgb":[0.0,0.0,0.0]},
                "rows":[{"id":"header"},{"id":"body","allow_split":true}],
                "cells":(0..4).map(|i|serde_json::json!({"id":format!("c{i}"),"row":i/2,"column":i%2})).collect::<Vec<_>>()}
        })).unwrap()
    }
    #[test]
    fn table_growth_headers_native_grid_and_reopen_contraction() {
        let input = fixture(false);
        let request = request(&input, true);
        let (output, preview) = apply_linked_story(&input, &request).unwrap();
        assert!(preview.generated_pages > 1);
        let populated = preview
            .frames
            .iter()
            .filter(|f| !f.table_cells.is_empty())
            .collect::<Vec<_>>();
        assert!(populated[0]
            .table_cells
            .iter()
            .filter(|c| c.row == 0)
            .all(|c| !c.repeated_header));
        assert!(populated.iter().skip(1).all(|f| f
            .table_cells
            .iter()
            .filter(|c| c.row == 0)
            .all(|c| c.repeated_header)));
        for frame in &populated {
            assert!(!frame.decorations.is_empty());
            for cell in &frame.table_cells {
                assert!(
                    cell.rect[0] >= frame.frame.rect[0]
                        && cell.rect[1] >= frame.frame.rect[1] - 1e-7
                        && cell.rect[2] <= frame.frame.rect[2] + 1e-7
                        && cell.rect[3] <= frame.frame.rect[3] + 1e-7
                );
            }
        }
        let body = preview
            .frames
            .iter()
            .flat_map(|f| f.lines.iter().zip(&f.paragraph_ids))
            .filter(|(_, id)| id.as_str() == "c2")
            .map(|(line, _)| line.text.as_str())
            .collect::<String>();
        assert_eq!(body, request.paragraphs[2].text);
        let mut saved = load_linked_stories(&output).unwrap().remove(0).request;
        assert!(saved.table_layout.as_ref().unwrap().source_paint.is_empty());
        saved.paragraphs[2].text = "Smaller".into();
        let (short, preview) = apply_linked_story(&output, &saved).unwrap();
        assert_eq!(preview.generated_pages, 0);
        assert!(preview
            .frames
            .iter()
            .skip(1)
            .all(|f| f.table_cells.is_empty() && f.decorations.is_empty() && f.lines.is_empty()));
        assert!(ContentEngine::open_bytes(short.clone())
            .unwrap()
            .get_page_text(1)
            .unwrap()
            .contains("Smaller"));
        assert!(load_linked_stories(&short).unwrap()[0]
            .request
            .table_layout
            .is_some());
    }
    #[test]
    fn source_grid_requires_occurrence_decisions_and_is_replaced_once() {
        let input = fixture(true);
        let mut request = request(&input, false);
        assert!(preview_linked_story(&input, &request).is_err());
        let objects = crate::advanced_editing::list_vector_objects(&input, 1)
            .unwrap()
            .objects;
        assert_eq!(objects.len(), 1);
        request
            .table_layout
            .as_mut()
            .unwrap()
            .source_paint
            .push(SourcePaintDecision {
                page: 1,
                stable_id: objects[0].stable_id.clone(),
                action: SourcePaintAction::Remove,
            });
        let (output, preview) = apply_linked_story(&input, &request).unwrap();
        assert_eq!(preview.generated_pages, 0);
        let objects = crate::advanced_editing::list_vector_objects(&output, 1)
            .unwrap()
            .objects;
        assert!(!objects
            .iter()
            .any(|o| (o.bbox[1] - 90.0).abs() < 1e-7 && (o.bbox[3] - 90.0).abs() < 1e-7));
        let saved = load_linked_stories(&output).unwrap().remove(0).request;
        apply_linked_story(&output, &saved).unwrap();
    }
    #[test]
    fn merged_topology_overlaps_and_header_crossing_are_rejected() {
        let input = fixture(false);
        let mut request = request(&input, false);
        request.table_layout.as_mut().unwrap().cells[0].row_span = 2;
        assert!(validate_topology(&request).is_err());
        request.table_layout.as_mut().unwrap().header_rows = 0;
        assert!(validate_topology(&request).is_err());
        request.table_layout.as_mut().unwrap().cells.remove(2);
        request.paragraphs.remove(2);
        assert!(validate_topology(&request).is_ok());
        let preview = preview_linked_story(&input, &request).unwrap();
        let merged = &preview.frames[0]
            .table_cells
            .iter()
            .find(|c| c.cell_id == "c0")
            .unwrap();
        let others = preview.frames[0]
            .table_cells
            .iter()
            .filter(|c| c.column == 1)
            .collect::<Vec<_>>();
        assert!(
            (merged.rect[3] - merged.rect[1] - (others[0].rect[3] - others[1].rect[1])).abs()
                < 1e-7
        );
    }
    #[test]
    fn minimum_row_height_survives_geometry_only_fragments() {
        let input = fixture(false);
        let mut request = request(&input, false);
        request.table_layout.as_mut().unwrap().rows[1].min_height = 400.0;
        let preview = preview_linked_story(&input, &request).unwrap();
        assert!(preview.generated_pages >= 2);
        let total = preview
            .frames
            .iter()
            .flat_map(|f| &f.table_cells)
            .filter(|c| c.cell_id == "c2")
            .map(|c| c.rect[3] - c.rect[1])
            .sum::<f64>();
        assert!(total >= 400.0 - 1e-7);
    }
    #[test]
    fn rowspan_fragments_preserve_cell_order_minima_and_reopen() {
        let input = fixture(false);
        let mut request = request(&input, true);
        let mut last = request.paragraphs[3].clone();
        last.id = "c4".into();
        last.text = "Final adjacent row".into();
        request.paragraphs.push(last);
        request.paragraphs[3].text = "First adjacent row".into();
        let table = request.table_layout.as_mut().unwrap();
        table.rows[1].min_height = 160.0;
        table.rows.push(TableRow {
            id: "body2".into(),
            min_height: 180.0,
            allow_split: true,
            break_before: false,
            keep_with_next: false,
        });
        table.cells[2].row_span = 2;
        let mut cell = table.cells[3].clone();
        cell.id = "c4".into();
        cell.row = 2;
        table.cells.push(cell);
        let (output, preview) = apply_linked_story(&input, &request).unwrap();
        assert!(preview.generated_pages > 1);
        let fragments = preview
            .frames
            .iter()
            .flat_map(|f| &f.table_cells)
            .filter(|c| c.cell_id == "c2")
            .collect::<Vec<_>>();
        assert!(fragments.len() > 1 && fragments.iter().all(|c| c.continued));
        assert!(fragments.iter().map(|c| c.rect[3] - c.rect[1]).sum::<f64>() >= 340.0 - 1e-7);
        for paragraph in &request.paragraphs {
            let text = preview
                .frames
                .iter()
                .flat_map(|f| f.lines.iter().zip(&f.paragraph_ids))
                .filter(|(line, id)| !line.artifact && *id == &paragraph.id)
                .map(|(line, _)| line.text.as_str())
                .collect::<String>();
            assert_eq!(text, paragraph.text);
        }
        let saved = load_linked_stories(&output).unwrap().remove(0).request;
        apply_linked_story(&output, &saved).unwrap();
    }
    #[test]
    fn rowspan_can_break_between_non_splittable_rows_but_keep_chain_cannot() {
        let input = fixture(false);
        let mut request = request(&input, false);
        request.paragraphs[2].text = "Spanning cell".into();
        let mut last = request.paragraphs[3].clone();
        last.id = "c4".into();
        request.paragraphs.push(last);
        let table = request.table_layout.as_mut().unwrap();
        table.rows[1].allow_split = false;
        table.rows[1].min_height = 90.0;
        table.rows.push(TableRow {
            id: "body2".into(),
            min_height: 90.0,
            allow_split: false,
            break_before: false,
            keep_with_next: false,
        });
        table.cells[2].row_span = 2;
        let mut last = table.cells[3].clone();
        last.id = "c4".into();
        last.row = 2;
        table.cells.push(last);
        let preview = preview_linked_story(&input, &request).unwrap();
        assert_eq!(preview.generated_pages, 1);
        request.table_layout.as_mut().unwrap().rows[1].keep_with_next = true;
        assert!(preview_linked_story(&input, &request).is_err());
    }
    #[test]
    fn rejected_formula_draft_is_unchanged() {
        let input = fixture(false);
        let mut request = request(&input, false);
        request.table_layout.as_mut().unwrap().cells[2].value = Some(TableValue::Text {
            text: "changed".into(),
        });
        request.table_layout.as_mut().unwrap().column_weights[0] = 0.0;
        let before = serde_json::to_value(&request).unwrap();
        assert!(synchronize_values(&mut request).is_err());
        assert_eq!(serde_json::to_value(&request).unwrap(), before);
    }
    #[test]
    fn typed_formulas_are_shared_with_the_fixed_grid_evaluator() {
        let input = fixture(false);
        let mut request = request(&input, false);
        let table = request.table_layout.as_mut().unwrap();
        table.cells[2].value = Some(TableValue::Decimal {
            value: crate::typed_tables::DecimalValue {
                coefficient: "10".into(),
                scale: 2,
            },
        });
        table.cells[3].value = Some(TableValue::Formula {
            expression: crate::typed_tables::TableFormula::Add {
                left: Box::new(crate::typed_tables::TableFormula::Cell { id: "c2".into() }),
                right: Box::new(crate::typed_tables::TableFormula::Constant {
                    value: crate::typed_tables::DecimalValue {
                        coefficient: "20".into(),
                        scale: 2,
                    },
                }),
            },
            display_scale: 2,
        });
        assert!(validate_topology(&request).is_err());
        synchronize_values(&mut request).unwrap();
        assert_eq!(request.paragraphs[3].text, "0.30");
        let (output, _) = apply_linked_story(&input, &request).unwrap();
        let saved = load_linked_stories(&output).unwrap().remove(0).request;
        assert_eq!(evaluated_values(&saved).unwrap()["c3"], "0.30");
    }
    #[test]
    fn inline_styled_table_cell_uses_shared_measurement_paint_and_reopen_model() {
        let input = fixture(false);
        let mut request = request(&input, false);
        request.paragraphs[2].text = "Styled cell".into();
        request.paragraphs[2].inline_styles = vec![StoryInlineStyleSpan {
            logical_range: [0, 6],
            preferred_font: None,
            font_size: Some(14.0),
            rgb: Some([0.7, 0.1, 0.2]),
            shaping: None,
        }];
        request.paragraphs[2].line_height = 17.0;
        let (output, preview) = apply_linked_story(&input, &request).unwrap();
        let styled = preview
            .frames
            .iter()
            .flat_map(|frame| frame.lines.iter().zip(&frame.paragraph_ids))
            .filter(|(_, id)| id.as_str() == "c2")
            .flat_map(|(line, _)| &line.style_spans)
            .collect::<Vec<_>>();
        assert!(styled
            .iter()
            .any(|span| { span.font_size == 14.0 && span.rgb == [0.7, 0.1, 0.2] }));
        let saved = load_linked_stories(&output).unwrap().remove(0).request;
        assert_eq!(
            saved.paragraphs[2].inline_styles,
            request.paragraphs[2].inline_styles
        );
        assert!(ContentEngine::open_bytes(output)
            .unwrap()
            .get_page_text(1)
            .unwrap()
            .contains("Styled cell"));
    }
    #[test]
    fn styled_cell_blocks_empty_paragraph_and_reopen_keep_their_identity() {
        let input = fixture(false);
        let mut request = request(&input, false);
        request.paragraphs[2].text = "Cell heading".into();
        request.paragraphs[2].font_size = 12.0;
        request.paragraphs[2].line_height = 15.0;
        request.paragraphs[2].rgb = [0.6, 0.1, 0.2];
        request.paragraphs[2].keep_with_next = true;
        let mut blank = request.paragraphs[2].clone();
        blank.id = "blank-block".into();
        blank.text.clear();
        blank.line_height = 18.0;
        let mut body = request.paragraphs[3].clone();
        body.id = "body-block".into();
        body.text = "Independently styled body words. ".repeat(30);
        body.preferred_font = "Courier".into();
        body.font_size = 9.0;
        body.line_height = 11.0;
        body.rgb = [0.0, 0.2, 0.7];
        body.space_before = 4.0;
        body.space_after = 7.0;
        let mut tail = body.clone();
        tail.id = "empty-tail".into();
        tail.text.clear();
        request.paragraphs.extend([blank, body, tail]);
        request.table_layout.as_mut().unwrap().cells[2].paragraph_ids = vec![
            "c2".into(),
            "blank-block".into(),
            "body-block".into(),
            "empty-tail".into(),
        ];
        let (output, preview) = apply_linked_story(&input, &request).unwrap();
        assert!(preview.generated_pages > 1);
        for p in &request.paragraphs {
            let lines = preview
                .frames
                .iter()
                .flat_map(|f| f.lines.iter().zip(&f.paragraph_ids))
                .filter(|(line, id)| !line.artifact && *id == &p.id)
                .collect::<Vec<_>>();
            assert_eq!(
                lines
                    .iter()
                    .map(|(line, _)| line.text.as_str())
                    .collect::<String>(),
                p.text
            );
            assert!(lines
                .iter()
                .all(|(line, _)| line.font_size == p.font_size && line.rgb == p.rgb));
            let ranges = preview
                .frames
                .iter()
                .flat_map(|f| &f.table_cells)
                .filter(|c| !c.repeated_header)
                .flat_map(|c| &c.paragraph_fragments)
                .filter(|part| part.paragraph_id == p.id)
                .collect::<Vec<_>>();
            assert!(!ranges.is_empty());
            assert_eq!(ranges.first().unwrap().logical_byte_range[0], 0);
            assert_eq!(ranges.last().unwrap().logical_byte_range[1], p.text.len());
        }
        let first = preview
            .frames
            .iter()
            .find(|f| f.paragraph_ids.iter().any(|id| id == "c2"))
            .unwrap();
        let heading = first
            .lines
            .iter()
            .zip(&first.paragraph_ids)
            .find(|(_, id)| id.as_str() == "c2")
            .unwrap()
            .0;
        let body = first
            .lines
            .iter()
            .zip(&first.paragraph_ids)
            .find(|(_, id)| id.as_str() == "body-block")
            .unwrap()
            .0;
        assert!(heading.baseline - body.baseline > 18.0);
        let mut saved = load_linked_stories(&output).unwrap().remove(0).request;
        assert_eq!(
            saved.table_layout.as_ref().unwrap().cells[2].paragraph_ids,
            request.table_layout.as_ref().unwrap().cells[2].paragraph_ids
        );
        saved
            .paragraphs
            .iter_mut()
            .find(|p| p.id == "body-block")
            .unwrap()
            .text = "Edited again".into();
        let (again, short) = apply_linked_story(&output, &saved).unwrap();
        assert_eq!(short.generated_pages, 0);
        assert!(ContentEngine::open_bytes(again)
            .unwrap()
            .get_page_text(1)
            .unwrap()
            .contains("Edited again"));
    }
    #[test]
    fn cell_block_order_is_not_story_storage_order_and_values_do_not_flatten_it() {
        let input = fixture(false);
        let mut request = request(&input, false);
        let mut block = request.paragraphs[2].clone();
        block.id = "another".into();
        block.text = "Before original".into();
        request.paragraphs.push(block);
        request.table_layout.as_mut().unwrap().cells[2].paragraph_ids =
            vec!["another".into(), "c2".into()];
        let before = serde_json::to_value(&request).unwrap();
        synchronize_values(&mut request).unwrap();
        assert_eq!(serde_json::to_value(&request).unwrap(), before);
        let preview = preview_linked_story(&input, &request).unwrap();
        let order = preview
            .frames
            .iter()
            .flat_map(|f| &f.table_cells)
            .filter(|c| c.cell_id == "c2")
            .flat_map(|c| &c.paragraph_fragments)
            .map(|p| p.paragraph_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(order, vec!["another", "c2"]);
        request.table_layout.as_mut().unwrap().cells[2].value = Some(TableValue::Text {
            text: "Do not flatten".into(),
        });
        let before = serde_json::to_value(&request).unwrap();
        assert!(synchronize_values(&mut request).is_err());
        assert_eq!(serde_json::to_value(&request).unwrap(), before);
    }
    #[test]
    fn aliased_single_block_values_and_duplicate_block_rejection() {
        let input = fixture(false);
        let mut request = request(&input, false);
        request.paragraphs[3].id = "amount-block".into();
        let cell = &mut request.table_layout.as_mut().unwrap().cells[3];
        cell.paragraph_ids = vec!["amount-block".into()];
        cell.value = Some(TableValue::Decimal {
            value: crate::typed_tables::DecimalValue {
                coefficient: "1234".into(),
                scale: 2,
            },
        });
        synchronize_values(&mut request).unwrap();
        assert_eq!(request.paragraphs[3].text, "12.34");
        request.table_layout.as_mut().unwrap().cells[2].paragraph_ids =
            vec!["c2".into(), "amount-block".into()];
        assert!(validate_topology(&request).is_err());
        assert!(evaluated_values(&request).is_err());
    }
    #[test]
    fn empty_blocks_consume_height_and_virtual_cursor_without_fake_text() {
        let input = fixture(false);
        let mut request = request(&input, false);
        request.paragraphs[2].text.clear();
        request.paragraphs[2].line_height = 20.0;
        let mut empty = request.paragraphs[2].clone();
        empty.id = "empty-two".into();
        request.paragraphs.push(empty);
        request.table_layout.as_mut().unwrap().cells[2].paragraph_ids =
            vec!["c2".into(), "empty-two".into()];
        let preview = preview_linked_story(&input, &request).unwrap();
        let cell = preview.frames[0]
            .table_cells
            .iter()
            .find(|c| c.cell_id == "c2")
            .unwrap();
        assert_eq!(cell.logical_byte_range, [0, 2]);
        assert_eq!(cell.paragraph_fragments.len(), 2);
        assert_eq!(cell.line_count, 0);
        assert!(cell.rect[3] - cell.rect[1] >= 46.0 - 1e-7);
        assert!(!preview.frames[0]
            .paragraph_ids
            .iter()
            .any(|id| id == "c2" || id == "empty-two"));
    }
    #[test]
    fn block_keep_chain_moves_to_larger_existing_frame() {
        let input = fixture(false);
        let mut request = request(&input, false);
        request.paragraphs.truncate(2);
        request.paragraphs[0].text = "Heading".into();
        request.paragraphs[0].line_height = 24.0;
        request.paragraphs[0].keep_with_next = true;
        request.paragraphs[1].text = "Body".into();
        request.paragraphs[1].line_height = 30.0;
        request.paragraphs[1].keep_together = true;
        let table = request.table_layout.as_mut().unwrap();
        table.column_weights = vec![1.0];
        table.header_rows = 0;
        table.rows.truncate(1);
        table.rows[0].allow_split = true;
        table.cells.truncate(1);
        table.cells[0].paragraph_ids = vec!["c0".into(), "c1".into()];
        request.frames[0].rect = [10.0, 110.0, 200.0, 150.0];
        let mut next = request.frames[0].clone();
        next.id = "larger-frame".into();
        next.rect = [10.0, 10.0, 200.0, 90.0];
        next.logical_range = [next.logical_range[1]; 2];
        next.expected_text.clear();
        request.frames.push(next);
        request.allow_page_creation = false;
        let preview = preview_linked_story(&input, &request).unwrap();
        assert!(preview.frames[0].lines.is_empty());
        assert_eq!(preview.frames[1].paragraph_ids, vec!["c0", "c1"]);
        assert_eq!(preview.generated_pages, 0);
    }
}
