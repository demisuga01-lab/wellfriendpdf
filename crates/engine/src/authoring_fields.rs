//! Deferred, fixed-capacity body fields and named destinations for fresh
//! authoring. Pagination uses conservative placeholders; final values resolve
//! on a private serialization clone without selecting new line breaks.
use super::*;
use std::ops::Range;

#[cfg(test)]
#[path = "authoring_fields_tests.rs"]
mod tests;

const EPS: f64 = 1e-7;
const MAX_PARTS: usize = 4096;
const MAX_TEXT_BYTES: usize = 16 * 1024 * 1024;
const MAX_FIELD_CHARACTERS: usize = 64;
const MAX_ANCHORS: usize = 100_000;
const MAX_LINKS: usize = 100_000;
const NAME_TREE_FANOUT: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BodyField {
    DocumentPage,
    DocumentPages,
    SectionPage,
    SectionPages,
    SectionLastPage,
    AnchorDocumentPage(String),
    AnchorSectionPage(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyFieldFormat {
    pub max_characters: usize,
    pub prefix: String,
    pub suffix: String,
    pub link_to_anchor: bool,
    pub value_align: TextAlign,
}

impl BodyFieldFormat {
    pub fn new(max_characters: usize) -> Self {
        Self {
            max_characters,
            prefix: String::new(),
            suffix: String::new(),
            link_to_anchor: false,
            value_align: TextAlign::Right,
        }
    }

    pub fn affixes(mut self, prefix: impl Into<String>, suffix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self.suffix = suffix.into();
        self
    }

    /// Turn an anchor-page field into a clickable PDF link to that anchor.
    /// Non-anchor fields reject this option before mutating the document.
    pub fn link_to_anchor(mut self, enabled: bool) -> Self {
        self.link_to_anchor = enabled;
        self
    }

    /// Align the resolved value inside its fixed character-capacity box.
    pub fn value_align(mut self, align: TextAlign) -> Self {
        self.value_align = align;
        self
    }

    fn validate(&self) -> Result<()> {
        if self.max_characters == 0
            || self.max_characters > MAX_FIELD_CHARACTERS
            || self.prefix.len().saturating_add(self.suffix.len()) > 4096
            || self
                .prefix
                .chars()
                .chain(self.suffix.chars())
                .any(crate::fonts::hard_break::is_hard_break)
        {
            return Err(WellfriendError::invalid_input(
                "invalid authored body-field capacity or affix",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BodyFieldPart {
    Text(String),
    Field {
        field: BodyField,
        format: BodyFieldFormat,
    },
}

impl BodyFieldPart {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(text.into())
    }

    pub fn field(field: BodyField, format: BodyFieldFormat) -> Self {
        Self::Field { field, format }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BodyAnchorInfo {
    pub name: String,
    pub page: usize,
    pub section: usize,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyFieldInfo {
    pub field: usize,
    pub template_utf8_range: [usize; 2],
    pub line: usize,
    pub max_characters: usize,
    pub clickable: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FieldParagraphReport {
    pub line_pages: Vec<usize>,
    pub fields: Vec<BodyFieldInfo>,
}

#[derive(Debug, Clone)]
pub(super) struct BodyAnchor {
    name: String,
    pub(super) page_index: usize,
    pub(super) section_index: usize,
    y: f64,
}

#[derive(Debug, Clone)]
struct FieldSpan {
    field_index: usize,
    template_range: Range<usize>,
    line_index: usize,
    field: BodyField,
    format: BodyFieldFormat,
}

#[derive(Debug)]
struct DeferredFieldPlan {
    id: u64,
    template: String,
    line_ranges: Vec<Range<usize>>,
    spans: Vec<FieldSpan>,
    style: TextStyle,
    align: TextAlign,
    tab_stops: crate::fonts::tab_stops::TabStops,
    structure_id: Option<u64>,
}

#[derive(Debug, Clone)]
pub(super) struct DeferredFieldLine {
    plan: Arc<DeferredFieldPlan>,
    line_index: usize,
    x: f64,
    y: f64,
    reserved_width: f64,
    reserved_height: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct AuthoredLink {
    pub(super) name: String,
    pub(super) destination: String,
    pub(super) rect: [f64; 4],
    pub(super) contents: String,
    pub(super) structure_id: Option<u64>,
}

#[derive(Clone, Copy)]
struct Placement {
    page_index: usize,
    x: f64,
    y: f64,
    reserved_width: f64,
    reserved_height: f64,
}

struct PlanWork {
    plan: Arc<DeferredFieldPlan>,
    placements: Vec<Option<Placement>>,
}

struct ResolvedPlan {
    commands: Vec<PageCommand>,
    links: Vec<(usize, AuthoredLink)>,
}

pub(super) fn validate_anchor_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 4096
        || name
            .chars()
            .any(|ch| ch == '\0' || crate::fonts::hard_break::is_hard_break(ch))
    {
        return Err(WellfriendError::invalid_input(
            "authored anchor name must be a bounded nonempty single line",
        ));
    }
    Ok(())
}

pub(super) fn add_anchor(flow: &mut FlowDocument, name: String) -> Result<BodyAnchorInfo> {
    validate_anchor_name(&name)?;
    if flow.builder.anchors.len() >= MAX_ANCHORS {
        return Err(WellfriendError::ResourceLimit(
            "authored anchor count".into(),
        ));
    }
    if flow.builder.anchors.contains_key(&name) {
        return Err(WellfriendError::invalid_input(
            "authored anchor name is already defined",
        ));
    }
    let anchor = BodyAnchor {
        name: name.clone(),
        page_index: flow.current_page,
        section_index: flow.current_section,
        y: flow.cursor_y,
    };
    if !anchor.y.is_finite()
        || anchor.y < 0.0
        || anchor.y > flow.page_size.height
        || anchor.page_index >= flow.builder.pages.len()
    {
        return Err(WellfriendError::invalid_input(
            "invalid authored anchor position",
        ));
    }
    flow.builder.anchors.insert(name.clone(), anchor);
    Ok(BodyAnchorInfo {
        name,
        page: flow.current_page + 1,
        section: flow.current_section,
        y: flow.cursor_y,
    })
}

fn placeholder(format: &BodyFieldFormat) -> String {
    format!(
        "{}{}{}",
        format.prefix,
        "8".repeat(format.max_characters),
        format.suffix
    )
}

pub(super) fn append_paragraph(
    flow: &mut FlowDocument,
    parts: &[BodyFieldPart],
    style: &TextStyle,
    paragraph: &ParagraphStyle,
    structure_id: Option<u64>,
) -> Result<FieldParagraphReport> {
    let width = flow.content_width()?;
    let left = flow.margins.left;
    append_paragraph_in_region_with_structure(
        flow,
        parts,
        style,
        paragraph,
        left,
        width,
        structure_id,
    )
}

#[allow(dead_code)] // Reserved by the field-layout module's staged authoring API.
pub(super) fn append_paragraph_in_region(
    flow: &mut FlowDocument,
    parts: &[BodyFieldPart],
    style: &TextStyle,
    paragraph: &ParagraphStyle,
    left: f64,
    width: f64,
) -> Result<FieldParagraphReport> {
    append_paragraph_in_region_with_structure(flow, parts, style, paragraph, left, width, None)
}

pub(super) fn append_paragraph_in_region_with_structure(
    flow: &mut FlowDocument,
    parts: &[BodyFieldPart],
    style: &TextStyle,
    paragraph: &ParagraphStyle,
    left: f64,
    width: f64,
    structure_id: Option<u64>,
) -> Result<FieldParagraphReport> {
    if !left.is_finite() || !width.is_finite() || width <= 0.0 {
        return Err(WellfriendError::invalid_input(
            "invalid authored body-field region",
        ));
    }
    let content_right = flow.page_size.width - flow.margins.right;
    if left < flow.margins.left - EPS || left + width > content_right + EPS {
        return Err(WellfriendError::invalid_input(
            "authored body-field region exceeds the flow content area",
        ));
    }
    let left_inset = left - flow.margins.left;
    if parts.is_empty() || parts.len() > MAX_PARTS {
        return Err(WellfriendError::invalid_input(
            "body-field paragraph requires a bounded nonempty part list",
        ));
    }
    let mut template = String::new();
    let mut spans = Vec::new();
    for part in parts {
        crate::cancel::check_current_cancel("authoring body-field preparation")?;
        match part {
            BodyFieldPart::Text(text) => template.push_str(text),
            BodyFieldPart::Field { field, format } => {
                format.validate()?;
                if format.link_to_anchor
                    && !matches!(
                        field,
                        BodyField::AnchorDocumentPage(_) | BodyField::AnchorSectionPage(_)
                    )
                {
                    return Err(WellfriendError::invalid_input(
                        "only an authored anchor-page field can create a link",
                    ));
                }
                match field {
                    BodyField::AnchorDocumentPage(name) | BodyField::AnchorSectionPage(name) => {
                        validate_anchor_name(name)?
                    }
                    _ => {}
                }
                let start = template.len();
                template.push_str(&placeholder(format));
                spans.push(FieldSpan {
                    field_index: spans.len(),
                    template_range: start..template.len(),
                    line_index: usize::MAX,
                    field: field.clone(),
                    format: format.clone(),
                });
            }
        }
        if template.len() > MAX_TEXT_BYTES {
            return Err(WellfriendError::ResourceLimit(
                "authored body-field paragraph byte budget".into(),
            ));
        }
    }
    if template.is_empty() {
        return Err(WellfriendError::invalid_input(
            "body-field paragraph cannot resolve to empty text",
        ));
    }
    let lines = layout::prepare_with_tabs(
        flow.current_page_ref(),
        &template,
        width,
        style,
        &paragraph.tab_stops,
    )?;
    if lines.iter().any(|line| line.metrics.width() > width + EPS) {
        return Err(WellfriendError::UnsupportedFeature(
            "body-field placeholder exceeds the available line width".into(),
        ));
    }
    let mut line_ranges = Vec::with_capacity(lines.len());
    let mut cursor = 0usize;
    for line in &lines {
        let end = cursor
            .checked_add(line.logical.len())
            .ok_or_else(|| WellfriendError::invalid_input("body-field line range overflow"))?;
        line_ranges.push(cursor..end);
        cursor = end;
    }
    if cursor != template.len() {
        return Err(WellfriendError::invalid_input(
            "body-field layout did not consume its template",
        ));
    }
    for span in &mut spans {
        span.line_index = line_ranges
            .iter()
            .position(|line| {
                span.template_range.start >= line.start && span.template_range.end <= line.end
            })
            .ok_or_else(|| {
                WellfriendError::UnsupportedFeature(
                    "body field cannot cross a reserved line boundary; increase width or reduce capacity"
                        .into(),
                )
            })?;
    }
    let id = flow.builder.next_field_plan_id;
    flow.builder.next_field_plan_id = id.checked_add(1).ok_or_else(|| {
        WellfriendError::ResourceLimit("body-field plan identity overflow".into())
    })?;
    let plan = Arc::new(DeferredFieldPlan {
        id,
        template,
        line_ranges,
        spans,
        style: style.clone(),
        align: paragraph.align,
        tab_stops: paragraph.tab_stops.clone(),
        structure_id,
    });
    let line_height = paragraph.line_height_points(style.size)?;
    let mut report = FieldParagraphReport::default();
    for (line_index, line) in lines.into_iter().enumerate() {
        crate::cancel::check_current_cancel("authoring body-field pagination")?;
        let force_page = line
            .logical
            .chars()
            .next_back()
            .is_some_and(crate::fonts::hard_break::is_form_feed);
        let height = line.occupied_height(line_height);
        flow.ensure_space(height)?;
        let page_left = flow.margins.left + left_inset;
        let page_right = flow.page_size.width - flow.margins.right;
        if page_left < flow.margins.left - EPS || page_left + width > page_right + EPS {
            return Err(WellfriendError::UnsupportedFeature(
                "authored body-field region does not fit the destination page".into(),
            ));
        }
        let page = flow.current_page + 1;
        let x = line.aligned_x(page_left, width, paragraph.align);
        let y = flow.cursor_y - line.metrics.ascent;
        if let Some(structure_id) = structure_id {
            flow.current_page_mut()
                .commands
                .push(PageCommand::BeginStructure(structure_id));
        }
        flow.current_page_mut()
            .commands
            .push(PageCommand::DeferredField(DeferredFieldLine {
                plan: Arc::clone(&plan),
                line_index,
                x,
                y,
                reserved_width: line.metrics.width(),
                reserved_height: height,
            }));
        if let Some(structure_id) = structure_id {
            flow.current_page_mut()
                .commands
                .push(PageCommand::EndStructure(structure_id));
        }
        flow.cursor_y -= height;
        report.line_pages.push(page);
        if force_page {
            flow.add_page_break_to(FlowPageBreak::NextPage);
        }
    }
    report.fields = plan
        .spans
        .iter()
        .map(|span| BodyFieldInfo {
            field: span.field_index,
            template_utf8_range: [span.template_range.start, span.template_range.end],
            line: span.line_index,
            max_characters: span.format.max_characters,
            clickable: span.format.link_to_anchor,
        })
        .collect();
    Ok(report)
}

pub(super) fn anchor_position(builder: &PdfBuilder, name: &str) -> Result<(usize, usize, f64)> {
    validate_anchor_name(name)?;
    let anchor = builder.anchors.get(name).ok_or_else(|| {
        WellfriendError::invalid_input(format!("unresolved authored anchor {name:?}"))
    })?;
    Ok((anchor.page_index, anchor.section_index, anchor.y))
}

pub(super) fn validate_anchor_shift(
    builder: &PdfBuilder,
    page_delta: usize,
    section_delta: usize,
) -> Result<()> {
    for anchor in builder.anchors.values() {
        let page = builder.pages.get(anchor.page_index).ok_or_else(|| {
            WellfriendError::invalid_input("authored anchor page is missing before shift")
        })?;
        if page.section_index != Some(anchor.section_index)
            || !anchor.y.is_finite()
            || anchor.y < 0.0
            || anchor.y > page.size.height
        {
            return Err(WellfriendError::invalid_input(
                "authored anchor ownership is invalid before shift",
            ));
        }
        anchor.page_index.checked_add(page_delta).ok_or_else(|| {
            WellfriendError::ResourceLimit("authored anchor page shift overflow".into())
        })?;
        anchor
            .section_index
            .checked_add(section_delta)
            .ok_or_else(|| {
                WellfriendError::ResourceLimit("authored anchor section shift overflow".into())
            })?;
    }
    Ok(())
}

pub(super) fn apply_anchor_shift(
    builder: &mut PdfBuilder,
    page_delta: usize,
    section_delta: usize,
) {
    for anchor in builder.anchors.values_mut() {
        anchor.page_index += page_delta;
        anchor.section_index += section_delta;
    }
}

fn page_sections(builder: &PdfBuilder) -> Result<(Vec<Vec<usize>>, Vec<(usize, usize)>)> {
    let assignments = sections::assignments(builder)?;
    let mut by_page = vec![(usize::MAX, usize::MAX); builder.pages.len()];
    for (section, pages) in assignments.iter().enumerate() {
        for (ordinal, page) in pages.iter().copied().enumerate() {
            by_page[page] = (section, ordinal);
        }
    }
    if by_page
        .iter()
        .any(|(section, ordinal)| *section == usize::MAX || *ordinal == usize::MAX)
    {
        return Err(WellfriendError::invalid_input(
            "body fields require every authored page to have a section owner",
        ));
    }
    Ok((assignments, by_page))
}

fn section_number(
    builder: &PdfBuilder,
    assignments: &[Vec<usize>],
    by_page: &[(usize, usize)],
    page_index: usize,
    last: bool,
    count: bool,
) -> Result<String> {
    let (section_index, ordinal) = by_page[page_index];
    let section = &builder.sections[section_index];
    let value = if count {
        assignments[section_index].len()
    } else if last {
        section
            .page_number_start
            .checked_add(assignments[section_index].len() - 1)
            .ok_or_else(|| WellfriendError::ResourceLimit("section field overflow".into()))?
    } else {
        section
            .page_number_start
            .checked_add(ordinal)
            .ok_or_else(|| WellfriendError::ResourceLimit("section field overflow".into()))?
    };
    sections::number(value, section.page_number_style)
}

fn field_value(
    builder: &PdfBuilder,
    assignments: &[Vec<usize>],
    by_page: &[(usize, usize)],
    line_page: usize,
    field: &BodyField,
) -> Result<String> {
    Ok(match field {
        BodyField::DocumentPage => (line_page + 1).to_string(),
        BodyField::DocumentPages => builder.pages.len().to_string(),
        BodyField::SectionPage => {
            section_number(builder, assignments, by_page, line_page, false, false)?
        }
        BodyField::SectionPages => {
            section_number(builder, assignments, by_page, line_page, false, true)?
        }
        BodyField::SectionLastPage => {
            section_number(builder, assignments, by_page, line_page, true, false)?
        }
        BodyField::AnchorDocumentPage(name) => {
            let anchor = builder.anchors.get(name).ok_or_else(|| {
                WellfriendError::invalid_input(format!("unresolved authored anchor {name:?}"))
            })?;
            (anchor.page_index + 1).to_string()
        }
        BodyField::AnchorSectionPage(name) => {
            let anchor = builder.anchors.get(name).ok_or_else(|| {
                WellfriendError::invalid_input(format!("unresolved authored anchor {name:?}"))
            })?;
            section_number(
                builder,
                assignments,
                by_page,
                anchor.page_index,
                false,
                false,
            )?
        }
    })
}

fn formatted_value(raw: &str, format: &BodyFieldFormat) -> Result<(String, String, Range<usize>)> {
    let count = raw.chars().count();
    if count > format.max_characters || !raw.is_ascii() {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "body field value {raw:?} exceeds its declared {}-character capacity",
            format.max_characters
        )));
    }
    let padding = format.max_characters - count;
    let (before, after) = match format.value_align {
        TextAlign::Left => (0, padding),
        TextAlign::Center => (padding / 2, padding - padding / 2),
        TextAlign::Right => (padding, 0),
    };
    let visual = format!(
        "{}{}{}{}{}",
        format.prefix,
        " ".repeat(before),
        raw,
        " ".repeat(after),
        format.suffix
    );
    let logical = format!("{}{}{}", format.prefix, raw, format.suffix);
    let visible_start = format
        .prefix
        .len()
        .checked_add(before)
        .ok_or_else(|| WellfriendError::invalid_input("body-field range overflow"))?;
    let visible_end = visible_start
        .checked_add(raw.len())
        .ok_or_else(|| WellfriendError::invalid_input("body-field range overflow"))?;
    Ok((visual, logical, visible_start..visible_end))
}

fn aligned_x(line: &layout::Line, placement: Placement, align: TextAlign) -> f64 {
    let unused = (placement.reserved_width - line.metrics.width()).max(0.0);
    placement.x
        + match align {
            TextAlign::Left => 0.0,
            TextAlign::Center => unused / 2.0,
            TextAlign::Right => unused,
        }
}

fn command_with_logical(
    line: &layout::Line,
    logical: &str,
    placement: Placement,
    style: &TextStyle,
    align: TextAlign,
) -> Result<PageCommand> {
    let mut command = line.command(aligned_x(line, placement, align), placement.y, style)?;
    match &mut command {
        PageCommand::Text {
            text, logical_text, ..
        } => {
            if text != logical {
                *logical_text = Some(logical.to_string());
            }
        }
        PageCommand::TextGroup { logical_text, .. } => *logical_text = logical.to_string(),
        PageCommand::LogicalBreak { text, .. } if text == logical => {}
        _ => {
            return Err(WellfriendError::invalid_input(
                "body field resolved to an incompatible line command",
            ))
        }
    }
    Ok(command)
}

fn resolve_plan(
    builder: &PdfBuilder,
    assignments: &[Vec<usize>],
    by_page: &[(usize, usize)],
    work: &PlanWork,
) -> Result<ResolvedPlan> {
    let placements = work
        .placements
        .iter()
        .map(|placement| {
            placement.ok_or_else(|| {
                WellfriendError::invalid_input("body-field plan has a missing line placement")
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut visual = work.plan.template.clone();
    let mut logical_lines = work
        .plan
        .line_ranges
        .iter()
        .map(|range| work.plan.template[range.clone()].to_string())
        .collect::<Vec<_>>();
    let mut clickable_ranges = BTreeMap::<usize, (Range<usize>, String)>::new();
    for span in work.plan.spans.iter().rev() {
        let line_page = placements[span.line_index].page_index;
        let raw = field_value(builder, assignments, by_page, line_page, &span.field)?;
        let (painted, logical, visible) = formatted_value(&raw, &span.format)?;
        if painted.len() != span.template_range.len() {
            return Err(WellfriendError::invalid_input(
                "body-field fixed-capacity replacement changed template bytes",
            ));
        }
        visual.replace_range(span.template_range.clone(), &painted);
        if span.format.link_to_anchor {
            let destination = match &span.field {
                BodyField::AnchorDocumentPage(name) | BodyField::AnchorSectionPage(name) => {
                    name.clone()
                }
                _ => {
                    return Err(WellfriendError::invalid_input(
                        "non-anchor body field retained a link request",
                    ))
                }
            };
            let start = span
                .template_range
                .start
                .checked_add(visible.start)
                .ok_or_else(|| WellfriendError::invalid_input("body-field link range overflow"))?;
            let end = span
                .template_range
                .start
                .checked_add(visible.end)
                .ok_or_else(|| WellfriendError::invalid_input("body-field link range overflow"))?;
            clickable_ranges.insert(span.field_index, (start..end, destination));
        }
        let line_start = work.plan.line_ranges[span.line_index].start;
        let relative = span.template_range.start - line_start..span.template_range.end - line_start;
        logical_lines[span.line_index].replace_range(relative, &logical);
    }
    let page = &builder.pages[placements[0].page_index];
    let resolved = layout::prepare_ranges_with_tabs(
        page,
        &visual,
        &work.plan.line_ranges,
        &work.plan.style,
        &work.plan.tab_stops,
    )?;
    if resolved.len() != placements.len() {
        return Err(WellfriendError::invalid_input(
            "body-field resolution changed its line count",
        ));
    }
    let commands = resolved
        .iter()
        .zip(&logical_lines)
        .zip(placements.iter().copied())
        .map(|((line, logical), placement)| {
            let height = line.metrics.ascent + line.metrics.descent;
            if line.metrics.width() > placement.reserved_width + EPS
                || height > placement.reserved_height + EPS
            {
                return Err(WellfriendError::UnsupportedFeature(
                    "resolved body field exceeds its reserved line geometry".into(),
                ));
            }
            command_with_logical(line, logical, placement, &work.plan.style, work.plan.align)
        })
        .collect::<Result<Vec<_>>>()?;

    let mut links = Vec::with_capacity(clickable_ranges.len());
    for span in &work.plan.spans {
        let Some((field_range, destination)) = clickable_ranges.get(&span.field_index) else {
            continue;
        };
        let placement = placements[span.line_index];
        let line = &resolved[span.line_index];
        let (field_left, field_right) = layout::range_x_bounds_with_tabs(
            &builder.pages[placement.page_index],
            &visual,
            work.plan.line_ranges[span.line_index].clone(),
            field_range.clone(),
            &work.plan.style,
            &work.plan.tab_stops,
        )?;
        let base_x = aligned_x(line, placement, work.plan.align);
        let rect = [
            base_x + field_left,
            placement.y - line.metrics.descent,
            base_x + field_right,
            placement.y + line.metrics.ascent,
        ];
        let page = &builder.pages[placement.page_index];
        if !rect.iter().all(|value| value.is_finite())
            || rect[0] < -EPS
            || rect[1] < -EPS
            || rect[2] > page.size.width + EPS
            || rect[3] > page.size.height + EPS
            || rect[2] - rect[0] <= EPS
            || rect[3] - rect[1] <= EPS
        {
            return Err(WellfriendError::UnsupportedFeature(
                "resolved body-field link exceeds valid page geometry".into(),
            ));
        }
        links.push((
            placement.page_index,
            AuthoredLink {
                name: format!("WFAuthoredLink-{}-{}", work.plan.id, span.field_index),
                destination: destination.clone(),
                rect,
                contents: format!("Go to {destination}"),
                structure_id: work.plan.structure_id,
            },
        ));
    }
    Ok(ResolvedPlan { commands, links })
}

fn rectangles_overlap(left: [f64; 4], right: [f64; 4]) -> bool {
    left[0] < right[2] - EPS
        && right[0] < left[2] - EPS
        && left[1] < right[3] - EPS
        && right[1] < left[3] - EPS
}

fn add_link(page: &mut PdfPageBuilder, link: AuthoredLink) -> Result<()> {
    for existing in &page.links {
        if rectangles_overlap(existing.rect, link.rect) {
            return Err(WellfriendError::UnsupportedFeature(
                if existing.destination == link.destination && existing.rect == link.rect {
                    "duplicate authored body-field link rectangle"
                } else {
                    "overlapping authored body-field link rectangles"
                }
                .into(),
            ));
        }
    }
    page.links.push(link);
    Ok(())
}

pub(super) fn materialize(builder: &PdfBuilder) -> Result<PdfBuilder> {
    if builder.fields_materialized
        || builder.pages.iter().all(|page| {
            page.commands
                .iter()
                .all(|command| !matches!(command, PageCommand::DeferredField(_)))
        })
    {
        return Ok(builder.clone());
    }
    let (assignments, by_page) = page_sections(builder)?;
    let mut work = BTreeMap::<u64, PlanWork>::new();
    for (page_index, page) in builder.pages.iter().enumerate() {
        crate::cancel::check_current_cancel("authoring body-field plan collection")?;
        for command in &page.commands {
            let PageCommand::DeferredField(line) = command else {
                continue;
            };
            let entry = work.entry(line.plan.id).or_insert_with(|| PlanWork {
                plan: Arc::clone(&line.plan),
                placements: vec![None; line.plan.line_ranges.len()],
            });
            if !Arc::ptr_eq(&entry.plan, &line.plan)
                || line.line_index >= entry.placements.len()
                || entry.placements[line.line_index].is_some()
            {
                return Err(WellfriendError::invalid_input(
                    "duplicate or inconsistent body-field line identity",
                ));
            }
            entry.placements[line.line_index] = Some(Placement {
                page_index,
                x: line.x,
                y: line.y,
                reserved_width: line.reserved_width,
                reserved_height: line.reserved_height,
            });
        }
    }
    let mut resolved = BTreeMap::new();
    let retained_links = builder.pages.iter().try_fold(0usize, |total, page| {
        total
            .checked_add(page.links.len())
            .ok_or_else(|| WellfriendError::ResourceLimit("authored link annotation count".into()))
    })?;
    let requested_links = work.values().try_fold(0usize, |total, plan| {
        total
            .checked_add(
                plan.plan
                    .spans
                    .iter()
                    .filter(|span| span.format.link_to_anchor)
                    .count(),
            )
            .ok_or_else(|| WellfriendError::ResourceLimit("authored link annotation count".into()))
    })?;
    if retained_links
        .checked_add(requested_links)
        .is_none_or(|count| count > MAX_LINKS)
    {
        return Err(WellfriendError::ResourceLimit(
            "authored link annotation count".into(),
        ));
    }
    for (id, plan) in &work {
        crate::cancel::check_current_cancel("authoring body-field materialization")?;
        resolved.insert(*id, resolve_plan(builder, &assignments, &by_page, plan)?);
    }
    let mut output = builder.clone();
    for page in &mut output.pages {
        crate::cancel::check_current_cancel("authoring body-field publication")?;
        for command in &mut page.commands {
            if let PageCommand::DeferredField(line) = command {
                *command = resolved
                    .get(&line.plan.id)
                    .and_then(|plan| plan.commands.get(line.line_index))
                    .cloned()
                    .ok_or_else(|| {
                        WellfriendError::invalid_input("resolved body-field command is missing")
                    })?;
            }
        }
    }
    for plan in resolved.values() {
        for (page_index, link) in &plan.links {
            let page = output.pages.get_mut(*page_index).ok_or_else(|| {
                WellfriendError::invalid_input("resolved body-field link page is missing")
            })?;
            add_link(page, link.clone())?;
        }
    }
    output.fields_materialized = true;
    Ok(output)
}

pub(super) fn fixed_anchor_page_command(
    builder: &mut PdfBuilder,
    page_index: usize,
    anchor: String,
    section_page: bool,
    max_characters: usize,
    style: &TextStyle,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    align: TextAlign,
    structure_id: Option<u64>,
) -> Result<PageCommand> {
    validate_anchor_name(&anchor)?;
    let format = BodyFieldFormat::new(max_characters)
        .link_to_anchor(true)
        .value_align(align);
    format.validate()?;
    if page_index >= builder.pages.len()
        || ![x, y, width, height].iter().all(|value| value.is_finite())
        || width <= 0.0
        || height <= 0.0
    {
        return Err(WellfriendError::invalid_input(
            "invalid fixed anchor-page field geometry",
        ));
    }
    let template = placeholder(&format);
    let lines = layout::prepare(&builder.pages[page_index], &template, width, style)?;
    if lines.len() != 1 || lines[0].metrics.width() > width + EPS {
        return Err(WellfriendError::UnsupportedFeature(
            "fixed anchor-page field does not fit its reserved column".into(),
        ));
    }
    let id = builder.next_field_plan_id;
    builder.next_field_plan_id = id.checked_add(1).ok_or_else(|| {
        WellfriendError::ResourceLimit("body-field plan identity overflow".into())
    })?;
    let range = 0..template.len();
    let plan = Arc::new(DeferredFieldPlan {
        id,
        template,
        line_ranges: vec![range.clone()],
        spans: vec![FieldSpan {
            field_index: 0,
            template_range: range,
            line_index: 0,
            field: if section_page {
                BodyField::AnchorSectionPage(anchor)
            } else {
                BodyField::AnchorDocumentPage(anchor)
            },
            format,
        }],
        style: style.clone(),
        align,
        tab_stops: Default::default(),
        structure_id,
    });
    Ok(PageCommand::DeferredField(DeferredFieldLine {
        plan,
        line_index: 0,
        x,
        y,
        reserved_width: width,
        reserved_height: height,
    }))
}

pub(super) fn link_annotation_dict(
    link: &AuthoredLink,
    page_number: u32,
    struct_parent: Option<u32>,
) -> Result<PdfDictionary> {
    validate_anchor_name(&link.destination)?;
    validate_anchor_name(&link.name)?;
    if !link.name.is_ascii()
        || !link.name.starts_with("WFAuthoredLink-")
        || !link.rect.iter().all(|value| value.is_finite())
        || link.rect[2] - link.rect[0] <= EPS
        || link.rect[3] - link.rect[1] <= EPS
    {
        return Err(WellfriendError::invalid_input(
            "invalid authored link annotation rectangle",
        ));
    }
    let mut dictionary = dict(&[
        ("Type", PdfObject::Name("Annot".into())),
        ("Subtype", PdfObject::Name("Link".into())),
        (
            "Rect",
            PdfObject::Array(link.rect.into_iter().map(pdf_number).collect()),
        ),
        (
            "Border",
            PdfObject::Array(vec![
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                PdfObject::Integer(0),
            ]),
        ),
        ("H", PdfObject::Name("I".into())),
        ("P", reference(page_number)),
        ("NM", PdfObject::String(pdf_text_string(&link.name))),
        (
            "Dest",
            PdfObject::String(pdf_text_string(&link.destination)),
        ),
        (
            "Contents",
            PdfObject::String(pdf_text_string(&link.contents)),
        ),
        ("F", PdfObject::Integer(4)),
    ]);
    if let Some(struct_parent) = struct_parent {
        dictionary.insert("StructParent", PdfObject::Integer(i64::from(struct_parent)));
    }
    Ok(dictionary)
}

#[derive(Debug)]
pub(super) struct DestinationTreeObjects {
    pub(super) names: Option<PdfObject>,
    pub(super) objects: Vec<OutputObject>,
}

#[derive(Debug, Clone)]
struct DestinationEntry {
    key: Vec<u8>,
    page_number: u32,
    y: f64,
}

#[derive(Debug, Clone)]
struct DestinationNode {
    number: u32,
    first: Vec<u8>,
    last: Vec<u8>,
}

fn destination_entries(builder: &PdfBuilder, page_start: u32) -> Result<Vec<DestinationEntry>> {
    if builder.anchors.is_empty() {
        return Ok(Vec::new());
    }
    let (_, by_page) = page_sections(builder)?;
    let mut encoded = Vec::with_capacity(builder.anchors.len());
    for (name, anchor) in &builder.anchors {
        crate::cancel::check_current_cancel("authoring named destination")?;
        validate_anchor_name(name)?;
        if anchor.name != *name
            || anchor.page_index >= builder.pages.len()
            || by_page[anchor.page_index].0 != anchor.section_index
            || !anchor.y.is_finite()
            || anchor.y < 0.0
            || anchor.y > builder.pages[anchor.page_index].size.height
        {
            return Err(WellfriendError::invalid_input(
                "invalid retained authored anchor",
            ));
        }
        let page_number = page_start
            .checked_add(
                u32::try_from(anchor.page_index).map_err(|_| {
                    WellfriendError::ResourceLimit("authored anchor page index".into())
                })?,
            )
            .ok_or_else(|| WellfriendError::ResourceLimit("authored anchor page number".into()))?;
        encoded.push(DestinationEntry {
            key: pdf_text_string(name),
            page_number,
            y: anchor.y,
        });
    }
    // PDF name-tree keys are ordered by the encoded byte strings, not Rust's
    // Unicode scalar ordering. Keep the emitted tree canonical for every mix
    // of PDFDocEncoding and UTF-16BE keys.
    encoded.sort_by(|left, right| left.key.cmp(&right.key));
    if encoded.windows(2).any(|pair| pair[0].key == pair[1].key) {
        return Err(WellfriendError::invalid_input(
            "authored anchor names collide after PDF text encoding",
        ));
    }
    Ok(encoded)
}

fn destination_limits(first: Vec<u8>, last: Vec<u8>) -> PdfObject {
    PdfObject::Array(vec![PdfObject::String(first), PdfObject::String(last)])
}

fn destination_array(entries: &[DestinationEntry]) -> Result<Vec<PdfObject>> {
    let capacity = entries
        .len()
        .checked_mul(2)
        .ok_or_else(|| WellfriendError::ResourceLimit("authored destination array".into()))?;
    let mut items = Vec::with_capacity(capacity);
    for entry in entries {
        items.push(PdfObject::String(entry.key.clone()));
        items.push(PdfObject::Array(vec![
            reference(entry.page_number),
            PdfObject::Name("XYZ".into()),
            PdfObject::Null,
            pdf_number(entry.y),
            PdfObject::Null,
        ]));
    }
    Ok(items)
}

fn allocate_destination_object(next: &mut u32) -> Result<u32> {
    let number = *next;
    *next = next.checked_add(1).ok_or_else(|| {
        WellfriendError::ResourceLimit("authored destination object number".into())
    })?;
    Ok(number)
}

/// Build a balanced indirect name tree. Leaves and internal nodes are bounded
/// independently, so the public 100k-anchor budget cannot create one enormous
/// dictionary or a linear viewer lookup path.
pub(super) fn destination_tree_objects(
    builder: &PdfBuilder,
    page_start: u32,
    next: &mut u32,
) -> Result<DestinationTreeObjects> {
    let entries = destination_entries(builder, page_start)?;
    if entries.is_empty() {
        return Ok(DestinationTreeObjects {
            names: None,
            objects: Vec::new(),
        });
    }
    let mut objects = Vec::new();
    let mut level = Vec::new();
    for entries in entries.chunks(NAME_TREE_FANOUT) {
        crate::cancel::check_current_cancel("authoring destination leaf")?;
        let first = entries.first().unwrap().key.clone();
        let last = entries.last().unwrap().key.clone();
        let number = allocate_destination_object(next)?;
        let mut leaf = PdfDictionary::empty();
        leaf.insert("Names", PdfObject::Array(destination_array(entries)?));
        leaf.insert("Limits", destination_limits(first.clone(), last.clone()));
        objects.push(OutputObject {
            number,
            object: PdfObject::Dictionary(leaf),
        });
        level.push(DestinationNode {
            number,
            first,
            last,
        });
    }
    while level.len() > 1 {
        let mut parents = Vec::with_capacity(level.len().div_ceil(NAME_TREE_FANOUT));
        for children in level.chunks(NAME_TREE_FANOUT) {
            crate::cancel::check_current_cancel("authoring destination branch")?;
            let first = children.first().unwrap().first.clone();
            let last = children.last().unwrap().last.clone();
            let number = allocate_destination_object(next)?;
            let mut branch = PdfDictionary::empty();
            branch.insert(
                "Kids",
                PdfObject::Array(
                    children
                        .iter()
                        .map(|child| reference(child.number))
                        .collect(),
                ),
            );
            branch.insert("Limits", destination_limits(first.clone(), last.clone()));
            objects.push(OutputObject {
                number,
                object: PdfObject::Dictionary(branch),
            });
            parents.push(DestinationNode {
                number,
                first,
                last,
            });
        }
        level = parents;
    }
    let mut names = PdfDictionary::empty();
    names.insert("Dests", reference(level[0].number));
    Ok(DestinationTreeObjects {
        names: Some(PdfObject::Dictionary(names)),
        objects,
    })
}

#[cfg(test)]
pub(super) fn named_destinations(
    builder: &PdfBuilder,
    page_start: u32,
) -> Result<Option<PdfObject>> {
    if builder.anchors.is_empty() {
        return Ok(None);
    }
    let encoded = destination_entries(builder, page_start)?;
    let first = encoded
        .first()
        .map(|entry| entry.key.clone())
        .ok_or_else(|| WellfriendError::invalid_input("empty authored destination tree"))?;
    let last = encoded
        .last()
        .map(|entry| entry.key.clone())
        .ok_or_else(|| WellfriendError::invalid_input("empty authored destination tree"))?;
    let mut destinations = PdfDictionary::empty();
    destinations.insert("Names", PdfObject::Array(destination_array(&encoded)?));
    destinations.insert(
        "Limits",
        PdfObject::Array(vec![PdfObject::String(first), PdfObject::String(last)]),
    );
    let mut names = PdfDictionary::empty();
    names.insert("Dests", PdfObject::Dictionary(destinations));
    Ok(Some(PdfObject::Dictionary(names)))
}
