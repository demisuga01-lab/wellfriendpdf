//! JSON to PDF: a pretty-printed, syntax-coloured key/value tree laid out as
//! rich monospace paragraphs. Arrays of uniform objects render as tables.
use super::csv_pdf::{fit_columns, natural_column_widths};
use super::*;
use crate::authoring::{
    RichParagraphStyle, RichSpan, TableBuilder, TableCell, TableColumn, TableStyle,
};
use crate::content::Color;
use serde_json::Value;

const MAX_JSON_LINES: usize = 200_000;
const MAX_TABLE_COLUMNS: usize = 12;

/// Options for [`json_to_pdf`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct JsonToPdfOptions {
    pub page_size: NativePageSize,
    pub landscape: bool,
    /// `portrait` or `landscape`; overrides `landscape` when set.
    pub orientation: Option<NativeOrientation>,
    pub margins: NativeMargins,
    /// Text size in points, 7..=22.
    pub font_size: f64,
    /// Indent per nesting level, in points.
    pub indent: f64,
    /// Show arrays of objects that share one key set as tables.
    pub tables: bool,
    pub title: Option<String>,
    pub author: Option<String>,
}

impl Default for JsonToPdfOptions {
    fn default() -> Self {
        Self {
            page_size: NativePageSize::A4,
            landscape: false,
            orientation: None,
            margins: NativeMargins::default(),
            font_size: 9.0,
            indent: 14.0,
            tables: true,
            title: None,
            author: None,
        }
    }
}

pub fn json_to_pdf(input: &[u8], options: &JsonToPdfOptions) -> Result<Vec<u8>> {
    Ok(json_to_pdf_with_report(input, options)?.pdf)
}

pub fn json_to_pdf_with_report(
    input: &[u8],
    options: &JsonToPdfOptions,
) -> Result<NativeConversionOutput> {
    let text = decode_utf8(input)?;
    check_font_size(options.font_size)?;
    if !options.indent.is_finite() || !(0.0..=72.0).contains(&options.indent) {
        return Err(input_error(
            codes::INVALID_OPTIONS,
            "indent must be within 0..=72 points",
        ));
    }
    if text.trim().is_empty() {
        return Err(input_error(codes::EMPTY_INPUT, "JSON input is empty"));
    }
    let value: Value =
        serde_json::from_str(text).map_err(|error| input_error(codes::JSON_PARSE_ERROR, error))?;
    let page = oriented(
        options.page_size.page_size(),
        options.landscape,
        options.orientation,
    );
    let mut flow = FlowDocument::new(page, options.margins.resolve(page)?);
    apply_metadata(
        &mut flow,
        options.title.as_deref(),
        options.author.as_deref(),
        "json-to-pdf",
    );
    let mut writer = JsonWriter {
        flow,
        fonts: NativeFonts::new(),
        report: NativeConversionReport::default(),
        options: options.clone(),
        lines: 0,
    };
    writer.value(None, &value, 0, false)?;
    finish(writer.flow.into_builder(), writer.report)
}

#[derive(Clone, Copy)]
enum Token {
    Key,
    Punct,
    String,
    Number,
    Literal,
}

impl Token {
    fn color(self) -> Color {
        match self {
            Self::Key => Color::device_rgb(0.1, 0.2, 0.5),
            Self::Punct => Color::device_gray(0.35),
            Self::String => Color::device_rgb(0.08, 0.42, 0.16),
            Self::Number => Color::device_rgb(0.6, 0.25, 0.0),
            Self::Literal => Color::device_rgb(0.45, 0.1, 0.55),
        }
    }
}

struct JsonWriter {
    flow: FlowDocument,
    fonts: NativeFonts,
    report: NativeConversionReport,
    options: JsonToPdfOptions,
    lines: usize,
}

fn quoted(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| format!("\"{text}\""))
}

fn scalar(value: &Value) -> Option<(String, Token)> {
    match value {
        Value::Null => Some(("null".into(), Token::Literal)),
        Value::Bool(flag) => Some((flag.to_string(), Token::Literal)),
        Value::Number(number) => Some((number.to_string(), Token::Number)),
        Value::String(text) => Some((quoted(text), Token::String)),
        Value::Array(items) if items.is_empty() => Some(("[]".into(), Token::Punct)),
        Value::Object(map) if map.is_empty() => Some(("{}".into(), Token::Punct)),
        _ => None,
    }
}

/// Keys shared by every element when `items` is a uniform array of objects.
fn uniform_keys(items: &[Value]) -> Option<Vec<String>> {
    let first = items.first()?.as_object()?;
    if first.is_empty() || first.len() > MAX_TABLE_COLUMNS {
        return None;
    }
    let keys: Vec<String> = first.keys().cloned().collect();
    for item in items {
        let object = item.as_object()?;
        if object.len() != keys.len() || !keys.iter().all(|key| object.contains_key(key)) {
            return None;
        }
    }
    Some(keys)
}

impl JsonWriter {
    fn line(&mut self, depth: usize, parts: &[(String, Token, bool)]) -> Result<()> {
        self.lines += 1;
        if self.lines > MAX_JSON_LINES {
            return Err(limit_error(
                codes::JSON_TOO_LARGE,
                format!("the pretty-printed tree exceeds {MAX_JSON_LINES} lines"),
            ));
        }
        let size = self.options.font_size;
        let mut spans = Vec::with_capacity(parts.len());
        for (text, token, bold) in parts {
            let kind = if *bold {
                StackKind::MonoBold
            } else {
                StackKind::Mono
            };
            let text = self
                .fonts
                .sanitize(kind, text, &mut self.report.replaced_characters);
            let style = self
                .fonts
                .style(&mut self.flow, kind, size)?
                .fill(token.color());
            spans.push(RichSpan::new(text, style));
        }
        let max_indent = (self.flow.content_width_points()? * 0.5).max(0.0);
        let style = RichParagraphStyle {
            indent_left: (depth as f64 * self.options.indent).min(max_indent),
            line_height: 1.3,
            preserve_whitespace: true,
            ..RichParagraphStyle::default()
        };
        self.flow.add_rich_paragraph(&spans, &style)?;
        Ok(())
    }

    fn prefix(key: Option<&str>) -> Vec<(String, Token, bool)> {
        match key {
            Some(key) => vec![
                (quoted(key), Token::Key, true),
                (": ".into(), Token::Punct, false),
            ],
            None => Vec::new(),
        }
    }

    fn value(&mut self, key: Option<&str>, value: &Value, depth: usize, comma: bool) -> Result<()> {
        crate::cancel::check_current_cancel("json-to-pdf")?;
        let trailing = if comma { "," } else { "" };
        if let Some((text, token)) = scalar(value) {
            let mut parts = Self::prefix(key);
            parts.push((text, token, false));
            if comma {
                parts.push((",".into(), Token::Punct, false));
            }
            return self.line(depth, &parts);
        }
        match value {
            Value::Object(map) => {
                let mut parts = Self::prefix(key);
                parts.push(("{".into(), Token::Punct, false));
                self.line(depth, &parts)?;
                let count = map.len();
                for (index, (child_key, child)) in map.iter().enumerate() {
                    self.value(Some(child_key), child, depth + 1, index + 1 < count)?;
                }
                self.line(depth, &[(format!("}}{trailing}"), Token::Punct, false)])
            }
            Value::Array(items) => {
                if self.options.tables {
                    if let Some(keys) = uniform_keys(items) {
                        let mut parts = Self::prefix(key);
                        parts.push((
                            format!("[ {} objects ]{trailing}", items.len()),
                            Token::Punct,
                            false,
                        ));
                        self.line(depth, &parts)?;
                        return self.table(&keys, items);
                    }
                }
                let mut parts = Self::prefix(key);
                parts.push(("[".into(), Token::Punct, false));
                self.line(depth, &parts)?;
                let count = items.len();
                for (index, item) in items.iter().enumerate() {
                    self.value(None, item, depth + 1, index + 1 < count)?;
                }
                self.line(depth, &[(format!("]{trailing}"), Token::Punct, false)])
            }
            _ => unreachable!("scalars handled above"),
        }
    }

    fn table(&mut self, keys: &[String], items: &[Value]) -> Result<()> {
        let size = self.options.font_size;
        let body = self.fonts.style(&mut self.flow, StackKind::Sans, size)?;
        let head = self
            .fonts
            .style(&mut self.flow, StackKind::SansBold, size)?;
        let header: Vec<String> = keys
            .iter()
            .map(|key| {
                self.fonts.sanitize(
                    StackKind::SansBold,
                    key,
                    &mut self.report.replaced_characters,
                )
            })
            .collect();
        let mut rows = Vec::with_capacity(items.len());
        for item in items {
            let object = item.as_object().expect("uniform array of objects");
            let row = keys
                .iter()
                .map(|key| {
                    let cell = match &object[key] {
                        Value::String(text) => text.clone(),
                        Value::Null => String::new(),
                        other => serde_json::to_string(other).unwrap_or_default(),
                    };
                    self.fonts.sanitize(
                        StackKind::Sans,
                        &cell,
                        &mut self.report.replaced_characters,
                    )
                })
                .collect::<Vec<_>>();
            rows.push(row);
        }
        let padding = 3.0;
        let natural = natural_column_widths(&self.flow, &header, &rows, &body, &head, padding)?;
        let available = self.flow.content_width_points()? - 0.5;
        let Some(widths) = fit_columns(&natural, padding * 2.0 + size * 2.5, available) else {
            // Too wide for a table: fall back to the tree form.
            for (index, item) in items.iter().enumerate() {
                self.value(None, item, 1, index + 1 < items.len())?;
            }
            return Ok(());
        };
        let mut table = TableBuilder::new(widths.into_iter().map(TableColumn::new).collect())
            .body_style(body)
            .header_style(head)
            .style(TableStyle::new().padding(padding));
        table.set_header(header.into_iter().map(TableCell::text));
        for row in rows {
            table.add_row(row.into_iter().map(TableCell::text));
        }
        self.flow.add_table(&table)?;
        self.flow.add_spacer(size * 0.5);
        Ok(())
    }
}
