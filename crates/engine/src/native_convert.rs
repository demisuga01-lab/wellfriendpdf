//! Rust-native conversions of plain text, Markdown, CSV, JSON and SVG to PDF.
//!
//! Text, Markdown, CSV and JSON are laid out with the canonical authoring API
//! ([`crate::authoring::FlowDocument`], rich paragraphs and the paginated table
//! layout), so every glyph is real, selectable text embedded from the bundled
//! Liberation and DejaVu faces. SVG stays vector through `usvg` + `svg2pdf`.
//!
//! No converter performs network or file-system access unless the caller
//! explicitly supplies a Markdown `base_dir` for local images. Characters the
//! bundled fonts cannot draw are replaced with U+FFFD and counted in the
//! report instead of failing or vanishing silently.
//!
//! Errors keep the crate's [`WellfriendError`] type. Converter-specific
//! failures prefix their message with a stable snake_case code (for example
//! `csv_column_limit_exceeded: ...`); [`native_error_code`] extracts it.

use serde::{Deserialize, Serialize};

use crate::authoring::{
    FlowDocument, FontFace, Margins, PageSize, PdfBuilder, StandardFont, TextStyle,
};
use crate::error::{Result, WellfriendError};
use crate::render::get_fallback_font;

#[path = "native_convert_csv.rs"]
mod csv_pdf;
#[path = "native_convert_json.rs"]
mod json_pdf;
#[path = "native_convert_markdown.rs"]
mod markdown_pdf;
#[path = "native_convert_svg.rs"]
mod svg_pdf;
#[path = "native_convert_text.rs"]
mod text_pdf;

#[cfg(test)]
#[path = "native_convert_tests.rs"]
mod tests;

pub use csv_pdf::{csv_to_pdf, csv_to_pdf_with_report, CsvOrientation, CsvToPdfOptions};
pub use json_pdf::{json_to_pdf, json_to_pdf_with_report, JsonToPdfOptions};
pub use markdown_pdf::{markdown_to_pdf, markdown_to_pdf_with_report, MarkdownToPdfOptions};
pub use svg_pdf::{svg_to_pdf, svg_to_pdf_with_report, SvgPageMode, SvgToPdfOptions};
pub use text_pdf::{text_to_pdf, text_to_pdf_with_report, LineWrap, TextToPdfOptions};

/// Largest accepted converter input.
pub const MAX_NATIVE_INPUT_BYTES: usize = 64 * 1024 * 1024;
/// Smallest and largest body font size accepted by the text-like converters.
pub const MIN_NATIVE_FONT_SIZE: f64 = 7.0;
pub const MAX_NATIVE_FONT_SIZE: f64 = 22.0;

/// Stable converter error codes carried as message prefixes.
pub mod codes {
    pub const INVALID_OPTIONS: &str = "invalid_options";
    pub const INVALID_UTF8: &str = "invalid_utf8";
    pub const EMPTY_INPUT: &str = "empty_input";
    pub const INPUT_TOO_LARGE: &str = "input_too_large";
    pub const FONT_SIZE_OUT_OF_RANGE: &str = "font_size_out_of_range";
    pub const TEXT_LINE_TOO_WIDE: &str = "text_line_too_wide";
    pub const CSV_PARSE_ERROR: &str = "csv_parse_error";
    pub const CSV_COLUMN_LIMIT_EXCEEDED: &str = "csv_column_limit_exceeded";
    pub const CSV_ROW_LIMIT_EXCEEDED: &str = "csv_row_limit_exceeded";
    pub const CSV_TABLE_TOO_WIDE: &str = "csv_table_too_wide";
    pub const JSON_PARSE_ERROR: &str = "json_parse_error";
    pub const JSON_TOO_LARGE: &str = "json_too_large";
    pub const SVG_PARSE_ERROR: &str = "svg_parse_error";
    pub const SVG_CONVERT_ERROR: &str = "svg_convert_error";
    pub(super) const ALL: &[&str] = &[
        INVALID_OPTIONS,
        INVALID_UTF8,
        EMPTY_INPUT,
        INPUT_TOO_LARGE,
        FONT_SIZE_OUT_OF_RANGE,
        TEXT_LINE_TOO_WIDE,
        CSV_PARSE_ERROR,
        CSV_COLUMN_LIMIT_EXCEEDED,
        CSV_ROW_LIMIT_EXCEEDED,
        CSV_TABLE_TOO_WIDE,
        JSON_PARSE_ERROR,
        JSON_TOO_LARGE,
        SVG_PARSE_ERROR,
        SVG_CONVERT_ERROR,
    ];
}

/// The most specific stable code for an error: a converter code when the
/// message carries one, otherwise [`WellfriendError::code`].
pub fn native_error_code(error: &WellfriendError) -> &'static str {
    let message = match error {
        WellfriendError::MalformedPdf(message)
        | WellfriendError::ParseError(message)
        | WellfriendError::UnsupportedFeature(message)
        | WellfriendError::ResourceLimit(message) => message.as_str(),
        _ => return error.code(),
    };
    codes::ALL
        .iter()
        .find(|code| {
            message
                .strip_prefix(**code)
                .is_some_and(|rest| rest.starts_with(':'))
        })
        .copied()
        .unwrap_or_else(|| error.code())
}

/// Map an error onto the server/CLI contract vocabulary: `INVALID_INPUT`,
/// `UNSUPPORTED_INPUT`, `RESOURCE_LIMIT`, `INVALID_JSON`, `INVALID_OPTIONS`,
/// `ENCRYPTED_INPUT` or `INTERNAL`. The specific converter code stays the
/// message prefix.
pub fn native_contract_code(error: &WellfriendError) -> &'static str {
    match native_error_code(error) {
        codes::INVALID_OPTIONS | codes::FONT_SIZE_OUT_OF_RANGE => "INVALID_OPTIONS",
        codes::JSON_PARSE_ERROR => "INVALID_JSON",
        codes::INPUT_TOO_LARGE
        | codes::CSV_COLUMN_LIMIT_EXCEEDED
        | codes::CSV_ROW_LIMIT_EXCEEDED
        | codes::CSV_TABLE_TOO_WIDE
        | codes::JSON_TOO_LARGE
        | "resource_limit" => "RESOURCE_LIMIT",
        codes::INVALID_UTF8
        | codes::EMPTY_INPUT
        | codes::TEXT_LINE_TOO_WIDE
        | codes::CSV_PARSE_ERROR
        | codes::SVG_PARSE_ERROR
        | "malformed_pdf"
        | "parse"
        | "missing_object" => "INVALID_INPUT",
        codes::SVG_CONVERT_ERROR | "unsupported_feature" => "UNSUPPORTED_INPUT",
        "encrypted" | "authentication_failure" => "ENCRYPTED_INPUT",
        _ => "INTERNAL",
    }
}

pub(crate) fn input_error(code: &str, detail: impl std::fmt::Display) -> WellfriendError {
    WellfriendError::invalid_input(format!("{code}: {detail}"))
}

pub(crate) fn limit_error(code: &str, detail: impl std::fmt::Display) -> WellfriendError {
    WellfriendError::ResourceLimit(format!("{code}: {detail}"))
}

/// Explicit page orientation for the text-like converters and fitted SVG.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NativeOrientation {
    Portrait,
    Landscape,
}

/// Apply `landscape` / `orientation` to a portrait page size.
pub(crate) fn oriented(
    page: PageSize,
    landscape: bool,
    orientation: Option<NativeOrientation>,
) -> PageSize {
    match orientation {
        Some(NativeOrientation::Landscape) => page.landscape(),
        Some(NativeOrientation::Portrait) => page.portrait(),
        None if landscape => page.landscape(),
        None => page,
    }
}

/// Named page sizes accepted by the converters (case-insensitive in options).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NativePageSize {
    #[default]
    A4,
    Letter,
    Legal,
}

impl NativePageSize {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "a4" => Some(Self::A4),
            "letter" | "us-letter" => Some(Self::Letter),
            "legal" | "us-legal" => Some(Self::Legal),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::A4 => "a4",
            Self::Letter => "letter",
            Self::Legal => "legal",
        }
    }

    /// Portrait dimensions in PDF points.
    pub fn page_size(self) -> PageSize {
        match self {
            Self::A4 => PageSize::A4,
            Self::Letter => PageSize::LETTER,
            Self::Legal => PageSize::LEGAL,
        }
    }
}

/// Page margins in PDF points: one number for all sides, or per side.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum NativeMargins {
    All(f64),
    Sides {
        top: f64,
        right: f64,
        bottom: f64,
        left: f64,
    },
}

impl Default for NativeMargins {
    fn default() -> Self {
        Self::All(54.0)
    }
}

impl NativeMargins {
    /// Validate against the page: every side must be finite, non-negative and
    /// leave at least a third of each page dimension for content.
    pub fn resolve(self, page: PageSize) -> Result<Margins> {
        let margins = match self {
            Self::All(value) => Margins::all(value),
            Self::Sides {
                top,
                right,
                bottom,
                left,
            } => Margins {
                left,
                right,
                top,
                bottom,
            },
        };
        let sides = [margins.left, margins.right, margins.top, margins.bottom];
        if sides.iter().any(|side| !side.is_finite() || *side < 0.0)
            || margins.left + margins.right > page.width * 2.0 / 3.0
            || margins.top + margins.bottom > page.height * 2.0 / 3.0
        {
            return Err(input_error(
                codes::INVALID_OPTIONS,
                "margins must be finite, non-negative and leave a third of the page",
            ));
        }
        Ok(margins)
    }
}

/// What a conversion produced.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeConversionReport {
    pub pages: usize,
    pub bytes: usize,
    /// Characters replaced with U+FFFD because no bundled font covers them.
    pub replaced_characters: usize,
    /// Human-readable notes about content that was represented differently
    /// (for example a remote Markdown image shown as a placeholder).
    pub warnings: Vec<String>,
}

/// PDF bytes plus the conversion report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeConversionOutput {
    pub pdf: Vec<u8>,
    pub report: NativeConversionReport,
}

pub(crate) fn check_input_size(input: &[u8]) -> Result<()> {
    if input.len() > MAX_NATIVE_INPUT_BYTES {
        return Err(limit_error(
            codes::INPUT_TOO_LARGE,
            format!(
                "input is {} bytes; the limit is {MAX_NATIVE_INPUT_BYTES}",
                input.len()
            ),
        ));
    }
    Ok(())
}

pub(crate) fn decode_utf8(input: &[u8]) -> Result<&str> {
    check_input_size(input)?;
    let input = input.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(input);
    std::str::from_utf8(input).map_err(|error| {
        input_error(
            codes::INVALID_UTF8,
            format!(
                "input is not valid UTF-8 (first invalid byte at offset {})",
                error.valid_up_to()
            ),
        )
    })
}

pub(crate) fn check_font_size(size: f64) -> Result<()> {
    if !size.is_finite() || !(MIN_NATIVE_FONT_SIZE..=MAX_NATIVE_FONT_SIZE).contains(&size) {
        return Err(input_error(
            codes::FONT_SIZE_OUT_OF_RANGE,
            format!(
                "font size {size} is outside {MIN_NATIVE_FONT_SIZE}..={MAX_NATIVE_FONT_SIZE} points"
            ),
        ));
    }
    Ok(())
}

pub(crate) fn finish(
    mut builder: PdfBuilder,
    mut report: NativeConversionReport,
) -> Result<NativeConversionOutput> {
    builder.set_compress_content(true);
    report.pages = builder.pages().len();
    let pdf = builder.into_bytes()?;
    report.bytes = pdf.len();
    Ok(NativeConversionOutput { pdf, report })
}

pub(crate) fn apply_metadata(
    flow: &mut FlowDocument,
    title: Option<&str>,
    author: Option<&str>,
    creator: &str,
) {
    let builder = flow.builder_mut();
    if let Some(title) = title.map(str::trim).filter(|title| !title.is_empty()) {
        builder.set_title(title.chars().take(512).collect::<String>());
    }
    if let Some(author) = author.map(str::trim).filter(|author| !author.is_empty()) {
        builder.set_author(author.chars().take(512).collect::<String>());
    }
    builder.set_creator(format!("Wellfriend PDF SDK {creator}"));
}

/// The physical faces of one fallback stack, for coverage checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StackKind {
    Sans,
    SansBold,
    SansItalic,
    SansBoldItalic,
    Mono,
    MonoBold,
}

impl StackKind {
    fn standard(self) -> StandardFont {
        match self {
            Self::Sans => StandardFont::Helvetica,
            Self::SansBold => StandardFont::HelveticaBold,
            Self::SansItalic => StandardFont::HelveticaOblique,
            Self::SansBoldItalic => StandardFont::HelveticaBoldOblique,
            Self::Mono => StandardFont::Courier,
            Self::MonoBold => StandardFont::CourierBold,
        }
    }
}

/// Lazily registered fallback stacks: Liberation (Standard-14 equivalent)
/// first, then DejaVu Sans for Greek, Cyrillic, Arabic, Hebrew, Armenian,
/// Georgian, symbols and more.
pub(crate) struct NativeFonts {
    stacks: Vec<(StackKind, FontFace)>,
    coverage: Vec<(StackKind, GlyphCoverage)>,
}

impl NativeFonts {
    pub(crate) fn new() -> Self {
        Self {
            stacks: Vec::new(),
            coverage: Vec::new(),
        }
    }

    pub(crate) fn face(&mut self, flow: &mut FlowDocument, kind: StackKind) -> Result<FontFace> {
        if let Some((_, face)) = self.stacks.iter().find(|(stack, _)| *stack == kind) {
            return Ok(*face);
        }
        let face = flow.builder_mut().register_font_stack(&[
            FontFace::Standard(kind.standard()),
            FontFace::BuiltinUnicode,
        ])?;
        self.stacks.push((kind, face));
        Ok(face)
    }

    pub(crate) fn style(
        &mut self,
        flow: &mut FlowDocument,
        kind: StackKind,
        size: f64,
    ) -> Result<TextStyle> {
        Ok(TextStyle::new(self.face(flow, kind)?, size))
    }

    /// Replace characters no face of `kind` covers with U+FFFD, drop C0/C1
    /// controls (except tab and newline) and default-ignorable code points the
    /// faces lack.
    pub(crate) fn sanitize(&mut self, kind: StackKind, text: &str, replaced: &mut usize) -> String {
        if !self.coverage.iter().any(|(stack, _)| *stack == kind) {
            self.coverage.push((kind, GlyphCoverage::new(kind)));
        }
        let coverage = &self
            .coverage
            .iter()
            .find(|(stack, _)| *stack == kind)
            .expect("coverage inserted above")
            .1;
        coverage.sanitize(text, replaced)
    }
}

pub(crate) struct GlyphCoverage {
    faces: Vec<ttf_parser::Face<'static>>,
}

impl GlyphCoverage {
    fn new(kind: StackKind) -> Self {
        let names = [kind.standard().base_font_name(), "Symbol"];
        let faces = names
            .iter()
            .filter_map(|name| get_fallback_font(name))
            .filter_map(|bytes| ttf_parser::Face::parse(bytes, 0).ok())
            .collect();
        Self { faces }
    }

    fn covers(&self, ch: char) -> bool {
        self.faces.iter().any(|face| face.glyph_index(ch).is_some())
    }

    fn sanitize(&self, text: &str, replaced: &mut usize) -> String {
        let mut out = String::with_capacity(text.len());
        for ch in text.chars() {
            if ch == '\n' || ch == '\t' || ch == ' ' {
                out.push(ch);
            } else if ch.is_control() {
                continue;
            } else if self.covers(ch) {
                out.push(ch);
            } else if is_default_ignorable(ch) {
                continue;
            } else if ch.is_whitespace() {
                out.push(' ');
            } else {
                *replaced += 1;
                out.push('\u{FFFD}');
            }
        }
        out
    }
}

fn is_default_ignorable(ch: char) -> bool {
    matches!(
        ch,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{061C}'
            | '\u{115F}'..='\u{1160}'
            | '\u{17B4}'..='\u{17B5}'
            | '\u{180B}'..='\u{180F}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{3164}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FEFF}'
            | '\u{FFA0}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0000}'..='\u{E0FFF}'
    )
}

/// Expand U+0009 to the next multiple of `tab_width` columns.
pub(crate) fn expand_tabs(line: &str, tab_width: usize) -> String {
    if !line.contains('\t') {
        return line.to_string();
    }
    let tab_width = tab_width.clamp(1, 16);
    let mut out = String::with_capacity(line.len() + 8);
    let mut column = 0usize;
    for ch in line.chars() {
        if ch == '\t' {
            let spaces = tab_width - column % tab_width;
            out.extend(std::iter::repeat_n(' ', spaces));
            column += spaces;
        } else {
            out.push(ch);
            column += 1;
        }
    }
    out
}

/// Parse an options JSON object for a converter; `None`, blank or `null`
/// means defaults. Unknown keys are ignored. The server's camelCase names are
/// accepted (`pageSize`, `orientation`, `fontSize`, `margin`, `delimiter`,
/// `headerRow`, `title`, `author`) and `pageSize`/`orientation` values are
/// case-insensitive. Known keys with invalid values fail with
/// `invalid_options`.
pub fn parse_native_options<T>(json: Option<&str>) -> Result<T>
where
    T: Default + for<'de> Deserialize<'de>,
{
    let text = match json.map(str::trim) {
        None | Some("") | Some("null") => return Ok(T::default()),
        Some(text) => text,
    };
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|error| input_error(codes::INVALID_OPTIONS, error))?;
    let serde_json::Value::Object(map) = value else {
        return Err(input_error(
            codes::INVALID_OPTIONS,
            "options must be a JSON object",
        ));
    };
    let mut normalized = serde_json::Map::new();
    for (key, mut value) in map {
        let key = match key.as_str() {
            "pageSize" => "page_size",
            "fontSize" => "font_size",
            "margin" | "margins" => "margins",
            "headerRow" => "has_header",
            "lineWrap" => "line_wrap",
            "maxColumns" => "max_columns",
            "tabWidth" => "tab_width",
            "lineHeight" => "line_height",
            "autoOrient" => "auto_orient",
            "baseDir" => "base_dir",
            other => other,
        }
        .to_string();
        if matches!(
            key.as_str(),
            "page_size" | "orientation" | "line_wrap" | "page"
        ) {
            if let serde_json::Value::String(text) = &value {
                value = serde_json::Value::String(text.trim().to_ascii_lowercase());
            }
        }
        if value.is_null() {
            continue;
        }
        normalized.insert(key, value);
    }
    serde_json::from_value(serde_json::Value::Object(normalized))
        .map_err(|error| input_error(codes::INVALID_OPTIONS, error))
}
