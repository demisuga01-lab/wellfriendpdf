//! Painted, paginated tables of contents derived from the explicit authored
//! outline. Title and page columns are independently shaped; page values remain
//! deferred until final page ownership is known.
use super::*;

#[cfg(test)]
#[path = "authoring_toc_tests.rs"]
mod tests;

const MAX_TOC_ROWS: usize = 100_000;
const MAX_TOC_LEVEL_STYLES: usize = 128;
const MAX_LEADER_BYTES: usize = 32;
const MAX_LEADER_REPETITIONS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TableOfContentsPageSide {
    Left,
    #[default]
    Right,
}

/// Per-level leader behavior. Inheritance is explicit so a level can disable
/// a document-wide leader without relying on a sentinel string.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TableOfContentsLeaderStyle {
    #[default]
    Inherit,
    Disabled,
    Text(String),
}

/// Exact overrides for one zero-based outline level.
///
/// Unset properties inherit from [`TableOfContentsStyle`]. Duplicate level
/// declarations are rejected rather than silently depending on vector order.
#[derive(Debug, Clone, PartialEq)]
pub struct TableOfContentsLevelStyle {
    pub level: usize,
    pub title_style: Option<TextStyle>,
    pub page_style: Option<TextStyle>,
    pub line_height: Option<f64>,
    /// Absolute indentation from the title-column edge for this level.
    pub indent: Option<f64>,
    pub title_align: Option<TextAlign>,
    pub page_align: Option<TextAlign>,
    pub row_gap: Option<f64>,
    pub leader: TableOfContentsLeaderStyle,
    pub keep_with_previous: Option<bool>,
    pub keep_with_next: Option<bool>,
}

impl TableOfContentsLevelStyle {
    pub fn new(level: usize) -> Self {
        Self {
            level,
            title_style: None,
            page_style: None,
            line_height: None,
            indent: None,
            title_align: None,
            page_align: None,
            row_gap: None,
            leader: TableOfContentsLeaderStyle::Inherit,
            keep_with_previous: None,
            keep_with_next: None,
        }
    }

    pub fn title_style(mut self, style: TextStyle) -> Self {
        self.title_style = Some(style);
        self
    }

    pub fn page_style(mut self, style: TextStyle) -> Self {
        self.page_style = Some(style);
        self
    }

    pub fn line_height(mut self, line_height: f64) -> Self {
        self.line_height = Some(line_height);
        self
    }

    pub fn indent(mut self, indent: f64) -> Self {
        self.indent = Some(indent);
        self
    }

    pub fn title_align(mut self, align: TextAlign) -> Self {
        self.title_align = Some(align);
        self
    }

    pub fn page_align(mut self, align: TextAlign) -> Self {
        self.page_align = Some(align);
        self
    }

    pub fn row_gap(mut self, row_gap: f64) -> Self {
        self.row_gap = Some(row_gap);
        self
    }

    pub fn leader(mut self, leader: impl Into<String>) -> Self {
        self.leader = TableOfContentsLeaderStyle::Text(leader.into());
        self
    }

    pub fn without_leader(mut self) -> Self {
        self.leader = TableOfContentsLeaderStyle::Disabled;
        self
    }

    pub fn keep_with_next(mut self, keep: bool) -> Self {
        self.keep_with_next = Some(keep);
        self
    }

    pub fn keep_with_previous(mut self, keep: bool) -> Self {
        self.keep_with_previous = Some(keep);
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TableOfContentsStyle {
    pub title_style: TextStyle,
    pub page_style: TextStyle,
    pub line_height: f64,
    pub indent_per_level: f64,
    pub page_column_width: f64,
    pub column_gap: f64,
    pub page_side: TableOfContentsPageSide,
    pub title_align: TextAlign,
    pub page_align: TextAlign,
    pub row_gap: f64,
    pub max_page_characters: usize,
    pub leader: Option<String>,
    pub section_page_numbers: bool,
    pub level_styles: Vec<TableOfContentsLevelStyle>,
}

impl Default for TableOfContentsStyle {
    fn default() -> Self {
        Self {
            title_style: TextStyle::unicode(11.0),
            page_style: TextStyle::unicode(11.0),
            line_height: 1.2,
            indent_per_level: 14.0,
            // Eight decimal digits at the default 11 pt Unicode face require
            // slightly more than 42 points. Keep the declared capacity and
            // the default geometry consistent instead of making every default
            // TOC fail during its preflight probe.
            page_column_width: 56.0,
            column_gap: 8.0,
            page_side: TableOfContentsPageSide::Right,
            title_align: TextAlign::Left,
            page_align: TextAlign::Right,
            row_gap: 3.0,
            max_page_characters: 8,
            leader: Some(". ".into()),
            section_page_numbers: false,
            level_styles: Vec::new(),
        }
    }
}

impl TableOfContentsStyle {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_level_style(mut self, style: TableOfContentsLevelStyle) -> Self {
        self.level_styles.push(style);
        self
    }

    fn validate(&self) -> Result<()> {
        validate_text_style(&self.title_style)?;
        validate_text_style(&self.page_style)?;
        if ![
            self.line_height,
            self.indent_per_level,
            self.page_column_width,
            self.column_gap,
            self.row_gap,
        ]
        .iter()
        .all(|value| value.is_finite())
            || self.line_height <= 0.0
            || self.indent_per_level < 0.0
            || self.page_column_width <= 0.0
            || self.column_gap < 0.0
            || self.row_gap < 0.0
            || self.max_page_characters == 0
            || self.max_page_characters > 64
        {
            return Err(WellfriendError::invalid_input(
                "invalid table-of-contents geometry or page capacity",
            ));
        }
        validate_leader(self.leader.as_deref())?;
        if self.level_styles.len() > MAX_TOC_LEVEL_STYLES {
            return Err(WellfriendError::ResourceLimit(
                "table-of-contents level style count".into(),
            ));
        }
        let mut levels = std::collections::BTreeSet::new();
        for override_style in &self.level_styles {
            if let Some(title_style) = &override_style.title_style {
                validate_text_style(title_style)?;
            }
            if let Some(page_style) = &override_style.page_style {
                validate_text_style(page_style)?;
            }
            if override_style.level >= MAX_TOC_LEVEL_STYLES {
                return Err(WellfriendError::ResourceLimit(
                    "table-of-contents styled outline level".into(),
                ));
            }
            if !levels.insert(override_style.level) {
                return Err(WellfriendError::invalid_input(
                    "duplicate table-of-contents level style",
                ));
            }
            if override_style
                .line_height
                .is_some_and(|value| !value.is_finite() || value <= 0.0)
                || override_style
                    .indent
                    .is_some_and(|value| !value.is_finite() || value < 0.0)
                || override_style
                    .row_gap
                    .is_some_and(|value| !value.is_finite() || value < 0.0)
            {
                return Err(WellfriendError::invalid_input(
                    "invalid table-of-contents level geometry",
                ));
            }
            if let TableOfContentsLeaderStyle::Text(leader) = &override_style.leader {
                validate_leader(Some(leader))?;
            }
        }
        Ok(())
    }

    fn resolve(&self, level: usize) -> ResolvedLevelStyle {
        let override_style = self
            .level_styles
            .iter()
            .find(|candidate| candidate.level == level);
        let leader = match override_style.map(|value| &value.leader) {
            Some(TableOfContentsLeaderStyle::Disabled) => None,
            Some(TableOfContentsLeaderStyle::Text(value)) => Some(value.clone()),
            Some(TableOfContentsLeaderStyle::Inherit) | None => self.leader.clone(),
        };
        ResolvedLevelStyle {
            title_style: override_style
                .and_then(|value| value.title_style.clone())
                .unwrap_or_else(|| self.title_style.clone()),
            page_style: override_style
                .and_then(|value| value.page_style.clone())
                .unwrap_or_else(|| self.page_style.clone()),
            line_height: override_style
                .and_then(|value| value.line_height)
                .unwrap_or(self.line_height),
            indent: override_style
                .and_then(|value| value.indent)
                .unwrap_or(self.indent_per_level * level as f64),
            title_align: override_style
                .and_then(|value| value.title_align)
                .unwrap_or(self.title_align),
            page_align: override_style
                .and_then(|value| value.page_align)
                .unwrap_or(self.page_align),
            row_gap: override_style
                .and_then(|value| value.row_gap)
                .unwrap_or(self.row_gap),
            leader,
            keep_with_previous: override_style
                .and_then(|value| value.keep_with_previous)
                .unwrap_or(false),
            keep_with_next: override_style
                .and_then(|value| value.keep_with_next)
                .unwrap_or(false),
        }
    }
}

fn validate_text_style(style: &TextStyle) -> Result<()> {
    if !style.size.is_finite() || style.size <= 0.0 {
        return Err(WellfriendError::invalid_input(
            "table-of-contents font size must be finite and positive",
        ));
    }
    Ok(())
}

fn validate_leader(leader: Option<&str>) -> Result<()> {
    if let Some(leader) = leader {
        if leader.is_empty()
            || leader.len() > MAX_LEADER_BYTES
            || leader
                .chars()
                .any(|ch| ch == '\0' || crate::fonts::hard_break::is_hard_break(ch))
        {
            return Err(WellfriendError::invalid_input(
                "table-of-contents leader must be a bounded nonempty single line",
            ));
        }
    }
    Ok(())
}

#[derive(Clone)]
struct ResolvedLevelStyle {
    title_style: TextStyle,
    page_style: TextStyle,
    line_height: f64,
    indent: f64,
    title_align: TextAlign,
    page_align: TextAlign,
    row_gap: f64,
    leader: Option<String>,
    keep_with_previous: bool,
    keep_with_next: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TableOfContentsRow {
    pub title: String,
    pub anchor: String,
    pub level: usize,
    pub page: usize,
    pub title_lines: usize,
    pub page_column: [f64; 4],
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct TableOfContentsReport {
    pub rows: Vec<TableOfContentsRow>,
}

#[derive(Clone)]
struct Entry {
    title: String,
    anchor: String,
    level: usize,
}

fn flatten(entries: &[PdfOutlineEntry], level: usize, output: &mut Vec<Entry>) -> Result<()> {
    for entry in entries {
        crate::cancel::check_current_cancel("table-of-contents outline traversal")?;
        if output.len() >= MAX_TOC_ROWS {
            return Err(WellfriendError::ResourceLimit(
                "table-of-contents row count".into(),
            ));
        }
        output.push(Entry {
            title: entry.title.clone(),
            anchor: entry.anchor.clone(),
            level,
        });
        flatten(&entry.children, level + 1, output)?;
    }
    Ok(())
}

fn repeat_leader(
    page: &PdfPageBuilder,
    token: &str,
    available: f64,
    style: &TextStyle,
) -> Result<Option<(String, f64)>> {
    if available <= 0.0 {
        return Ok(None);
    }
    let token_width = layout::text_width(page, token, style)?;
    if !token_width.is_finite() || token_width <= 0.0 {
        return Err(WellfriendError::invalid_input(
            "table-of-contents leader has no positive advance",
        ));
    }
    let count = ((available / token_width).floor() as usize).min(MAX_LEADER_REPETITIONS);
    if count == 0 {
        return Ok(None);
    }
    let text = token.repeat(count);
    let width = layout::text_width(page, &text, style)?;
    if width > available + 1e-7 {
        return Err(WellfriendError::invalid_input(
            "table-of-contents leader exceeded measured gap",
        ));
    }
    Ok(Some((text, width)))
}

struct PreparedRow {
    lines: Vec<layout::Line>,
    page_probe: layout::Line,
    style: ResolvedLevelStyle,
    title_width: f64,
    relative_baselines: Vec<f64>,
    content_top: f64,
    row_height: f64,
}

fn prepare_row(
    flow: &FlowDocument,
    entry: &Entry,
    style: &TableOfContentsStyle,
) -> Result<PreparedRow> {
    let resolved = style.resolve(entry.level);
    let content_width = flow.content_width()?;
    let title_width = content_width - resolved.indent - style.page_column_width - style.column_gap;
    if !resolved.indent.is_finite() || !title_width.is_finite() || title_width <= 0.0 {
        return Err(WellfriendError::UnsupportedFeature(
            "table-of-contents level leaves no title column".into(),
        ));
    }
    let lines = layout::prepare(
        flow.current_page_ref(),
        &entry.title,
        title_width,
        &resolved.title_style,
    )?;
    if lines.is_empty() {
        return Err(WellfriendError::invalid_input(
            "table-of-contents title produced no line",
        ));
    }
    let line_height = resolved.title_style.size * resolved.line_height;
    if !line_height.is_finite() || line_height <= 0.0 {
        return Err(WellfriendError::invalid_input(
            "invalid table-of-contents line height",
        ));
    }
    let page_placeholder = "8".repeat(style.max_page_characters);
    let mut page_probe = layout::prepare(
        flow.current_page_ref(),
        &page_placeholder,
        style.page_column_width,
        &resolved.page_style,
    )?;
    if page_probe.len() != 1 {
        return Err(WellfriendError::UnsupportedFeature(
            "table-of-contents page capacity does not fit its column".into(),
        ));
    }
    let page_probe = page_probe.remove(0);
    let mut relative_cursor = 0.0f64;
    let mut relative_baselines = Vec::with_capacity(lines.len());
    for line in &lines {
        relative_cursor -= line.metrics.ascent;
        relative_baselines.push(relative_cursor);
        relative_cursor -= line.occupied_height(line_height) - line.metrics.ascent;
    }
    let last_relative_baseline = *relative_baselines.last().unwrap();
    let content_top = 0.0f64.max(last_relative_baseline + page_probe.metrics.ascent);
    let content_bottom = relative_cursor.min(last_relative_baseline - page_probe.metrics.descent);
    let row_height = content_top - content_bottom + resolved.row_gap;
    if !row_height.is_finite() || row_height <= 0.0 {
        return Err(WellfriendError::invalid_input(
            "invalid table-of-contents row extent",
        ));
    }
    Ok(PreparedRow {
        lines,
        page_probe,
        style: resolved,
        title_width,
        relative_baselines,
        content_top,
        row_height,
    })
}

fn keep_chain_height(
    flow: &FlowDocument,
    entries: &[Entry],
    start: usize,
    first: &PreparedRow,
    style: &TableOfContentsStyle,
) -> Result<Option<f64>> {
    if start + 1 >= entries.len() {
        return Ok(None);
    }
    let usable = flow.page_size.height - flow.margins.top - flow.margins.bottom;
    if !usable.is_finite() || usable <= 0.0 {
        return Err(WellfriendError::invalid_input(
            "invalid table-of-contents page extent",
        ));
    }
    let mut height = first.row_height;
    let mut index = start;
    let mut current_keeps_next = first.style.keep_with_next;
    loop {
        index += 1;
        let next = prepare_row(flow, &entries[index], style)?;
        if !current_keeps_next && !next.style.keep_with_previous {
            return Ok((index != start + 1).then_some(height));
        }
        height += next.row_height;
        if height > usable {
            // A keep chain that cannot fit an empty page degrades to ordinary
            // row pagination rather than making the document impossible.
            return Ok(None);
        }
        current_keeps_next = next.style.keep_with_next;
        if index + 1 >= entries.len() {
            return Ok(Some(height));
        }
    }
}

pub(super) fn append(
    flow: &mut FlowDocument,
    style: &TableOfContentsStyle,
) -> Result<TableOfContentsReport> {
    style.validate()?;
    let mut entries = Vec::new();
    flatten(&flow.builder.outline, 0, &mut entries)?;
    if entries.is_empty() {
        return Err(WellfriendError::invalid_input(
            "table of contents requires a nonempty authored outline",
        ));
    }
    let mut report = TableOfContentsReport::default();
    flow.append_transaction(|flow| {
        let toc_structure = structure::register(
            &mut flow.builder,
            structure::Role::Toc,
            None,
            Some("Table of contents".into()),
        )?;
        for (entry_index, entry) in entries.iter().enumerate() {
            crate::cancel::check_current_cancel("table-of-contents row layout")?;
            let prepared = prepare_row(flow, entry, style)?;
            if let Some(chain_height) =
                keep_chain_height(flow, &entries, entry_index, &prepared, style)?
            {
                flow.ensure_space(chain_height)?;
            } else {
                flow.ensure_space(prepared.row_height)?;
            }

            let (title_x, page_x) = match style.page_side {
                TableOfContentsPageSide::Right => (
                    flow.margins.left + prepared.style.indent,
                    flow.page_size.width - flow.margins.right - style.page_column_width,
                ),
                TableOfContentsPageSide::Left => (
                    flow.margins.left
                        + style.page_column_width
                        + style.column_gap
                        + prepared.style.indent,
                    flow.margins.left,
                ),
            };
            let row_top = flow.cursor_y;
            let row_structure = structure::register(
                &mut flow.builder,
                structure::Role::Toci,
                Some(toc_structure),
                Some(entry.title.clone()),
            )?;
            flow.current_page_mut()
                .commands
                .push(PageCommand::BeginStructure(row_structure));
            let last_relative_baseline = *prepared.relative_baselines.last().unwrap();
            let mut last_baseline = row_top + last_relative_baseline - prepared.content_top;
            let mut last_end = title_x;
            for (line, relative_baseline) in prepared.lines.iter().zip(&prepared.relative_baselines)
            {
                let baseline = row_top + relative_baseline - prepared.content_top;
                let x = line.aligned_x(title_x, prepared.title_width, prepared.style.title_align);
                flow.current_page_mut().commands.push(line.command(
                    x,
                    baseline,
                    &prepared.style.title_style,
                )?);
                last_baseline = baseline;
                last_end = x + line.metrics.width();
            }
            if let Some(token) = prepared.style.leader.as_deref() {
                let (leader_left, available) = match style.page_side {
                    TableOfContentsPageSide::Right => {
                        let left = last_end;
                        (left, (page_x - style.column_gap - left).max(0.0))
                    }
                    TableOfContentsPageSide::Left => {
                        let left = page_x + style.page_column_width + style.column_gap;
                        let title_start = prepared.lines.last().unwrap().aligned_x(
                            title_x,
                            prepared.title_width,
                            prepared.style.title_align,
                        );
                        (left, (title_start - left).max(0.0))
                    }
                };
                if let Some((leader, leader_width)) = repeat_leader(
                    flow.current_page_ref(),
                    token,
                    available,
                    &prepared.style.title_style,
                )? {
                    let leader_x = match style.page_side {
                        TableOfContentsPageSide::Right => page_x - style.column_gap - leader_width,
                        TableOfContentsPageSide::Left => leader_left,
                    };
                    flow.current_page_mut().draw_text(
                        leader,
                        leader_x,
                        last_baseline,
                        &prepared.style.title_style,
                    )?;
                }
            }
            let page_index = flow.current_page;
            let command = fields::fixed_anchor_page_command(
                &mut flow.builder,
                page_index,
                entry.anchor.clone(),
                style.section_page_numbers,
                style.max_page_characters,
                &prepared.style.page_style,
                page_x,
                last_baseline,
                style.page_column_width,
                prepared.page_probe.metrics.ascent + prepared.page_probe.metrics.descent,
                prepared.style.page_align,
                Some(row_structure),
            )?;
            flow.current_page_mut().commands.push(command);
            flow.current_page_mut()
                .commands
                .push(PageCommand::EndStructure(row_structure));
            flow.cursor_y = row_top - prepared.row_height;
            report.rows.push(TableOfContentsRow {
                title: entry.title.clone(),
                anchor: entry.anchor.clone(),
                level: entry.level,
                page: flow.current_page + 1,
                title_lines: prepared.lines.len(),
                page_column: [
                    page_x,
                    last_baseline - prepared.page_probe.metrics.descent,
                    page_x + style.page_column_width,
                    last_baseline + prepared.page_probe.metrics.ascent,
                ],
            });
        }
        Ok(())
    })?;
    Ok(report)
}
