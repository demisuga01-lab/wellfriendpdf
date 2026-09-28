//! Source-range-bound footnotes for fresh flow authoring. Page-bottom content
//! is reserved during pagination and painted only on a private final clone.
use super::*;
use std::ops::Range;

#[cfg(test)]
#[path = "authoring_notes_tests.rs"]
mod tests;

const EPS: f64 = 1e-7;
const RULE_SPACE: f64 = 8.0;
const FRAGMENT_GAP: f64 = 2.0;
const MAX_NOTES: usize = 10_000;
const MAX_NOTE_BYTES: usize = 16 * 1024 * 1024;
const MAX_NOTE_LINES: usize = 1_000_000;
const MAX_NOTE_FRAGMENTS: usize = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NoteNumberStyle {
    #[default]
    Decimal,
    LowerRoman,
    UpperRoman,
    LowerAlpha,
    UpperAlpha,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NoteNumberScope {
    Document,
    #[default]
    Section,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FootnoteNumbering {
    pub scope: NoteNumberScope,
    pub style: NoteNumberStyle,
    pub start: usize,
    pub prefix: String,
    pub suffix: String,
}

impl Default for FootnoteNumbering {
    fn default() -> Self {
        Self {
            scope: NoteNumberScope::Section,
            style: NoteNumberStyle::Decimal,
            start: 1,
            prefix: "[".into(),
            suffix: "]".into(),
        }
    }
}

impl FootnoteNumbering {
    pub fn new(scope: NoteNumberScope, style: NoteNumberStyle) -> Self {
        Self {
            scope,
            style,
            ..Self::default()
        }
    }

    pub fn start(mut self, start: usize) -> Self {
        self.start = start;
        self
    }

    pub fn affixes(mut self, prefix: impl Into<String>, suffix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self.suffix = suffix.into();
        self
    }

    pub(super) fn validate(&self) -> Result<()> {
        let bytes = self.prefix.len().saturating_add(self.suffix.len());
        if self.start == 0
            || self.start > i64::MAX as usize
            || bytes > 4096
            || self
                .prefix
                .chars()
                .chain(self.suffix.chars())
                .any(crate::fonts::hard_break::is_hard_break)
        {
            return Err(invalid("invalid automatic footnote numbering policy"));
        }
        Ok(())
    }

    fn label(&self, value: usize) -> Result<String> {
        let number = note_number(value, self.style)?;
        let mut label = String::with_capacity(self.prefix.len() + number.len() + self.suffix.len());
        label.push_str(&self.prefix);
        label.push_str(&number);
        label.push_str(&self.suffix);
        if label.is_empty() || label.len() > 4096 {
            return Err(invalid("automatic footnote label is empty or too large"));
        }
        Ok(label)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NumberedFootnote {
    pub at_utf8_offset: usize,
    pub body: String,
    pub style: TextStyle,
    pub paragraph: ParagraphStyle,
}

impl NumberedFootnote {
    pub fn new(at_utf8_offset: usize, body: impl Into<String>, style: TextStyle) -> Self {
        Self {
            at_utf8_offset,
            body: body.into(),
            style,
            paragraph: ParagraphStyle::new(),
        }
    }

    pub fn paragraph(mut self, paragraph: ParagraphStyle) -> Self {
        self.paragraph = paragraph;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsertedFootnoteMarkerInfo {
    pub note: usize,
    pub number: usize,
    pub label: String,
    pub original_utf8_offset: usize,
    pub enriched_utf8_range: [usize; 2],
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NumberedFootnotedParagraphReport {
    pub enriched_text: String,
    pub markers: Vec<InsertedFootnoteMarkerInfo>,
    pub layout: FootnotedParagraphReport,
}

fn roman(mut value: usize) -> Result<String> {
    if value == 0 || value > 3999 {
        return Err(invalid("Roman note numbering supports 1..=3999"));
    }
    let mut out = String::new();
    for (number, glyph) in [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ] {
        while value >= number {
            out.push_str(glyph);
            value -= number;
        }
    }
    Ok(out)
}

fn alpha(mut value: usize) -> Result<String> {
    if value == 0 {
        return Err(invalid("alphabetic note number is zero"));
    }
    let mut reversed = Vec::new();
    while value > 0 {
        value -= 1;
        reversed.push((b'A' + (value % 26) as u8) as char);
        value /= 26;
        if reversed.len() > 64 {
            return Err(WellfriendError::ResourceLimit(
                "alphabetic note number length".into(),
            ));
        }
    }
    Ok(reversed.into_iter().rev().collect())
}

fn note_number(value: usize, style: NoteNumberStyle) -> Result<String> {
    Ok(match style {
        NoteNumberStyle::Decimal => value.to_string(),
        NoteNumberStyle::LowerRoman => roman(value)?.to_lowercase(),
        NoteNumberStyle::UpperRoman => roman(value)?,
        NoteNumberStyle::LowerAlpha => alpha(value)?.to_lowercase(),
        NoteNumberStyle::UpperAlpha => alpha(value)?,
    })
}

/// One note bound to an existing, nonempty UTF-8 range in its body paragraph.
/// The selected body text is reused as the visible note label.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowFootnote {
    pub reference_utf8_range: Range<usize>,
    pub body: String,
    pub style: TextStyle,
    pub paragraph: ParagraphStyle,
}

/// One explicitly labelled endnote. Placement is owned by the endnote
/// collection rather than by a page-bottom reservation.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowEndnote {
    pub label: String,
    pub body: String,
    pub style: TextStyle,
    pub paragraph: ParagraphStyle,
}

impl FlowEndnote {
    pub fn new(label: impl Into<String>, body: impl Into<String>, style: TextStyle) -> Self {
        Self {
            label: label.into(),
            body: body.into(),
            style,
            paragraph: ParagraphStyle::new(),
        }
    }

    pub fn paragraph(mut self, paragraph: ParagraphStyle) -> Self {
        self.paragraph = paragraph;
        self
    }
}

impl FlowFootnote {
    pub fn new(
        reference_utf8_range: Range<usize>,
        body: impl Into<String>,
        style: TextStyle,
    ) -> Self {
        Self {
            reference_utf8_range,
            body: body.into(),
            style,
            paragraph: ParagraphStyle::new(),
        }
    }

    pub fn paragraph(mut self, paragraph: ParagraphStyle) -> Self {
        self.paragraph = paragraph;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FootnoteFragmentInfo {
    pub note: usize,
    pub reference_utf8_range: [usize; 2],
    pub reference_page: usize,
    pub page: usize,
    /// Range in the displayed `marker + space + body` string.
    pub display_utf8_range: [usize; 2],
    pub continued_from_previous: bool,
    pub continues: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FootnotedParagraphReport {
    /// One-based output page for each logical body line.
    pub body_line_pages: Vec<usize>,
    pub fragments: Vec<FootnoteFragmentInfo>,
    pub added_pages: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndnoteItemInfo {
    pub note: usize,
    pub first_page: usize,
    pub last_page: usize,
    pub display_utf8_range: [usize; 2],
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EndnoteReport {
    pub items: Vec<EndnoteItemInfo>,
    pub added_pages: usize,
}

#[derive(Debug, Clone)]
pub(super) struct FootnoteFragment {
    owner: u64,
    note: usize,
    reference_utf8_range: Range<usize>,
    reference_page: usize,
    display_utf8_range: Range<usize>,
    continued_from_previous: bool,
    continues: bool,
    lines: Vec<layout::Line>,
    style: TextStyle,
    align: TextAlign,
    line_height: f64,
    structure_id: u64,
    height: f64,
}

pub(super) fn validate_reference_page_shift(
    pages: &[PdfPageBuilder],
    page_delta: usize,
) -> Result<()> {
    for (page_index, page) in pages.iter().enumerate() {
        for fragment in &page.footnotes {
            if fragment.reference_page == 0 || fragment.reference_page > page_index + 1 {
                return Err(WellfriendError::invalid_input(
                    "footnote reference page is invalid before shift",
                ));
            }
            fragment
                .reference_page
                .checked_add(page_delta)
                .ok_or_else(|| {
                    WellfriendError::ResourceLimit("footnote reference page shift overflow".into())
                })?;
        }
    }
    Ok(())
}

pub(super) fn apply_reference_page_shift(pages: &mut [PdfPageBuilder], page_delta: usize) {
    for page in pages {
        for fragment in &mut page.footnotes {
            fragment.reference_page += page_delta;
        }
    }
}

#[cfg(test)]
pub(super) fn reference_pages(page: &PdfPageBuilder) -> Vec<usize> {
    page.footnotes
        .iter()
        .map(|fragment| fragment.reference_page)
        .collect()
}

struct PreparedNote {
    owner: u64,
    note: usize,
    reference_utf8_range: Range<usize>,
    reference_line: usize,
    lines: Vec<layout::Line>,
    offsets: Vec<usize>,
    style: TextStyle,
    align: TextAlign,
    line_height: f64,
    structure_id: Option<u64>,
}

#[derive(Debug, Clone)]
struct BodySemanticSpan {
    range: Range<usize>,
    structure_id: u64,
}

fn invalid(message: &str) -> WellfriendError {
    WellfriendError::invalid_input(message)
}

fn body_ranges(lines: &[layout::Line], source_len: usize) -> Result<Vec<Range<usize>>> {
    let mut ranges = Vec::with_capacity(lines.len());
    let mut start = 0usize;
    for line in lines {
        let end = start
            .checked_add(line.logical.len())
            .ok_or_else(|| invalid("footnoted paragraph source range overflow"))?;
        ranges.push(start..end);
        start = end;
    }
    if start != source_len {
        return Err(invalid("footnoted paragraph did not consume its source"));
    }
    Ok(ranges)
}

fn prepare_notes(
    page: &PdfPageBuilder,
    source: &str,
    body_lines: &[layout::Line],
    width: f64,
    notes: &[FlowFootnote],
    owner_base: u64,
) -> Result<Vec<Vec<PreparedNote>>> {
    if notes.len() > MAX_NOTES {
        return Err(WellfriendError::ResourceLimit(
            "authoring footnote count".into(),
        ));
    }
    let ranges = body_ranges(body_lines, source.len())?;
    let mut previous_end = 0usize;
    let mut total_bytes = 0usize;
    let mut total_lines = 0usize;
    let mut grouped = (0..body_lines.len())
        .map(|_| Vec::new())
        .collect::<Vec<_>>();
    for (note, spec) in notes.iter().enumerate() {
        crate::cancel::check_current_cancel("authoring footnote preparation")?;
        let range = spec.reference_utf8_range.clone();
        if range.start >= range.end
            || range.start < previous_end
            || range.end > source.len()
            || !source.is_char_boundary(range.start)
            || !source.is_char_boundary(range.end)
            || spec
                .body
                .chars()
                .any(crate::fonts::hard_break::is_form_feed)
        {
            return Err(invalid(
                "footnote references must be sorted, disjoint, nonempty UTF-8 ranges and note bodies cannot contain form feed",
            ));
        }
        let reference_line = ranges
            .iter()
            .position(|line| range.start >= line.start && range.end <= line.end)
            .ok_or_else(|| invalid("footnote reference crosses a laid-out body line"))?;
        let marker = source
            .get(range.clone())
            .ok_or_else(|| invalid("invalid footnote marker range"))?;
        if marker.chars().any(crate::fonts::hard_break::is_hard_break) {
            return Err(invalid(
                "footnote marker cannot contain a hard line/page separator",
            ));
        }
        let display = if spec.body.is_empty() {
            marker.to_string()
        } else {
            format!("{marker} {}", spec.body)
        };
        total_bytes = total_bytes.checked_add(display.len()).ok_or_else(|| {
            WellfriendError::ResourceLimit("authoring footnote byte count".into())
        })?;
        if total_bytes > MAX_NOTE_BYTES {
            return Err(WellfriendError::ResourceLimit(
                "authoring footnote byte budget".into(),
            ));
        }
        let lines = layout::prepare_with_tabs(
            page,
            &display,
            width,
            &spec.style,
            &spec.paragraph.tab_stops,
        )?;
        total_lines = total_lines.checked_add(lines.len()).ok_or_else(|| {
            WellfriendError::ResourceLimit("authoring footnote line count".into())
        })?;
        if lines.is_empty() || total_lines > MAX_NOTE_LINES {
            return Err(WellfriendError::ResourceLimit(
                "authoring footnote line budget".into(),
            ));
        }
        let mut offsets: Vec<usize> = Vec::with_capacity(lines.len() + 1);
        offsets.push(0);
        for line in &lines {
            offsets.push(
                offsets
                    .last()
                    .copied()
                    .unwrap()
                    .checked_add(line.logical.len())
                    .ok_or_else(|| invalid("footnote display range overflow"))?,
            );
        }
        if offsets.last().copied() != Some(display.len()) {
            return Err(invalid("footnote layout did not consume its display text"));
        }
        grouped[reference_line].push(PreparedNote {
            owner: owner_base.checked_add(note as u64).ok_or_else(|| {
                WellfriendError::ResourceLimit("footnote identity overflow".into())
            })?,
            note,
            reference_utf8_range: range.clone(),
            reference_line,
            lines,
            offsets,
            style: spec.style.clone(),
            align: spec.paragraph.align,
            line_height: spec.paragraph.line_height_points(spec.style.size)?,
            structure_id: None,
        });
        previous_end = range.end;
    }
    Ok(grouped)
}

fn line_height(note: &PreparedNote, index: usize) -> f64 {
    note.lines[index].occupied_height(note.line_height)
}

fn register_body_semantics(
    builder: &mut PdfBuilder,
    paragraph: u64,
    source_len: usize,
    notes: &[FlowFootnote],
    note_structures: &[u64],
) -> Result<Vec<BodySemanticSpan>> {
    if notes.len() != note_structures.len() {
        return Err(invalid("footnote semantic owner count mismatch"));
    }
    let mut spans = Vec::with_capacity(notes.len().saturating_mul(2).saturating_add(1));
    let mut cursor = 0usize;
    for (note, note_structure) in notes.iter().zip(note_structures) {
        let range = note.reference_utf8_range.clone();
        if range.start < cursor || range.start >= range.end || range.end > source_len {
            return Err(invalid("invalid footnote semantic source partition"));
        }
        if cursor < range.start {
            spans.push(BodySemanticSpan {
                range: cursor..range.start,
                structure_id: structure::register_span(builder, paragraph)?,
            });
        }
        spans.push(BodySemanticSpan {
            range: range.clone(),
            structure_id: structure::register_reference(builder, paragraph, *note_structure)?,
        });
        cursor = range.end;
    }
    if cursor < source_len {
        spans.push(BodySemanticSpan {
            range: cursor..source_len,
            structure_id: structure::register_span(builder, paragraph)?,
        });
    }
    if source_len != 0 && spans.is_empty() {
        return Err(invalid("footnote semantic partition is empty"));
    }
    Ok(spans)
}

fn semantic_line_spans(
    spans: &[BodySemanticSpan],
    line: Range<usize>,
) -> Result<Vec<layout::OwnedTextSpan>> {
    let mut output = Vec::new();
    for span in spans {
        let start = span.range.start.max(line.start);
        let end = span.range.end.min(line.end);
        if start < end {
            output.push(layout::OwnedTextSpan {
                range: start - line.start..end - line.start,
                element: span.structure_id,
            });
        }
    }
    if line.start < line.end
        && (output.first().map(|span| span.range.start) != Some(0)
            || output.last().map(|span| span.range.end) != Some(line.len()))
    {
        return Err(invalid("footnote semantic spans do not cover a body line"));
    }
    Ok(output)
}

fn is_fresh_page(flow: &FlowDocument) -> bool {
    let page = flow.current_page_ref();
    let top = flow.page_size.height - flow.margins.top;
    page.commands.is_empty() && page.footnotes.is_empty() && (flow.cursor_y - top).abs() <= EPS
}

fn reserve_fragment(
    flow: &mut FlowDocument,
    note: &PreparedNote,
    start: usize,
    body_height: f64,
    reference_page: usize,
    report: &mut FootnotedParagraphReport,
) -> Result<Option<usize>> {
    if start >= note.lines.len() {
        return Err(invalid("invalid footnote continuation cursor"));
    }
    if report.fragments.len() >= MAX_NOTE_FRAGMENTS {
        return Err(WellfriendError::ResourceLimit(
            "authoring footnote fragment budget".into(),
        ));
    }
    let page = flow.current_page_ref();
    let overhead = if page.footnotes.is_empty() {
        RULE_SPACE
    } else {
        FRAGMENT_GAP
    };
    let available = flow.cursor_y - body_height - flow.current_bottom() - overhead;
    if !available.is_finite() || available < 0.0 {
        return Ok(None);
    }
    let mut height = 0.0;
    let mut end = start;
    while end < note.lines.len() {
        let next = height + line_height(note, end);
        if !next.is_finite() || next > available + EPS {
            break;
        }
        height = next;
        end += 1;
    }
    if end == start {
        return Ok(None);
    }
    let page_number = flow.current_page + 1;
    let fragment = FootnoteFragment {
        owner: note.owner,
        note: note.note,
        reference_utf8_range: note.reference_utf8_range.clone(),
        reference_page,
        display_utf8_range: note.offsets[start]..note.offsets[end],
        continued_from_previous: start > 0,
        continues: end < note.lines.len(),
        lines: note.lines[start..end].to_vec(),
        style: note.style.clone(),
        align: note.align,
        line_height: note.line_height,
        structure_id: note.structure_id.ok_or_else(|| {
            invalid("footnote structure identity was not assigned before pagination")
        })?,
        height,
    };
    let page = flow.current_page_mut();
    page.footnote_reserved_height += overhead + height;
    if !page.footnote_reserved_height.is_finite() {
        return Err(invalid("footnote reservation overflow"));
    }
    page.footnotes.push(fragment);
    report.fragments.push(FootnoteFragmentInfo {
        note: note.note,
        reference_utf8_range: [
            note.reference_utf8_range.start,
            note.reference_utf8_range.end,
        ],
        reference_page,
        page: page_number,
        display_utf8_range: [note.offsets[start], note.offsets[end]],
        continued_from_previous: start > 0,
        continues: end < note.lines.len(),
    });
    Ok(Some(end))
}

fn start_fragment(
    flow: &mut FlowDocument,
    note: &PreparedNote,
    start: usize,
    body_height: f64,
    reference_page: Option<usize>,
    report: &mut FootnotedParagraphReport,
) -> Result<(usize, usize)> {
    loop {
        crate::cancel::check_current_cancel("authoring footnote pagination")?;
        let reference_page = reference_page.unwrap_or(flow.current_page + 1);
        if let Some(end) = reserve_fragment(flow, note, start, body_height, reference_page, report)?
        {
            return Ok((end, reference_page));
        }
        if is_fresh_page(flow) {
            return Err(WellfriendError::UnsupportedFeature(
                "footnote line and its body reference cannot fit a fresh page".into(),
            ));
        }
        flow.add_page_break();
    }
}

pub(super) fn append_paragraph(
    flow: &mut FlowDocument,
    text: &str,
    style: &TextStyle,
    paragraph: &ParagraphStyle,
    notes: &[FlowFootnote],
) -> Result<FootnotedParagraphReport> {
    let initial_pages = flow.builder.pages.len();
    let owner_base = flow.builder.next_footnote_id;
    flow.builder.next_footnote_id = owner_base
        .checked_add(notes.len() as u64)
        .ok_or_else(|| WellfriendError::ResourceLimit("footnote identity overflow".into()))?;
    let width = flow.content_width()?;
    let lines = layout::prepare_with_tabs(
        flow.current_page_ref(),
        text,
        width,
        style,
        &paragraph.tab_stops,
    )?;
    let mut grouped = prepare_notes(
        flow.current_page_ref(),
        text,
        &lines,
        width,
        notes,
        owner_base,
    )?;
    let paragraph_structure = if lines.is_empty() {
        None
    } else {
        Some(structure::register(
            &mut flow.builder,
            structure::Role::Paragraph,
            None,
            None,
        )?)
    };
    let note_structures = (0..notes.len())
        .map(|_| structure::register(&mut flow.builder, structure::Role::Note, None, None))
        .collect::<Result<Vec<_>>>()?;
    let body_semantics = if let Some(paragraph_structure) = paragraph_structure {
        register_body_semantics(
            &mut flow.builder,
            paragraph_structure,
            text.len(),
            notes,
            &note_structures,
        )?
    } else {
        Vec::new()
    };
    for prepared in grouped.iter_mut().flatten() {
        prepared.structure_id = note_structures.get(prepared.note).copied();
    }
    let line_height = paragraph.line_height_points(style.size)?;
    let mut report = FootnotedParagraphReport::default();

    let mut body_cursor = 0usize;
    for (line_index, line) in lines.into_iter().enumerate() {
        crate::cancel::check_current_cancel("authoring footnoted body")?;
        let line_range = body_cursor..body_cursor + line.logical.len();
        let owned = semantic_line_spans(&body_semantics, line_range.clone())?;
        body_cursor = line_range.end;
        let force_page = line
            .logical
            .chars()
            .next_back()
            .is_some_and(crate::fonts::hard_break::is_form_feed);
        let height = line.occupied_height(line_height);
        let mut attached = std::mem::take(&mut grouped[line_index]).into_iter();
        if let Some(first) = attached.next() {
            let (mut end, reference_page) =
                start_fragment(flow, &first, 0, height, None, &mut report)?;
            let x = line.aligned_x(flow.margins.left, width, paragraph.align);
            let commands = line.owned_commands(
                flow.current_page_ref(),
                x,
                flow.cursor_y - line.metrics.ascent,
                style,
                &owned,
            )?;
            flow.current_page_mut().commands.extend(commands);
            flow.cursor_y -= height;
            report.body_line_pages.push(reference_page);

            while end < first.lines.len() {
                let placed =
                    start_fragment(flow, &first, end, 0.0, Some(reference_page), &mut report)?;
                end = placed.0;
            }
            for note in attached {
                debug_assert_eq!(note.reference_line, line_index);
                let mut start = 0usize;
                while start < note.lines.len() {
                    let placed =
                        start_fragment(flow, &note, start, 0.0, Some(reference_page), &mut report)?;
                    start = placed.0;
                }
            }
        } else {
            flow.ensure_space(height)?;
            let page = flow.current_page + 1;
            let x = line.aligned_x(flow.margins.left, width, paragraph.align);
            let commands = line.owned_commands(
                flow.current_page_ref(),
                x,
                flow.cursor_y - line.metrics.ascent,
                style,
                &owned,
            )?;
            flow.current_page_mut().commands.extend(commands);
            flow.cursor_y -= height;
            report.body_line_pages.push(page);
        }
        if force_page {
            flow.add_page_break_to(FlowPageBreak::NextPage);
        }
    }
    if grouped.iter().any(|notes| !notes.is_empty()) {
        return Err(invalid(
            "footnote reference was not consumed by body layout",
        ));
    }
    if body_cursor != text.len() {
        return Err(invalid(
            "footnote semantic layout did not consume its source",
        ));
    }
    report.added_pages = flow.builder.pages.len() - initial_pages;
    Ok(report)
}

pub(super) fn append_numbered_paragraph(
    flow: &mut FlowDocument,
    text: &str,
    style: &TextStyle,
    paragraph: &ParagraphStyle,
    notes: &[NumberedFootnote],
) -> Result<NumberedFootnotedParagraphReport> {
    if notes.len() > MAX_NOTES {
        return Err(WellfriendError::ResourceLimit(
            "automatic footnote count".into(),
        ));
    }
    let numbering = flow.builder.sections[flow.current_section]
        .footnote_numbering
        .clone();
    numbering.validate()?;
    let (mut next, started) = match numbering.scope {
        NoteNumberScope::Document => (
            if flow.document_footnote_started {
                flow.document_footnote_next
            } else {
                numbering.start
            },
            flow.document_footnote_started,
        ),
        NoteNumberScope::Section => (
            if flow.section_footnote_started {
                flow.section_footnote_next
            } else {
                numbering.start
            },
            flow.section_footnote_started,
        ),
    };

    let mut enriched = String::with_capacity(text.len().saturating_add(notes.len() * 4));
    let mut markers = Vec::with_capacity(notes.len());
    let mut explicit = Vec::with_capacity(notes.len());
    let mut source_cursor = 0usize;
    let mut previous_offset = 0usize;
    for (note, spec) in notes.iter().enumerate() {
        crate::cancel::check_current_cancel("automatic footnote marker insertion")?;
        let offset = spec.at_utf8_offset;
        if offset < previous_offset
            || offset > text.len()
            || !text.is_char_boundary(offset)
            || spec
                .body
                .chars()
                .any(crate::fonts::hard_break::is_form_feed)
        {
            return Err(invalid(
                "automatic footnote offsets must be sorted UTF-8 boundaries and note bodies cannot contain form feed",
            ));
        }
        enriched.push_str(
            text.get(source_cursor..offset)
                .ok_or_else(|| invalid("automatic footnote source mapping is invalid"))?,
        );
        let label = numbering.label(next)?;
        let start = enriched.len();
        enriched.push_str(&label);
        let end = enriched.len();
        markers.push(InsertedFootnoteMarkerInfo {
            note,
            number: next,
            label,
            original_utf8_offset: offset,
            enriched_utf8_range: [start, end],
        });
        explicit.push(
            FlowFootnote::new(start..end, spec.body.clone(), spec.style.clone())
                .paragraph(spec.paragraph.clone()),
        );
        next = next
            .checked_add(1)
            .ok_or_else(|| WellfriendError::ResourceLimit("footnote number overflow".into()))?;
        source_cursor = offset;
        previous_offset = offset;
    }
    enriched.push_str(
        text.get(source_cursor..)
            .ok_or_else(|| invalid("automatic footnote source tail is invalid"))?,
    );
    if enriched.len() > MAX_NOTE_BYTES {
        return Err(WellfriendError::ResourceLimit(
            "automatic footnoted paragraph byte budget".into(),
        ));
    }

    match numbering.scope {
        NoteNumberScope::Document => {
            flow.document_footnote_next = next;
            flow.document_footnote_started = started || !notes.is_empty();
        }
        NoteNumberScope::Section => {
            flow.section_footnote_next = next;
            flow.section_footnote_started = started || !notes.is_empty();
        }
    }
    let layout = append_paragraph(flow, &enriched, style, paragraph, &explicit)?;
    Ok(NumberedFootnotedParagraphReport {
        enriched_text: enriched,
        markers,
        layout,
    })
}

pub(super) fn append_endnotes(
    flow: &mut FlowDocument,
    notes: &[FlowEndnote],
    break_before: FlowPageBreak,
) -> Result<EndnoteReport> {
    if notes.len() > MAX_NOTES {
        return Err(WellfriendError::ResourceLimit(
            "authoring endnote count".into(),
        ));
    }
    let initial_pages = flow.builder.pages.len();
    let mut displays = Vec::with_capacity(notes.len());
    let mut total_bytes = 0usize;
    for note in notes {
        if note.label.is_empty()
            || note
                .label
                .chars()
                .any(crate::fonts::hard_break::is_hard_break)
            || note
                .body
                .chars()
                .any(crate::fonts::hard_break::is_form_feed)
        {
            return Err(invalid(
                "endnote labels must be nonempty single lines and bodies cannot contain form feed",
            ));
        }
        let display = if note.body.is_empty() {
            note.label.clone()
        } else {
            format!("{} {}", note.label, note.body)
        };
        total_bytes = total_bytes
            .checked_add(display.len())
            .ok_or_else(|| WellfriendError::ResourceLimit("authoring endnote byte count".into()))?;
        if total_bytes > MAX_NOTE_BYTES {
            return Err(WellfriendError::ResourceLimit(
                "authoring endnote byte budget".into(),
            ));
        }
        displays.push(display);
    }
    if notes.is_empty() {
        return Ok(EndnoteReport::default());
    }

    flow.add_page_break_to(break_before);
    let width = flow.content_width()?;
    let mut report = EndnoteReport::default();
    let mut total_lines = 0usize;
    for (note_index, (note, display)) in notes.iter().zip(displays).enumerate() {
        crate::cancel::check_current_cancel("authoring endnote pagination")?;
        let lines = layout::prepare_with_tabs(
            flow.current_page_ref(),
            &display,
            width,
            &note.style,
            &note.paragraph.tab_stops,
        )?;
        total_lines = total_lines
            .checked_add(lines.len())
            .ok_or_else(|| WellfriendError::ResourceLimit("authoring endnote line count".into()))?;
        if lines.is_empty() || total_lines > MAX_NOTE_LINES {
            return Err(WellfriendError::ResourceLimit(
                "authoring endnote line budget".into(),
            ));
        }
        let line_height = note.paragraph.line_height_points(note.style.size)?;
        let note_structure =
            structure::register(&mut flow.builder, structure::Role::Note, None, None)?;
        let mut first_page = None;
        let mut last_page = 0usize;
        for line in lines {
            let height = line.occupied_height(line_height);
            flow.ensure_space(height)?;
            let page = flow.current_page + 1;
            first_page.get_or_insert(page);
            last_page = page;
            let x = line.aligned_x(flow.margins.left, width, note.paragraph.align);
            let command = line.command(x, flow.cursor_y - line.metrics.ascent, &note.style)?;
            flow.current_page_mut()
                .commands
                .push(PageCommand::BeginStructure(note_structure));
            flow.current_page_mut().commands.push(command);
            flow.current_page_mut()
                .commands
                .push(PageCommand::EndStructure(note_structure));
            flow.cursor_y -= height;
        }
        report.items.push(EndnoteItemInfo {
            note: note_index,
            first_page: first_page.unwrap(),
            last_page,
            display_utf8_range: [0, display.len()],
        });
    }
    report.added_pages = flow.builder.pages.len() - initial_pages;
    Ok(report)
}

pub(super) fn materialize(builder: &PdfBuilder) -> Result<PdfBuilder> {
    if builder.notes_materialized || builder.pages.iter().all(|page| page.footnotes.is_empty()) {
        return Ok(builder.clone());
    }
    let mut result = builder.clone();
    let mut chains = BTreeMap::<u64, (Range<usize>, usize, bool)>::new();
    for (page_index, page) in result.pages.iter().enumerate() {
        for fragment in &page.footnotes {
            if fragment.reference_page == 0
                || fragment.reference_page > page_index + 1
                || fragment.display_utf8_range.start >= fragment.display_utf8_range.end
            {
                return Err(invalid("invalid footnote fragment page/source identity"));
            }
            match chains.get_mut(&fragment.owner) {
                None => {
                    if fragment.continued_from_previous || fragment.display_utf8_range.start != 0 {
                        return Err(invalid("footnote continuation has no first fragment"));
                    }
                    chains.insert(
                        fragment.owner,
                        (
                            fragment.reference_utf8_range.clone(),
                            fragment.display_utf8_range.end,
                            !fragment.continues,
                        ),
                    );
                }
                Some((reference, next, complete)) => {
                    if *complete
                        || !fragment.continued_from_previous
                        || *reference != fragment.reference_utf8_range
                        || *next != fragment.display_utf8_range.start
                    {
                        return Err(invalid("noncontiguous footnote fragment chain"));
                    }
                    *next = fragment.display_utf8_range.end;
                    *complete = !fragment.continues;
                }
            }
        }
    }
    if chains.values().any(|(_, _, complete)| !*complete) {
        return Err(invalid("unterminated footnote fragment chain"));
    }
    for page in &mut result.pages {
        crate::cancel::check_current_cancel("authoring footnote materialization")?;
        if page.footnotes.is_empty() {
            if page.footnote_reserved_height.abs() > EPS {
                return Err(invalid("empty footnote page retained reserved height"));
            }
            continue;
        }
        let expected = RULE_SPACE
            + page
                .footnotes
                .iter()
                .map(|fragment| fragment.height)
                .sum::<f64>()
            + FRAGMENT_GAP * page.footnotes.len().saturating_sub(1) as f64;
        if !expected.is_finite()
            || (expected - page.footnote_reserved_height).abs() > EPS
            || expected > page.size.height - page.margins.top - page.margins.bottom + EPS
        {
            return Err(invalid("footnote reservation does not match its fragments"));
        }
        let width = page.size.width - page.margins.left - page.margins.right;
        let top = page.margins.bottom + expected;
        let rule_y = top - 2.0;
        page.commands.push(PageCommand::BeginArtifact);
        page.commands.push(PageCommand::Path {
            path: PathBuilder::new()
                .move_to(page.margins.left, rule_y)
                .line_to(page.margins.left + width.min(72.0), rule_y),
            style: GraphicsStyle::stroke(Color::black(), 0.5),
        });
        page.commands.push(PageCommand::EndArtifact);
        let mut cursor = top - RULE_SPACE;
        for (index, fragment) in page.footnotes.iter().enumerate() {
            if index > 0 {
                cursor -= FRAGMENT_GAP;
            }
            let before = cursor;
            for line in &fragment.lines {
                let x = line.aligned_x(page.margins.left, width, fragment.align);
                page.commands
                    .push(PageCommand::BeginStructure(fragment.structure_id));
                page.commands.push(line.command(
                    x,
                    cursor - line.metrics.ascent,
                    &fragment.style,
                )?);
                page.commands
                    .push(PageCommand::EndStructure(fragment.structure_id));
                cursor -= line.occupied_height(fragment.line_height);
            }
            if (before - cursor - fragment.height).abs() > EPS
                || fragment.note >= MAX_NOTES
                || fragment.reference_utf8_range.start >= fragment.reference_utf8_range.end
                || fragment.reference_page == 0
                || fragment.display_utf8_range.start >= fragment.display_utf8_range.end
                || (fragment.continued_from_previous && fragment.display_utf8_range.start == 0)
                || (fragment.continues && fragment.display_utf8_range.end == 0)
            {
                return Err(invalid("invalid retained footnote fragment"));
            }
        }
        if cursor < page.margins.bottom - EPS || (cursor - page.margins.bottom).abs() > EPS {
            return Err(invalid(
                "footnote materialization escaped its reserved region",
            ));
        }
    }
    result.notes_materialized = true;
    Ok(result)
}
