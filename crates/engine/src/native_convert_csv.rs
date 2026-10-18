//! CSV to PDF via the `csv` crate and the paginated authoring table layout.
//! The header row repeats on every page; wide tables switch to landscape and
//! then wrap columns. Column overflow is an explicit error, never truncation.
use super::*;
use crate::authoring::{TableBuilder, TableCell, TableColumn, TableStyle};

/// Hard ceiling for `max_columns`.
pub const CSV_MAX_COLUMNS_CEILING: usize = 200;
/// Largest number of data rows rendered.
pub const CSV_MAX_ROWS: usize = 100_000;
const MEASURE_ROWS: usize = 400;
const MEASURE_CHARS: usize = 80;
const MAX_NATURAL_WIDTH: f64 = 260.0;

/// Page orientation policy for [`csv_to_pdf`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CsvOrientation {
    /// Portrait unless the natural table width needs landscape.
    #[default]
    Auto,
    Portrait,
    Landscape,
}

/// Options for [`csv_to_pdf`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CsvToPdfOptions {
    pub page_size: NativePageSize,
    pub orientation: CsvOrientation,
    pub margins: NativeMargins,
    /// Cell text size in points, 7..=22.
    pub font_size: f64,
    /// One ASCII delimiter character, `"tab"`, or `None`/`"auto"` to detect
    /// among `,` `;` tab and `|`.
    pub delimiter: Option<String>,
    /// Treat the first record as the header row (repeated on every page).
    pub has_header: bool,
    /// Column cap (1..=200). More columns fail with
    /// `csv_column_limit_exceeded`.
    pub max_columns: usize,
    pub title: Option<String>,
    pub author: Option<String>,
}

impl Default for CsvToPdfOptions {
    fn default() -> Self {
        Self {
            page_size: NativePageSize::A4,
            orientation: CsvOrientation::Auto,
            margins: NativeMargins::All(36.0),
            font_size: 8.0,
            delimiter: None,
            has_header: true,
            max_columns: 50,
            title: None,
            author: None,
        }
    }
}

pub fn csv_to_pdf(input: &[u8], options: &CsvToPdfOptions) -> Result<Vec<u8>> {
    Ok(csv_to_pdf_with_report(input, options)?.pdf)
}

/// Resolve the delimiter option to one byte.
pub(super) fn resolve_delimiter(option: Option<&str>, sample: &[u8]) -> Result<u8> {
    match option.map(str::trim) {
        None | Some("") | Some("auto") => Ok(detect_delimiter(sample)),
        Some("tab") | Some("\\t") | Some("\t") => Ok(b'\t'),
        Some(value) if value.len() == 1 && value.is_ascii() && !value.starts_with('"') => {
            Ok(value.as_bytes()[0])
        }
        Some(value) => Err(input_error(
            codes::INVALID_OPTIONS,
            format!("delimiter {value:?} must be one ASCII character, \"tab\" or \"auto\""),
        )),
    }
}

/// Pick the candidate delimiter whose first records most consistently split
/// into more than one field.
pub(super) fn detect_delimiter(sample: &[u8]) -> u8 {
    let sample = &sample[..sample.len().min(64 * 1024)];
    let mut best = (b',', 0usize);
    for candidate in [b',', b';', b'\t', b'|'] {
        let mut reader = csv::ReaderBuilder::new()
            .delimiter(candidate)
            .has_headers(false)
            .flexible(true)
            .from_reader(sample);
        let mut first = None;
        let mut consistent = 0usize;
        for record in reader.records().take(50) {
            let Ok(record) = record else { break };
            let fields = record.len();
            match first {
                None => first = Some(fields),
                Some(expected) if expected == fields => {}
                Some(_) => continue,
            }
            if fields > 1 {
                consistent += fields;
            }
        }
        if consistent > best.1 {
            best = (candidate, consistent);
        }
    }
    best.0
}

pub fn csv_to_pdf_with_report(
    input: &[u8],
    options: &CsvToPdfOptions,
) -> Result<NativeConversionOutput> {
    let text = decode_utf8(input)?;
    check_font_size(options.font_size)?;
    if options.max_columns == 0 || options.max_columns > CSV_MAX_COLUMNS_CEILING {
        return Err(input_error(
            codes::INVALID_OPTIONS,
            format!("max_columns must be within 1..={CSV_MAX_COLUMNS_CEILING}"),
        ));
    }
    let delimiter = resolve_delimiter(options.delimiter.as_deref(), text.as_bytes())?;
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .flexible(true)
        .from_reader(text.as_bytes());
    let mut records: Vec<Vec<String>> = Vec::new();
    let mut columns = 0usize;
    for record in reader.records() {
        crate::cancel::check_current_cancel("csv-to-pdf")?;
        let record = record.map_err(|error| input_error(codes::CSV_PARSE_ERROR, error))?;
        if record.iter().all(|field| field.trim().is_empty()) && record.len() <= 1 {
            continue;
        }
        columns = columns.max(record.len());
        if columns > options.max_columns {
            return Err(limit_error(
                codes::CSV_COLUMN_LIMIT_EXCEEDED,
                format!(
                    "record {} has {} columns; max_columns is {}",
                    records.len() + 1,
                    record.len(),
                    options.max_columns
                ),
            ));
        }
        if records.len() > CSV_MAX_ROWS {
            return Err(limit_error(
                codes::CSV_ROW_LIMIT_EXCEEDED,
                format!("more than {CSV_MAX_ROWS} rows"),
            ));
        }
        records.push(record.iter().map(str::to_string).collect());
    }
    if records.is_empty() {
        return Err(input_error(codes::EMPTY_INPUT, "CSV contains no records"));
    }

    let portrait = options.page_size.page_size().portrait();
    let landscape = portrait.landscape();
    let initial = if options.orientation == CsvOrientation::Landscape {
        landscape
    } else {
        portrait
    };
    let mut flow = FlowDocument::new(initial, options.margins.resolve(initial)?);
    let mut fonts = NativeFonts::new();
    let mut report = NativeConversionReport::default();
    let body = fonts.style(&mut flow, StackKind::Sans, options.font_size)?;
    let head = fonts.style(&mut flow, StackKind::SansBold, options.font_size)?;

    let pad = |row: &[String], kind, fonts: &mut NativeFonts, replaced: &mut usize| {
        (0..columns)
            .map(|index| {
                fonts.sanitize(
                    kind,
                    row.get(index).map(String::as_str).unwrap_or(""),
                    replaced,
                )
            })
            .collect::<Vec<_>>()
    };
    let mut rows = records.into_iter();
    let header = if options.has_header {
        let first = rows.next().unwrap_or_default();
        let mut header = pad(
            &first,
            StackKind::SansBold,
            &mut fonts,
            &mut report.replaced_characters,
        );
        for (index, cell) in header.iter_mut().enumerate() {
            if cell.trim().is_empty() {
                *cell = format!("Column {}", index + 1);
            }
        }
        Some(header)
    } else {
        None
    };
    let rows: Vec<Vec<String>> = rows
        .map(|row| {
            pad(
                &row,
                StackKind::Sans,
                &mut fonts,
                &mut report.replaced_characters,
            )
        })
        .collect();

    let padding = 3.0;
    let empty_header;
    let header_ref = match &header {
        Some(header) => header,
        None => {
            empty_header = vec![String::new(); columns];
            &empty_header
        }
    };
    let natural = natural_column_widths(&flow, header_ref, &rows, &body, &head, padding)?;
    let min = padding * 2.0 + options.font_size * 2.5;
    let natural_total: f64 = natural.iter().sum();
    let mut available = flow.content_width_points()? - 0.5;
    if options.orientation == CsvOrientation::Auto && natural_total > available {
        flow = FlowDocument::new(landscape, options.margins.resolve(landscape)?);
        fonts = NativeFonts::new();
        available = flow.content_width_points()? - 0.5;
        report
            .warnings
            .push("table is wider than portrait; switched to landscape".to_string());
    }
    let body = fonts.style(&mut flow, StackKind::Sans, options.font_size)?;
    let head = fonts.style(&mut flow, StackKind::SansBold, options.font_size)?;
    let widths = fit_columns(&natural, min, available).ok_or_else(|| {
        limit_error(
            codes::CSV_TABLE_TOO_WIDE,
            format!(
                "{columns} columns need at least {:.0}pt but the page offers {:.0}pt; lower font_size, margins or max_columns",
                min * columns as f64,
                available
            ),
        )
    })?;
    if widths
        .iter()
        .zip(&natural)
        .any(|(fit, natural)| fit + 0.01 < *natural)
    {
        report
            .warnings
            .push("columns were narrowed; long cells wrap".to_string());
    }
    apply_metadata(
        &mut flow,
        options.title.as_deref(),
        options.author.as_deref(),
        "csv-to-pdf",
    );
    let mut table = TableBuilder::new(widths.into_iter().map(TableColumn::new).collect())
        .body_style(body)
        .header_style(head)
        .style(TableStyle::new().padding(padding));
    if let Some(header) = header {
        table.set_header(header.into_iter().map(TableCell::text));
    }
    for row in rows {
        table.add_row(row.into_iter().map(TableCell::text));
    }
    flow.add_table(&table)?;
    finish(flow.into_builder(), report)
}

/// Measured single-line width of each column (header bold, first rows body),
/// including padding and capped so one long cell wraps instead of dominating.
pub(super) fn natural_column_widths(
    flow: &FlowDocument,
    header: &[String],
    rows: &[Vec<String>],
    body: &TextStyle,
    head: &TextStyle,
    padding: f64,
) -> Result<Vec<f64>> {
    let page = flow.builder().pages().last().expect("flow has a page");
    let measure = |text: &str, style: &TextStyle| -> Result<f64> {
        let longest = text
            .split('\n')
            .map(|line| line.chars().take(MEASURE_CHARS).collect::<String>())
            .max_by_key(|line| line.chars().count())
            .unwrap_or_default();
        let line = longest.replace('\t', " ");
        if line.trim().is_empty() {
            return Ok(0.0);
        }
        page.text_width(&line, style)
    };
    let columns = header.len();
    let mut widths = vec![0.0f64; columns];
    for (index, cell) in header.iter().enumerate() {
        widths[index] = widths[index].max(measure(cell, head)?);
    }
    for row in rows.iter().take(MEASURE_ROWS) {
        crate::cancel::check_current_cancel("table column measurement")?;
        for (index, cell) in row.iter().enumerate().take(columns) {
            widths[index] = widths[index].max(measure(cell, body)?);
        }
    }
    Ok(widths
        .into_iter()
        .map(|width| (width + padding * 2.0 + 1.0).min(MAX_NATURAL_WIDTH))
        .collect())
}

/// Shrink natural widths proportionally to their slack above `min` so they
/// sum to at most `available`. `None` when even `min` per column cannot fit.
pub(super) fn fit_columns(natural: &[f64], min: f64, available: f64) -> Option<Vec<f64>> {
    let widths: Vec<f64> = natural.iter().map(|width| width.max(min)).collect();
    let total: f64 = widths.iter().sum();
    if total <= available {
        return Some(widths);
    }
    let floor = min * widths.len() as f64;
    if floor > available {
        return None;
    }
    let slack: f64 = widths.iter().map(|width| width - min).sum();
    let factor = (total - available) / slack;
    Some(
        widths
            .iter()
            .map(|width| width - (width - min) * factor)
            .collect(),
    )
}
