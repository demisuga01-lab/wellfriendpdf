//! XFA `<barcode>` widgets (PDFium `CXFA_FFBarcode`, `fxbarcode`): Code 39,
//! Code 128 (auto, B, C), Codabar, EAN-8, EAN-13, UPC-A, QR Code, PDF417 and
//! Data Matrix symbols built from the field's value, with PDFium's module
//! placement (`CBC_OneDimWriter`, `CBC_TwoDimWriter`).
//!
//! QR Code and Data Matrix use the `qrcode` and `datamatrix` crates (MIT OR
//! Apache-2.0); the linear symbologies and PDF417 (text, byte and numeric
//! compaction, Reed-Solomon over GF(929)) are encoded here following the
//! symbology specifications and PDFium's (ZXing-derived) encoders.

use super::barcode_pdf417::CODEWORDS;
use super::form::Template;
use super::xml::XmlNode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Symbology {
    Code39,
    Code128,
    Code128B,
    Code128C,
    Codabar,
    Ean8,
    Ean13,
    UpcA,
    Qr,
    Pdf417,
    DataMatrix,
}

impl Symbology {
    /// PDFium `GetBarcodeTypeByName` (the types it renders).
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "code3Of9" => Self::Code39,
            "code128" => Self::Code128,
            "code128B" => Self::Code128B,
            "code128C" => Self::Code128C,
            "codabar" => Self::Codabar,
            "ean8" => Self::Ean8,
            "ean13" => Self::Ean13,
            "upcA" => Self::UpcA,
            "QRCode" => Self::Qr,
            "pdf417" => Self::Pdf417,
            "dataMatrix" => Self::DataMatrix,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TextLocation {
    None,
    Above,
    Below,
    AboveEmbedded,
    BelowEmbedded,
}

/// The `<barcode>` element's settings.
#[derive(Clone, Debug)]
pub(crate) struct BarcodeSpec {
    pub kind: Symbology,
    pub checksum: bool,
    pub text: TextLocation,
    pub wide_narrow: f64,
    /// Module size in whole points (PDFium truncates; 1..=10, else 1).
    pub module_width: i32,
    pub module_height: i32,
    pub ec_level: Option<i32>,
    pub start_char: Option<char>,
    pub end_char: Option<char>,
}

pub(crate) fn parse_spec<'t>(
    tpl: &Template<'t>,
    barcode: &'t XmlNode,
) -> Result<BarcodeSpec, String> {
    let name = tpl.attr(barcode, "type").unwrap_or("").trim();
    let kind = Symbology::from_name(name).ok_or_else(|| format!("xfa.render.barcode.{name}"))?;
    let module = |attr: &str| {
        tpl.measure(barcode, attr)
            .map(|v| v as i32)
            .filter(|v| (1..=10).contains(v))
            .unwrap_or(1)
    };
    let checksum = match tpl.attr(barcode, "checksum") {
        Some("none") | None => false,
        Some(_) => true,
    };
    let text = match tpl.attr(barcode, "textLocation") {
        Some("none") => TextLocation::None,
        Some("above") => TextLocation::Above,
        Some("aboveEmbedded") => TextLocation::AboveEmbedded,
        Some("belowEmbedded") => TextLocation::BelowEmbedded,
        _ => TextLocation::Below,
    };
    let wide_narrow = tpl
        .attr(barcode, "wideNarrowRatio")
        .and_then(|ratio| {
            let mut parts = ratio.split(':');
            let wide = parts.next()?.trim().parse::<f64>().ok()?;
            let narrow = parts
                .next()
                .map_or(Some(1.0), |n| n.trim().parse::<f64>().ok())?;
            (narrow > 0.0).then(|| wide / narrow)
        })
        .filter(|ratio| (2.0..=3.0).contains(ratio))
        .unwrap_or(3.0);
    let char_attr = |attr: &str| tpl.attr(barcode, attr).and_then(|v| v.chars().next());
    Ok(BarcodeSpec {
        kind,
        checksum,
        text,
        wide_narrow,
        module_width: module("moduleWidth"),
        module_height: module("moduleHeight"),
        ec_level: tpl
            .attr(barcode, "errorCorrectionLevel")
            .and_then(|v| v.trim().parse::<i32>().ok()),
        start_char: char_attr("startChar"),
        end_char: char_attr("endChar"),
    })
}

/// An encoded symbol.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Symbol {
    /// Bars and spaces (dark, width in modules), plus the human-readable
    /// text.
    Linear {
        runs: Vec<(bool, f64)>,
        text: String,
    },
    /// A module matrix (row-major, `true` = dark).
    Matrix {
        width: usize,
        height: usize,
        cells: Vec<bool>,
    },
}

pub(crate) fn encode(spec: &BarcodeSpec, value: &str) -> Option<Symbol> {
    if value.is_empty() || value.len() > 4096 {
        return None;
    }
    match spec.kind {
        Symbology::Code39 => code39(value, spec),
        Symbology::Code128 => code128(value, None),
        Symbology::Code128B => code128(value, Some(CodeSet::B)),
        Symbology::Code128C => code128(value, Some(CodeSet::C)),
        Symbology::Codabar => codabar(value, spec),
        Symbology::Ean8 => ean(value, 8),
        Symbology::Ean13 => ean(value, 13),
        Symbology::UpcA => ean(value, 12),
        Symbology::Qr => qr(value, spec.ec_level.unwrap_or(1)),
        Symbology::Pdf417 => pdf417(value, spec.ec_level.unwrap_or(1), None),
        Symbology::DataMatrix => data_matrix(value),
    }
}

fn runs_from_modules(modules: &[u8]) -> Vec<(bool, f64)> {
    let mut runs: Vec<(bool, f64)> = Vec::new();
    for &module in modules {
        let dark = module != 0;
        match runs.last_mut() {
            Some((last, width)) if *last == dark => *width += 1.0,
            _ => runs.push((dark, 1.0)),
        }
    }
    runs
}

// ------------------------------------------------------------------ Code 39

const CODE39_CHARS: &str = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ-. $/+%";
/// Narrow/wide patterns (bar, space, ... ; 1 = wide) by `CODE39_CHARS`
/// index, then `*`.
const CODE39_PATTERNS: [u16; 44] = [
    0x034, 0x121, 0x061, 0x160, 0x031, 0x130, 0x070, 0x025, 0x124, 0x064, 0x109, 0x049, 0x148,
    0x019, 0x118, 0x058, 0x00d, 0x10c, 0x04c, 0x01c, 0x103, 0x043, 0x142, 0x013, 0x112, 0x052,
    0x007, 0x106, 0x046, 0x016, 0x181, 0x0c1, 0x1c0, 0x091, 0x190, 0x0d0, 0x085, 0x184, 0x0c4,
    0x0a8, 0x0a2, 0x08a, 0x02a, 0x094,
];

fn code39(value: &str, spec: &BarcodeSpec) -> Option<Symbol> {
    let mut data: String = value
        .to_ascii_uppercase()
        .chars()
        .filter(|c| CODE39_CHARS.contains(*c))
        .collect();
    if data.is_empty() {
        return None;
    }
    if spec.checksum {
        let sum: usize = data
            .chars()
            .map(|c| CODE39_CHARS.find(c).unwrap_or(0))
            .sum();
        data.push(CODE39_CHARS.as_bytes()[sum % 43] as char);
    }
    let wide = spec.wide_narrow;
    let mut runs = Vec::new();
    let symbols = std::iter::once(43usize)
        .chain(data.chars().map(|c| CODE39_CHARS.find(c).unwrap_or(0)))
        .chain(std::iter::once(43usize));
    for (index, symbol) in symbols.enumerate() {
        if index > 0 {
            runs.push((false, 1.0));
        }
        let pattern = CODE39_PATTERNS[symbol];
        for element in 0..9 {
            let is_wide = pattern & (1 << (8 - element)) != 0;
            runs.push((element % 2 == 0, if is_wide { wide } else { 1.0 }));
        }
    }
    Some(Symbol::Linear {
        runs,
        text: format!("*{data}*"),
    })
}

// ----------------------------------------------------------------- Code 128

const CODE128: [&str; 107] = [
    "212222", "222122", "222221", "121223", "121322", "131222", "122213", "122312", "132212",
    "221213", "221312", "231212", "112232", "122132", "122231", "113222", "123122", "123221",
    "223211", "221132", "221231", "213212", "223112", "312131", "311222", "321122", "321221",
    "312212", "322112", "322211", "212123", "212321", "232121", "111323", "131123", "131321",
    "112313", "132113", "132311", "211313", "231113", "231311", "112133", "112331", "132131",
    "113123", "113321", "133121", "313121", "211331", "231131", "213113", "213311", "213131",
    "311123", "311321", "331121", "312113", "312311", "332111", "314111", "221411", "431111",
    "111224", "111422", "121124", "121421", "141122", "141221", "112214", "112412", "122114",
    "122411", "142112", "142211", "241211", "221114", "413111", "241112", "134111", "111242",
    "121142", "121241", "114212", "124112", "124211", "411212", "421112", "421211", "212141",
    "214121", "412121", "111143", "111341", "131141", "114113", "114311", "411113", "411311",
    "113141", "114131", "311141", "411131", "211412", "211214", "211232", "2331112",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum CodeSet {
    B,
    C,
}

fn code128(value: &str, set: Option<CodeSet>) -> Option<Symbol> {
    let digits_only = value.bytes().all(|b| b.is_ascii_digit());
    let set = set.unwrap_or(
        if digits_only && value.len() >= 4 && value.len().is_multiple_of(2) {
            CodeSet::C
        } else {
            CodeSet::B
        },
    );
    let mut codes = Vec::new();
    let text = match set {
        CodeSet::C => {
            let mut digits: String = value.chars().filter(char::is_ascii_digit).collect();
            if digits.is_empty() {
                return None;
            }
            if digits.len() % 2 == 1 {
                digits.insert(0, '0');
            }
            codes.push(105);
            for pair in digits.as_bytes().chunks(2) {
                codes.push(usize::from((pair[0] - b'0') * 10 + (pair[1] - b'0')));
            }
            digits
        }
        CodeSet::B => {
            let data: String = value
                .chars()
                .filter(|c| (' '..='\u{7f}').contains(c))
                .collect();
            if data.is_empty() {
                return None;
            }
            codes.push(104);
            codes.extend(data.bytes().map(|b| usize::from(b - 32)));
            data
        }
    };
    let checksum = codes
        .iter()
        .enumerate()
        .map(|(i, &c)| if i == 0 { c } else { c * i })
        .sum::<usize>()
        % 103;
    codes.push(checksum);
    codes.push(106);
    let mut runs = Vec::new();
    for code in codes {
        for (index, width) in CODE128[code].bytes().enumerate() {
            runs.push((index % 2 == 0, f64::from(width - b'0')));
        }
    }
    Some(Symbol::Linear { runs, text })
}

// ------------------------------------------------------------------ Codabar

const CODABAR_CHARS: &str = "0123456789-$:/.+ABCD";
/// 7 elements (bar first; 1 = wide).
const CODABAR_PATTERNS: [u8; 20] = [
    0x03, 0x06, 0x09, 0x60, 0x12, 0x42, 0x21, 0x24, 0x30, 0x48, 0x0c, 0x18, 0x45, 0x51, 0x54, 0x15,
    0x1a, 0x29, 0x0b, 0x0e,
];

fn codabar(value: &str, spec: &BarcodeSpec) -> Option<Symbol> {
    let upper = value.to_ascii_uppercase();
    let mut data: String = upper
        .chars()
        .filter(|c| CODABAR_CHARS[..16].contains(*c))
        .collect();
    if data.is_empty() {
        return None;
    }
    let guard = |c: Option<char>, default: char| {
        c.map(|c| c.to_ascii_uppercase())
            .filter(|c| "ABCD".contains(*c))
            .unwrap_or(default)
    };
    data.insert(0, guard(spec.start_char, 'A'));
    data.push(guard(spec.end_char, 'B'));
    let mut runs = Vec::new();
    for (index, ch) in data.chars().enumerate() {
        if index > 0 {
            runs.push((false, 1.0));
        }
        let pattern = CODABAR_PATTERNS[CODABAR_CHARS.find(ch)?];
        for element in 0..7 {
            let is_wide = pattern & (1 << (6 - element)) != 0;
            runs.push((
                element % 2 == 0,
                if is_wide { spec.wide_narrow } else { 1.0 },
            ));
        }
    }
    Some(Symbol::Linear { runs, text: data })
}

// --------------------------------------------------------- EAN / UPC

const EAN_L: [&str; 10] = [
    "0001101", "0011001", "0010011", "0111101", "0100011", "0110001", "0101111", "0111011",
    "0110111", "0001011",
];
const EAN_FIRST: [&str; 10] = [
    "LLLLLL", "LLGLGG", "LLGGLG", "LLGGGL", "LGLLGG", "LGGLLG", "LGGGLL", "LGLGLG", "LGLGGL",
    "LGGLGL",
];

fn ean_check(digits: &[u8]) -> u8 {
    // Weight 3 on the digit next to the check digit, alternating.
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &d)| u32::from(d) * if i % 2 == 0 { 3 } else { 1 })
        .sum();
    ((10 - sum % 10) % 10) as u8
}

/// EAN-8 (`len` 8), EAN-13 (13) and UPC-A (12).
fn ean(value: &str, len: usize) -> Option<Symbol> {
    let mut digits: Vec<u8> = value
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| b - b'0')
        .collect();
    if digits.is_empty() {
        return None;
    }
    digits.truncate(len - 1);
    while digits.len() < len - 1 {
        digits.insert(0, 0);
    }
    digits.push(ean_check(&digits));
    let text: String = digits.iter().map(|d| char::from(b'0' + d)).collect();
    let code = |d: u8, set: char| -> String {
        let l = EAN_L[usize::from(d)];
        match set {
            'L' => l.to_string(),
            // R: complement of L; G: R reversed.
            'R' => l
                .chars()
                .map(|c| if c == '0' { '1' } else { '0' })
                .collect(),
            _ => l
                .chars()
                .rev()
                .map(|c| if c == '0' { '1' } else { '0' })
                .collect(),
        }
    };
    let mut bits = String::from("101");
    match len {
        8 => {
            for &d in &digits[..4] {
                bits.push_str(&code(d, 'L'));
            }
            bits.push_str("01010");
            for &d in &digits[4..] {
                bits.push_str(&code(d, 'R'));
            }
        }
        12 => {
            for &d in &digits[..6] {
                bits.push_str(&code(d, 'L'));
            }
            bits.push_str("01010");
            for &d in &digits[6..] {
                bits.push_str(&code(d, 'R'));
            }
        }
        _ => {
            let parity = EAN_FIRST[usize::from(digits[0])];
            for (&d, set) in digits[1..7].iter().zip(parity.chars()) {
                bits.push_str(&code(d, set));
            }
            bits.push_str("01010");
            for &d in &digits[7..] {
                bits.push_str(&code(d, 'R'));
            }
        }
    }
    bits.push_str("101");
    let modules: Vec<u8> = bits.bytes().map(|b| b - b'0').collect();
    Some(Symbol::Linear {
        runs: runs_from_modules(&modules),
        text,
    })
}

/// Where PDFium draws the human-readable digits of EAN-8, EAN-13 and UPC-A
/// symbols (`CBC_OnedEAN13Writer::ShowChars`, `CBC_OnedEAN8Writer`,
/// `CBC_OnedUPCAWriter`): in module units of the quiet-zone-padded
/// symbol, the digit groups sit in white boxes below the bars, between the
/// guard bars, which run down to the bottom edge.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EanText {
    /// Modules across the widget: the symbol plus PDFium's 7-module quiet
    /// zones (left for EAN-13; both sides for UPC-A).
    pub total_modules: f64,
    /// Modules before the first bar.
    pub left_pad: f64,
    /// Digit groups: (digit range of the text, module x, module width).
    /// The digits are centred in the width; the white box is half a module
    /// narrower.
    pub groups: Vec<(std::ops::Range<usize>, f64, f64)>,
}

pub(crate) fn ean_text_layout(kind: Symbology) -> Option<EanText> {
    Some(match kind {
        // 95 modules + left padding; the leading digit (encoded in the
        // parity pattern) is drawn in the quiet zone.
        Symbology::Ean13 => EanText {
            total_modules: 102.0,
            left_pad: 7.0,
            groups: vec![(0..1, 0.0, 7.0), (1..7, 10.0, 42.0), (7..13, 57.0, 42.0)],
        },
        Symbology::Ean8 => EanText {
            total_modules: 67.0,
            left_pad: 0.0,
            groups: vec![(0..4, 3.0, 28.0), (4..8, 36.0, 28.0)],
        },
        // 95 modules + padding on both sides; the number-system and check
        // digits are drawn outside, beside the guard-height bars of their
        // symbol characters.
        Symbology::UpcA => EanText {
            total_modules: 109.0,
            left_pad: 7.0,
            groups: vec![
                (0..1, 0.0, 7.0),
                (1..6, 17.0, 35.0),
                (6..11, 57.0, 35.0),
                (11..12, 102.0, 7.0),
            ],
        },
        _ => return None,
    })
}

// ----------------------------------------------------------------- QR Code

fn qr(value: &str, level: i32) -> Option<Symbol> {
    use qrcode::{Color, EcLevel, QrCode};
    let level = match level {
        0 => EcLevel::L,
        2 => EcLevel::Q,
        3 => EcLevel::H,
        _ => EcLevel::M,
    };
    let code = QrCode::with_error_correction_level(value.as_bytes(), level).ok()?;
    let width = code.width();
    let cells = code
        .to_colors()
        .into_iter()
        .map(|color| color == Color::Dark)
        .collect();
    Some(Symbol::Matrix {
        width,
        height: width,
        cells,
    })
}

// ------------------------------------------------------------- Data Matrix

fn data_matrix(value: &str) -> Option<Symbol> {
    use datamatrix::{DataMatrix, SymbolList};
    let code = DataMatrix::encode(value.as_bytes(), SymbolList::default()).ok()?;
    let bitmap = code.bitmap();
    let (width, height) = (bitmap.width(), bitmap.height());
    let mut cells = vec![false; width * height];
    for (x, y) in bitmap.pixels() {
        if x < width && y < height {
            cells[y * width + x] = true;
        }
    }
    Some(Symbol::Matrix {
        width,
        height,
        cells,
    })
}

// ------------------------------------------------------------------ PDF417

const PDF417_START: u32 = 0x1fea8;
const PDF417_STOP: u32 = 0x3fa29;
const LATCH_TEXT: u16 = 900;
const LATCH_BYTE_PADDED: u16 = 901;
const LATCH_NUMERIC: u16 = 902;
const SHIFT_BYTE: u16 = 913;
const LATCH_BYTE: u16 = 924;

const MIXED: [i8; 128] = [
    -1, -1, -1, -1, -1, -1, -1, -1, -1, 12, -1, -1, -1, 11, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
    -1, -1, -1, -1, -1, -1, -1, -1, 26, -1, -1, 15, 18, 21, 10, -1, -1, -1, 22, 20, 13, 16, 17, 19,
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 14, -1, -1, 23, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
    -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, 24, -1, -1, -1,
    -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
    -1, -1, -1, -1, -1, -1,
];
const PUNCTUATION: [i8; 128] = [
    -1, -1, -1, -1, -1, -1, -1, -1, -1, 12, 15, -1, -1, 11, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
    -1, -1, -1, -1, -1, -1, -1, -1, -1, 10, 20, -1, 18, -1, -1, 28, 23, 24, 22, -1, 13, 16, 17, 19,
    -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, 14, 0, 1, -1, 2, 25, 3, -1, -1, -1, -1, -1, -1, -1, -1,
    -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, 4, 5, 6, -1, 7, 8, -1,
    -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
    -1, 26, 21, 27, 9, -1,
];

fn is_mixed(b: u8) -> bool {
    b < 128 && MIXED[usize::from(b)] != -1
}

fn is_punct(b: u8) -> bool {
    b < 128 && PUNCTUATION[usize::from(b)] != -1
}

fn is_text(b: u8) -> bool {
    (32..=126).contains(&b) || matches!(b, b'\t' | b'\n' | b'\r')
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SubMode {
    Alpha,
    Lower,
    Mixed,
    Punct,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Unknown,
    Text,
    Byte,
    Numeric,
}

/// PDFium `CBC_PDF417HighLevelEncoder::EncodeHighLevel`.
fn pdf417_high_level(bytes: &[u8]) -> Vec<u16> {
    let len = bytes.len();
    let mut out = Vec::new();
    let mut p = 0;
    let mut submode = SubMode::Alpha;
    let mut mode = Mode::Unknown;
    while p < len {
        let n = digit_count(bytes, p);
        if n >= 13 {
            out.push(LATCH_NUMERIC);
            mode = Mode::Numeric;
            submode = SubMode::Alpha;
            encode_numeric(&bytes[p..p + n], &mut out);
            p += n;
            continue;
        }
        let t = text_count(bytes, p);
        if t >= 5 || n == len {
            if mode != Mode::Text {
                out.push(LATCH_TEXT);
                mode = Mode::Text;
                submode = SubMode::Alpha;
            }
            submode = encode_text(&bytes[p..p + t], submode, &mut out);
            p += t;
            continue;
        }
        let b = binary_count(bytes, p).max(1);
        if b == 1 && mode == Mode::Text {
            encode_binary(&bytes[p..p + 1], Mode::Text, &mut out);
        } else {
            encode_binary(&bytes[p..p + b], mode, &mut out);
            mode = Mode::Byte;
            submode = SubMode::Alpha;
        }
        p += b;
    }
    let _ = submode;
    out
}

fn digit_count(bytes: &[u8], start: usize) -> usize {
    bytes[start..]
        .iter()
        .take_while(|b| b.is_ascii_digit())
        .count()
}

fn text_count(bytes: &[u8], start: usize) -> usize {
    let mut idx = start;
    while idx < bytes.len() {
        let digits = digit_count(bytes, idx).min(13);
        if digits >= 13 {
            return idx - start;
        }
        if digits > 0 {
            idx += digits;
            continue;
        }
        if !is_text(bytes[idx]) {
            break;
        }
        idx += 1;
    }
    idx - start
}

fn binary_count(bytes: &[u8], start: usize) -> usize {
    let mut idx = start;
    while idx < bytes.len() {
        if digit_count(bytes, idx) >= 13 {
            return idx - start;
        }
        let texts = bytes[idx..]
            .iter()
            .take(5)
            .take_while(|&&b| is_text(b))
            .count();
        if texts >= 5 {
            return idx - start;
        }
        idx += 1;
    }
    idx - start
}

fn encode_text(chars: &[u8], initial: SubMode, out: &mut Vec<u16>) -> SubMode {
    let mut tmp: Vec<u16> = Vec::new();
    let mut submode = initial;
    let mut idx = 0;
    while idx < chars.len() {
        let ch = chars[idx];
        let upper = ch == b' ' || ch.is_ascii_uppercase();
        let lower = ch == b' ' || ch.is_ascii_lowercase();
        match submode {
            SubMode::Alpha => {
                if upper {
                    tmp.push(if ch == b' ' { 26 } else { u16::from(ch - b'A') });
                } else if lower {
                    submode = SubMode::Lower;
                    tmp.push(27);
                    continue;
                } else if is_mixed(ch) {
                    submode = SubMode::Mixed;
                    tmp.push(28);
                    continue;
                } else if is_punct(ch) {
                    tmp.push(29);
                    tmp.push(PUNCTUATION[usize::from(ch)] as u16);
                }
            }
            SubMode::Lower => {
                if lower {
                    tmp.push(if ch == b' ' { 26 } else { u16::from(ch - b'a') });
                } else if upper {
                    tmp.push(27);
                    tmp.push(u16::from(ch - b'A'));
                } else if is_mixed(ch) {
                    submode = SubMode::Mixed;
                    tmp.push(28);
                    continue;
                } else if is_punct(ch) {
                    tmp.push(29);
                    tmp.push(PUNCTUATION[usize::from(ch)] as u16);
                }
            }
            SubMode::Mixed => {
                if is_mixed(ch) {
                    tmp.push(MIXED[usize::from(ch)] as u16);
                } else if upper {
                    submode = SubMode::Alpha;
                    tmp.push(28);
                    continue;
                } else if lower {
                    submode = SubMode::Lower;
                    tmp.push(27);
                    continue;
                } else {
                    if idx + 1 < chars.len() && is_punct(chars[idx + 1]) {
                        submode = SubMode::Punct;
                        tmp.push(25);
                        continue;
                    }
                    if is_punct(ch) {
                        tmp.push(29);
                        tmp.push(PUNCTUATION[usize::from(ch)] as u16);
                    }
                }
            }
            SubMode::Punct => {
                if is_punct(ch) {
                    tmp.push(PUNCTUATION[usize::from(ch)] as u16);
                } else {
                    submode = SubMode::Alpha;
                    tmp.push(29);
                    continue;
                }
            }
        }
        idx += 1;
    }
    let mut h = 0u16;
    for (i, &v) in tmp.iter().enumerate() {
        if i % 2 == 1 {
            out.push(h * 30 + v);
        } else {
            h = v;
        }
    }
    if tmp.len() % 2 == 1 {
        out.push(h * 30 + 29);
    }
    submode
}

fn encode_binary(bytes: &[u8], mode: Mode, out: &mut Vec<u16>) {
    if bytes.len() == 1 && mode == Mode::Text {
        out.push(SHIFT_BYTE);
    }
    let mut idx = 0;
    if bytes.len() >= 6 {
        out.push(LATCH_BYTE);
        while bytes.len() - idx >= 6 {
            let mut t: u64 = 0;
            for &b in &bytes[idx..idx + 6] {
                t = (t << 8) + u64::from(b);
            }
            let mut chars = [0u16; 5];
            for slot in &mut chars {
                *slot = (t % 900) as u16;
                t /= 900;
            }
            out.extend(chars.iter().rev());
            idx += 6;
        }
    }
    if idx < bytes.len() {
        out.push(LATCH_BYTE_PADDED);
    }
    out.extend(bytes[idx..].iter().map(|&b| u16::from(b)));
}

/// Base-900 conversion of "1" + up to 44 digits per group.
fn encode_numeric(digits: &[u8], out: &mut Vec<u16>) {
    for group in digits.chunks(44) {
        let mut number: Vec<u8> = std::iter::once(1u8)
            .chain(group.iter().map(|d| d - b'0'))
            .collect();
        let mut tmp = Vec::new();
        loop {
            // number /= 900, remainder pushed.
            let mut remainder = 0u32;
            let mut quotient = Vec::with_capacity(number.len());
            for &d in &number {
                let current = remainder * 10 + u32::from(d);
                let q = current / 900;
                remainder = current % 900;
                if !quotient.is_empty() || q != 0 {
                    quotient.push(q as u8);
                }
            }
            tmp.push(remainder as u16);
            if quotient.is_empty() {
                break;
            }
            number = quotient;
        }
        out.extend(tmp.iter().rev());
    }
}

/// Reed-Solomon error correction codewords over GF(929) (generator
/// roots 3^1..3^k).
fn pdf417_ec(data: &[u16], level: i32) -> Vec<u16> {
    let k = 1usize << (level + 1);
    // Generator polynomial coefficients, constant term first.
    let mut g = vec![1u32];
    let mut alpha = 1u32;
    for _ in 0..k {
        alpha = alpha * 3 % 929;
        // g *= (x - alpha)
        let mut next = vec![0u32; g.len() + 1];
        for (i, &c) in g.iter().enumerate() {
            next[i + 1] = (next[i + 1] + c) % 929;
            next[i] = (next[i] + 929 - c * alpha % 929) % 929;
        }
        g = next;
    }
    let coef = &g[..k];
    let mut ech = vec![0u32; k];
    for &d in data {
        let t1 = (u32::from(d) + ech[k - 1]) % 929;
        for j in (1..k).rev() {
            let t2 = t1 * coef[j] % 929;
            ech[j] = (ech[j - 1] + 929 - t2) % 929;
        }
        let t2 = t1 * coef[0] % 929;
        ech[0] = (929 - t2) % 929;
    }
    ech.iter()
        .rev()
        .map(|&e| if e != 0 { (929 - e) as u16 } else { 0 })
        .collect()
}

/// PDF417 symbol; `width_hint` (widget width, height in points) selects
/// columns and rows like PDFium's `CBC_PDF417Writer::Encode`.
fn pdf417(value: &str, level: i32, hint: Option<(i32, i32)>) -> Option<Symbol> {
    let level = level.clamp(0, 8);
    let k = 1i32 << (level + 1);
    let high = pdf417_high_level(value.as_bytes());
    let m = high.len() as i32;
    let (mut min_cols, mut max_cols, mut min_rows, mut max_rows) = (1, 30, 3, 90);
    if let Some((width, height)) = hint {
        let col = (width - 69) / 17;
        let row = height / 20;
        if (3..=90).contains(&row) && (1..=30).contains(&col) {
            (max_cols, min_cols, max_rows, min_rows) = (col, 1, row, 3);
        } else if (1..=30).contains(&col) {
            (max_cols, min_cols, max_rows, min_rows) = (col, col, 90, 3);
        } else if (3..=90).contains(&row) {
            (max_cols, min_cols, max_rows, min_rows) = (30, 1, row, row);
        }
    }
    let rows_for = |c: i32| {
        let mut r = (m + 1 + k) / c + 1;
        if c * r >= m + 1 + k + c {
            r -= 1;
        }
        r
    };
    let mut dims: Option<(i32, i32)> = None;
    let mut ratio = 0.0f32;
    for cols in min_cols..=max_cols {
        let rows = rows_for(cols);
        if rows < min_rows {
            break;
        }
        if rows > max_rows {
            continue;
        }
        let new_ratio = ((17 * cols + 69) as f32 * 0.357) / (rows as f32 * 2.0);
        if dims.is_some() && (new_ratio - 3.0).abs() > (ratio - 3.0).abs() {
            continue;
        }
        ratio = new_ratio;
        dims = Some((cols, rows));
    }
    if dims.is_none() {
        let rows = rows_for(max_cols);
        if rows < min_rows {
            dims = Some((max_cols, min_rows));
        } else if (3..=90).contains(&rows) {
            dims = Some((max_cols, rows));
        }
    }
    let (cols, rows) = dims?;
    let pad = {
        let n = cols * rows - k;
        if n > m + 1 {
            n - m - 1
        } else {
            0
        }
    };
    if m + k + 1 > 929 || m + pad + 1 >= 929 {
        return None;
    }
    let mut data = Vec::with_capacity((m + pad + 1) as usize);
    data.push((m + pad + 1) as u16);
    data.extend(&high);
    data.extend(std::iter::repeat_n(900u16, pad as usize));
    let ec = pdf417_ec(&data, level);
    let full: Vec<u16> = data.into_iter().chain(ec).collect();
    let width = ((cols + 4) * 17 + 1) as usize;
    let mut cells = Vec::with_capacity(width * rows as usize);
    let push = |cells: &mut Vec<bool>, pattern: u32, len: u32| {
        for bit in (0..len).rev() {
            cells.push(pattern & (1 << bit) != 0);
        }
    };
    let mut idx = 0usize;
    for y in 0..rows {
        let cluster = (y % 3) as usize;
        let base = 30 * (y / 3);
        let (left, right) = match cluster {
            0 => (base + (rows - 1) / 3, base + (cols - 1)),
            1 => (base + level * 3 + (rows - 1) % 3, base + (rows - 1) / 3),
            _ => (base + (cols - 1), base + level * 3 + (rows - 1) % 3),
        };
        let word =
            |value: i32| 0x10000 | u32::from(CODEWORDS[cluster][value.clamp(0, 928) as usize]);
        push(&mut cells, PDF417_START, 17);
        push(&mut cells, word(left), 17);
        for _ in 0..cols {
            let codeword = full.get(idx).copied().unwrap_or(900);
            push(&mut cells, word(i32::from(codeword)), 17);
            idx += 1;
        }
        push(&mut cells, word(right), 17);
        push(&mut cells, PDF417_STOP, 18);
    }
    Some(Symbol::Matrix {
        width,
        height: rows as usize,
        cells,
    })
}

/// [`encode`] with the widget size (PDF417 column selection).
pub(crate) fn encode_for(
    spec: &BarcodeSpec,
    value: &str,
    width: f64,
    height: f64,
) -> Option<Symbol> {
    if spec.kind == Symbology::Pdf417 && !value.is_empty() && value.len() <= 4096 {
        let module = spec.module_width.max(1);
        return pdf417(
            value,
            spec.ec_level.unwrap_or(1),
            Some(((width as i32) / module, (height as i32) / module)),
        )
        .or_else(|| encode(spec, value));
    }
    encode(spec, value)
}

/// PDFium `CBC_TwoDimWriter::RenderResult`: module rectangles (x, y, w, h)
/// for a matrix symbol in a `width` x `height` widget, or `None` when a
/// fixed-size symbol (QR, Data Matrix) does not fit.
pub(crate) fn matrix_modules(
    spec: &BarcodeSpec,
    symbol_width: usize,
    symbol_height: usize,
    width: f64,
    height: f64,
) -> Option<MatrixLayout> {
    // PDF417 matrices are rotated to landscape.
    let fixed = spec.kind != Symbology::Pdf417;
    let (w, h) = (width as i32, height as i32);
    let temp_w = symbol_width as i32 + 2;
    let temp_h = symbol_height as i32 + 2;
    let module = spec.module_width.min(spec.module_height).clamp(1, 8);
    let mut out_w = temp_w.checked_mul(module)?;
    let mut out_h = temp_h.checked_mul(module)?;
    if fixed {
        if w < out_w || h < out_h {
            return None;
        }
    } else if w > out_w || h > out_h {
        let wf = (f64::from(w) / f64::from(out_w)).floor().max(1.0) as i32;
        let hf = (f64::from(h) / f64::from(out_h)).floor().max(1.0) as i32;
        out_w *= wf;
        out_h *= hf;
    }
    let mut multi_x = (f64::from(out_w) / f64::from(temp_w)).ceil() as i32;
    let mut multi_y = (f64::from(out_h) / f64::from(temp_h)).ceil() as i32;
    if fixed {
        multi_x = multi_x.min(multi_y);
        multi_y = multi_x;
    }
    let left = ((w - out_w) / 2).max(0);
    let top = ((h - out_h) / 2).max(0);
    // Scaled down to the widget when the symbol is larger.
    let (sx, sy) = if w < out_w && h < out_h {
        (
            f64::from(w) / f64::from(out_w),
            f64::from(h) / f64::from(out_h),
        )
    } else {
        (1.0, 1.0)
    };
    Some(MatrixLayout {
        left: f64::from(left),
        top: f64::from(top),
        module_w: f64::from(multi_x),
        module_h: f64::from(multi_y),
        scale_x: sx,
        scale_y: sy,
    })
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct MatrixLayout {
    pub left: f64,
    pub top: f64,
    pub module_w: f64,
    pub module_h: f64,
    pub scale_x: f64,
    pub scale_y: f64,
}

/// Rotates a portrait PDF417 matrix to landscape (PDFium `RotateArray`).
pub(crate) fn landscape(
    width: usize,
    height: usize,
    cells: Vec<bool>,
) -> (usize, usize, Vec<bool>) {
    if width >= height {
        return (width, height, cells);
    }
    let mut out = vec![false; width * height];
    for y in 0..height {
        for x in 0..width {
            // New matrix is height wide, width tall.
            let nx = height - 1 - y;
            let ny = x;
            out[ny * height + nx] = cells[y * width + x];
        }
    }
    (height, width, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(kind: Symbology) -> BarcodeSpec {
        BarcodeSpec {
            kind,
            checksum: false,
            text: TextLocation::Below,
            wide_narrow: 3.0,
            module_width: 1,
            module_height: 1,
            ec_level: None,
            start_char: None,
            end_char: None,
        }
    }

    fn modules(symbol: &Symbol) -> String {
        let Symbol::Linear { runs, .. } = symbol else {
            panic!()
        };
        runs.iter()
            .map(|(dark, width)| {
                std::iter::repeat_n(if *dark { '1' } else { '0' }, *width as usize)
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn ean13_matches_the_reference_pattern() {
        let symbol = ean("400638133393", 13).unwrap();
        let Symbol::Linear { text, .. } = &symbol else {
            panic!()
        };
        assert_eq!(text, "4006381333931");
        let bits = modules(&symbol);
        assert_eq!(bits.len(), 95);
        assert!(bits.starts_with("101"));
        assert!(bits.ends_with("101"));
        assert_eq!(&bits[45..50], "01010");
        // UPC-A is EAN-13 with a leading 0 and the same check digit rule.
        let upc = ean("03600029145", 12).unwrap();
        let Symbol::Linear { text, .. } = &upc else {
            panic!()
        };
        assert_eq!(text, "036000291452");
        let ean8 = ean("9638507", 8).unwrap();
        let Symbol::Linear { text, .. } = &ean8 else {
            panic!()
        };
        assert_eq!(text, "96385074");
        assert_eq!(modules(&ean8).len(), 67);
        // PDFium's text layout: the groups cover the digits, in order and
        // within the padded symbol, clear of the guard bars.
        for (kind, digits, bits) in [
            (Symbology::Ean13, 13, 95.0),
            (Symbology::Ean8, 8, 67.0),
            (Symbology::UpcA, 12, 95.0),
        ] {
            let layout = ean_text_layout(kind).unwrap();
            let covered: Vec<usize> = layout.groups.iter().flat_map(|g| g.0.clone()).collect();
            assert_eq!(covered, (0..digits).collect::<Vec<_>>());
            assert!(layout.left_pad + bits <= layout.total_modules);
            for pair in layout.groups.windows(2) {
                assert!(pair[0].1 + pair[0].2 <= pair[1].1 + 0.01);
            }
        }
    }

    #[test]
    fn code128_and_code39_checksums() {
        // "PJJ123C" in code set B: checksum 'B'... compare against the spec
        // example module count: (start + 7 data + check) * 11 + 13.
        let symbol = code128("PJJ123C", Some(CodeSet::B)).unwrap();
        assert_eq!(modules(&symbol).len(), (1 + 7 + 1) * 11 + 13);
        let symbol = code128("123456", None).unwrap();
        assert_eq!(modules(&symbol).len(), (1 + 3 + 1) * 11 + 13);
        let mut s = spec(Symbology::Code39);
        s.checksum = true;
        let symbol = code39("code39", &s).unwrap();
        let Symbol::Linear { text, runs } = &symbol else {
            panic!()
        };
        // C O D E 3 9 = 12+24+13+14+3+9 = 75 -> 75 % 43 = 32 -> 'W'.
        assert_eq!(text, "*CODE39W*");
        assert_eq!(runs.len(), 9 * 9 + 8);
    }

    #[test]
    fn pdf417_error_correction_and_layout() {
        // Level 0 generator: x^2 + 917x + 27.
        let ec = pdf417_ec(&[5, 453, 178, 121, 239], 0);
        assert_eq!(ec.len(), 2);
        let symbol = pdf417("PDF417 test 0123456789012345", 2, None).unwrap();
        let Symbol::Matrix {
            width,
            height,
            cells,
        } = &symbol
        else {
            panic!()
        };
        assert_eq!((width - 69) % 17, 0);
        assert!(*height >= 3);
        assert_eq!(cells.len(), width * height);
        // Every row starts with the start pattern 11111111010101000.
        for row in 0..*height {
            let start: String = cells[row * width..row * width + 17]
                .iter()
                .map(|&c| if c { '1' } else { '0' })
                .collect();
            assert_eq!(start, "11111111010101000");
        }
        let mut out = Vec::new();
        encode_numeric(b"000213298174000", &mut out);
        assert_eq!(out, vec![1, 624, 434, 632, 282, 200]);
    }

    #[test]
    fn two_dimensional_symbols() {
        let Some(Symbol::Matrix { width, height, .. }) = qr("https://example.com", 1) else {
            panic!()
        };
        assert_eq!((width, height), (25, 25));
        let Some(Symbol::Matrix { width, height, .. }) = data_matrix("Wellfriend") else {
            panic!()
        };
        assert_eq!(width, height);
        assert!(width >= 10);
        let layout = matrix_modules(&spec(Symbology::Qr), 25, 25, 100.0, 60.0).unwrap();
        assert_eq!(layout.module_w, 1.0);
        assert!(matrix_modules(&spec(Symbology::Qr), 25, 25, 20.0, 20.0).is_none());
        let layout = matrix_modules(&spec(Symbology::Pdf417), 120, 6, 300.0, 64.0).unwrap();
        assert_eq!((layout.module_w, layout.module_h), (2.0, 8.0));
    }
}
