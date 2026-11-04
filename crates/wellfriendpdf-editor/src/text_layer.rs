//! Text for scanned pages: OCR words and lines become PDF text.
//!
//! Two operations take what an OCR engine read on a page (lines with their
//! words and boxes in PDF user space):
//!
//! - [`EditorOperation::AddTextLayer`](crate::ops::EditorOperation): an
//!   invisible text layer (rendering mode `3 Tr`) in an embedded library
//!   font. Each line is set at one size that fills its box height, every word
//!   starts at its box and is scaled horizontally (`Tz`) to its box width, so
//!   selection, search and copy follow the scan while nothing visible
//!   changes.
//! - [`EditorOperation::ConvertOcrText`](crate::ops::EditorOperation): real,
//!   visible, editable text that keeps the scan's look. The page is rendered
//!   once by the engine; for each line the paper colour around it, the ink
//!   colour in it, its ink top, baseline and left/right ink edges are
//!   measured. The line's area is painted with the paper colour (the scanned
//!   glyph pixels disappear under it) and the line is set on the measured
//!   baseline in the chosen library font, sized so the font's ascender
//!   height matches the scan's ink and scaled (`Tz`) to the scan's line
//!   width. A colour the caller passes wins over the measured one.
//!
//! Sampling happens here, in the SDK, so the browser preview (WASM) and the
//! server replay (native) measure the same rendered pixels with the same
//! code: the operation list stays the only input of a saved version.
//!
//! Both are drawn as one overlay Form XObject after the page content (see
//! [`crate::overlay`]); the scan image itself is never modified.

use lopdf::Document;
use serde::{Deserialize, Serialize};
use wellfriendpdf_engine::{
    AuthorPageSize, ContentEngine, FontFace, GraphicsStyle, PageTextLayout, PdfBuilder, TextStyle,
};

use crate::apply::OperationReport;
use crate::error::EditorError;
use crate::fonts::{covers, FontSource};
use crate::ops::{valid_font_id, Paint, Rect, Rgb, MAX_COORDINATE, MAX_TEXT_CHARS};
use crate::overlay;

/// The library font of an invisible layer when the operation names none
/// (wide script coverage; its glyphs are never seen).
pub const DEFAULT_LAYER_FONT: &str = "noto-sans-regular";
/// The library font of converted text when the operation names none.
pub const DEFAULT_TEXT_FONT: &str = "liberation-serif-regular";
/// Fonts tried, in order, for characters the chosen font cannot draw.
const FALLBACK_FONTS: [&str; 2] = ["noto-sans-regular", "dejavu-sans-regular"];
pub const MAX_LINES: usize = 2_000;
pub const MAX_WORDS: usize = 10_000;
/// Rendering density of the sampling pass.
const SAMPLE_DPI: u32 = 150;
/// Pixel budget of the sampling pass (larger pages render at a lower density).
const SAMPLE_PIXELS: f64 = 24_000_000.0;

/// One word an OCR engine read, with its box in PDF user space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OcrWord {
    pub text: String,
    pub rect: Rect,
}

/// One OCR line: its text, its box in PDF user space, optionally its words
/// (each placed at its own box) and, for a conversion, colours that replace
/// the measured ink and paper colours.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OcrLine {
    pub text: String,
    pub rect: Rect,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub words: Vec<OcrWord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<Rgb>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<Rgb>,
}

fn invalid(message: impl Into<String>) -> EditorError {
    EditorError::new("EDITOR_OPERATION_INVALID", message)
}

fn rect_ok(rect: &Rect, what: &str) -> Result<(), EditorError> {
    let values = [rect.x, rect.y, rect.width, rect.height];
    if values.iter().any(|v| !v.is_finite() || v.abs() > MAX_COORDINATE) {
        return Err(invalid(format!("{what} must be finite coordinates within ±{MAX_COORDINATE}")));
    }
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return Err(invalid(format!("{what} width and height must be positive")));
    }
    Ok(())
}

fn text_ok(text: &str, what: &str) -> Result<(), EditorError> {
    if text.trim().is_empty() {
        return Err(invalid(format!("{what} must not be empty")));
    }
    if text.chars().any(char::is_control) {
        return Err(invalid(format!("{what} must be one line without control characters")));
    }
    Ok(())
}

fn paint_ok(paint: &Rgb) -> Result<(), EditorError> {
    match paint {
        Paint::Rgb { .. } if paint.components().iter().all(|c| c.is_finite() && (0.0..=1.0).contains(c)) => Ok(()),
        Paint::Rgb { .. } => Err(invalid("colour components must be within 0..1")),
        _ => Err(invalid("OCR line colours must be RGB")),
    }
}

/// The admission rule of both operations (called by `ops::validate_operation`).
pub fn validate(lines: &[OcrLine], font: Option<&str>) -> Result<(), EditorError> {
    if lines.is_empty() {
        return Err(invalid("an OCR text operation needs at least one line"));
    }
    if lines.len() > MAX_LINES {
        return Err(invalid(format!("at most {MAX_LINES} OCR lines per page")));
    }
    if let Some(font) = font {
        if !valid_font_id(font) {
            return Err(invalid("font must be a font library id"));
        }
    }
    let mut chars = 0usize;
    let mut words = 0usize;
    for line in lines {
        text_ok(&line.text, "line text")?;
        rect_ok(&line.rect, "line rect")?;
        chars += line.text.chars().count();
        words += line.words.len();
        for word in &line.words {
            text_ok(&word.text, "word text")?;
            rect_ok(&word.rect, "word rect")?;
            if word.text.chars().any(char::is_whitespace) {
                return Err(invalid("a word must not contain spaces"));
            }
        }
        for paint in [&line.color, &line.background].into_iter().flatten() {
            paint_ok(paint)?;
        }
    }
    if chars > MAX_TEXT_CHARS {
        return Err(invalid(format!("OCR text is limited to {MAX_TEXT_CHARS} characters per operation")));
    }
    if words > MAX_WORDS {
        return Err(invalid(format!("at most {MAX_WORDS} OCR words per page")));
    }
    Ok(())
}

/// One text run to draw: `text` starting at `(x, baseline)`, scaled
/// horizontally so its advance is `width`.
#[derive(Debug, Clone)]
struct Run {
    text: String,
    x: f64,
    baseline: f64,
    size: f64,
    width: f64,
    color: Rgb,
}

/// The chosen library font, plus a fallback for characters it cannot draw.
struct Faces {
    id: String,
    bytes: Vec<u8>,
    fallback: Option<(String, Vec<u8>)>,
}

fn faces(fonts: &dyn FontSource, id: &str, text: &str, report: &mut OperationReport) -> Result<Faces, EditorError> {
    let bytes = fonts.font_bytes(id).ok_or_else(|| {
        EditorError::new("EDITOR_FONT_UNAVAILABLE", format!("font {id} is not in the font library"))
    })?;
    let fallback = if covers(&bytes, text) {
        None
    } else {
        FALLBACK_FONTS
            .iter()
            .filter(|candidate| **candidate != id)
            .find_map(|candidate| fonts.font_bytes(candidate).filter(|b| covers(b, text)).map(|b| (candidate.to_string(), b)))
    };
    match &fallback {
        Some((fallback_id, _)) => report.notes.push(format!("Characters missing from {id} are set in {fallback_id}.")),
        None if !covers(&bytes, text) => {
            report.notes.push(format!("Some recognised characters are missing from {id} and every fallback font."))
        }
        None => {}
    }
    report.fonts_added.push(id.to_owned());
    Ok(Faces { id: id.to_owned(), bytes, fallback })
}

/// Vertical metrics of a font, as fractions of the em: ascender, descender
/// (positive, below the baseline) and the ink height of a tall lowercase
/// letter or capital (what a scan's line ink reaches above the baseline).
struct Metrics {
    ascender: f64,
    descender: f64,
    ink_top: f64,
}

fn metrics(bytes: &[u8]) -> Metrics {
    let Ok(face) = ttf_parser::Face::parse(bytes, 0) else {
        return Metrics { ascender: 0.8, descender: 0.2, ink_top: 0.7 };
    };
    let units = f64::from(face.units_per_em().max(1));
    let ascender = (f64::from(face.ascender()) / units).clamp(0.5, 1.2);
    let descender = (f64::from(face.descender()).abs() / units).clamp(0.05, 0.6);
    let ink_top = ['d', 'l', 'H']
        .iter()
        .filter_map(|c| face.glyph_index(*c))
        .filter_map(|glyph| face.glyph_bounding_box(glyph))
        .map(|bbox| f64::from(bbox.y_max) / units)
        .fold(0.0_f64, f64::max);
    Metrics { ascender, descender, ink_top: if ink_top > 0.3 { ink_top } else { 0.7 } }
}

/// Author the runs (and the paper-coloured areas under them) on a page the
/// size of the target page's MediaBox, then merge it onto `page`. `mode 3`
/// draws invisible text.
fn author(
    input: &[u8],
    page: u32,
    faces: &Faces,
    areas: &[(Rect, Rgb)],
    runs: &[Run],
    invisible: bool,
) -> Result<Vec<u8>, EditorError> {
    let document = overlay::load(input)?;
    let page_object = overlay::page_id(&document, page)?;
    let [x0, y0, x1, y1] = overlay::media_box(&document, page_object)?;
    drop(document);
    let mut builder = PdfBuilder::new();
    builder.set_compress_content(false);
    let primary = builder.register_font_bytes(postscript_name(&faces.bytes, &faces.id), faces.bytes.clone())?;
    let face: FontFace = match &faces.fallback {
        Some((id, bytes)) => {
            let extra = builder.register_font_bytes(postscript_name(bytes, id), bytes.clone())?;
            builder.register_font_stack(&[primary, extra])?
        }
        None => primary,
    };
    // Natural widths, measured with the same face the runs are drawn in.
    let mut placed: Vec<(f64, f64, f64)> = Vec::with_capacity(runs.len());
    {
        let sheet = builder.add_page(AuthorPageSize::custom(x1 - x0, y1 - y0));
        for (area, paint) in areas {
            let style = GraphicsStyle { stroke: None, fill: Some(paint.engine()), ..GraphicsStyle::default() };
            sheet.draw_rect(area.x - x0, area.y - y0, area.width, area.height, &style);
        }
        for run in runs {
            let style = TextStyle::new(face, run.size).fill(run.color.engine());
            let natural = sheet.text_width(run.text.trim_end(), &style)?.max(0.01);
            let scale = (100.0 * run.width / natural).clamp(10.0, 1000.0);
            placed.push((run.x - x0, run.baseline - y0, scale));
            sheet.draw_text(run.text.clone(), run.x - x0, run.baseline - y0, &style)?;
        }
    }
    let authored = builder.to_bytes()?;
    let authored = set_text_state(&authored, &placed, invisible)?;
    overlay::merge(input, page, &authored, &[])
}

/// Give every text object of the authored page its run's horizontal scale
/// (`Tz`) and, for a layer, rendering mode 3. The authored stream is patched
/// line by line (the builder writes one operator per line), so everything
/// else stays byte for byte as the builder wrote it. A text object is matched
/// to its run by its start point; the start of a later font segment of the
/// same run is moved by the run's scale too.
fn set_text_state(authored: &[u8], placed: &[(f64, f64, f64)], invisible: bool) -> Result<Vec<u8>, EditorError> {
    let failed = |message: String| EditorError::new("EDITOR_TEXT_LAYER_FAILED", message);
    let mut document = Document::load_mem(authored).map_err(|e| failed(format!("authored text is unreadable: {e}")))?;
    let page = *document.get_pages().get(&1).ok_or_else(|| failed("authored text has no page".into()))?;
    let content = document.get_page_content(page);
    let content = String::from_utf8(content).map_err(|_| failed("authored text is not line-based".into()))?;
    let content = without_plain_actual_text(&content);
    let mut out = String::with_capacity(content.len() + placed.len() * 24 + 8);
    if invisible {
        out.push_str("3 Tr\n");
    }
    let mut matched = vec![false; placed.len()];
    let mut in_text = false;
    let mut positioned = false;
    for line in content.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.first() == Some(&"BT") {
            in_text = true;
            positioned = false;
        }
        if fields.first() == Some(&"ET") || fields.last() == Some(&"ET") {
            in_text = false;
        }
        // `a b c d e f Tm`, possibly followed by the run's show operator.
        if in_text && !positioned && fields.len() >= 7 && fields[6] == "Tm" {
            if let [a, b, c, d, e, f, ..] = fields[..] {
                positioned = true;
                let parse = |v: &str| v.parse::<f64>().map_err(|_| failed(format!("authored text position {v:?} is not a number")));
                let (x, y) = (parse(e)?, parse(f)?);
                // The run on this baseline whose start is nearest (at or before) x.
                let index = placed
                    .iter()
                    .enumerate()
                    .filter(|(_, (rx, ry, _))| (ry - y).abs() < 0.01 && *rx <= x + 0.01)
                    .min_by(|p, q| (x - p.1 .0).total_cmp(&(x - q.1 .0)))
                    .map(|(index, _)| index)
                    .ok_or_else(|| failed(format!("authored text at ({x:.2}, {y:.2}) has no run")))?;
                matched[index] = true;
                let (rx, _, scale) = placed[index];
                let moved = rx + (x - rx) * scale / 100.0;
                let rest = &line[line.find(" Tm").map_or(line.len(), |i| i + 3)..];
                out.push_str(&format!("{scale:.4} Tz\n{a} {b} {c} {d} {moved:.4} {f} Tm{rest}\n"));
                continue;
            }
        }
        if invisible && fields.contains(&"Tr") {
            // The layer stays invisible whatever the authoring wrote.
            let mut fields = fields.clone();
            for index in 1..fields.len() {
                if fields[index] == "Tr" {
                    fields[index - 1] = "3";
                }
            }
            out.push_str(&fields.join(" "));
            out.push('\n');
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if let Some(missing) = matched.iter().position(|m| !m) {
        return Err(failed(format!("run {missing} was not authored")));
    }
    document
        .change_page_content(page, out.into_bytes())
        .map_err(|e| failed(format!("text state could not be written: {e}")))?;
    document.compress();
    overlay::save(&mut document)
}

/// Characters whose glyphs read back in logical order through a ToUnicode
/// map alone: no shaping, joining or right-to-left reordering.
fn plain_char(c: char) -> bool {
    c < '\u{0590}' || ('\u{1E00}'..='\u{20CF}').contains(&c) || ('\u{2100}'..='\u{24FF}').contains(&c)
}

/// The text of an authored `/Span << /ActualText <FEFF…> >> BDC` line.
fn actual_text(line: &str) -> Option<String> {
    let start = line.find("/ActualText")? + "/ActualText".len();
    let rest = line[start..].trim_start().strip_prefix('<')?;
    let hex: String = rest[..rest.find('>')?].chars().filter(|c| !c.is_whitespace()).collect();
    if !hex.len().is_multiple_of(4) || !hex.is_ascii() {
        return None;
    }
    let bytes: Vec<u8> = (0..hex.len() / 2)
        .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16))
        .collect::<Result<_, _>>()
        .ok()?;
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|p| u16::from_be_bytes(*p)).collect();
    let units = units.strip_prefix(&[0xFEFF]).unwrap_or(&units);
    String::from_utf16(units).ok()
}

/// The authoring builder wraps each text run in `/Span <</ActualText …>>
/// BDC … EMC`. For plain-script text (see [`plain_char`]) the run's
/// ToUnicode map already reads back exactly, and the engine's in-place edit
/// refuses text under an ActualText span (the span would keep the old
/// words: QF-2590), so those spans are left out; shaped or right-to-left
/// text keeps its span, which carries its logical order.
pub(crate) fn without_plain_actual_text(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    // One entry per open marked-content sequence: whether it was left out.
    let mut open: Vec<bool> = Vec::new();
    for line in content.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        match fields.last() {
            Some(&"BDC") | Some(&"BMC") => {
                let drop = fields.first() == Some(&"/Span")
                    && actual_text(line).is_some_and(|text| text.chars().all(plain_char));
                open.push(drop);
                if drop {
                    continue;
                }
            }
            Some(&"EMC") if fields.len() == 1 && open.pop() == Some(true) => continue,
            _ => {}
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// [`without_plain_actual_text`] on a one-page authored PDF (Add Text).
pub(crate) fn authored_without_plain_actual_text(authored: Vec<u8>) -> Result<Vec<u8>, EditorError> {
    let Ok(mut document) = Document::load_mem(&authored) else {
        return Ok(authored);
    };
    let Some(page) = document.get_pages().get(&1).copied() else {
        return Ok(authored);
    };
    let Ok(content) = String::from_utf8(document.get_page_content(page)) else {
        return Ok(authored);
    };
    if !content.contains("/ActualText") {
        return Ok(authored);
    }
    let plain = without_plain_actual_text(&content);
    document
        .change_page_content(page, plain.into_bytes())
        .map_err(|e| EditorError::new("EDITOR_WRITE_FAILED", e.to_string()))?;
    document.compress();
    overlay::save(&mut document)
}

/// The font's PostScript name (so the PDF names the real typeface), else the id.
fn postscript_name(bytes: &[u8], id: &str) -> String {
    ttf_parser::Face::parse(bytes, 0)
        .ok()
        .and_then(|face| {
            face.names()
                .into_iter()
                .filter(|n| n.name_id == ttf_parser::name_id::POST_SCRIPT_NAME)
                .find_map(|n| n.to_string())
        })
        .filter(|name| !name.is_empty() && name.is_ascii())
        .unwrap_or_else(|| id.to_owned())
}

fn all_text(lines: &[OcrLine]) -> String {
    lines.iter().map(|line| line.text.as_str()).collect::<Vec<_>>().join(" ")
}

fn upright(input: &[u8], page: u32) -> Result<PageTextLayout, EditorError> {
    let layout = PageTextLayout::from_bytes(input, page as usize)?;
    if layout.rotation % 360 != 0 {
        return Err(EditorError::new(
            "EDITOR_OCR_ROTATED_PAGE",
            "OCR text can be added to upright pages only; rotate the page back to 0° first",
        ));
    }
    Ok(layout)
}

/// `addTextLayer`: invisible OCR text over a scanned page.
pub fn add_text_layer(
    input: &[u8],
    page: u32,
    lines: &[OcrLine],
    font: Option<&str>,
    fonts: &dyn FontSource,
    report: &mut OperationReport,
) -> Result<Vec<u8>, EditorError> {
    upright(input, page)?;
    let faces = faces(fonts, font.unwrap_or(DEFAULT_LAYER_FONT), &all_text(lines), report)?;
    let m = metrics(&faces.bytes);
    let mut runs = Vec::new();
    let mut region: Option<[f64; 4]> = None;
    for line in lines {
        // One size per line: the em box (ascender + descender) fills the line box.
        let size = (line.rect.height / (m.ascender + m.descender)).clamp(1.0, 500.0);
        let baseline = line.rect.y + m.descender * size;
        let words: Vec<(String, Rect)> = if line.words.is_empty() {
            vec![(line.text.trim().to_owned(), line.rect)]
        } else {
            line.words.iter().map(|w| (w.text.clone(), w.rect)).collect()
        };
        let last = words.len() - 1;
        for (index, (text, rect)) in words.into_iter().enumerate() {
            // A space after each word but the last keeps words apart in the
            // extracted text; the scale is set for the word itself.
            let text = if index < last { format!("{text} ") } else { text };
            runs.push(Run { text, x: rect.x, baseline, size, width: rect.width, color: Paint::BLACK });
        }
        region = Some(union(region, line.rect));
    }
    report.changed_region = region;
    report.notes.push(format!(
        "Added an invisible text layer of {} line{} in {}: the page looks the same, its text can now be selected, searched and copied.",
        lines.len(),
        if lines.len() == 1 { "" } else { "s" },
        faces.id
    ));
    author(input, page, &faces, &[], &runs, true)
}

fn union(region: Option<[f64; 4]>, rect: Rect) -> [f64; 4] {
    let r = [rect.x, rect.y, rect.x + rect.width, rect.y + rect.height];
    match region {
        None => r,
        Some(a) => [a[0].min(r[0]), a[1].min(r[1]), a[2].max(r[2]), a[3].max(r[3])],
    }
}

/// A rendered page in RGB and the mapping between its pixels and user space.
struct Raster {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
    layout: PageTextLayout,
    dpi: u32,
}

impl Raster {
    fn render(input: &[u8], page: u32, layout: PageTextLayout) -> Result<Self, EditorError> {
        let [bx0, by0, bx1, by1] = layout.page_box;
        let unit = layout.user_unit.max(0.01);
        let area = ((bx1 - bx0).abs() * unit / 72.0) * ((by1 - by0).abs() * unit / 72.0);
        let mut dpi = SAMPLE_DPI;
        if area * f64::from(dpi * dpi) > SAMPLE_PIXELS {
            dpi = ((SAMPLE_PIXELS / area.max(1e-6)).sqrt().floor() as u32).max(36);
        }
        let engine = ContentEngine::open_bytes(input.to_vec())?;
        let raw = engine.render_page(page as usize, dpi)?.into_raw_image();
        let (width, height) = (raw.width as usize, raw.height as usize);
        let pixels = match raw.channels {
            3 => raw.pixels,
            4 => raw.pixels.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect(),
            1 => raw.pixels.iter().flat_map(|g| [*g, *g, *g]).collect(),
            other => {
                return Err(EditorError::new(
                    "EDITOR_OCR_SAMPLING_FAILED",
                    format!("the page rendered with {other} channels"),
                ))
            }
        };
        Ok(Raster { width, height, pixels, layout, dpi })
    }

    /// The pixel box `[x0, y0, x1, y1)` covering a user-space rectangle.
    fn pixel_box(&self, rect: &Rect, pad: f64) -> [usize; 4] {
        let corners = [
            [rect.x, rect.y],
            [rect.x + rect.width, rect.y],
            [rect.x, rect.y + rect.height],
            [rect.x + rect.width, rect.y + rect.height],
        ];
        let device: Vec<[f64; 2]> = corners.iter().map(|c| self.layout.to_device(*c, self.dpi)).collect();
        let min_x = device.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min) - pad;
        let max_x = device.iter().map(|p| p[0]).fold(f64::NEG_INFINITY, f64::max) + pad;
        let min_y = device.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min) - pad;
        let max_y = device.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max) + pad;
        let clamp_x = |v: f64| (v.max(0.0) as usize).min(self.width);
        let clamp_y = |v: f64| (v.max(0.0) as usize).min(self.height);
        [clamp_x(min_x.floor()), clamp_y(min_y.floor()), clamp_x(max_x.ceil()), clamp_y(max_y.ceil())]
    }

    fn pixel(&self, x: usize, y: usize) -> [u8; 3] {
        let i = (y * self.width + x) * 3;
        [self.pixels[i], self.pixels[i + 1], self.pixels[i + 2]]
    }

    fn user(&self, x: f64, y: f64) -> [f64; 2] {
        self.layout.from_device([x, y], self.dpi).unwrap_or([0.0, 0.0])
    }

    /// Pixels per point along the page's vertical axis.
    fn scale(&self) -> f64 {
        f64::from(self.dpi) / 72.0 * self.layout.user_unit.max(0.01)
    }
}

fn luminance(p: [u8; 3]) -> f64 {
    0.2126 * f64::from(p[0]) + 0.7152 * f64::from(p[1]) + 0.0722 * f64::from(p[2])
}

fn mean(pixels: &[[u8; 3]]) -> [f64; 3] {
    let n = pixels.len().max(1) as f64;
    let mut sum = [0.0; 3];
    for p in pixels {
        for c in 0..3 {
            sum[c] += f64::from(p[c]);
        }
    }
    [sum[0] / n / 255.0, sum[1] / n / 255.0, sum[2] / n / 255.0]
}

/// What the scan shows for one line.
#[derive(Debug, Clone, Copy)]
struct Measured {
    paper: [f64; 3],
    ink: Option<[f64; 3]>,
    /// Ink extent in pixels: left, top (ascenders), right, baseline.
    extent: Option<[usize; 4]>,
}

/// Paper is the bright majority of a ring around the line box; ink is every
/// pixel inside the box clearly darker than the paper.
fn measure(raster: &Raster, rect: &Rect) -> Measured {
    let inner = raster.pixel_box(rect, 0.0);
    let outer = raster.pixel_box(rect, 3.0);
    let mut ring = Vec::new();
    for y in outer[1]..outer[3] {
        for x in outer[0]..outer[2] {
            let inside = x >= inner[0] && x < inner[2] && y >= inner[1] && y < inner[3];
            if !inside {
                ring.push(raster.pixel(x, y));
            }
        }
    }
    if ring.is_empty() {
        for y in inner[1]..inner[3] {
            for x in inner[0]..inner[2] {
                ring.push(raster.pixel(x, y));
            }
        }
    }
    ring.sort_by(|a, b| luminance(*a).total_cmp(&luminance(*b)));
    let bright = if ring.len() >= 4 { &ring[ring.len() / 2..ring.len() * 9 / 10 + 1] } else { &ring[..] };
    let paper = if bright.is_empty() { [1.0, 1.0, 1.0] } else { mean(bright) };
    let paper_luminance = 0.2126 * paper[0] * 255.0 + 0.7152 * paper[1] * 255.0 + 0.0722 * paper[2] * 255.0;
    let threshold = paper_luminance - (paper_luminance * 0.3).max(40.0);
    let (w, h) = (inner[2].saturating_sub(inner[0]), inner[3].saturating_sub(inner[1]));
    let mut rows = vec![0usize; h];
    let mut columns = vec![0usize; w];
    let mut ink = Vec::new();
    for (y, row) in rows.iter_mut().enumerate() {
        for (x, column) in columns.iter_mut().enumerate() {
            let p = raster.pixel(inner[0] + x, inner[1] + y);
            if luminance(p) < threshold {
                *row += 1;
                *column += 1;
                ink.push(p);
            }
        }
    }
    if ink.len() < 4 {
        return Measured { paper, ink: None, extent: None };
    }
    ink.sort_by(|a, b| luminance(*a).total_cmp(&luminance(*b)));
    let core = &ink[..(ink.len() * 3 / 10).max(1)];
    let peak = rows.iter().copied().max().unwrap_or(0);
    // Ascenders are sparse but present; the baseline is where the dense
    // x-height band ends (descenders below it are sparse).
    let top = rows.iter().position(|&n| n >= 1 && n * 50 >= peak).unwrap_or(0);
    let baseline = rows.iter().rposition(|&n| n * 10 >= peak * 3).map(|r| r + 1).unwrap_or(h);
    let left = columns.iter().position(|&n| n > 0).unwrap_or(0);
    let right = columns.iter().rposition(|&n| n > 0).map(|c| c + 1).unwrap_or(w);
    Measured {
        paper,
        ink: Some(mean(core)),
        extent: Some([inner[0] + left, inner[1] + top, inner[0] + right, inner[1] + baseline]),
    }
}

fn rgb_of(value: [f64; 3]) -> Rgb {
    let q = |v: f64| (v.clamp(0.0, 1.0) * 1000.0).round() / 1000.0;
    Paint::rgb(q(value[0]), q(value[1]), q(value[2]))
}

/// `convertOcrText`: visible, editable text in place of the scanned lines.
pub fn convert_ocr_text(
    input: &[u8],
    page: u32,
    lines: &[OcrLine],
    font: Option<&str>,
    fonts: &dyn FontSource,
    report: &mut OperationReport,
) -> Result<Vec<u8>, EditorError> {
    let layout = upright(input, page)?;
    let faces = faces(fonts, font.unwrap_or(DEFAULT_TEXT_FONT), &all_text(lines), report)?;
    let m = metrics(&faces.bytes);
    let raster = Raster::render(input, page, layout)?;
    let px = raster.scale();
    let pad = 0.75; // points of paper added around each line area
    let mut areas = Vec::with_capacity(lines.len());
    let mut runs = Vec::with_capacity(lines.len());
    let mut region: Option<[f64; 4]> = None;
    for line in lines {
        let measured = measure(&raster, &line.rect);
        let paper = line.background.clone().unwrap_or_else(|| rgb_of(measured.paper));
        let ink = line.color.clone().or_else(|| measured.ink.map(rgb_of)).unwrap_or(Paint::BLACK);
        let text = line.text.trim().to_owned();
        let (x, baseline, size, width, area) = match measured.extent {
            Some([left, top, right, base]) => {
                let ink_height = (base.saturating_sub(top)) as f64 / px;
                let size = (ink_height / m.ink_top).clamp(1.0, 500.0);
                let [ux0, uy_base] = raster.user(left as f64, base as f64);
                let [ux1, uy_top] = raster.user(right as f64, top as f64);
                let ink_rect = Rect { x: ux0.min(ux1), y: uy_base.min(uy_top), width: (ux1 - ux0).abs(), height: (uy_top - uy_base).abs() };
                let area = union(Some(union(None, line.rect)), ink_rect);
                (ux0.min(ux1), uy_base, size, (ux1 - ux0).abs().max(0.5), area)
            }
            None => {
                let size = (line.rect.height / (m.ascender + m.descender)).clamp(1.0, 500.0);
                let area = union(None, line.rect);
                (line.rect.x, line.rect.y + m.descender * size, size, line.rect.width, area)
            }
        };
        let area = Rect { x: area[0] - pad, y: area[1] - pad, width: area[2] - area[0] + 2.0 * pad, height: area[3] - area[1] + 2.0 * pad };
        region = Some(union(region, area));
        areas.push((area, paper));
        runs.push(Run { text, x, baseline, size, width, color: ink });
    }
    report.changed_region = region;
    report.notes.push(format!(
        "Converted {} scanned line{} to editable text in {}; the scanned letters under them are painted over with the paper colour.",
        lines.len(),
        if lines.len() == 1 { "" } else { "s" },
        faces.id
    ));
    author(input, page, &faces, &areas, &runs, false)
}
