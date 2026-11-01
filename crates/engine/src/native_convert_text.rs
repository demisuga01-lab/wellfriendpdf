//! Plain UTF-8 text to PDF through `FlowDocument::add_paragraph`.
use super::*;
use crate::authoring::ParagraphStyle;
use unicode_segmentation::UnicodeSegmentation;

/// How source lines wider than the content area are handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LineWrap {
    /// Wrap at spaces (overlong words break between characters).
    #[default]
    Word,
    /// Wrap at the last character that fits, ignoring word boundaries.
    Char,
    /// Never wrap; a line wider than the content area is an explicit
    /// `text_line_too_wide` error rather than being clipped.
    None,
}

impl LineWrap {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "word" | "words" | "true" | "on" => Some(Self::Word),
            "char" | "chars" | "character" | "anywhere" => Some(Self::Char),
            "none" | "off" | "false" => Some(Self::None),
            _ => None,
        }
    }
}

/// Options for [`text_to_pdf`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextToPdfOptions {
    pub page_size: NativePageSize,
    pub landscape: bool,
    /// `portrait` or `landscape`; overrides `landscape` when set.
    pub orientation: Option<NativeOrientation>,
    pub margins: NativeMargins,
    /// Body size in points, 7..=22.
    pub font_size: f64,
    pub line_wrap: LineWrap,
    /// Liberation Mono (with DejaVu Sans fallback) instead of Liberation Sans.
    pub monospace: bool,
    /// Columns per tab stop when expanding U+0009.
    pub tab_width: usize,
    /// Line height as a multiple of the font size.
    pub line_height: f64,
    pub title: Option<String>,
    pub author: Option<String>,
}

impl Default for TextToPdfOptions {
    fn default() -> Self {
        Self {
            page_size: NativePageSize::A4,
            landscape: false,
            orientation: None,
            margins: NativeMargins::default(),
            font_size: 10.0,
            line_wrap: LineWrap::Word,
            monospace: false,
            tab_width: 4,
            line_height: 1.25,
            title: None,
            author: None,
        }
    }
}

pub fn text_to_pdf(input: &[u8], options: &TextToPdfOptions) -> Result<Vec<u8>> {
    Ok(text_to_pdf_with_report(input, options)?.pdf)
}

pub fn text_to_pdf_with_report(
    input: &[u8],
    options: &TextToPdfOptions,
) -> Result<NativeConversionOutput> {
    let text = decode_utf8(input)?;
    check_font_size(options.font_size)?;
    if !options.line_height.is_finite() || !(0.8..=3.0).contains(&options.line_height) {
        return Err(input_error(
            codes::INVALID_OPTIONS,
            "line_height must be within 0.8..=3.0",
        ));
    }
    let page = oriented(
        options.page_size.page_size(),
        options.landscape,
        options.orientation,
    );
    let margins = options.margins.resolve(page)?;
    let mut flow = FlowDocument::new(page, margins);
    apply_metadata(
        &mut flow,
        options.title.as_deref(),
        options.author.as_deref(),
        "text-to-pdf",
    );
    let mut fonts = NativeFonts::new();
    let kind = if options.monospace {
        StackKind::Mono
    } else {
        StackKind::Sans
    };
    let style = fonts.style(&mut flow, kind, options.font_size)?;
    let paragraph = ParagraphStyle::new().line_height(options.line_height);
    let line_height = options.font_size * options.line_height;
    let width = flow.content_width_points()? - 0.5;
    let mut report = NativeConversionReport::default();

    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    // Word-wrapped lines without page breaks are laid out in parallel in one
    // batch; the per-line path below keeps the other modes unchanged.
    if options.line_wrap == LineWrap::Word && !normalized.contains('\u{000C}') {
        let mut paragraphs = Vec::new();
        for raw_line in normalized.split('\n') {
            crate::cancel::check_current_cancel("text-to-pdf")?;
            let expanded = expand_tabs(raw_line, options.tab_width);
            let line = fonts.sanitize(kind, &expanded, &mut report.replaced_characters);
            paragraphs.push((!line.trim().is_empty()).then_some(line));
        }
        flow.add_paragraphs(&paragraphs, &style, &paragraph)?;
        return finish(flow.into_builder(), report);
    }
    for (index, raw_line) in normalized.split('\n').enumerate() {
        crate::cancel::check_current_cancel("text-to-pdf")?;
        // U+000C starts a new page at that point of the line.
        for (part_index, part) in raw_line.split('\u{000C}').enumerate() {
            if part_index > 0 {
                flow.add_page_break();
            }
            let expanded = expand_tabs(part, options.tab_width);
            let line = fonts.sanitize(kind, &expanded, &mut report.replaced_characters);
            if line.trim().is_empty() {
                if part_index == 0 || !part.is_empty() {
                    flow.add_spacer(line_height);
                }
                continue;
            }
            match options.line_wrap {
                LineWrap::Word => {
                    flow.add_paragraph(&line, &style, &paragraph)?;
                }
                LineWrap::None => {
                    let measured = flow
                        .builder()
                        .pages()
                        .last()
                        .expect("flow has a page")
                        .text_width(&line, &style)?;
                    if measured > width {
                        return Err(input_error(
                            codes::TEXT_LINE_TOO_WIDE,
                            format!(
                                "line {} is {:.1}pt wide but the content area is {:.1}pt; use line_wrap \"word\" or \"char\", a smaller font_size or landscape",
                                index + 1,
                                measured,
                                width
                            ),
                        ));
                    }
                    flow.add_paragraph(&line, &style, &paragraph)?;
                }
                LineWrap::Char => {
                    for chunk in char_chunks(&flow, &line, &style, width)? {
                        flow.add_paragraph(&chunk, &style, &paragraph)?;
                    }
                }
            }
        }
    }
    finish(flow.into_builder(), report)
}

/// Split one line into the longest grapheme prefixes that fit `width`.
fn char_chunks(
    flow: &FlowDocument,
    line: &str,
    style: &TextStyle,
    width: f64,
) -> Result<Vec<String>> {
    let page = flow.builder().pages().last().expect("flow has a page");
    if page.text_width(line, style)? <= width {
        return Ok(vec![line.to_string()]);
    }
    let mut cache = std::collections::HashMap::<&str, f64>::new();
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_width = 0.0;
    for grapheme in line.graphemes(true) {
        let grapheme_width = match cache.get(grapheme) {
            Some(width) => *width,
            None => {
                let measured = page.text_width(grapheme, style)?;
                cache.insert(grapheme, measured);
                measured
            }
        };
        // Leave headroom for kerning between independently measured clusters.
        if current_width + grapheme_width > width * 0.98 && !current.is_empty() {
            chunks.push(std::mem::take(&mut current));
            current_width = 0.0;
        }
        current.push_str(grapheme);
        current_width += grapheme_width;
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    Ok(chunks)
}
