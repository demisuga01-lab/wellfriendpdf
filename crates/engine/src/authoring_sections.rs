//! Typed fresh-document sections. Running content is materialized on a private
//! serialization clone after final page/section counts are known.
use super::*;

#[cfg(test)]
#[path = "authoring_sections_tests.rs"]
mod tests;

const MAX_RUNNING_PARTS: usize = 1024;
const MAX_RUNNING_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PageNumberStyle {
    #[default]
    Decimal,
    LowerRoman,
    UpperRoman,
    LowerAlpha,
    UpperAlpha,
}
impl PageNumberStyle {
    pub(super) fn pdf_name(self) -> &'static str {
        match self {
            Self::Decimal => "D",
            Self::LowerRoman => "r",
            Self::UpperRoman => "R",
            Self::LowerAlpha => "a",
            Self::UpperAlpha => "A",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageNumberField {
    /// One-based physical page index in the authored PDF.
    DocumentPage,
    /// Total physical page count in the authored PDF.
    DocumentPages,
    /// Section page label number, including the section's configured start.
    SectionPage,
    /// Number of physical pages owned by the section.
    SectionPages,
    /// Final section page label number (`start + page_count - 1`).
    SectionLastPage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunningTextPart {
    Text(String),
    Field(PageNumberField),
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunningText {
    pub parts: Vec<RunningTextPart>,
    pub style: TextStyle,
    pub align: TextAlign,
    /// Header: distance from top edge. Footer: distance from bottom edge.
    pub baseline_from_edge: f64,
}
impl RunningText {
    pub fn new(parts: Vec<RunningTextPart>, style: TextStyle) -> Self {
        Self {
            parts,
            style,
            align: TextAlign::Left,
            baseline_from_edge: 24.0,
        }
    }
    pub fn literal(text: impl Into<String>, style: TextStyle) -> Self {
        Self::new(vec![RunningTextPart::Text(text.into())], style)
    }
    pub fn align(mut self, align: TextAlign) -> Self {
        self.align = align;
        self
    }
    pub fn baseline_from_edge(mut self, points: f64) -> Self {
        self.baseline_from_edge = points;
        self
    }
    fn validate(&self) -> Result<()> {
        let bytes = self
            .parts
            .iter()
            .map(|part| match part {
                RunningTextPart::Text(text) => text.len(),
                RunningTextPart::Field(_) => 0,
            })
            .sum::<usize>();
        if self.parts.len() > MAX_RUNNING_PARTS
            || bytes > MAX_RUNNING_BYTES
            || !self.baseline_from_edge.is_finite()
            || self.baseline_from_edge < 0.0
            || self.parts.iter().any(|part| match part {
                RunningTextPart::Text(text) => {
                    text.chars().any(crate::fonts::hard_break::is_hard_break)
                }
                RunningTextPart::Field(_) => false,
            })
        {
            return Err(WellfriendError::invalid_input(
                "invalid running header/footer content or geometry",
            ));
        }
        layout::validate_single_line("", 0.0, 0.0, &self.style)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SectionPageMaster {
    pub header: Option<RunningText>,
    pub footer: Option<RunningText>,
}
impl SectionPageMaster {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn header(mut self, header: RunningText) -> Self {
        self.header = Some(header);
        self
    }
    pub fn footer(mut self, footer: RunningText) -> Self {
        self.footer = Some(footer);
        self
    }
    fn validate(&self) -> Result<()> {
        if let Some(header) = &self.header {
            header.validate()?;
        }
        if let Some(footer) = &self.footer {
            footer.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FlowSection {
    pub page_size: PageSize,
    pub margins: Margins,
    /// Swap left/right margins on even physical pages for facing-page layout.
    pub mirror_margins: bool,
    /// Used on the first page when present; otherwise odd/even selection applies.
    pub first: Option<SectionPageMaster>,
    pub odd: SectionPageMaster,
    pub even: SectionPageMaster,
    pub page_number_start: usize,
    pub page_number_style: PageNumberStyle,
    pub footnote_numbering: FootnoteNumbering,
}
impl FlowSection {
    pub fn new(page_size: PageSize, margins: Margins) -> Self {
        Self {
            page_size,
            margins,
            mirror_margins: false,
            first: None,
            odd: SectionPageMaster::default(),
            even: SectionPageMaster::default(),
            page_number_start: 1,
            page_number_style: PageNumberStyle::Decimal,
            footnote_numbering: FootnoteNumbering::default(),
        }
    }
    pub fn first_master(mut self, master: SectionPageMaster) -> Self {
        self.first = Some(master);
        self
    }
    pub fn mirrored_margins(mut self, enabled: bool) -> Self {
        self.mirror_margins = enabled;
        self
    }
    pub fn odd_master(mut self, master: SectionPageMaster) -> Self {
        self.odd = master;
        self
    }
    pub fn even_master(mut self, master: SectionPageMaster) -> Self {
        self.even = master;
        self
    }
    pub fn page_numbering(mut self, start: usize, style: PageNumberStyle) -> Self {
        self.page_number_start = start;
        self.page_number_style = style;
        self
    }
    pub fn footnote_numbering(mut self, numbering: FootnoteNumbering) -> Self {
        self.footnote_numbering = numbering;
        self
    }
    pub(super) fn validate(&self) -> Result<()> {
        let size = self.page_size;
        let margins = self.margins;
        if ![
            size.width,
            size.height,
            margins.left,
            margins.right,
            margins.top,
            margins.bottom,
        ]
        .iter()
        .all(|value| value.is_finite() && *value >= 0.0)
            || size.width <= 0.0
            || size.height <= 0.0
            || margins.left + margins.right >= size.width
            || margins.top + margins.bottom >= size.height
            || self.page_number_start == 0
            || self.page_number_start > i64::MAX as usize
        {
            return Err(WellfriendError::invalid_input(
                "invalid authoring section geometry/numbering",
            ));
        }
        if let Some(first) = &self.first {
            first.validate()?;
        }
        self.odd.validate()?;
        self.even.validate()?;
        self.footnote_numbering.validate()
    }

    pub(super) fn effective_margins(&self, physical_page: usize) -> Margins {
        if self.mirror_margins && physical_page.is_multiple_of(2) {
            Margins {
                left: self.margins.right,
                right: self.margins.left,
                top: self.margins.top,
                bottom: self.margins.bottom,
            }
        } else {
            self.margins
        }
    }
}

struct PageContext {
    document_page: usize,
    document_pages: usize,
    section_page: usize,
    section_pages: usize,
    section_last_page: usize,
    section_style: PageNumberStyle,
}

fn roman(mut value: usize) -> Result<String> {
    if value == 0 || value > 3999 {
        return Err(WellfriendError::invalid_input(
            "Roman page numbering supports 1..=3999",
        ));
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
        return Err(WellfriendError::invalid_input(
            "alphabetic page number is zero",
        ));
    }
    let mut reversed = Vec::new();
    while value > 0 {
        value -= 1;
        reversed.push((b'A' + (value % 26) as u8) as char);
        value /= 26;
        if reversed.len() > 64 {
            return Err(WellfriendError::ResourceLimit(
                "alphabetic page number length".into(),
            ));
        }
    }
    Ok(reversed.into_iter().rev().collect())
}

pub(super) fn number(value: usize, style: PageNumberStyle) -> Result<String> {
    Ok(match style {
        PageNumberStyle::Decimal => value.to_string(),
        PageNumberStyle::LowerRoman => roman(value)?.to_lowercase(),
        PageNumberStyle::UpperRoman => roman(value)?,
        PageNumberStyle::LowerAlpha => alpha(value)?.to_lowercase(),
        PageNumberStyle::UpperAlpha => alpha(value)?,
    })
}

fn running_text(spec: &RunningText, context: &PageContext) -> Result<String> {
    let mut out = String::new();
    for part in &spec.parts {
        crate::cancel::check_current_cancel("authoring running text")?;
        match part {
            RunningTextPart::Text(text) => out.push_str(text),
            RunningTextPart::Field(PageNumberField::DocumentPage) => {
                out.push_str(&context.document_page.to_string())
            }
            RunningTextPart::Field(PageNumberField::DocumentPages) => {
                out.push_str(&context.document_pages.to_string())
            }
            RunningTextPart::Field(PageNumberField::SectionPage) => {
                out.push_str(&number(context.section_page, context.section_style)?)
            }
            RunningTextPart::Field(PageNumberField::SectionPages) => {
                out.push_str(&number(context.section_pages, context.section_style)?)
            }
            RunningTextPart::Field(PageNumberField::SectionLastPage) => {
                out.push_str(&number(context.section_last_page, context.section_style)?)
            }
        }
        if out.len() > MAX_RUNNING_BYTES {
            return Err(WellfriendError::ResourceLimit(
                "materialized running text byte budget".into(),
            ));
        }
    }
    Ok(out)
}

fn paint_running(
    page: &mut PdfPageBuilder,
    spec: &RunningText,
    context: &PageContext,
    header: bool,
) -> Result<()> {
    let text = running_text(spec, context)?;
    if text.is_empty() {
        return Ok(());
    }
    let width = page.size.width - page.margins.left - page.margins.right;
    let lines = layout::prepare(page, &text, width, &spec.style)?;
    if lines.len() != 1 || lines[0].logical != text {
        return Err(WellfriendError::UnsupportedFeature(
            "running header/footer must fit one line in its section margin".into(),
        ));
    }
    let line = &lines[0];
    let baseline = if header {
        page.size.height - spec.baseline_from_edge
    } else {
        spec.baseline_from_edge
    };
    let within_margin = if header {
        baseline + line.metrics.ascent <= page.size.height + 1e-7
            && baseline - line.metrics.descent >= page.size.height - page.margins.top - 1e-7
    } else {
        baseline - line.metrics.descent >= -1e-7
            && baseline + line.metrics.ascent <= page.margins.bottom + 1e-7
    };
    if !within_margin {
        return Err(WellfriendError::invalid_input(
            "running header/footer does not fit inside the reserved page margin",
        ));
    }
    let x = line.aligned_x(page.margins.left, width, spec.align);
    let command = line.command(x, baseline, &spec.style)?;
    page.commands.push(PageCommand::BeginArtifact);
    page.commands.push(command);
    page.commands.push(PageCommand::EndArtifact);
    Ok(())
}

pub(super) fn assignments(builder: &PdfBuilder) -> Result<Vec<Vec<usize>>> {
    let mut pages = vec![Vec::new(); builder.sections.len()];
    let mut last = None;
    for (index, page) in builder.pages.iter().enumerate() {
        let Some(section) = page.section_index else {
            if last.is_some() {
                return Err(WellfriendError::invalid_input(
                    "unsectioned page interrupts authored section sequence",
                ));
            }
            continue;
        };
        if section >= builder.sections.len() || last.is_some_and(|previous| section < previous) {
            return Err(WellfriendError::invalid_input(
                "invalid or nonmonotone authored section assignment",
            ));
        }
        if page.size != builder.sections[section].page_size
            || page.margins != builder.sections[section].effective_margins(index + 1)
        {
            return Err(WellfriendError::invalid_input(
                "authored section geometry changed after page layout",
            ));
        }
        last = Some(section);
        pages[section].push(index);
    }
    if pages.iter().any(Vec::is_empty) {
        return Err(WellfriendError::invalid_input(
            "authored section has no pages",
        ));
    }
    Ok(pages)
}

pub(super) fn materialize(builder: &PdfBuilder) -> Result<PdfBuilder> {
    if builder.sections.is_empty() || builder.section_masters_materialized {
        return Ok(builder.clone());
    }
    for section in &builder.sections {
        section.validate()?;
    }
    let section_pages = assignments(builder)?;
    let mut result = builder.clone();
    let document_pages = result.pages.len();
    for (section_index, indexes) in section_pages.iter().enumerate() {
        let section = result.sections[section_index].clone();
        for (ordinal, &page_index) in indexes.iter().enumerate() {
            crate::cancel::check_current_cancel("authoring section materialization")?;
            let physical = page_index + 1;
            let master = if ordinal == 0 {
                section.first.as_ref().unwrap_or({
                    if physical % 2 == 0 {
                        &section.even
                    } else {
                        &section.odd
                    }
                })
            } else if physical % 2 == 0 {
                &section.even
            } else {
                &section.odd
            };
            let section_page = section
                .page_number_start
                .checked_add(ordinal)
                .ok_or_else(|| {
                    WellfriendError::ResourceLimit("section page number overflow".into())
                })?;
            let section_last_page = section
                .page_number_start
                .checked_add(indexes.len() - 1)
                .ok_or_else(|| {
                    WellfriendError::ResourceLimit("section page total overflow".into())
                })?;
            let context = PageContext {
                document_page: physical,
                document_pages,
                section_page,
                section_pages: indexes.len(),
                section_last_page,
                section_style: section.page_number_style,
            };
            let page = &mut result.pages[page_index];
            if page.suppress_section_master {
                continue;
            }
            if let Some(header) = &master.header {
                paint_running(page, header, &context, true)?;
            }
            if let Some(footer) = &master.footer {
                paint_running(page, footer, &context, false)?;
            }
        }
    }
    result.section_masters_materialized = true;
    Ok(result)
}

pub(super) fn page_labels(builder: &PdfBuilder) -> Result<Option<PdfObject>> {
    if builder.sections.is_empty() {
        return Ok(None);
    }
    if builder.sections.len() == 1
        && builder.sections[0].page_number_start == 1
        && builder.sections[0].page_number_style == PageNumberStyle::Decimal
    {
        return Ok(None);
    }
    let pages = assignments(builder)?;
    let mut nums = Vec::with_capacity(pages.len() * 2);
    for (index, section_pages) in pages.iter().enumerate() {
        let section = &builder.sections[index];
        let mut label = PdfDictionary::empty();
        label.insert(
            "S",
            PdfObject::Name(section.page_number_style.pdf_name().into()),
        );
        if section.page_number_start != 1 {
            label.insert(
                "St",
                PdfObject::Integer(i64::try_from(section.page_number_start).map_err(|_| {
                    WellfriendError::ResourceLimit("section page label overflow".into())
                })?),
            );
        }
        nums.push(PdfObject::Integer(section_pages[0] as i64));
        nums.push(PdfObject::Dictionary(label));
    }
    let mut labels = PdfDictionary::empty();
    labels.insert("Nums", PdfObject::Array(nums));
    Ok(Some(PdfObject::Dictionary(labels)))
}
