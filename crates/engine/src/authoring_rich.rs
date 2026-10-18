//! Rich inline flow paragraphs: mixed text styles, external URI links,
//! indentation, list markers and per-line block decorations on the canonical
//! [`FlowDocument`] cursor. Text is painted through `draw_text`, so every run
//! stays real, selectable text using the requested (fallback-stack) fonts.
//! Pagination, structure tagging and rollback reuse the flow transaction.
use super::*;
use unicode_segmentation::UnicodeSegmentation;

#[cfg(test)]
#[path = "authoring_rich_tests.rs"]
mod tests;

const MAX_RICH_SPANS: usize = 100_000;
const MAX_RICH_BYTES: usize = 16 * 1024 * 1024;
const MAX_URI_BYTES: usize = 8 * 1024;
const BLOCK_PAD: f64 = 4.0;

/// One styled inline run. `link` is an external `http`, `https` or `mailto`
/// URI; any other scheme is painted as text without an annotation.
#[derive(Debug, Clone, PartialEq)]
pub struct RichSpan {
    pub text: String,
    pub style: TextStyle,
    pub link: Option<String>,
    /// Paint a strike-through line over the run (decorative artifact).
    pub strike: bool,
}

impl RichSpan {
    pub fn new(text: impl Into<String>, style: TextStyle) -> Self {
        Self {
            text: text.into(),
            style,
            link: None,
            strike: false,
        }
    }

    pub fn strike(mut self, strike: bool) -> Self {
        self.strike = strike;
        self
    }

    pub fn link(mut self, uri: impl Into<String>) -> Self {
        self.link = Some(uri.into());
        self
    }
}

/// Block options for [`FlowDocument::add_rich_paragraph`].
#[derive(Debug, Clone, PartialEq)]
pub struct RichParagraphStyle {
    /// Extra left indent from the flow margin, in points.
    pub indent_left: f64,
    /// Extra right indent from the flow margin, in points.
    pub indent_right: f64,
    /// Multiplier over the largest span size on each line.
    pub line_height: f64,
    /// Keep leading, repeated and wrapped whitespace (code blocks). U+0009 is
    /// expanded to `tab_width` spaces in both modes.
    pub preserve_whitespace: bool,
    pub tab_width: usize,
    /// Marker painted in the indent gutter of the first line (list bullets).
    pub marker: Option<(String, TextStyle)>,
    /// Background fill painted behind every line (code blocks).
    pub background: Option<Color>,
    /// Vertical rule painted in the indent gutter (block quotes).
    pub left_rule: Option<Color>,
    /// Underline link runs with their own fill colour.
    pub underline_links: bool,
    /// Tag the block as `/H1`..`/H6` instead of `/P`.
    pub heading_level: Option<u8>,
    /// Vertical space appended after the block.
    pub space_after: f64,
}

impl Default for RichParagraphStyle {
    fn default() -> Self {
        Self {
            indent_left: 0.0,
            indent_right: 0.0,
            line_height: 1.3,
            preserve_whitespace: false,
            tab_width: 4,
            marker: None,
            background: None,
            left_rule: None,
            underline_links: true,
            heading_level: None,
            space_after: 0.0,
        }
    }
}

/// What one rich paragraph produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RichParagraphReport {
    pub lines: usize,
    /// External link annotations created.
    pub links: usize,
    /// Link runs whose URI was not an allowed external scheme.
    pub unlinked_uris: usize,
    /// One-based physical pages touched.
    pub first_page: usize,
    pub last_page: usize,
}

/// An external URI link annotation on one authored page.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct UriLink {
    pub(super) rect: [f64; 4],
    pub(super) uri: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PieceKind {
    Word,
    Space,
    Break,
}

#[derive(Debug, Clone)]
struct Piece {
    span: usize,
    text: String,
    kind: PieceKind,
}

#[derive(Debug, Clone)]
struct Segment {
    span: usize,
    text: String,
    x: f64,
    width: f64,
}

#[derive(Debug, Clone, Default)]
struct RichLine {
    segments: Vec<Segment>,
    max_size: f64,
}

/// Return a normalized ASCII URI when `uri` uses an allowed external scheme.
pub(super) fn normalize_external_uri(uri: &str) -> Option<String> {
    let trimmed = uri.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_URI_BYTES {
        return None;
    }
    let parsed = url::Url::parse(trimmed).ok()?;
    if !matches!(parsed.scheme(), "http" | "https" | "mailto") {
        return None;
    }
    let normalized = parsed.as_str().to_string();
    normalized.is_ascii().then_some(normalized)
}

pub(super) fn uri_link_annotation_dict(link: &UriLink, page_number: u32) -> Result<PdfDictionary> {
    if !link.rect.iter().all(|value| value.is_finite())
        || link.rect[2] - link.rect[0] <= 0.0
        || link.rect[3] - link.rect[1] <= 0.0
        || !link.uri.is_ascii()
        || link.uri.is_empty()
        || link.uri.len() > MAX_URI_BYTES
    {
        return Err(WellfriendError::invalid_input(
            "invalid authored URI link annotation",
        ));
    }
    let action = dict(&[
        ("Type", PdfObject::Name("Action".into())),
        ("S", PdfObject::Name("URI".into())),
        ("URI", PdfObject::String(link.uri.as_bytes().to_vec())),
    ]);
    Ok(dict(&[
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
        ("F", PdfObject::Integer(4)),
        ("A", PdfObject::Dictionary(action)),
    ]))
}

fn split_pieces(spans: &[RichSpan], tab_width: usize) -> Result<Vec<Piece>> {
    let tab = " ".repeat(tab_width.clamp(1, 16));
    let mut pieces = Vec::new();
    for (index, span) in spans.iter().enumerate() {
        let mut current = String::new();
        let mut current_kind = PieceKind::Word;
        let flush = |pieces: &mut Vec<Piece>, current: &mut String, kind: PieceKind| {
            if !current.is_empty() {
                pieces.push(Piece {
                    span: index,
                    text: std::mem::take(current),
                    kind,
                });
            }
        };
        let mut chars = span.text.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\r' {
                if chars.peek() == Some(&'\n') {
                    continue;
                }
                flush(&mut pieces, &mut current, current_kind);
                pieces.push(Piece {
                    span: index,
                    text: String::new(),
                    kind: PieceKind::Break,
                });
                continue;
            }
            if ch == '\n' || crate::fonts::hard_break::is_hard_break(ch) {
                flush(&mut pieces, &mut current, current_kind);
                pieces.push(Piece {
                    span: index,
                    text: String::new(),
                    kind: PieceKind::Break,
                });
                continue;
            }
            let (kind, text): (PieceKind, &str) = if ch == '\t' {
                (PieceKind::Space, tab.as_str())
            } else if ch == ' ' {
                (PieceKind::Space, " ")
            } else if ch.is_control() {
                continue;
            } else {
                (PieceKind::Word, "")
            };
            if kind != current_kind {
                flush(&mut pieces, &mut current, current_kind);
                current_kind = kind;
            }
            if text.is_empty() {
                current.push(ch);
            } else {
                current.push_str(text);
            }
        }
        flush(&mut pieces, &mut current, current_kind);
    }
    Ok(pieces)
}

impl FlowDocument {
    /// Usable width between the current flow margins, in points.
    pub fn content_width_points(&self) -> Result<f64> {
        self.content_width()
    }

    /// One-based physical page that receives the next flow content.
    pub fn current_page_number(&self) -> usize {
        self.current_page + 1
    }

    /// Paint a full-width horizontal rule as a decorative artifact.
    pub fn add_horizontal_rule(&mut self, color: Color, line_width: f64) -> Result<&mut Self> {
        if !line_width.is_finite() || line_width <= 0.0 || line_width > 72.0 {
            return Err(WellfriendError::invalid_input(
                "horizontal rule width must be finite and positive",
            ));
        }
        let width = self.content_width()?;
        self.append_transaction(|flow| {
            let height = line_width + 8.0;
            flow.ensure_space(height)?;
            let y = flow.cursor_y - height / 2.0;
            let left = flow.margins.left;
            let page = flow.current_page_mut();
            page.commands.push(PageCommand::BeginArtifact);
            page.draw_line(
                left,
                y,
                left + width,
                y,
                &GraphicsStyle::stroke(color, line_width),
            );
            page.commands.push(PageCommand::EndArtifact);
            flow.cursor_y -= height;
            Ok(())
        })
    }

    /// Lay out mixed-style inline runs as one wrapped block. Words wrap at
    /// spaces; a word wider than the line breaks between grapheme clusters.
    /// U+000A (and other hard separators) force a new line. External link runs
    /// receive `/Link` annotations with `/URI` actions covering each painted
    /// fragment; nothing is fetched.
    pub fn add_rich_paragraph(
        &mut self,
        spans: &[RichSpan],
        style: &RichParagraphStyle,
    ) -> Result<RichParagraphReport> {
        if spans.len() > MAX_RICH_SPANS {
            return Err(WellfriendError::ResourceLimit(
                "rich paragraph span count".into(),
            ));
        }
        let total: usize = spans.iter().map(|span| span.text.len()).sum();
        if total > MAX_RICH_BYTES {
            return Err(WellfriendError::ResourceLimit(
                "rich paragraph exceeds 16 MiB".into(),
            ));
        }
        if ![
            style.indent_left,
            style.indent_right,
            style.line_height,
            style.space_after,
        ]
        .iter()
        .all(|value| value.is_finite())
            || style.indent_left < 0.0
            || style.indent_right < 0.0
            || style.line_height <= 0.0
            || style.line_height > 10.0
        {
            return Err(WellfriendError::invalid_input(
                "invalid rich paragraph style",
            ));
        }
        if matches!(style.heading_level, Some(level) if level == 0 || level > 6) {
            return Err(WellfriendError::invalid_input(
                "rich heading level must be 1..=6",
            ));
        }
        for span in spans {
            layout::validate_size(&span.style)?;
        }
        let width = self.content_width()? - style.indent_left - style.indent_right;
        if !width.is_finite() || width < 12.0 {
            return Err(WellfriendError::invalid_input(
                "rich paragraph indents leave no usable width",
            ));
        }
        let lines = self.layout_rich_lines(spans, style, width)?;
        let mut report = RichParagraphReport::default();
        if lines.is_empty() && style.marker.is_none() {
            return Ok(report);
        }
        let links: Vec<Option<String>> = spans
            .iter()
            .map(|span| span.link.as_deref().and_then(normalize_external_uri))
            .collect();
        report.unlinked_uris = spans
            .iter()
            .zip(&links)
            .filter(|(span, normalized)| span.link.is_some() && normalized.is_none())
            .count();
        let mut pending_links: Vec<(usize, UriLink)> = Vec::new();
        let role = match style.heading_level {
            Some(level) => structure::Role::heading(level),
            None => structure::Role::Paragraph,
        };
        let title = style.heading_level.map(|_| {
            spans
                .iter()
                .map(|span| span.text.as_str())
                .collect::<String>()
                .trim()
                .to_string()
        });
        let lines = if lines.is_empty() {
            vec![RichLine {
                segments: Vec::new(),
                max_size: style
                    .marker
                    .as_ref()
                    .map_or(10.0, |(_, marker)| marker.size),
            }]
        } else {
            lines
        };
        self.append_transaction(|flow| {
            let structure_id = structure::register(&mut flow.builder, role, None, title)?;
            let left = flow.margins.left + style.indent_left;
            report.first_page = flow.current_page + 1;
            for (line_index, line) in lines.iter().enumerate() {
                crate::cancel::check_current_cancel("rich paragraph layout")?;
                let line_height = line.max_size * style.line_height;
                flow.ensure_space(line_height)?;
                let top = flow.cursor_y;
                let baseline = top - (line_height - line.max_size) / 2.0 - line.max_size * 0.8;
                let page_index = flow.current_page;
                let page = flow.current_page_mut();
                if let Some(fill) = &style.background {
                    page.commands.push(PageCommand::BeginArtifact);
                    page.draw_rect(
                        left - BLOCK_PAD,
                        top - line_height,
                        width + BLOCK_PAD * 2.0,
                        line_height,
                        &GraphicsStyle::fill(fill.clone()),
                    );
                    page.commands.push(PageCommand::EndArtifact);
                }
                if let Some(rule) = &style.left_rule {
                    let x = left - 8.0;
                    page.commands.push(PageCommand::BeginArtifact);
                    page.draw_line(
                        x,
                        top,
                        x,
                        top - line_height,
                        &GraphicsStyle::stroke(rule.clone(), 2.0),
                    );
                    page.commands.push(PageCommand::EndArtifact);
                }
                page.commands
                    .push(PageCommand::BeginStructure(structure_id));
                if line_index == 0 {
                    if let Some((marker, marker_style)) = &style.marker {
                        let marker_width = page.text_width(marker, marker_style)?;
                        page.draw_text(
                            marker.clone(),
                            (left - 6.0 - marker_width).max(0.0),
                            baseline,
                            marker_style,
                        )?;
                    }
                }
                for segment in &line.segments {
                    if segment.text.trim().is_empty() {
                        continue;
                    }
                    let span = &spans[segment.span];
                    let x = left + segment.x;
                    page.draw_text(segment.text.clone(), x, baseline, &span.style)?;
                    if let Some(uri) = &links[segment.span] {
                        let size = span.style.size;
                        pending_links.push((
                            page_index,
                            UriLink {
                                rect: [
                                    x,
                                    baseline - size * 0.25,
                                    x + segment.width.max(1.0),
                                    baseline + size * 0.9,
                                ],
                                uri: uri.clone(),
                            },
                        ));
                    }
                }
                page.commands.push(PageCommand::EndStructure(structure_id));
                for segment in &line.segments {
                    let span = &spans[segment.span];
                    let underline = style.underline_links && links[segment.span].is_some();
                    if segment.text.trim().is_empty() {
                        continue;
                    }
                    for (enabled, offset) in [(underline, -0.12), (span.strike, 0.3)] {
                        if !enabled {
                            continue;
                        }
                        let y = baseline + span.style.size * offset;
                        let x = left + segment.x;
                        page.commands.push(PageCommand::BeginArtifact);
                        page.draw_line(
                            x,
                            y,
                            x + segment.width,
                            y,
                            &GraphicsStyle::stroke(
                                span.style.fill.clone(),
                                (span.style.size / 18.0).max(0.4),
                            ),
                        );
                        page.commands.push(PageCommand::EndArtifact);
                    }
                }
                flow.cursor_y -= line_height;
                report.lines += 1;
            }
            report.last_page = flow.current_page + 1;
            Ok(())
        })?;
        report.links = pending_links.len();
        for (page_index, link) in pending_links {
            self.builder.pages[page_index].uri_links.push(link);
        }
        if style.space_after > 0.0 {
            self.add_spacer(style.space_after);
        }
        Ok(report)
    }

    fn layout_rich_lines(
        &self,
        spans: &[RichSpan],
        style: &RichParagraphStyle,
        width: f64,
    ) -> Result<Vec<RichLine>> {
        let pieces = split_pieces(spans, style.tab_width)?;
        let page = self.current_page_ref();
        let mut cache: HashMap<(usize, String), f64> = HashMap::new();
        let mut measure = |span: usize, text: &str| -> Result<f64> {
            if let Some(width) = cache.get(&(span, text.to_string())) {
                return Ok(*width);
            }
            let width = page.text_width(text, &spans[span].style)?;
            cache.insert((span, text.to_string()), width);
            Ok(width)
        };
        let mut lines = Vec::new();
        let mut line = RichLine::default();
        let mut cursor = 0.0f64;
        let mut pending_spaces: Vec<(usize, String, f64)> = Vec::new();
        let default_size = spans.first().map_or(10.0, |span| span.style.size);

        fn push_text(line: &mut RichLine, span: usize, text: &str, x: f64, width: f64, size: f64) {
            line.max_size = line.max_size.max(size);
            if let Some(last) = line.segments.last_mut() {
                if last.span == span && (last.x + last.width - x).abs() < 0.01 {
                    last.text.push_str(text);
                    last.width += width;
                    return;
                }
            }
            line.segments.push(Segment {
                span,
                text: text.to_string(),
                x,
                width,
            });
        }

        let finish = |lines: &mut Vec<RichLine>, line: &mut RichLine, fallback: f64| {
            let mut done = std::mem::take(line);
            if done.max_size <= 0.0 {
                done.max_size = fallback;
            }
            lines.push(done);
        };

        for piece in pieces {
            let size = spans[piece.span].style.size;
            match piece.kind {
                PieceKind::Break => {
                    pending_spaces.clear();
                    if line.max_size <= 0.0 {
                        line.max_size = size;
                    }
                    finish(&mut lines, &mut line, default_size);
                    cursor = 0.0;
                }
                PieceKind::Space => {
                    if line.segments.is_empty() && !style.preserve_whitespace {
                        continue;
                    }
                    let w = measure(piece.span, &piece.text)?;
                    pending_spaces.push((piece.span, piece.text, w));
                }
                PieceKind::Word => {
                    let w = measure(piece.span, &piece.text)?;
                    let space_width: f64 = pending_spaces.iter().map(|(_, _, w)| *w).sum();
                    if cursor + space_width + w <= width + 0.01 {
                        for (span, text, sw) in pending_spaces.drain(..) {
                            push_text(&mut line, span, &text, cursor, sw, spans[span].style.size);
                            cursor += sw;
                        }
                        push_text(&mut line, piece.span, &piece.text, cursor, w, size);
                        cursor += w;
                        continue;
                    }
                    pending_spaces.clear();
                    if !line.segments.is_empty() {
                        finish(&mut lines, &mut line, default_size);
                        cursor = 0.0;
                    }
                    if w <= width + 0.01 {
                        push_text(&mut line, piece.span, &piece.text, 0.0, w, size);
                        cursor = w;
                        continue;
                    }
                    // Break an overlong word between grapheme clusters.
                    let mut chunk = String::new();
                    let mut chunk_width = 0.0;
                    for grapheme in piece.text.graphemes(true) {
                        let mut candidate = chunk.clone();
                        candidate.push_str(grapheme);
                        let candidate_width = measure(piece.span, &candidate)?;
                        if candidate_width > width + 0.01 && !chunk.is_empty() {
                            push_text(&mut line, piece.span, &chunk, 0.0, chunk_width, size);
                            finish(&mut lines, &mut line, default_size);
                            chunk = grapheme.to_string();
                            chunk_width = measure(piece.span, &chunk)?;
                        } else {
                            chunk = candidate;
                            chunk_width = candidate_width;
                        }
                    }
                    if !chunk.is_empty() {
                        push_text(&mut line, piece.span, &chunk, 0.0, chunk_width, size);
                        cursor = chunk_width;
                    }
                }
            }
        }
        if !line.segments.is_empty() {
            finish(&mut lines, &mut line, default_size);
        }
        Ok(lines)
    }
}
