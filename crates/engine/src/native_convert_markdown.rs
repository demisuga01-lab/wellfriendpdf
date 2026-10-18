//! CommonMark + GFM Markdown to PDF via `pulldown-cmark` and rich flow
//! paragraphs. Remote URLs are never fetched: remote images become visible
//! placeholders and links become `/URI` link annotations.
use super::csv_pdf::{fit_columns, natural_column_widths};
use super::*;
use crate::authoring::{
    RichParagraphStyle, RichSpan, TableBuilder, TableCell, TableColumn, TableStyle, TextAlign,
};
use crate::content::Color;
use pulldown_cmark::{Alignment, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use std::path::{Path, PathBuf};

const MAX_IMAGE_BYTES: usize = 32 * 1024 * 1024;

/// Options for [`markdown_to_pdf`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MarkdownToPdfOptions {
    pub page_size: NativePageSize,
    pub landscape: bool,
    /// `portrait` or `landscape`; overrides `landscape` when set.
    pub orientation: Option<NativeOrientation>,
    pub margins: NativeMargins,
    /// Body size in points, 7..=22. Headings and code scale from it.
    pub font_size: f64,
    /// Directory that relative local image paths resolve against. Images
    /// outside it, and all local images when unset, become placeholders.
    pub base_dir: Option<PathBuf>,
    /// Document title; defaults to the first level-1 heading.
    pub title: Option<String>,
    pub author: Option<String>,
}

impl Default for MarkdownToPdfOptions {
    fn default() -> Self {
        Self {
            page_size: NativePageSize::A4,
            landscape: false,
            orientation: None,
            margins: NativeMargins::default(),
            font_size: 10.5,
            base_dir: None,
            title: None,
            author: None,
        }
    }
}

pub fn markdown_to_pdf(input: &[u8], options: &MarkdownToPdfOptions) -> Result<Vec<u8>> {
    Ok(markdown_to_pdf_with_report(input, options)?.pdf)
}

pub fn markdown_to_pdf_with_report(
    input: &[u8],
    options: &MarkdownToPdfOptions,
) -> Result<NativeConversionOutput> {
    let text = decode_utf8(input)?;
    check_font_size(options.font_size)?;
    let page = oriented(
        options.page_size.page_size(),
        options.landscape,
        options.orientation,
    );
    let margins = options.margins.resolve(page)?;
    let flow = FlowDocument::new(page, margins);
    let base_dir = match &options.base_dir {
        Some(dir) => Some(dir.canonicalize().map_err(|error| {
            input_error(
                codes::INVALID_OPTIONS,
                format!("base_dir {} is not readable: {error}", dir.display()),
            )
        })?),
        None => None,
    };
    let mut renderer = Renderer {
        flow,
        fonts: NativeFonts::new(),
        report: NativeConversionReport::default(),
        size: options.font_size,
        base_dir,
        spans: Vec::new(),
        strong: 0,
        emphasis: 0,
        strike: 0,
        links: Vec::new(),
        quote_depth: 0,
        lists: Vec::new(),
        pending_marker: None,
        heading: None,
        code_block: None,
        image: None,
        table: None,
        first_heading: None,
        blocks: 0,
    };
    let parser = Parser::new_ext(
        text,
        Options::ENABLE_TABLES
            | Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_TASKLISTS
            | Options::ENABLE_GFM,
    );
    for event in parser {
        crate::cancel::check_current_cancel("markdown-to-pdf")?;
        renderer.event(event)?;
    }
    renderer.flush_paragraph()?;
    let title = options
        .title
        .clone()
        .or_else(|| renderer.first_heading.clone());
    let mut flow = renderer.flow;
    apply_metadata(
        &mut flow,
        title.as_deref(),
        options.author.as_deref(),
        "markdown-to-pdf",
    );
    finish(flow.into_builder(), renderer.report)
}

struct ListState {
    next: Option<u64>,
}

struct TableState {
    alignments: Vec<Alignment>,
    header: Vec<String>,
    rows: Vec<Vec<String>>,
    current_row: Vec<String>,
    cell: Option<String>,
    in_head: bool,
}

struct ImageState {
    url: String,
    alt: String,
}

struct Renderer {
    flow: FlowDocument,
    fonts: NativeFonts,
    report: NativeConversionReport,
    size: f64,
    base_dir: Option<PathBuf>,
    spans: Vec<RichSpan>,
    strong: usize,
    emphasis: usize,
    strike: usize,
    links: Vec<String>,
    quote_depth: usize,
    lists: Vec<ListState>,
    pending_marker: Option<String>,
    heading: Option<u8>,
    code_block: Option<String>,
    image: Option<ImageState>,
    table: Option<TableState>,
    first_heading: Option<String>,
    blocks: usize,
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn link_color() -> Color {
    Color::device_rgb(0.05, 0.27, 0.63)
}

impl Renderer {
    fn indent(&self) -> f64 {
        self.lists.len() as f64 * 18.0 + self.quote_depth as f64 * 14.0
    }

    fn heading_size(&self, level: u8) -> f64 {
        self.size
            * match level {
                1 => 2.0,
                2 => 1.6,
                3 => 1.3,
                4 => 1.15,
                _ => 1.0,
            }
    }

    fn current_kind(&self, code: bool) -> StackKind {
        let bold = self.strong > 0 || self.heading.is_some();
        match (code, bold, self.emphasis > 0) {
            (true, false, _) => StackKind::Mono,
            (true, true, _) => StackKind::MonoBold,
            (false, true, true) => StackKind::SansBoldItalic,
            (false, true, false) => StackKind::SansBold,
            (false, false, true) => StackKind::SansItalic,
            (false, false, false) => StackKind::Sans,
        }
    }

    fn push_text(&mut self, text: &str, code: bool) -> Result<()> {
        if let Some(image) = &mut self.image {
            image.alt.push_str(text);
            return Ok(());
        }
        if let Some(table) = &mut self.table {
            if let Some(cell) = &mut table.cell {
                cell.push_str(text);
            }
            return Ok(());
        }
        let kind = self.current_kind(code);
        let size = match self.heading {
            Some(level) => self.heading_size(level),
            None if code => self.size * 0.92,
            None => self.size,
        };
        let text = self
            .fonts
            .sanitize(kind, text, &mut self.report.replaced_characters);
        let mut style = self.fonts.style(&mut self.flow, kind, size)?;
        if code {
            style = style.fill(Color::device_rgb(0.55, 0.1, 0.1));
        }
        let mut span = RichSpan::new(text, style).strike(self.strike > 0);
        if let Some(link) = self.links.last() {
            if !link.is_empty() {
                span.style = span.style.fill(link_color());
                span = span.link(link.clone());
            }
        }
        self.spans.push(span);
        Ok(())
    }

    fn block_gap(&mut self) {
        if self.blocks > 0 {
            self.flow.add_spacer(self.size * 0.55);
        }
        self.blocks += 1;
    }

    fn flush_paragraph(&mut self) -> Result<()> {
        let marker = self.pending_marker.take();
        if self.spans.is_empty() && marker.is_none() {
            return Ok(());
        }
        let spans = std::mem::take(&mut self.spans);
        let marker = match marker {
            Some(marker) => {
                let kind = StackKind::Sans;
                let text = self
                    .fonts
                    .sanitize(kind, &marker, &mut self.report.replaced_characters);
                Some((text, self.fonts.style(&mut self.flow, kind, self.size)?))
            }
            None => None,
        };
        let in_list_item = marker.is_some();
        if !in_list_item {
            self.block_gap();
        } else {
            self.blocks += 1;
        }
        let style = RichParagraphStyle {
            indent_left: self.indent(),
            marker,
            left_rule: (self.quote_depth > 0).then(|| Color::device_gray(0.75)),
            ..RichParagraphStyle::default()
        };
        let report = self.flow.add_rich_paragraph(&spans, &style)?;
        if report.unlinked_uris > 0 {
            self.report.warnings.push(format!(
                "{} link(s) used a scheme other than http, https or mailto and were kept as text",
                report.unlinked_uris
            ));
        }
        Ok(())
    }

    fn event(&mut self, event: Event<'_>) -> Result<()> {
        if let Some(code) = &mut self.code_block {
            match event {
                Event::Text(text) => {
                    code.push_str(&text);
                    return Ok(());
                }
                Event::End(TagEnd::CodeBlock) => {
                    let code = self.code_block.take().unwrap_or_default();
                    return self.code_block_end(&code);
                }
                _ => return Ok(()),
            }
        }
        match event {
            Event::Start(tag) => self.start(tag)?,
            Event::End(tag) => self.end(tag)?,
            Event::Text(text) => self.push_text(&text, false)?,
            Event::Code(text) => self.push_text(&text, true)?,
            Event::InlineMath(text) | Event::DisplayMath(text) => self.push_text(&text, true)?,
            // Raw HTML is never interpreted; it is shown as literal source.
            Event::Html(text) | Event::InlineHtml(text) => self.push_text(&text, true)?,
            Event::FootnoteReference(text) => self.push_text(&format!("[{text}]"), false)?,
            Event::SoftBreak => self.push_text(" ", false)?,
            Event::HardBreak => self.push_text("\n", false)?,
            Event::Rule => {
                self.flush_paragraph()?;
                self.block_gap();
                self.flow
                    .add_horizontal_rule(Color::device_gray(0.7), 0.75)?;
            }
            Event::TaskListMarker(checked) => {
                self.pending_marker = Some(if checked { "\u{2611}" } else { "\u{2610}" }.into());
            }
        }
        Ok(())
    }

    fn start(&mut self, tag: Tag<'_>) -> Result<()> {
        match tag {
            Tag::Paragraph => {}
            Tag::Heading { level, .. } => {
                self.flush_paragraph()?;
                if self.blocks > 0 {
                    self.flow.add_spacer(self.size * 0.6);
                }
                self.heading = Some(heading_level(level));
            }
            Tag::BlockQuote(_) => {
                self.flush_paragraph()?;
                self.quote_depth += 1;
            }
            Tag::CodeBlock(_) => {
                self.flush_paragraph()?;
                self.code_block = Some(String::new());
            }
            Tag::HtmlBlock => self.flush_paragraph()?,
            Tag::List(start) => {
                self.flush_paragraph()?;
                self.lists.push(ListState { next: start });
            }
            Tag::Item => {
                self.flush_paragraph()?;
                let marker = match self.lists.last_mut() {
                    Some(ListState { next: Some(number) }) => {
                        let marker = format!("{number}.");
                        *number += 1;
                        marker
                    }
                    _ => "\u{2022}".to_string(),
                };
                self.pending_marker = Some(marker);
            }
            Tag::Table(alignments) => {
                self.flush_paragraph()?;
                self.table = Some(TableState {
                    alignments,
                    header: Vec::new(),
                    rows: Vec::new(),
                    current_row: Vec::new(),
                    cell: None,
                    in_head: false,
                });
            }
            Tag::TableHead => {
                if let Some(table) = &mut self.table {
                    table.in_head = true;
                    table.current_row.clear();
                }
            }
            Tag::TableRow => {
                if let Some(table) = &mut self.table {
                    table.current_row.clear();
                }
            }
            Tag::TableCell => {
                if let Some(table) = &mut self.table {
                    table.cell = Some(String::new());
                }
            }
            Tag::Emphasis => self.emphasis += 1,
            Tag::Strong => self.strong += 1,
            Tag::Strikethrough => self.strike += 1,
            Tag::Link { dest_url, .. } => self.links.push(dest_url.to_string()),
            Tag::Image { dest_url, .. } => {
                if self.table.is_none() {
                    self.image = Some(ImageState {
                        url: dest_url.to_string(),
                        alt: String::new(),
                    });
                }
            }
            Tag::FootnoteDefinition(label) => {
                self.flush_paragraph()?;
                self.push_text(&format!("[{label}] "), false)?;
            }
            Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition
            | Tag::Superscript
            | Tag::Subscript
            | Tag::MetadataBlock(_) => {}
        }
        Ok(())
    }

    fn end(&mut self, tag: TagEnd) -> Result<()> {
        match tag {
            TagEnd::Paragraph => self.flush_paragraph()?,
            TagEnd::Heading(level) => {
                let level = heading_level(level);
                let spans = std::mem::take(&mut self.spans);
                self.heading = None;
                if spans.is_empty() {
                    return Ok(());
                }
                let text: String = spans.iter().map(|span| span.text.as_str()).collect();
                if level == 1 && self.first_heading.is_none() {
                    self.first_heading = Some(text.trim().to_string());
                }
                self.blocks += 1;
                let style = RichParagraphStyle {
                    indent_left: self.indent(),
                    heading_level: Some(level),
                    line_height: 1.2,
                    space_after: self.size * 0.35,
                    left_rule: (self.quote_depth > 0).then(|| Color::device_gray(0.75)),
                    ..RichParagraphStyle::default()
                };
                self.flow.add_rich_paragraph(&spans, &style)?;
            }
            TagEnd::BlockQuote(_) => {
                self.flush_paragraph()?;
                self.quote_depth = self.quote_depth.saturating_sub(1);
            }
            TagEnd::CodeBlock => {}
            TagEnd::HtmlBlock => self.flush_paragraph()?,
            TagEnd::List(_) => {
                self.flush_paragraph()?;
                self.lists.pop();
            }
            TagEnd::Item => self.flush_paragraph()?,
            TagEnd::Table => {
                if let Some(table) = self.table.take() {
                    self.table_end(table)?;
                }
            }
            TagEnd::TableHead => {
                if let Some(table) = &mut self.table {
                    table.header = std::mem::take(&mut table.current_row);
                    table.in_head = false;
                }
            }
            TagEnd::TableRow => {
                if let Some(table) = &mut self.table {
                    if !table.in_head {
                        let row = std::mem::take(&mut table.current_row);
                        table.rows.push(row);
                    }
                }
            }
            TagEnd::TableCell => {
                if let Some(table) = &mut self.table {
                    let cell = table.cell.take().unwrap_or_default();
                    table.current_row.push(cell);
                }
            }
            TagEnd::Emphasis => self.emphasis = self.emphasis.saturating_sub(1),
            TagEnd::Strong => self.strong = self.strong.saturating_sub(1),
            TagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
            TagEnd::Link => {
                self.links.pop();
            }
            TagEnd::Image => {
                if let Some(image) = self.image.take() {
                    self.image_end(image)?;
                }
            }
            TagEnd::FootnoteDefinition => self.flush_paragraph()?,
            TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition
            | TagEnd::Superscript
            | TagEnd::Subscript
            | TagEnd::MetadataBlock(_) => {}
        }
        Ok(())
    }

    fn code_block_end(&mut self, code: &str) -> Result<()> {
        self.flush_paragraph()?;
        self.block_gap();
        let code = code.strip_suffix('\n').unwrap_or(code);
        let kind = StackKind::Mono;
        let expanded = code
            .split('\n')
            .map(|line| expand_tabs(line, 4))
            .collect::<Vec<_>>()
            .join("\n");
        let text = self
            .fonts
            .sanitize(kind, &expanded, &mut self.report.replaced_characters);
        let style = self.fonts.style(&mut self.flow, kind, self.size * 0.9)?;
        let spans = [RichSpan::new(
            if text.is_empty() { " " } else { &text },
            style,
        )];
        let block = RichParagraphStyle {
            indent_left: self.indent() + 4.0,
            indent_right: 4.0,
            line_height: 1.25,
            preserve_whitespace: true,
            background: Some(Color::device_gray(0.95)),
            left_rule: (self.quote_depth > 0).then(|| Color::device_gray(0.75)),
            ..RichParagraphStyle::default()
        };
        self.flow.add_rich_paragraph(&spans, &block)?;
        Ok(())
    }

    fn table_end(&mut self, table: TableState) -> Result<()> {
        self.block_gap();
        let columns = table
            .header
            .len()
            .max(table.rows.iter().map(Vec::len).max().unwrap_or(0));
        if columns == 0 {
            return Ok(());
        }
        let body_size = (self.size * 0.9).max(MIN_NATIVE_FONT_SIZE);
        let body = self
            .fonts
            .style(&mut self.flow, StackKind::Sans, body_size)?;
        let head = self
            .fonts
            .style(&mut self.flow, StackKind::SansBold, body_size)?;
        let mut sanitize_row = |fonts: &mut NativeFonts, row: &[String], kind| -> Vec<String> {
            (0..columns)
                .map(|index| {
                    fonts.sanitize(
                        kind,
                        row.get(index).map(String::as_str).unwrap_or(""),
                        &mut self.report.replaced_characters,
                    )
                })
                .collect()
        };
        let header = sanitize_row(&mut self.fonts, &table.header, StackKind::SansBold);
        let rows: Vec<Vec<String>> = table
            .rows
            .iter()
            .map(|row| sanitize_row(&mut self.fonts, row, StackKind::Sans))
            .collect();
        let padding = 4.0;
        let natural = natural_column_widths(&self.flow, &header, &rows, &body, &head, padding)?;
        let available = self.flow.content_width_points()? - 0.5;
        let min = padding * 2.0 + body_size * 2.5;
        let widths = fit_columns(&natural, min, available).ok_or_else(|| {
            limit_error(
                codes::CSV_TABLE_TOO_WIDE,
                format!("a Markdown table with {columns} columns cannot fit the page width"),
            )
        })?;
        let table_columns = widths
            .iter()
            .enumerate()
            .map(|(index, width)| {
                let align = match table.alignments.get(index) {
                    Some(Alignment::Center) => TextAlign::Center,
                    Some(Alignment::Right) => TextAlign::Right,
                    _ => TextAlign::Left,
                };
                TableColumn::new(*width).align(align)
            })
            .collect();
        let mut builder = TableBuilder::new(table_columns)
            .body_style(body)
            .header_style(head)
            .style(TableStyle::new().padding(padding));
        if table.header.iter().any(|cell| !cell.trim().is_empty()) {
            builder.set_header(header.into_iter().map(TableCell::text));
        }
        for row in rows {
            builder.add_row(row.into_iter().map(TableCell::text));
        }
        self.flow.add_table(&builder)?;
        Ok(())
    }

    fn image_end(&mut self, image: ImageState) -> Result<()> {
        self.flush_paragraph()?;
        let alt = image.alt.trim().to_string();
        match self.load_image(&image.url) {
            Ok(bytes) => match self.place_image(&bytes, &alt) {
                Ok(()) => return Ok(()),
                Err(error) => self.report.warnings.push(format!(
                    "image {} could not be embedded: {error}",
                    short(&image.url)
                )),
            },
            Err(reason) => self.report.warnings.push(format!(
                "image {} not embedded: {reason}",
                short(&image.url)
            )),
        }
        let label = if alt.is_empty() {
            "[image not embedded]".to_string()
        } else {
            format!("[image not embedded: {alt}]")
        };
        self.emphasis += 1;
        self.push_text(&label, false)?;
        self.emphasis -= 1;
        self.flush_paragraph()
    }

    fn load_image(&self, url: &str) -> std::result::Result<Vec<u8>, String> {
        let url = url.trim();
        if let Some(rest) = url.strip_prefix("data:") {
            return decode_data_uri(rest).ok_or_else(|| "unsupported data URI".to_string());
        }
        let lower = url.to_ascii_lowercase();
        if lower.contains("://") || lower.starts_with("//") || lower.starts_with("mailto:") {
            return Err("remote URLs are never fetched".to_string());
        }
        let Some(base) = &self.base_dir else {
            return Err("local images need a base directory (CLI input file directory)".into());
        };
        let path = url.strip_prefix("file:").unwrap_or(url);
        let path = percent_decode(path);
        let candidate = if Path::new(&path).is_absolute() {
            PathBuf::from(&path)
        } else {
            base.join(&path)
        };
        let resolved = candidate
            .canonicalize()
            .map_err(|_| "local image not found".to_string())?;
        if !resolved.starts_with(base) {
            return Err("local image is outside the base directory".to_string());
        }
        let metadata = std::fs::metadata(&resolved).map_err(|error| error.to_string())?;
        if !metadata.is_file() || metadata.len() > MAX_IMAGE_BYTES as u64 {
            return Err("local image is not a regular file under 32 MiB".to_string());
        }
        std::fs::read(&resolved).map_err(|error| error.to_string())
    }

    fn place_image(&mut self, bytes: &[u8], alt: &str) -> Result<()> {
        let (handle, width_px, height_px) = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            if bytes.len() < 24 {
                return Err(WellfriendError::invalid_input("truncated PNG"));
            }
            let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
            let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
            let handle = self.flow.builder_mut().add_png_image(bytes)?;
            (handle, width, height)
        } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            let (_, width, height, _) =
                crate::images::decoder::ImageDecoder::decode_jpeg_with_info(bytes)?;
            let handle = self.flow.builder_mut().add_jpeg_image(bytes.to_vec())?;
            (handle, width, height)
        } else {
            return Err(WellfriendError::UnsupportedFeature(
                "only PNG and JPEG images are embedded".into(),
            ));
        };
        if width_px == 0 || height_px == 0 {
            return Err(WellfriendError::invalid_input("image has no pixels"));
        }
        self.block_gap();
        // CSS pixels (96 per inch) to points, scaled down to the content box.
        let mut width = f64::from(width_px) * 0.75;
        let mut height = f64::from(height_px) * 0.75;
        let max_width = self.flow.content_width_points()? - self.indent();
        let page = self.flow.builder().pages().last().expect("flow has a page");
        let margins = page.margins();
        let max_height = (page.size().height - margins.top - margins.bottom) * 0.9;
        let scale = (max_width / width).min(max_height / height).min(1.0);
        width *= scale;
        height *= scale;
        let alt = if alt.is_empty() { "image" } else { alt };
        self.flow.add_figure(handle, width, height, alt)?;
        Ok(())
    }
}

fn short(url: &str) -> String {
    if url.starts_with("data:") {
        return "(data URI)".to_string();
    }
    let mut out: String = url.chars().take(80).collect();
    if url.chars().count() > 80 {
        out.push('…');
    }
    out
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Decode the part of a `data:` URI after the scheme. Only base64 payloads
/// are supported because images are binary.
pub(super) fn decode_data_uri(rest: &str) -> Option<Vec<u8>> {
    let (meta, payload) = rest.split_once(',')?;
    if !meta
        .split(';')
        .any(|part| part.trim().eq_ignore_ascii_case("base64"))
    {
        return None;
    }
    decode_base64(payload)
}

pub(super) fn decode_base64(input: &str) -> Option<Vec<u8>> {
    fn value(byte: u8) -> Option<u8> {
        match byte {
            b'A'..=b'Z' => Some(byte - b'A'),
            b'a'..=b'z' => Some(byte - b'a' + 26),
            b'0'..=b'9' => Some(byte - b'0' + 52),
            b'+' | b'-' => Some(62),
            b'/' | b'_' => Some(63),
            _ => None,
        }
    }
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0u32;
    for byte in input.bytes() {
        if byte.is_ascii_whitespace() {
            continue;
        }
        if byte == b'=' {
            break;
        }
        buffer = (buffer << 6) | u32::from(value(byte)?);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
        if out.len() > MAX_IMAGE_BYTES {
            return None;
        }
    }
    Some(out)
}
