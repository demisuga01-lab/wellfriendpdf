//! XFA text: font selection, rich-text (XHTML subset) parsing and line
//! layout in the style of PDFium's `CXFA_TextLayout` (first line as tall as
//! the font size, later lines 1.2 em unless `lineHeight` says otherwise;
//! para margins, indent, space above/below, horizontal alignment).

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;
use crate::render::annotation_synthesis::{n, Resources, TextFonts};

use super::style::{parse_color, parse_h_align, FontSpec, HAlign, ParaSpec, Rgb};
use super::xml::XmlNode;

/// Font family class used to pick a standard-14 stand-in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    Sans,
    Serif,
    Mono,
}

fn family_of(typeface: &str) -> Family {
    let lower = typeface.to_ascii_lowercase();
    let mono = [
        "courier",
        "mono",
        "consol",
        "lucida console",
        "ocr",
        "letter gothic",
    ];
    if mono.iter().any(|name| lower.contains(name)) {
        return Family::Mono;
    }
    if lower.contains("sans") {
        return Family::Sans;
    }
    let serif = [
        "times",
        "minion",
        "garamond",
        "georgia",
        "serif",
        "cambria",
        "book antiqua",
        "palatino",
        "century",
        "bodoni",
        "baskerville",
        "caslon",
        "song",
        "ming",
        "batang",
        "mincho",
    ];
    if serif.iter().any(|name| lower.contains(name)) {
        return Family::Serif;
    }
    Family::Sans
}

fn standard_base(family: Family, bold: bool, italic: bool) -> &'static str {
    match (family, bold, italic) {
        (Family::Sans, false, false) => "Helvetica",
        (Family::Sans, true, false) => "Helvetica-Bold",
        (Family::Sans, false, true) => "Helvetica-Oblique",
        (Family::Sans, true, true) => "Helvetica-BoldOblique",
        (Family::Serif, false, false) => "Times-Roman",
        (Family::Serif, true, false) => "Times-Bold",
        (Family::Serif, false, true) => "Times-Italic",
        (Family::Serif, true, true) => "Times-BoldItalic",
        (Family::Mono, false, false) => "Courier",
        (Family::Mono, true, false) => "Courier-Bold",
        (Family::Mono, false, true) => "Courier-Oblique",
        (Family::Mono, true, true) => "Courier-BoldOblique",
    }
}

/// PDFium places a glyph baseline `size * ascent / (ascent - descent)` below
/// the line top (`CFGAS_TxtBreak::GetDisplayPos`); this is that ratio for
/// the faces PDFium finds for the standard families (Arial 905/-212,
/// Times New Roman 891/-216, Courier New 833/-300).
fn standard_ascent(family: Family) -> f64 {
    let (ascent, descent) = match family {
        Family::Sans => (0.905, 0.212),
        Family::Serif => (0.891, 0.216),
        Family::Mono => (0.833, 0.300),
    };
    ascent / (ascent + descent)
}

fn normalize_family(name: &str) -> String {
    name.chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Splits a PDF `/BaseFont` into a normalised family and bold/italic flags.
fn base_font_style(base: &str) -> (String, bool, bool) {
    let base = match base.split_once('+') {
        Some((prefix, rest)) if prefix.len() == 6 => rest,
        _ => base,
    };
    let (family, style) = match base.find([',', '-']) {
        Some(split) => (&base[..split], &base[split + 1..]),
        None => (base, ""),
    };
    let family_lower = family.to_ascii_lowercase();
    let style = style.to_ascii_lowercase();
    let mut family = normalize_family(family);
    for suffix in ["mt", "ps", "psmt"] {
        if family.len() > suffix.len() + 2 && family.ends_with(suffix) {
            family.truncate(family.len() - suffix.len());
        }
    }
    let bold = style.contains("bold")
        || style.contains("black")
        || style.contains("heavy")
        || style == "bd"
        || style.starts_with("semibold")
        || family_lower.ends_with("bold");
    let italic = style.contains("it") || style.contains("oblique");
    (family, bold, italic)
}

struct DocFont {
    family: String,
    bold: bool,
    italic: bool,
    object: PdfObject,
    dict: PdfDictionary,
}

struct BookFont {
    fonts: TextFonts,
    ascent: f64,
}

/// The fonts of one painted XFA document: each distinct typeface/style gets
/// a resource `XF<n>` (an embedded `/AcroForm /DR` font of that family when
/// the PDF carries a usable one, else a standard-14 stand-in) with Unicode
/// fallbacks `XF<n>U...`.
pub(crate) struct FontBook<'r> {
    fonts: Vec<BookFont>,
    index: HashMap<(String, bool, bool), usize>,
    doc_fonts: Vec<DocFont>,
    reader: Option<&'r PdfReader>,
}

impl<'r> FontBook<'r> {
    /// `dr_fonts` are candidate document fonts (`/DR /Font` values) that are
    /// only used when `reader` is given.
    pub(crate) fn new(
        reader: Option<&'r PdfReader>,
        dr_fonts: Vec<(PdfObject, PdfDictionary)>,
    ) -> Self {
        let doc_fonts = if reader.is_some() {
            dr_fonts
                .into_iter()
                .filter_map(|(object, dict)| {
                    let base = dict.get_name("BaseFont")?.to_string();
                    // Subset fonts lack the glyphs field values need.
                    if base
                        .split_once('+')
                        .is_some_and(|(prefix, _)| prefix.len() == 6)
                    {
                        return None;
                    }
                    let embedded = dict
                        .get("FontDescriptor")
                        .and_then(|descriptor| match descriptor {
                            PdfObject::Dictionary(d) => Some(d.clone()),
                            other => reader?
                                .resolve(other.clone())
                                .ok()
                                .and_then(|o| o.as_dict().cloned()),
                        })
                        .is_some_and(|descriptor| {
                            ["FontFile", "FontFile2", "FontFile3"]
                                .iter()
                                .any(|key| descriptor.get(key).is_some())
                        });
                    if !embedded {
                        return None;
                    }
                    let (family, bold, italic) = base_font_style(&base);
                    Some(DocFont {
                        family,
                        bold,
                        italic,
                        object,
                        dict,
                    })
                })
                .collect()
        } else {
            Vec::new()
        };
        Self {
            fonts: Vec::new(),
            index: HashMap::new(),
            doc_fonts,
            reader,
        }
    }

    pub(crate) fn font(&mut self, spec: &FontSpec) -> usize {
        let key = (normalize_family(&spec.typeface), spec.bold, spec.italic);
        if let Some(index) = self.index.get(&key) {
            return *index;
        }
        let number = self.fonts.len() + 1;
        let resource = format!("XF{number}");
        let fallback = format!("XF{number}U");
        let doc = self.reader.and_then(|reader| {
            self.doc_fonts
                .iter()
                .find(|font| font.family == key.0 && font.bold == key.1 && font.italic == key.2)
                .and_then(|font| {
                    TextFonts::simple_document_font(
                        font.object.clone(),
                        &font.dict,
                        reader,
                        &resource,
                        &fallback,
                    )
                })
        });
        let book_font = match doc {
            Some(fonts) => {
                let ascent = fonts.ascent();
                let descent = -fonts.descent();
                let ascent = if ascent > 0.3 && descent >= 0.0 {
                    ascent / (ascent + descent)
                } else {
                    0.8
                };
                BookFont { fonts, ascent }
            }
            None => {
                let family = family_of(&spec.typeface);
                let ascent = standard_ascent(family);
                BookFont {
                    fonts: TextFonts::standard_named(
                        standard_base(family, spec.bold, spec.italic),
                        &resource,
                        &fallback,
                    ),
                    ascent,
                }
            }
        };
        self.fonts.push(book_font);
        self.index.insert(key, self.fonts.len() - 1);
        self.fonts.len() - 1
    }

    pub(crate) fn measure(&mut self, font: usize, text: &str, size: f64) -> f64 {
        self.fonts[font].fonts.measure(text, size)
    }

    pub(crate) fn ascent(&self, font: usize) -> f64 {
        self.fonts[font].ascent
    }

    /// Text-showing operators for `text` (inside BT, after the matrix).
    pub(crate) fn show(&mut self, font: usize, out: &mut String, text: &str, size: f64) {
        self.fonts[font].fonts.show(out, text, size);
    }

    pub(crate) fn add_to(&self, resources: &mut Resources) {
        for font in &self.fonts {
            font.fonts.add_to(resources);
        }
    }
}

/// A run of identically styled text.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Run {
    pub text: String,
    pub font: FontSpec,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Paragraph {
    pub para: ParaSpec,
    pub runs: Vec<Run>,
}

/// Plain text (paragraphs at line breaks) in one font and para style.
pub(crate) fn plain_paragraphs(text: &str, font: &FontSpec, para: &ParaSpec) -> Vec<Paragraph> {
    // Line feeds, carriage returns and the Unicode line/paragraph separators
    // (Designer writes U+2029 between paragraphs of plain text).
    let normalized = text
        .replace("\r\n", "\n")
        .replace(['\r', '\u{2028}', '\u{2029}'], "\n");
    normalized
        .split('\n')
        .map(|line| Paragraph {
            para: *para,
            runs: vec![Run {
                text: line.replace('\t', "    "),
                font: font.clone(),
            }],
        })
        .collect()
}

/// Parses an `exData` XHTML body into paragraphs.
pub(crate) fn rich_paragraphs(root: &XmlNode, font: &FontSpec, para: &ParaSpec) -> Vec<Paragraph> {
    let mut builder = RichBuilder {
        paragraphs: Vec::new(),
        current: None,
        depth: 0,
    };
    let style = Style {
        font: font.clone(),
        para: *para,
        spacerun: false,
    };
    builder.walk(root, &style);
    builder.finish();
    let mut paragraphs = builder.paragraphs;
    while paragraphs
        .last()
        .is_some_and(|p| p.runs.iter().all(|r| r.text.trim().is_empty()))
        && paragraphs.len() > 1
    {
        paragraphs.pop();
    }
    paragraphs
}

#[derive(Clone)]
struct Style {
    font: FontSpec,
    para: ParaSpec,
    spacerun: bool,
}

struct RichBuilder {
    paragraphs: Vec<Paragraph>,
    current: Option<Paragraph>,
    depth: usize,
}

impl RichBuilder {
    fn finish(&mut self) {
        if let Some(mut paragraph) = self.current.take() {
            if let Some(first) = paragraph.runs.first_mut() {
                let trimmed = first.text.trim_start().to_string();
                first.text = trimmed;
            }
            if let Some(last) = paragraph.runs.last_mut() {
                let trimmed = last.text.trim_end().to_string();
                last.text = trimmed;
            }
            self.paragraphs.push(paragraph);
        }
    }

    fn start_paragraph(&mut self, para: ParaSpec) {
        self.finish();
        self.current = Some(Paragraph {
            para,
            runs: Vec::new(),
        });
    }

    fn push_text(&mut self, text: &str, style: &Style) {
        if text.is_empty() {
            return;
        }
        let text = if style.spacerun {
            text.replace(['\n', '\r', '\t'], " ")
        } else {
            collapse_whitespace(text)
        };
        let paragraph = self.current.get_or_insert_with(|| Paragraph {
            para: style.para,
            runs: Vec::new(),
        });
        // Collapse a space against a preceding space.
        let text = if !style.spacerun
            && text.starts_with(' ')
            && paragraph
                .runs
                .last()
                .is_none_or(|run| run.text.is_empty() || run.text.ends_with(' '))
        {
            text.trim_start().to_string()
        } else {
            text
        };
        if text.is_empty() {
            return;
        }
        match paragraph.runs.last_mut() {
            Some(run) if run.font == style.font => run.text.push_str(&text),
            _ => paragraph.runs.push(Run {
                text,
                font: style.font.clone(),
            }),
        }
    }

    fn walk(&mut self, node: &XmlNode, inherited: &Style) {
        self.depth += 1;
        if self.depth > 64 {
            self.depth -= 1;
            return;
        }
        let mut style = inherited.clone();
        match node.local_name.as_str() {
            "b" | "strong" => style.font.bold = true,
            "i" | "em" => style.font.italic = true,
            "u" => style.font.underline = true,
            _ => {}
        }
        if let Some(css) = node.attr("style") {
            apply_css(css, &mut style);
        }
        let block = matches!(
            node.local_name.as_str(),
            "p" | "div" | "li" | "h1" | "h2" | "h3"
        );
        if block {
            self.start_paragraph(style.para);
        }
        if node.local_name == "br" {
            let para = self
                .current
                .as_ref()
                .map(|p| ParaSpec {
                    space_above: 0.0,
                    text_indent: 0.0,
                    ..p.para
                })
                .unwrap_or(style.para);
            self.start_paragraph(para);
        }
        self.push_text(&node.head, &style);
        for child in &node.children {
            self.walk(child, &style);
            self.push_text(&child.tail, &style);
        }
        if block {
            self.finish();
        }
        self.depth -= 1;
    }
}

fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for ch in text.chars() {
        if ch.is_whitespace() && ch != '\u{a0}' {
            if !space {
                out.push(' ');
            }
            space = true;
        } else {
            out.push(ch);
            space = false;
        }
    }
    out
}

fn css_length(value: &str, font_size: f64) -> Option<f64> {
    let value = value.trim();
    if let Some(number) = value.strip_suffix("em") {
        return number.trim().parse::<f64>().ok().map(|v| v * font_size);
    }
    if let Some(number) = value.strip_suffix('%') {
        return number
            .trim()
            .parse::<f64>()
            .ok()
            .map(|v| v * font_size / 100.0);
    }
    super::parse_measurement(value).ok()
}

fn css_color(value: &str) -> Option<Rgb> {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix('#') {
        let parse = |range: std::ops::Range<usize>| u8::from_str_radix(hex.get(range)?, 16).ok();
        return match hex.len() {
            6 => Some([parse(0..2)?, parse(2..4)?, parse(4..6)?]),
            3 => {
                let one = |i: usize| {
                    u8::from_str_radix(hex.get(i..i + 1)?, 16)
                        .ok()
                        .map(|v| v * 17)
                };
                Some([one(0)?, one(1)?, one(2)?])
            }
            _ => None,
        };
    }
    if let Some(inner) = value
        .strip_prefix("rgb(")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        return parse_color(inner);
    }
    Some(match value.to_ascii_lowercase().as_str() {
        "black" => [0, 0, 0],
        "white" => [255, 255, 255],
        "red" => [255, 0, 0],
        "green" => [0, 128, 0],
        "blue" => [0, 0, 255],
        "gray" | "grey" => [128, 128, 128],
        _ => return None,
    })
}

fn apply_css(css: &str, style: &mut Style) {
    for declaration in css.split(';') {
        let Some((property, value)) = declaration.split_once(':') else {
            continue;
        };
        let property = property.trim().to_ascii_lowercase();
        let value = value.trim();
        let size = style.font.size;
        match property.as_str() {
            "font-family" => {
                let family = value
                    .split(',')
                    .next()
                    .unwrap_or(value)
                    .trim()
                    .trim_matches(['\'', '"']);
                if !family.is_empty() {
                    style.font.typeface = family.to_string();
                }
            }
            "font-size" => {
                if let Some(v) = css_length(value, size).filter(|v| *v > 0.0 && *v < 2000.0) {
                    style.font.size = v;
                }
            }
            "font-weight" => {
                style.font.bold = value == "bold"
                    || value == "bolder"
                    || value.parse::<u32>().is_ok_and(|weight| weight >= 600);
            }
            "font-style" => style.font.italic = value == "italic" || value == "oblique",
            "color" => {
                if let Some(color) = css_color(value) {
                    style.font.color = color;
                }
            }
            "text-decoration" => {
                style.font.underline = value.contains("underline");
                style.font.line_through = value.contains("line-through");
            }
            "text-align" => {
                if let Some(align) = parse_h_align(value) {
                    style.para.h_align = align;
                }
            }
            "margin-left" => style.para.margin_left = css_length(value, size).unwrap_or(0.0),
            "margin-right" => style.para.margin_right = css_length(value, size).unwrap_or(0.0),
            "margin-top" => {
                style.para.space_above = css_length(value, size).unwrap_or(0.0).max(0.0)
            }
            "margin-bottom" => {
                style.para.space_below = css_length(value, size).unwrap_or(0.0).max(0.0)
            }
            "text-indent" => style.para.text_indent = css_length(value, size).unwrap_or(0.0),
            "line-height" => {
                style.para.line_height = css_length(value, size).unwrap_or(0.0).max(0.0)
            }
            "xfa-spacerun" => style.spacerun = value == "yes",
            "vertical-align" => {
                style.font.baseline_shift = match value {
                    "super" => -size * 0.33,
                    "sub" => size * 0.2,
                    other => css_length(other, size).map_or(0.0, |v| -v),
                }
            }
            _ => {}
        }
    }
}

/// A laid-out piece of text: baseline origin `x`, `y` (y grows down).
#[derive(Clone, Debug)]
pub(crate) struct Piece {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub text: String,
    pub font: usize,
    pub size: f64,
    pub color: Rgb,
    pub underline: bool,
    pub line_through: bool,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct TextBlock {
    pub pieces: Vec<Piece>,
    pub height: f64,
    pub width: f64,
    pub lines: usize,
}

#[derive(Clone, Debug)]
struct Token {
    text: String,
    font: usize,
    spec: FontSpec,
    width: f64,
    space: bool,
    /// A line may break before this token.
    break_before: bool,
}

fn is_cjk(ch: char) -> bool {
    matches!(ch as u32,
        0x1100..=0x11FF | 0x2E80..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF)
}

fn tokenize(book: &mut FontBook<'_>, runs: &[Run]) -> Vec<Token> {
    let mut tokens: Vec<Token> = Vec::new();
    for run in runs {
        let font = book.font(&run.font);
        let mut current = String::new();
        let mut current_space = false;
        let flush = |book: &mut FontBook<'_>,
                     tokens: &mut Vec<Token>,
                     text: &mut String,
                     space: bool,
                     cjk: bool| {
            if text.is_empty() {
                return;
            }
            let break_before = space || cjk || can_break_after(tokens);
            let width = book.measure(font, text, run.font.size);
            tokens.push(Token {
                text: std::mem::take(text),
                font,
                spec: run.font.clone(),
                width,
                space,
                break_before,
            });
        };
        for ch in run.text.chars() {
            let space = ch == ' ';
            let cjk = is_cjk(ch);
            if !current.is_empty() && (space != current_space || cjk) {
                flush(book, &mut tokens, &mut current, current_space, false);
            }
            current.push(ch);
            current_space = space;
            if cjk {
                flush(book, &mut tokens, &mut current, false, true);
            }
        }
        flush(book, &mut tokens, &mut current, current_space, false);
    }
    tokens
}

/// Whether a line may break after the last token: after a space or a CJK
/// character (and at the very start).
fn can_break_after(tokens: &[Token]) -> bool {
    match tokens.last() {
        None => true,
        Some(prev) => prev.space || prev.text.chars().last().is_some_and(is_cjk),
    }
}

/// Lays out paragraphs in a box `width` wide (`None`: no wrapping).
pub(crate) fn layout_paragraphs(
    book: &mut FontBook<'_>,
    paragraphs: &[Paragraph],
    width: Option<f64>,
) -> TextBlock {
    layout_paragraphs_with(book, paragraphs, width, None)
}

/// Like [`layout_paragraphs`]; `edit_line` switches to the edit-control
/// model PDFium's `CFWL_Edit` uses: every line `edit_line` tall with the
/// glyphs centred vertically in it.
pub(crate) fn layout_paragraphs_with(
    book: &mut FontBook<'_>,
    paragraphs: &[Paragraph],
    width: Option<f64>,
    edit_line: Option<f64>,
) -> TextBlock {
    let mut block = TextBlock::default();
    let mut line_pos = 0.0f64;
    let mut line_count = 0usize;
    for paragraph in paragraphs {
        let para = paragraph.para;
        line_pos += para.space_above;
        let tokens = tokenize(book, &paragraph.runs);
        let avail = width.map(|w| (w - para.margin_left - para.margin_right).max(1.0));
        // Greedy line breaking.
        let mut lines: Vec<Vec<Token>> = Vec::new();
        let mut line: Vec<Token> = Vec::new();
        let mut line_width = 0.0;
        for token in tokens {
            let indent = if lines.is_empty() {
                para.text_indent
            } else {
                0.0
            };
            let limit = avail.map(|a| (a - indent).max(1.0));
            if token.space && line.is_empty() && !lines.is_empty() {
                continue;
            }
            let fits = limit.is_none_or(|limit| line_width + token.width <= limit + 0.01);
            if fits || token.space {
                line_width += token.width;
                line.push(token);
                continue;
            }
            // Break at the last opportunity in the current line.
            let split = line.iter().rposition(|t| t.break_before).unwrap_or(0);
            if split > 0 {
                let rest: Vec<Token> = line.split_off(split);
                lines.push(std::mem::take(&mut line));
                line = rest.into_iter().skip_while(|t| t.space).collect();
            } else if !line.is_empty() && token.break_before {
                lines.push(std::mem::take(&mut line));
            }
            line_width = line.iter().map(|t| t.width).sum();
            let limit = limit.unwrap_or(f64::MAX);
            if line_width + token.width <= limit + 0.01 {
                line_width += token.width;
                line.push(token);
                continue;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            // A word wider than the line: break between characters.
            let mut piece = String::new();
            let mut piece_width = 0.0;
            for ch in token.text.chars() {
                let cw = book.measure(token.font, ch.encode_utf8(&mut [0; 4]), token.spec.size);
                if piece_width + cw > limit && !piece.is_empty() {
                    let text = std::mem::take(&mut piece);
                    lines.push(vec![Token {
                        text,
                        width: piece_width,
                        ..token.clone()
                    }]);
                    piece_width = 0.0;
                }
                piece.push(ch);
                piece_width += cw;
            }
            line_width = piece_width;
            line = vec![Token {
                text: piece,
                width: piece_width,
                ..token
            }];
        }
        lines.push(line);
        let line_total = lines.len();
        for (index, mut tokens) in lines.into_iter().enumerate() {
            while tokens.last().is_some_and(|t| t.space) {
                tokens.pop();
            }
            // Line step (PDFium GetLineHeight).
            let mut step = 0.0f64;
            if tokens.is_empty() {
                let size = paragraph.runs.first().map_or(10.0, |run| run.font.size);
                step = if line_count == 0 {
                    size
                } else if para.line_height > 0.1 {
                    para.line_height
                } else {
                    size * 1.2
                };
            }
            for token in &tokens {
                let size = token.spec.size;
                let height = if line_count == 0 {
                    if para.line_height < 0.1 {
                        size
                    } else {
                        para.line_height.min(size)
                    }
                } else if para.line_height < 0.1 {
                    size * 1.2
                } else {
                    para.line_height
                };
                step = step.max(height);
            }
            if let Some(edit_line) = edit_line {
                step = edit_line;
            }
            let content_width: f64 = tokens.iter().map(|t| t.width).sum();
            let indent = if index == 0 { para.text_indent } else { 0.0 };
            let avail_line = avail.map(|a| a - indent);
            let mut x = para.margin_left + indent;
            let mut gap_extra = 0.0;
            let last_line = index + 1 == line_total;
            if let Some(avail_line) = avail_line {
                let slack = (avail_line - content_width).max(0.0);
                match para.h_align {
                    HAlign::Center => x += slack / 2.0,
                    HAlign::Right => x += slack,
                    HAlign::Justify | HAlign::JustifyAll
                        if !last_line || para.h_align == HAlign::JustifyAll =>
                    {
                        let gaps = tokens.iter().filter(|t| t.space).count();
                        if gaps > 0 {
                            gap_extra = slack / gaps as f64;
                        }
                    }
                    _ => {}
                }
            }
            block.width = block
                .width
                .max(para.margin_left + indent + content_width + para.margin_right);
            let line_start = block.pieces.len();
            for token in tokens {
                let size = token.spec.size;
                let top = match edit_line {
                    Some(line) => line_pos + (line - size) / 2.0,
                    None => line_pos + step - size,
                };
                let baseline = top + book.ascent(token.font) * size + token.spec.baseline_shift;
                let width = token.width;
                if token.space && gap_extra > 0.0 {
                    x += width + gap_extra;
                    continue;
                }
                // Contiguous, identically styled tokens share one piece.
                let merged = block.pieces.len() > line_start
                    && block.pieces.last_mut().is_some_and(|last| {
                        let alike = gap_extra == 0.0
                            && last.font == token.font
                            && (last.size - size).abs() < 1e-9
                            && last.color == token.spec.color
                            && last.underline == token.spec.underline
                            && last.line_through == token.spec.line_through
                            && (last.y - baseline).abs() < 1e-9;
                        if alike {
                            last.text.push_str(&token.text);
                            last.width += width;
                        }
                        alike
                    });
                if !merged {
                    block.pieces.push(Piece {
                        x,
                        y: baseline,
                        width,
                        text: token.text,
                        font: token.font,
                        size,
                        color: token.spec.color,
                        underline: token.spec.underline,
                        line_through: token.spec.line_through,
                    });
                }
                x += width;
            }
            line_pos += step;
            line_count += 1;
        }
        line_pos += para.space_below;
    }
    block.height = line_pos;
    block.lines = line_count;
    block
}

/// Emits a text block translated by (`dx`, `dy`) in a y-down user space.
pub(crate) fn emit_block(
    book: &mut FontBook<'_>,
    out: &mut String,
    block: &TextBlock,
    dx: f64,
    dy: f64,
) {
    for piece in &block.pieces {
        emit_piece(book, out, piece, dx, dy);
    }
}

pub(crate) fn emit_piece(
    book: &mut FontBook<'_>,
    out: &mut String,
    piece: &Piece,
    dx: f64,
    dy: f64,
) {
    if piece.text.is_empty() {
        return;
    }
    let [r, g, b] = piece.color;
    let x = piece.x + dx;
    let y = piece.y + dy;
    let _ = writeln!(
        out,
        "BT {} {} {} rg 1 0 0 -1 {} {} Tm",
        n(f64::from(r) / 255.0),
        n(f64::from(g) / 255.0),
        n(f64::from(b) / 255.0),
        n(x),
        n(y)
    );
    book.show(piece.font, out, &piece.text, piece.size);
    out.push_str("ET\n");
    if piece.underline || piece.line_through {
        let thickness = (piece.size / 20.0).max(0.5);
        let _ = writeln!(
            out,
            "{} {} {} RG {} w",
            n(f64::from(r) / 255.0),
            n(f64::from(g) / 255.0),
            n(f64::from(b) / 255.0),
            n(thickness)
        );
        if piece.underline {
            let uy = y + piece.size * 0.12;
            let _ = writeln!(
                out,
                "{} {} m {} {} l S",
                n(x),
                n(uy),
                n(x + piece.width),
                n(uy)
            );
        }
        if piece.line_through {
            let ly = y - piece.size * 0.3;
            let _ = writeln!(
                out,
                "{} {} m {} {} l S",
                n(x),
                n(ly),
                n(x + piece.width),
                n(ly)
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xfa::xml::parse_xml;
    use crate::xfa::XfaLimits;

    fn font(size: f64) -> FontSpec {
        FontSpec {
            typeface: "Arial".into(),
            size,
            ..FontSpec::default()
        }
    }

    #[test]
    fn wraps_words_and_uses_pdfium_line_steps() {
        let mut book = FontBook::new(None, Vec::new());
        let paragraphs = plain_paragraphs(
            "one two three four five six",
            &font(10.0),
            &ParaSpec::default(),
        );
        let block = layout_paragraphs(&mut book, &paragraphs, Some(60.0));
        assert!(block.lines >= 3, "lines {}", block.lines);
        // First line 10pt tall, later lines 12pt.
        assert!((block.height - (10.0 + 12.0 * (block.lines as f64 - 1.0))).abs() < 1e-6);
        assert!(block
            .pieces
            .iter()
            .all(|piece| piece.x + piece.width <= 60.5));
        // First baseline: ascent / (ascent - descent) em below the top.
        assert!((block.pieces[0].y - 10.0 * 0.905 / 1.117).abs() < 1e-6);
    }

    #[test]
    fn centers_and_right_aligns() {
        let mut book = FontBook::new(None, Vec::new());
        let para = ParaSpec {
            h_align: HAlign::Right,
            ..ParaSpec::default()
        };
        let block = layout_paragraphs(
            &mut book,
            &plain_paragraphs("Hi", &font(10.0), &para),
            Some(100.0),
        );
        let piece = &block.pieces[0];
        assert!((piece.x + piece.width - 100.0).abs() < 1e-6);
    }

    #[test]
    fn parses_rich_text_styles() {
        let root = parse_xml(
            br#"<body><p style="text-align:center;font-size:14pt">Big <b>bold</b> <span style="color:#ff0000">red</span></p><p>two</p></body>"#,
            &XfaLimits::default(),
        )
        .unwrap()
        .root;
        let paragraphs = rich_paragraphs(&root, &font(10.0), &ParaSpec::default());
        assert_eq!(paragraphs.len(), 2);
        assert_eq!(paragraphs[0].para.h_align, HAlign::Center);
        let texts: Vec<_> = paragraphs[0].runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(texts.concat(), "Big bold red");
        assert!(paragraphs[0]
            .runs
            .iter()
            .any(|r| r.font.bold && r.text == "bold"));
        assert!(paragraphs[0]
            .runs
            .iter()
            .any(|r| r.font.color == [255, 0, 0] && r.text == "red"));
        assert!(paragraphs[0].runs.iter().all(|r| r.font.size == 14.0));
        assert_eq!(paragraphs[1].runs[0].text, "two");
    }

    #[test]
    fn base_font_names_split_into_family_and_style() {
        assert_eq!(
            base_font_style("MyriadPro-Bold"),
            ("myriadpro".into(), true, false)
        );
        assert_eq!(
            base_font_style("ABCDEF+Arial,BoldItalic"),
            ("arial".into(), true, true)
        );
        assert_eq!(base_font_style("ArialMT"), ("arial".into(), false, false));
        assert_eq!(normalize_family("Myriad Pro"), "myriadpro");
    }
}
