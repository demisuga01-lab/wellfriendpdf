use crate::content::operation::Operand;
use crate::error::Result;
use crate::filters::{decode_stream_from_dict, decode_stream_lossless};
use crate::fonts::cmap::ToUnicodeCMap;
use crate::fonts::encoding::Encoding;
use crate::fonts::glyph_list::glyph_name_to_unicode;
use crate::fonts::predefined_cmap::{self, PredefinedCMapInfo};
use crate::fonts::type1;
use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;

#[derive(Debug, Clone, PartialEq)]
pub enum FontSubtype {
    Type0,
    Type1,
    TrueType,
    Type3,
    CIDFontType0,
    CIDFontType2,
    Unknown,
}

pub fn detect_font_subtype(font_dict: &PdfDictionary) -> FontSubtype {
    match font_dict.get_name("Subtype") {
        Some("Type0") => FontSubtype::Type0,
        Some("Type1") => FontSubtype::Type1,
        Some("TrueType") => FontSubtype::TrueType,
        Some("Type3") => FontSubtype::Type3,
        Some("CIDFontType0") => FontSubtype::CIDFontType0,
        Some("CIDFontType2") => FontSubtype::CIDFontType2,
        _ => FontSubtype::Unknown,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum FontType {
    Type1,
    MMType1,
    TrueType,
    Type3,
    Type0,
    CIDFontType0,
    CIDFontType2,
    Unknown(String),
}

impl FontType {
    pub fn from_name(s: &str) -> Self {
        match s {
            "Type1" => FontType::Type1,
            "MMType1" => FontType::MMType1,
            "TrueType" => FontType::TrueType,
            "Type3" => FontType::Type3,
            "Type0" => FontType::Type0,
            "CIDFontType0" => FontType::CIDFontType0,
            "CIDFontType2" => FontType::CIDFontType2,
            other => FontType::Unknown(other.to_string()),
        }
    }

    pub fn is_cid(&self) -> bool {
        matches!(
            self,
            FontType::Type0 | FontType::CIDFontType0 | FontType::CIDFontType2
        )
    }
}

pub struct FontResolver {
    font_type: FontType,
    to_unicode: Option<ToUnicodeCMap>,
    encoding_table: Option<Vec<String>>,
    widths: Vec<f64>,
    first_char: u32,
    last_char: u32,
    descendant_font: Option<PdfDictionary>,
    default_width: f64,
    code_size: u8,
    predefined_cmap: Option<PredefinedCMapInfo>,
    standard14_base: Option<String>,
    /// Writing mode of the font's encoding CMap: 0 = horizontal (glyphs advance
    /// left-to-right), 1 = vertical (glyphs advance top-to-bottom, columns
    /// arranged right-to-left). Only Type0 (composite) fonts can be vertical;
    /// every simple font is horizontal. Derived from the `/Encoding` CMap's
    /// `/WMode` entry, or from a predefined CMap name's `-V`/`-H` suffix
    /// (`Identity-V` ⇒ vertical). See PDF 32000-1 §9.7.4.3.
    wmode: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontDecodeSource {
    ActualText,
    ToUnicode,
    PredefinedCMap,
    EncodingDifferences,
    GlyphName,
    FontCMap,
    IdentityCid,
    NativePdfText,
    Unknown,
}

impl FontResolver {
    pub fn new(font_dict: &PdfDictionary, reader: &PdfReader) -> Self {
        Self::build(font_dict, Some(reader))
    }

    pub fn new_from_dict_only(font_dict: &PdfDictionary) -> Self {
        Self::build(font_dict, None)
    }

    pub fn decode_string(&self, bytes: &[u8]) -> String {
        let mut result = String::new();
        let mut idx = 0usize;
        let code_size = self.code_size.max(1);
        while idx < bytes.len() {
            let code = if code_size == 2 {
                let high = bytes[idx];
                let low = bytes.get(idx + 1).copied().unwrap_or(0);
                idx += 2;
                (u16::from(high) << 8) | u16::from(low)
            } else {
                let code = u16::from(bytes[idx]);
                idx += 1;
                code
            };

            if let Some(text) = self.to_unicode.as_ref().and_then(|cmap| cmap.lookup(code)) {
                result.push_str(text);
                continue;
            }

            let glyph_name = self
                .encoding_table
                .as_ref()
                .and_then(|table| table.get(code as usize))
                .map(String::as_str)
                .unwrap_or(".notdef");

            if glyph_name != ".notdef" {
                if let Some(ch) = glyph_name_to_unicode(glyph_name) {
                    result.push_str(&expand_ligature(ch));
                    continue;
                }
            }

            if let Some(ch) = char::from_u32(u32::from(code)) {
                if !ch.is_control() || ch.is_whitespace() {
                    result.push(ch);
                    continue;
                }
            }
            if let Some(text) = self
                .predefined_cmap
                .as_ref()
                .and_then(|info| predefined_cmap::unicode_for_code(info.name, code))
            {
                result.push_str(&text);
                continue;
            }
            log::warn!("font decode produced replacement character for code {code:#06X}");
            result.push('\u{FFFD}');
        }
        result
    }

    pub fn decode_char(&self, code: u16) -> String {
        self.decode_char_with_source(code).0
    }

    pub fn decode_char_with_source(&self, code: u16) -> (String, FontDecodeSource) {
        let bytes = if self.code_size == 2 {
            vec![(code >> 8) as u8, (code & 0xFF) as u8]
        } else {
            vec![code as u8]
        };
        self.decode_code_bytes_with_source(code, &bytes)
    }

    fn decode_code_bytes_with_source(
        &self,
        code: u16,
        _bytes: &[u8],
    ) -> (String, FontDecodeSource) {
        if let Some(text) = self.to_unicode.as_ref().and_then(|cmap| cmap.lookup(code)) {
            return (text.to_string(), FontDecodeSource::ToUnicode);
        }

        let glyph_name = self
            .encoding_table
            .as_ref()
            .and_then(|table| table.get(code as usize))
            .map(String::as_str)
            .unwrap_or(".notdef");

        if glyph_name != ".notdef" {
            if let Some(ch) = glyph_name_to_unicode(glyph_name) {
                let source = if glyph_name.starts_with("uni") || glyph_name.starts_with('u') {
                    FontDecodeSource::GlyphName
                } else {
                    FontDecodeSource::EncodingDifferences
                };
                return (expand_ligature(ch), source);
            }
        }

        if let Some(ch) = char::from_u32(u32::from(code)) {
            if !ch.is_control() || ch.is_whitespace() {
                let source = if self.font_type.is_cid() {
                    FontDecodeSource::IdentityCid
                } else {
                    FontDecodeSource::NativePdfText
                };
                return (ch.to_string(), source);
            }
        }
        if let Some(text) = self
            .predefined_cmap
            .as_ref()
            .and_then(|info| predefined_cmap::unicode_for_code(info.name, code))
        {
            return (text, FontDecodeSource::PredefinedCMap);
        }
        log::warn!("font decode produced replacement character for code {code:#06X}");
        ("\u{FFFD}".to_string(), FontDecodeSource::Unknown)
    }

    pub fn glyph_name(&self, code: u16) -> Option<&str> {
        self.encoding_table
            .as_ref()
            .and_then(|table| table.get(code as usize))
            .map(String::as_str)
            .filter(|name| *name != ".notdef")
    }

    pub fn code_size(&self) -> u8 {
        self.code_size
    }

    pub fn is_space_code(&self, code: u16) -> bool {
        if self.code_size == 1 && code == 0x0020 {
            return true;
        }
        self.decode_char(code) == " "
    }

    pub fn has_standard14_metrics(&self) -> bool {
        self.standard14_base.is_some()
    }

    pub fn glyph_width(&self, char_code: u16) -> f64 {
        if let Some(descendant_font) = &self.descendant_font {
            return lookup_cid_width(u32::from(char_code), descendant_font);
        }

        let index = u32::from(char_code);
        if index >= self.first_char && index <= self.last_char {
            let i = (index - self.first_char) as usize;
            self.widths
                .get(i)
                .copied()
                .or_else(|| self.standard14_width(char_code))
                .unwrap_or(self.default_width)
        } else {
            self.standard14_width(char_code)
                .unwrap_or(self.default_width)
        }
    }

    pub fn font_type(&self) -> &FontType {
        &self.font_type
    }

    /// Writing mode of the font: `false` = horizontal, `true` = vertical.
    /// Vertical text advances glyphs top-to-bottom and arranges columns
    /// right-to-left. Driven by the encoding CMap's WMode (PDF 32000-1 §9.7.4.3),
    /// never by the text matrix. Only Type0 fonts are ever vertical.
    pub fn is_vertical(&self) -> bool {
        self.wmode == 1
    }

    /// Vertical glyph metrics (W2) for the given CID, as `(w1y, v_x, v_y)` in
    /// glyph space (1000-unit em), per PDF 32000-1 §9.7.4.3:
    /// - `w1y` is the vertical displacement (the glyph's advance height, normally
    ///   negative since vertical writing proceeds downward),
    /// - `(v_x, v_y)` is the position vector from the glyph's horizontal origin
    ///   to its vertical origin.
    ///
    /// Falls back to the descendant font's `/DW2` (default `[880 -1000]`) when the
    /// CID has no explicit `/W2` entry. Returns the spec defaults for a font with
    /// no descendant (`v_y = 880`, `w1y = -1000`, `v_x = w0/2`).
    pub fn vertical_metrics(&self, char_code: u16) -> (f64, f64, f64) {
        let cid = u32::from(char_code);
        let w0 = self.glyph_width(char_code);
        match &self.descendant_font {
            Some(desc) => lookup_cid_vertical(cid, w0, desc),
            None => (-1000.0, w0 / 2.0, 880.0),
        }
    }

    fn build(font_dict: &PdfDictionary, reader: Option<&PdfReader>) -> Self {
        let font_type = font_dict
            .get_name("Subtype")
            .map(FontType::from_name)
            .unwrap_or_else(|| FontType::Unknown("Unknown".to_string()));
        let to_unicode = parse_to_unicode(font_dict, reader);
        let encoding_table = if font_type.is_cid() {
            None
        } else {
            Some(build_encoding_table(font_dict, reader, &font_type))
        };
        let descendant_font = if matches!(font_type, FontType::Type0) {
            get_descendant_font_optional(font_dict, reader)
        } else {
            None
        };
        let first_char = font_dict
            .get_integer("FirstChar")
            .filter(|value| *value >= 0)
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(0);
        let last_char = font_dict
            .get_integer("LastChar")
            .filter(|value| *value >= 0)
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(255);
        let widths = parse_widths(font_dict, reader, first_char, last_char);
        let default_width = if font_type.is_cid() {
            descendant_font
                .as_ref()
                .and_then(|dict| dict.get("DW"))
                .and_then(PdfObject::as_number)
                .unwrap_or(1000.0)
        } else if let Some(missing_width) =
            font_descriptor_number(font_dict, reader, "MissingWidth")
        {
            missing_width
        } else if widths.is_empty() {
            500.0
        } else {
            widths.iter().sum::<f64>() / widths.len() as f64
        };
        let predefined_cmap = if font_type.is_cid() {
            predefined_cmap_name(font_dict, reader).and_then(|name| predefined_cmap::lookup(&name))
        } else {
            None
        };
        let standard14_base = font_dict
            .get_name("BaseFont")
            .and_then(standard14_base_name);
        let code_size = if font_type.is_cid() {
            to_unicode
                .as_ref()
                .map(ToUnicodeCMap::code_size)
                .or_else(|| predefined_cmap.map(|info| info.code_size))
                .unwrap_or(2)
        } else {
            1
        };

        // Writing mode: only composite (Type0) fonts can be vertical, and only
        // via their /Encoding CMap. Simple fonts are always horizontal.
        let wmode = if matches!(font_type, FontType::Type0) {
            detect_wmode(font_dict, reader)
        } else {
            0
        };

        Self {
            font_type,
            to_unicode,
            encoding_table,
            widths,
            first_char,
            last_char,
            descendant_font,
            default_width,
            code_size,
            predefined_cmap,
            wmode,
            standard14_base,
        }
    }

    fn standard14_width(&self, char_code: u16) -> Option<f64> {
        let base = self.standard14_base.as_deref()?;
        let glyph_name = self.glyph_name(char_code)?;
        standard14_width(base, glyph_name)
    }
}

fn standard14_base_name(name: &str) -> Option<String> {
    let raw = name.trim_start_matches('/');
    let raw = raw.rsplit('+').next().unwrap_or(raw);
    match raw {
        "Courier" | "Courier-Bold" | "Courier-Oblique" | "Courier-BoldOblique" => {
            Some(raw.to_string())
        }
        "Helvetica" | "Helvetica-Bold" | "Helvetica-Oblique" | "Helvetica-BoldOblique" => {
            Some(raw.to_string())
        }
        "Times-Roman" | "Times-Bold" | "Times-Italic" | "Times-BoldItalic" => Some(raw.to_string()),
        _ => None,
    }
}

fn standard14_width(base: &str, glyph_name: &str) -> Option<f64> {
    match base {
        "Courier" | "Courier-Bold" | "Courier-Oblique" | "Courier-BoldOblique" => {
            (glyph_name != ".notdef").then_some(600.0)
        }
        "Helvetica" | "Helvetica-Oblique" => helvetica_standard_width(glyph_name),
        "Helvetica-Bold" | "Helvetica-BoldOblique" => helvetica_bold_standard_width(glyph_name),
        "Times-Roman" => times_roman_standard_width(glyph_name),
        "Times-Bold" => times_bold_standard_width(glyph_name),
        "Times-Italic" => times_italic_standard_width(glyph_name),
        "Times-BoldItalic" => times_bold_italic_standard_width(glyph_name),
        _ => None,
    }
}

fn helvetica_standard_width(glyph: &str) -> Option<f64> {
    Some(match glyph {
        "space" => 278.0,
        "exclam" => 278.0,
        "quotedbl" => 355.0,
        "numbersign" | "dollar" => 556.0,
        "percent" => 889.0,
        "ampersand" => 667.0,
        "quoteright" | "quotesingle" | "quoteleft" | "grave" | "acute" => 222.0,
        "parenleft" | "parenright" | "bracketleft" | "bracketright" => 333.0,
        "asterisk" => 389.0,
        "plus" | "less" | "equal" | "greater" | "asciitilde" => 584.0,
        "comma" | "period" | "colon" | "semicolon" => 278.0,
        "hyphen" => 333.0,
        "slash" | "backslash" => 278.0,
        "zero" | "one" | "two" | "three" | "four" | "five" | "six" | "seven" | "eight" | "nine" => {
            556.0
        }
        "question" => 556.0,
        "at" => 1015.0,
        "A" | "B" | "K" | "X" | "Y" => 667.0,
        "C" | "H" | "N" | "R" | "U" => 722.0,
        "D" | "G" | "O" | "Q" => 778.0,
        "E" => 667.0,
        "F" | "T" | "Z" => 611.0,
        "I" => 278.0,
        "J" => 500.0,
        "L" => 556.0,
        "M" => 833.0,
        "P" => 667.0,
        "S" => 667.0,
        "V" => 667.0,
        "W" => 944.0,
        "asciicircum" => 469.0,
        "underscore" => 556.0,
        "a" | "b" | "d" | "e" | "g" | "n" | "o" | "p" | "q" | "u" => 556.0,
        "c" | "k" | "s" | "v" | "x" | "y" | "z" => 500.0,
        "f" | "t" => 278.0,
        "h" => 556.0,
        "i" | "j" | "l" => 222.0,
        "m" => 833.0,
        "r" => 333.0,
        "w" => 722.0,
        "braceleft" | "braceright" => 334.0,
        "bar" => 260.0,
        _ => return None,
    })
}

fn helvetica_bold_standard_width(glyph: &str) -> Option<f64> {
    Some(match glyph {
        "space" | "quoteright" | "quotesingle" | "quoteleft" | "grave" | "acute" | "comma"
        | "period" | "slash" | "backslash" | "I" | "i" | "j" | "l" => 278.0,
        "exclam" | "parenleft" | "parenright" | "colon" | "semicolon" | "bracketleft"
        | "bracketright" | "f" | "t" => 333.0,
        "quotedbl" => 474.0,
        "numbersign" | "dollar" | "zero" | "one" | "two" | "three" | "four" | "five" | "six"
        | "seven" | "eight" | "nine" | "L" | "T" | "c" | "k" | "s" | "x" | "z" => 556.0,
        "percent" | "m" => 889.0,
        "ampersand" | "E" | "P" | "S" | "Y" => 667.0,
        "asterisk" | "r" => 389.0,
        "plus" | "less" | "equal" | "greater" | "asciicircum" | "asciitilde" => 584.0,
        "hyphen" => 333.0,
        "question" | "F" => 611.0,
        "at" => 975.0,
        "A" | "B" | "H" | "K" | "N" | "R" | "U" => 722.0,
        "C" | "D" | "G" | "O" | "Q" => 778.0,
        "J" | "a" => 556.0,
        "M" => 833.0,
        "V" => 722.0,
        "W" => 944.0,
        "X" => 722.0,
        "Z" => 611.0,
        "underscore" => 556.0,
        "b" | "d" | "g" | "h" | "n" | "o" | "p" | "q" | "u" => 611.0,
        "e" | "v" | "y" => 556.0,
        "w" => 778.0,
        "braceleft" | "braceright" => 389.0,
        "bar" => 280.0,
        _ => return None,
    })
}

fn times_roman_standard_width(glyph: &str) -> Option<f64> {
    Some(match glyph {
        "space" => 250.0,
        "exclam" => 333.0,
        "quotedbl" => 408.0,
        "numbersign" | "dollar" | "asterisk" => 500.0,
        "percent" => 833.0,
        "ampersand" => 778.0,
        "quoteright" | "quotesingle" | "quoteleft" => 180.0,
        "parenleft" | "parenright" | "bracketleft" | "bracketright" | "grave" | "acute" => 333.0,
        "plus" | "less" | "equal" | "greater" => 564.0,
        "comma" | "period" => 250.0,
        "hyphen" => 333.0,
        "slash" | "backslash" | "colon" | "semicolon" | "i" | "j" | "l" | "t" => 278.0,
        "zero" | "one" | "two" | "three" | "four" | "five" | "six" | "seven" | "eight" | "nine" => {
            500.0
        }
        "question" | "a" | "c" | "e" | "z" => 444.0,
        "at" => 921.0,
        "A" | "K" | "N" | "O" | "Q" | "V" | "X" | "Y" => 722.0,
        "B" | "C" | "R" => 667.0,
        "D" => 722.0,
        "E" | "L" | "T" | "Z" => 611.0,
        "F" | "S" => 556.0,
        "G" | "H" => 722.0,
        "I" => 333.0,
        "J" | "s" => 389.0,
        "M" => 889.0,
        "P" => 556.0,
        "U" => 722.0,
        "W" => 944.0,
        "asciicircum" => 469.0,
        "underscore" => 500.0,
        "b" | "d" | "g" | "h" | "k" | "n" | "o" | "p" | "q" | "u" | "v" | "x" | "y" => 500.0,
        "f" | "r" => 333.0,
        "m" => 778.0,
        "w" => 722.0,
        "braceleft" | "braceright" => 480.0,
        "bar" => 200.0,
        "asciitilde" => 541.0,
        _ => return None,
    })
}

fn times_bold_standard_width(glyph: &str) -> Option<f64> {
    Some(match glyph {
        "space" | "comma" | "period" => 250.0,
        "exclam" | "parenleft" | "parenright" | "colon" | "semicolon" | "bracketleft"
        | "bracketright" | "grave" | "acute" | "f" | "t" => 333.0,
        "quotedbl" | "J" | "P" | "S" | "a" | "o" | "v" | "x" | "y" => 500.0,
        "numbersign" | "dollar" | "asterisk" | "zero" | "one" | "two" | "three" | "four"
        | "five" | "six" | "seven" | "eight" | "nine" | "question" => 500.0,
        "percent" => 1000.0,
        "ampersand" => 833.0,
        "quoteright" | "quotesingle" | "quoteleft" | "slash" | "backslash" | "i" | "l" => 278.0,
        "plus" | "less" | "equal" | "greater" => 570.0,
        "at" => 930.0,
        "A" | "C" | "N" | "R" | "U" | "V" | "X" => 722.0,
        "B" | "E" | "L" | "Z" => 667.0,
        "D" | "H" | "K" | "O" | "Q" => 778.0,
        "F" => 611.0,
        "G" => 778.0,
        "I" => 389.0,
        "M" => 944.0,
        "T" => 667.0,
        "W" => 1000.0,
        "Y" => 722.0,
        "asciicircum" => 581.0,
        "underscore" => 500.0,
        "b" | "d" | "h" | "k" | "n" | "p" | "q" | "u" => 556.0,
        "c" | "e" | "z" => 444.0,
        "g" => 500.0,
        "j" => 333.0,
        "m" => 833.0,
        "r" => 444.0,
        "s" => 389.0,
        "w" => 722.0,
        "braceleft" | "braceright" => 394.0,
        "bar" => 220.0,
        "asciitilde" => 520.0,
        _ => return None,
    })
}

fn times_italic_standard_width(glyph: &str) -> Option<f64> {
    Some(match glyph {
        "space" | "comma" | "period" => 250.0,
        "exclam" | "parenleft" | "parenright" | "colon" | "semicolon" | "bracketleft"
        | "bracketright" | "grave" | "acute" => 333.0,
        "quotedbl" => 420.0,
        "numbersign" | "dollar" | "asterisk" | "zero" | "one" | "two" | "three" | "four"
        | "five" | "six" | "seven" | "eight" | "nine" | "question" | "a" | "b" | "d" | "g"
        | "h" | "n" | "o" | "p" | "q" | "u" => 500.0,
        "percent" | "M" | "W" => 833.0,
        "ampersand" => 778.0,
        "quoteright" | "quotesingle" | "quoteleft" => 214.0,
        "plus" | "less" | "equal" | "greater" => 675.0,
        "hyphen" => 333.0,
        "slash" | "backslash" | "i" | "j" | "l" | "t" => 278.0,
        "at" => 920.0,
        "A" | "B" | "E" | "F" | "P" | "R" | "V" | "X" => 611.0,
        "C" | "K" => 667.0,
        "D" | "G" | "H" | "O" | "Q" | "U" => 722.0,
        "I" => 333.0,
        "J" => 444.0,
        "L" | "T" | "Y" | "Z" => 556.0,
        "S" | "r" | "s" | "z" => 389.0,
        "asciicircum" | "x" | "y" => 422.0,
        "underscore" => 500.0,
        "c" | "e" | "k" | "v" => 444.0,
        "f" => 278.0,
        "m" => 722.0,
        "w" => 667.0,
        "braceleft" | "braceright" => 400.0,
        "bar" => 275.0,
        "asciitilde" => 541.0,
        _ => return None,
    })
}

fn times_bold_italic_standard_width(glyph: &str) -> Option<f64> {
    Some(match glyph {
        "space" | "comma" | "period" => 250.0,
        "exclam" | "I" | "colon" | "semicolon" | "f" => 389.0,
        "quotedbl" | "r" | "s" | "z" => 389.0,
        "numbersign" | "dollar" | "asterisk" | "zero" | "one" | "two" | "three" | "four"
        | "five" | "six" | "seven" | "eight" | "nine" | "question" | "a" | "b" | "d" | "g"
        | "k" | "o" | "p" | "q" | "x" => 500.0,
        "percent" => 833.0,
        "ampersand" | "H" | "m" => 778.0,
        "quoteright" | "quotesingle" | "quoteleft" | "slash" | "backslash" | "i" | "j" | "l"
        | "t" => 278.0,
        "parenleft" | "parenright" | "bracketleft" | "bracketright" | "grave" | "acute" => 333.0,
        "plus" | "less" | "equal" | "greater" | "asciicircum" | "asciitilde" => 570.0,
        "hyphen" => 333.0,
        "at" => 832.0,
        "A" | "B" | "C" | "E" | "K" | "R" | "V" | "X" => 667.0,
        "D" | "G" | "N" | "O" | "Q" | "U" => 722.0,
        "F" => 667.0,
        "J" => 500.0,
        "L" | "P" | "T" | "Y" | "Z" => 611.0,
        "M" | "W" => 889.0,
        "S" => 556.0,
        "underscore" => 500.0,
        "c" | "e" | "v" | "y" => 444.0,
        "h" | "n" | "u" => 556.0,
        "w" => 667.0,
        "braceleft" | "braceright" => 348.0,
        "bar" => 220.0,
        _ => return None,
    })
}

pub fn predefined_cmap_name(
    font_dict: &PdfDictionary,
    reader: Option<&PdfReader>,
) -> Option<String> {
    let encoding = font_dict.get("Encoding")?;
    let resolved = resolve_optional(encoding, reader).unwrap_or_else(|_| encoding.clone());
    match resolved {
        PdfObject::Name(name) => Some(name),
        PdfObject::Stream { dict, raw } => {
            if let Some(name) = dict.get_name("CMapName") {
                return Some(name.to_string());
            }
            let decoded = match reader {
                Some(reader) => {
                    let stream = PdfObject::Stream { dict, raw };
                    decode_stream_lossless(&stream, reader)
                        .map(|d| d.data)
                        .unwrap_or_default()
                }
                None => decode_stream_from_dict(&dict, &raw).unwrap_or_default(),
            };
            cmap_name_from_bytes(&decoded)
        }
        _ => None,
    }
}

pub fn get_descendant_font(
    type0_dict: &PdfDictionary,
    reader: &PdfReader,
) -> Option<PdfDictionary> {
    get_descendant_font_optional(type0_dict, Some(reader))
}

pub fn lookup_cid_width(cid: u32, desc_dict: &PdfDictionary) -> f64 {
    let dw = desc_dict
        .get("DW")
        .and_then(PdfObject::as_number)
        .unwrap_or(1000.0);

    let Some(w_arr) = desc_dict.get("W").and_then(PdfObject::as_array) else {
        return dw;
    };

    let mut idx = 0usize;
    while idx < w_arr.len() {
        let Some(c1) = w_arr[idx]
            .as_number()
            .filter(|value| *value >= 0.0)
            .map(|value| value as u32)
        else {
            break;
        };
        idx += 1;
        if idx >= w_arr.len() {
            break;
        }

        match &w_arr[idx] {
            PdfObject::Array(widths) => {
                for (offset, width_obj) in widths.iter().enumerate() {
                    if c1.saturating_add(offset as u32) == cid {
                        if let Some(width) = width_obj.as_number() {
                            return width;
                        }
                    }
                }
                idx += 1;
            }
            _ => {
                let Some(c2) = w_arr[idx]
                    .as_number()
                    .filter(|value| *value >= 0.0)
                    .map(|value| value as u32)
                else {
                    break;
                };
                idx += 1;
                if idx >= w_arr.len() {
                    break;
                }
                let Some(width) = w_arr[idx].as_number() else {
                    break;
                };
                idx += 1;

                if cid >= c1 && cid <= c2 {
                    return width;
                }
            }
        }
    }

    dw
}

/// Determine the writing mode (0 = horizontal, 1 = vertical) of a Type0 font
/// from its `/Encoding`. The encoding is either a predefined CMap name (whose
/// `-V`/`-H` suffix, or `Identity-V`/`Identity-H`, declares the mode) or an
/// embedded CMap stream carrying a `/WMode` entry. Defaults to horizontal.
fn detect_wmode(font_dict: &PdfDictionary, reader: Option<&PdfReader>) -> u8 {
    let Some(encoding) = font_dict.get("Encoding") else {
        return 0;
    };
    let resolved = resolve_optional(encoding, reader).unwrap_or_else(|_| encoding.clone());
    match resolved {
        // Predefined CMap referenced by name: the name's suffix is authoritative.
        PdfObject::Name(name) => wmode_from_cmap_name(&name),
        // Embedded CMap stream: read its /WMode key, falling back to the
        // CMapName / name suffix inside the decoded program.
        PdfObject::Stream { dict, raw } => {
            if let Some(w) = dict.get_integer("WMode") {
                return u8::from(w == 1);
            }
            let decoded = match reader {
                Some(reader) => {
                    let stream = PdfObject::Stream { dict, raw };
                    decode_stream_lossless(&stream, reader)
                        .map(|d| d.data)
                        .unwrap_or_default()
                }
                None => decode_stream_from_dict(&dict, &raw).unwrap_or_default(),
            };
            wmode_from_cmap_bytes(&decoded)
        }
        _ => 0,
    }
}

/// Vertical iff a predefined CMap name ends in `-V` (e.g. `Identity-V`,
/// `UniGB-UCS2-V`, `UniJIS-UCS2-V`). All `-H` names and anything else are
/// horizontal.
fn wmode_from_cmap_name(name: &str) -> u8 {
    predefined_cmap::wmode_from_name(name).unwrap_or_else(|| u8::from(name.ends_with("-V")))
}

/// Scan a decoded CMap program for an explicit `/WMode 1` declaration or a
/// `/CMapName` ending in `-V`. Conservative: defaults to horizontal.
fn wmode_from_cmap_bytes(bytes: &[u8]) -> u8 {
    let text = String::from_utf8_lossy(bytes);
    if let Some(idx) = text.find("/WMode") {
        let rest = text[idx + "/WMode".len()..].trim_start();
        if rest.starts_with('1') {
            return 1;
        }
        if rest.starts_with('0') {
            return 0;
        }
    }
    if let Some(idx) = text.find("/CMapName") {
        let rest = &text[idx..];
        // Match a token like `/CMapName /Something-V def`.
        if let Some(slash) = rest[1..].find('/') {
            let after = &rest[1 + slash + 1..];
            let token: String = after
                .chars()
                .take_while(|c| !c.is_whitespace() && *c != '/')
                .collect();
            return wmode_from_cmap_name(token.trim());
        }
    }
    0
}

fn cmap_name_from_bytes(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(bytes);
    let idx = text.find("/CMapName")?;
    let rest = &text[idx..];
    let slash = rest[1..].find('/')?;
    let after = &rest[1 + slash + 1..];
    let token: String = after
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != '/')
        .collect();
    (!token.is_empty()).then_some(token)
}

/// Look up vertical metrics `(w1y, v_x, v_y)` for a CID from a CIDFont's `/W2`
/// array, with `/DW2` as the per-font default. See PDF 32000-1 §9.7.4.3.
///
/// `/DW2` is `[v_y w1y]` (default `[880 -1000]`): `v_y` is the y of the position
/// vector and `w1y` the default vertical displacement; the default `v_x` is
/// `w0/2` (half the glyph's horizontal width).
///
/// `/W2` entries come in two forms:
/// - `c [w1y_1 v1x_1 v1y_1  w1y_2 v1x_2 v1y_2  …]` — consecutive CIDs from `c`,
///   three numbers each.
/// - `c_first c_last w1y v1x v1y` — a CID range sharing one triple.
pub fn lookup_cid_vertical(cid: u32, w0: f64, desc_dict: &PdfDictionary) -> (f64, f64, f64) {
    let (def_vy, def_w1y) = desc_dict
        .get("DW2")
        .and_then(PdfObject::as_array)
        .and_then(|a| {
            let vy = a.first().and_then(PdfObject::as_number)?;
            let w1y = a.get(1).and_then(PdfObject::as_number)?;
            Some((vy, w1y))
        })
        .unwrap_or((880.0, -1000.0));
    let default = (def_w1y, w0 / 2.0, def_vy);

    let Some(w2) = desc_dict.get("W2").and_then(PdfObject::as_array) else {
        return default;
    };

    let mut idx = 0usize;
    while idx < w2.len() {
        let Some(c1) = w2[idx].as_number().filter(|v| *v >= 0.0).map(|v| v as u32) else {
            break;
        };
        idx += 1;
        if idx >= w2.len() {
            break;
        }

        match &w2[idx] {
            PdfObject::Array(triples) => {
                // c [w1y vx vy  w1y vx vy …]
                let n = triples.len() / 3;
                for k in 0..n {
                    if c1.saturating_add(k as u32) == cid {
                        let w1y = triples[k * 3].as_number().unwrap_or(def_w1y);
                        let vx = triples[k * 3 + 1].as_number().unwrap_or(w0 / 2.0);
                        let vy = triples[k * 3 + 2].as_number().unwrap_or(def_vy);
                        return (w1y, vx, vy);
                    }
                }
                idx += 1;
            }
            _ => {
                // c_first c_last w1y vx vy
                let Some(c2) = w2[idx].as_number().filter(|v| *v >= 0.0).map(|v| v as u32) else {
                    break;
                };
                idx += 1;
                if idx + 2 > w2.len() {
                    break;
                }
                let w1y = w2[idx].as_number().unwrap_or(def_w1y);
                let vx = w2[idx + 1].as_number().unwrap_or(w0 / 2.0);
                let vy = w2[idx + 2].as_number().unwrap_or(def_vy);
                idx += 3;
                if cid >= c1 && cid <= c2 {
                    return (w1y, vx, vy);
                }
            }
        }
    }

    default
}

pub(crate) fn expand_ligature(ch: char) -> String {
    match ch {
        '\u{FB00}' => "ff".to_string(),
        '\u{FB01}' => "fi".to_string(),
        '\u{FB02}' => "fl".to_string(),
        '\u{FB03}' => "ffi".to_string(),
        '\u{FB04}' => "ffl".to_string(),
        '\u{FB05}' | '\u{FB06}' => "st".to_string(),
        other => other.to_string(),
    }
}

fn parse_to_unicode(
    font_dict: &PdfDictionary,
    reader: Option<&PdfReader>,
) -> Option<ToUnicodeCMap> {
    let object = font_dict.get("ToUnicode")?;
    let resolved = resolve_optional(object, reader).ok()?;
    let PdfObject::Stream { dict, raw } = resolved else {
        return None;
    };
    let decoded = match reader {
        Some(reader) => {
            let stream = PdfObject::Stream { dict, raw };
            decode_stream_lossless(&stream, reader).ok()?.data
        }
        None => decode_stream_from_dict(&dict, &raw).ok()?,
    };
    Some(ToUnicodeCMap::parse(&decoded))
}

fn build_encoding_table(
    font_dict: &PdfDictionary,
    reader: Option<&PdfReader>,
    font_type: &FontType,
) -> Vec<String> {
    // Symbol and ZapfDingbats are standard-14 fonts with their own built-in
    // encodings (spec Appendix D). When the BaseFont names one of them, that
    // encoding — not StandardEncoding/MacRoman — is the implicit default, used
    // both when /Encoding is absent and as the base for any /Differences.
    let symbolic_base = symbolic_builtin_encoding(font_dict);
    let default_base = symbolic_base.unwrap_or(match font_type {
        FontType::TrueType => "MacRomanEncoding",
        _ => "StandardEncoding",
    });
    let type1_builtin =
        if symbolic_base.is_none() && matches!(font_type, FontType::Type1 | FontType::MMType1) {
            embedded_font_program_bytes(font_dict, reader)
                .and_then(|bytes| type1::builtin_encoding(bytes.as_slice()))
        } else {
            None
        };

    let Some(encoding_obj) = font_dict.get("Encoding") else {
        return type1_builtin.unwrap_or_else(|| table_for(default_base));
    };

    let resolved = resolve_optional(encoding_obj, reader).unwrap_or_else(|_| encoding_obj.clone());
    match resolved {
        PdfObject::Name(name) => table_for(&name),
        PdfObject::Dictionary(dict) => {
            let diffs = dict
                .get_array("Differences")
                .map(pdf_objects_to_operands)
                .unwrap_or_default();
            let base_table = dict
                .get_name("BaseEncoding")
                .map(table_for)
                .unwrap_or_else(|| type1_builtin.unwrap_or_else(|| table_for(default_base)));
            if diffs.is_empty() {
                base_table
            } else {
                apply_differences_to_table(base_table, &diffs)
            }
        }
        _ => table_for(default_base),
    }
}

/// If the font's `/BaseFont` is the Symbol or ZapfDingbats standard-14 font,
/// return the name of its built-in encoding (so [`Encoding::lookup`] uses the
/// Appendix D tables). A subset prefix like `ABCDEF+Symbol` is handled.
fn symbolic_builtin_encoding(font_dict: &PdfDictionary) -> Option<&'static str> {
    let base = font_dict.get_name("BaseFont")?;
    let base = base.rsplit('+').next().unwrap_or(base);
    let lower = base.to_ascii_lowercase();
    if lower.contains("zapfdingbats") || lower.contains("dingbats") {
        Some("ZapfDingbatsEncoding")
    } else if lower == "symbol" || lower.starts_with("symbol") || lower.contains("-symbol") {
        Some("SymbolEncoding")
    } else {
        None
    }
}

fn table_for(name: &str) -> Vec<String> {
    (0u8..=255)
        .map(|byte| Encoding::lookup(name, byte).to_string())
        .collect()
}

fn apply_differences_to_table(mut table: Vec<String>, differences: &[Operand]) -> Vec<String> {
    if table.len() < 256 {
        table.resize(256, ".notdef".to_string());
    }
    let mut current_code: usize = 0;
    for item in differences {
        match item {
            Operand::Integer(n) if *n >= 0 => {
                current_code = *n as usize;
            }
            Operand::Name(name) => {
                if current_code < 256 {
                    table[current_code] = name.clone();
                }
                current_code = current_code.saturating_add(1);
            }
            _ => {}
        }
    }
    table
}

fn embedded_font_program_bytes(
    font_dict: &PdfDictionary,
    reader: Option<&PdfReader>,
) -> Option<Vec<u8>> {
    let reader = reader?;
    let descriptor = match reader
        .resolve(font_dict.get("FontDescriptor")?.clone())
        .ok()?
    {
        PdfObject::Dictionary(dict) => dict,
        _ => return None,
    };
    for key in ["FontFile", "FontFile3", "FontFile2"] {
        let Some(font_file) = descriptor.get(key) else {
            continue;
        };
        let PdfObject::Stream { dict, raw } = reader.resolve(font_file.clone()).ok()? else {
            continue;
        };
        if raw.is_empty() {
            continue;
        }
        let stream = PdfObject::Stream {
            dict: dict.clone(),
            raw: raw.clone(),
        };
        if let Ok(decoded) = decode_stream_lossless(&stream, reader) {
            if !decoded.data.is_empty() {
                return Some(decoded.data);
            }
        }
        return Some(raw);
    }
    None
}

fn font_descriptor_number(
    font_dict: &PdfDictionary,
    reader: Option<&PdfReader>,
    key: &str,
) -> Option<f64> {
    let descriptor = resolve_optional(font_dict.get("FontDescriptor")?, reader).ok()?;
    match descriptor {
        PdfObject::Dictionary(dict) => dict.get(key).and_then(PdfObject::as_number),
        _ => None,
    }
}

fn pdf_objects_to_operands(objects: &[PdfObject]) -> Vec<Operand> {
    objects
        .iter()
        .filter_map(|object| match object {
            PdfObject::Integer(value) => Some(Operand::Integer(*value)),
            PdfObject::Real(value) => Some(Operand::Real(*value)),
            PdfObject::Name(value) => Some(Operand::Name(value.clone())),
            PdfObject::String(value) => Some(Operand::String(value.clone())),
            PdfObject::Array(items) => Some(Operand::Array(pdf_objects_to_operands(items))),
            PdfObject::Boolean(value) => Some(Operand::Boolean(*value)),
            _ => None,
        })
        .collect()
}

fn parse_widths(
    font_dict: &PdfDictionary,
    reader: Option<&PdfReader>,
    first_char: u32,
    last_char: u32,
) -> Vec<f64> {
    let Some(widths_obj) = font_dict.get("Widths") else {
        return Vec::new();
    };
    let widths_obj = resolve_optional(widths_obj, reader).unwrap_or_else(|_| widths_obj.clone());
    let Some(widths) = widths_obj.as_array() else {
        return Vec::new();
    };
    let wanted_len = last_char
        .checked_sub(first_char)
        .and_then(|value| value.checked_add(1))
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(0);
    let mut values: Vec<f64> = widths
        .iter()
        .filter_map(|object| {
            object
                .as_number()
                .or_else(|| resolve_optional(object, reader).ok()?.as_number())
        })
        .collect();
    if wanted_len > 0 {
        values.truncate(wanted_len);
    }
    values
}

fn get_descendant_font_optional(
    type0_dict: &PdfDictionary,
    reader: Option<&PdfReader>,
) -> Option<PdfDictionary> {
    let descendants = match type0_dict.get("DescendantFonts")? {
        PdfObject::Array(items) => items.clone(),
        PdfObject::Reference { number, generation } => {
            let reader = reader?;
            match reader.get_and_resolve(*number, *generation).ok()? {
                PdfObject::Array(items) => items,
                _ => return None,
            }
        }
        _ => return None,
    };

    match descendants.first()?.clone() {
        PdfObject::Dictionary(dict) => Some(dict),
        PdfObject::Reference { number, generation } => {
            let reader = reader?;
            match reader.get_and_resolve(number, generation).ok()? {
                PdfObject::Dictionary(dict) => Some(dict),
                _ => None,
            }
        }
        _ => None,
    }
}

fn resolve_optional(object: &PdfObject, reader: Option<&PdfReader>) -> Result<PdfObject> {
    match reader {
        Some(reader) => reader.resolve(object.clone()),
        None => Ok(object.clone()),
    }
}

#[cfg(test)]
mod cid_font_tests {
    use super::*;

    #[test]
    fn lookup_cid_width_returns_dw_when_w_absent() {
        let mut dict = PdfDictionary::empty();
        dict.insert("DW", PdfObject::Integer(1000));
        assert_eq!(lookup_cid_width(65, &dict), 1000.0);
    }

    #[test]
    fn lookup_cid_width_defaults_to_1000_when_absent() {
        assert_eq!(lookup_cid_width(65, &PdfDictionary::empty()), 1000.0);
    }

    #[test]
    fn lookup_cid_width_format_array() {
        let mut dict = PdfDictionary::empty();
        dict.insert("DW", PdfObject::Integer(1000));
        dict.insert(
            "W",
            PdfObject::Array(vec![
                PdfObject::Integer(65),
                PdfObject::Array(vec![
                    PdfObject::Integer(722),
                    PdfObject::Integer(667),
                    PdfObject::Integer(611),
                ]),
            ]),
        );
        assert_eq!(lookup_cid_width(65, &dict), 722.0);
        assert_eq!(lookup_cid_width(66, &dict), 667.0);
        assert_eq!(lookup_cid_width(68, &dict), 1000.0);
    }

    #[test]
    fn lookup_cid_width_format_range() {
        let mut dict = PdfDictionary::empty();
        dict.insert("DW", PdfObject::Integer(1000));
        dict.insert(
            "W",
            PdfObject::Array(vec![
                PdfObject::Integer(100),
                PdfObject::Integer(200),
                PdfObject::Integer(400),
            ]),
        );
        assert_eq!(lookup_cid_width(150, &dict), 400.0);
        assert_eq!(lookup_cid_width(50, &dict), 1000.0);
    }

    #[test]
    fn lookup_cid_width_mixed_formats() {
        let mut dict = PdfDictionary::empty();
        dict.insert("DW", PdfObject::Integer(1000));
        dict.insert(
            "W",
            PdfObject::Array(vec![
                PdfObject::Integer(32),
                PdfObject::Array(vec![PdfObject::Integer(277), PdfObject::Integer(333)]),
                PdfObject::Integer(65),
                PdfObject::Integer(90),
                PdfObject::Integer(722),
            ]),
        );
        assert_eq!(lookup_cid_width(32, &dict), 277.0);
        assert_eq!(lookup_cid_width(33, &dict), 333.0);
        assert_eq!(lookup_cid_width(70, &dict), 722.0);
        assert_eq!(lookup_cid_width(10, &dict), 1000.0);
    }

    #[test]
    fn lookup_cid_width_empty_w_uses_dw() {
        let mut dict = PdfDictionary::empty();
        dict.insert("DW", PdfObject::Integer(500));
        dict.insert("W", PdfObject::Array(vec![]));
        assert_eq!(lookup_cid_width(100, &dict), 500.0);
    }

    #[test]
    fn detect_font_subtype_identifies_type0() {
        let mut dict = PdfDictionary::empty();
        dict.insert("Subtype", PdfObject::Name("Type0".to_string()));
        assert_eq!(detect_font_subtype(&dict), FontSubtype::Type0);
    }

    #[test]
    fn wmode_name_suffix_detection() {
        assert_eq!(wmode_from_cmap_name("Identity-V"), 1);
        assert_eq!(wmode_from_cmap_name("Identity-H"), 0);
        assert_eq!(wmode_from_cmap_name("UniJIS-UTF16-V"), 1);
        assert_eq!(wmode_from_cmap_name("UniGB-UTF16-H"), 0);
        assert_eq!(wmode_from_cmap_name("UniJIS-UCS2-V"), 1);
        assert_eq!(wmode_from_cmap_name("UniGB-UCS2-H"), 0);
        assert_eq!(wmode_from_cmap_name("90ms-RKSJ-V"), 1);
        assert_eq!(wmode_from_cmap_name("WeirdName"), 0);
    }

    #[test]
    fn supported_predefined_utf16_cmap_sets_code_size_and_decodes_unicode() {
        let mut dict = PdfDictionary::empty();
        dict.insert("Subtype", PdfObject::Name("Type0".to_string()));
        dict.insert("Encoding", PdfObject::Name("UniJIS-UTF16-H".to_string()));

        let resolver = FontResolver::new_from_dict_only(&dict);
        assert_eq!(resolver.code_size(), 2);
        assert_eq!(resolver.decode_string(&[0x65, 0xE5]), "日");
        assert!(!resolver.is_vertical());
    }

    #[test]
    fn wmode_from_embedded_cmap_bytes() {
        assert_eq!(wmode_from_cmap_bytes(b"/WMode 1 def"), 1);
        assert_eq!(wmode_from_cmap_bytes(b"/WMode 0 def"), 0);
        assert_eq!(wmode_from_cmap_bytes(b"/CMapName /Adobe-Japan1-V def"), 1);
        assert_eq!(wmode_from_cmap_bytes(b"no wmode here"), 0);
    }

    #[test]
    fn type0_identity_v_font_is_vertical() {
        let mut desc = PdfDictionary::empty();
        desc.insert("Subtype", PdfObject::Name("CIDFontType2".to_string()));
        let mut dict = PdfDictionary::empty();
        dict.insert("Subtype", PdfObject::Name("Type0".to_string()));
        dict.insert("Encoding", PdfObject::Name("Identity-V".to_string()));
        dict.insert(
            "DescendantFonts",
            PdfObject::Array(vec![PdfObject::Dictionary(desc)]),
        );
        let resolver = FontResolver::new_from_dict_only(&dict);
        assert!(resolver.is_vertical(), "Identity-V should be vertical");
    }

    #[test]
    fn type0_identity_h_font_is_horizontal() {
        let mut dict = PdfDictionary::empty();
        dict.insert("Subtype", PdfObject::Name("Type0".to_string()));
        dict.insert("Encoding", PdfObject::Name("Identity-H".to_string()));
        let resolver = FontResolver::new_from_dict_only(&dict);
        assert!(!resolver.is_vertical(), "Identity-H should be horizontal");
    }

    #[test]
    fn simple_font_is_never_vertical() {
        let mut dict = PdfDictionary::empty();
        dict.insert("Subtype", PdfObject::Name("Type1".to_string()));
        dict.insert("Encoding", PdfObject::Name("WinAnsiEncoding".to_string()));
        let resolver = FontResolver::new_from_dict_only(&dict);
        assert!(!resolver.is_vertical());
    }

    #[test]
    fn standard14_font_without_widths_uses_builtin_metrics() {
        let mut dict = PdfDictionary::empty();
        dict.insert("Subtype", PdfObject::Name("Type1".to_string()));
        dict.insert("BaseFont", PdfObject::Name("Helvetica".to_string()));
        let resolver = FontResolver::new_from_dict_only(&dict);

        assert!(resolver.has_standard14_metrics());
        assert_eq!(resolver.glyph_width(u16::from(b'A')), 667.0);
        assert_eq!(resolver.glyph_width(u16::from(b'i')), 222.0);
        assert_eq!(resolver.glyph_width(u16::from(b' ')), 278.0);
    }

    #[test]
    fn courier_standard14_font_without_widths_is_fixed_width() {
        let mut dict = PdfDictionary::empty();
        dict.insert("Subtype", PdfObject::Name("Type1".to_string()));
        dict.insert(
            "BaseFont",
            PdfObject::Name("Courier-BoldOblique".to_string()),
        );
        let resolver = FontResolver::new_from_dict_only(&dict);

        assert_eq!(resolver.glyph_width(u16::from(b'i')), 600.0);
        assert_eq!(resolver.glyph_width(u16::from(b'W')), 600.0);
    }

    #[test]
    fn standard14_variant_metrics_are_style_specific() {
        let mut helvetica_bold = PdfDictionary::empty();
        helvetica_bold.insert("Subtype", PdfObject::Name("Type1".to_string()));
        helvetica_bold.insert("BaseFont", PdfObject::Name("Helvetica-Bold".to_string()));
        let helvetica_bold = FontResolver::new_from_dict_only(&helvetica_bold);
        assert_eq!(helvetica_bold.glyph_width(u16::from(b'W')), 944.0);
        assert_eq!(helvetica_bold.glyph_width(u16::from(b'm')), 889.0);

        let mut times_italic = PdfDictionary::empty();
        times_italic.insert("Subtype", PdfObject::Name("Type1".to_string()));
        times_italic.insert("BaseFont", PdfObject::Name("Times-Italic".to_string()));
        let times_italic = FontResolver::new_from_dict_only(&times_italic);
        assert_eq!(times_italic.glyph_width(u16::from(b'A')), 611.0);
        assert_eq!(times_italic.glyph_width(u16::from(b'w')), 667.0);
    }

    #[test]
    fn simple_width_array_truncates_to_declared_code_span() {
        let mut dict = PdfDictionary::empty();
        dict.insert(
            "Widths",
            PdfObject::Array(vec![
                PdfObject::Integer(250),
                PdfObject::Integer(600),
                PdfObject::Integer(700),
            ]),
        );

        assert_eq!(parse_widths(&dict, None, 10, 11), vec![250.0, 600.0]);
    }

    #[test]
    fn simple_width_array_ignores_malformed_entries() {
        let mut dict = PdfDictionary::empty();
        dict.insert(
            "Widths",
            PdfObject::Array(vec![
                PdfObject::Integer(250),
                PdfObject::Name("bad".to_string()),
                PdfObject::Real(333.5),
            ]),
        );

        assert_eq!(parse_widths(&dict, None, 0, 2), vec![250.0, 333.5]);
    }

    #[test]
    fn simple_font_missing_width_uses_font_descriptor_before_average_width() {
        let mut descriptor = PdfDictionary::empty();
        descriptor.insert("MissingWidth", PdfObject::Integer(420));

        let mut dict = PdfDictionary::empty();
        dict.insert("Subtype", PdfObject::Name("Type1".to_string()));
        dict.insert("FirstChar", PdfObject::Integer(65));
        dict.insert("LastChar", PdfObject::Integer(66));
        dict.insert(
            "Widths",
            PdfObject::Array(vec![PdfObject::Integer(700), PdfObject::Integer(710)]),
        );
        dict.insert("FontDescriptor", PdfObject::Dictionary(descriptor));

        let resolver = FontResolver::new_from_dict_only(&dict);

        assert_eq!(resolver.glyph_width(10), 420.0);
    }

    #[test]
    fn lookup_cid_vertical_uses_dw2_default() {
        let dict = PdfDictionary::empty();
        let (w1y, vx, vy) = lookup_cid_vertical(5, 1000.0, &dict);
        assert_eq!(w1y, -1000.0);
        assert_eq!(vx, 500.0);
        assert_eq!(vy, 880.0);
    }

    #[test]
    fn lookup_cid_vertical_honors_explicit_dw2() {
        let mut dict = PdfDictionary::empty();
        dict.insert(
            "DW2",
            PdfObject::Array(vec![PdfObject::Integer(900), PdfObject::Integer(-1100)]),
        );
        let (w1y, vx, vy) = lookup_cid_vertical(5, 1000.0, &dict);
        assert_eq!(w1y, -1100.0);
        assert_eq!(vx, 500.0);
        assert_eq!(vy, 900.0);
    }

    #[test]
    fn lookup_cid_vertical_w2_array_form() {
        let mut dict = PdfDictionary::empty();
        dict.insert(
            "W2",
            PdfObject::Array(vec![
                PdfObject::Integer(10),
                PdfObject::Array(vec![
                    PdfObject::Integer(-900),
                    PdfObject::Integer(450),
                    PdfObject::Integer(800),
                    PdfObject::Integer(-950),
                    PdfObject::Integer(460),
                    PdfObject::Integer(810),
                ]),
            ]),
        );
        assert_eq!(
            lookup_cid_vertical(10, 1000.0, &dict),
            (-900.0, 450.0, 800.0)
        );
        assert_eq!(
            lookup_cid_vertical(11, 1000.0, &dict),
            (-950.0, 460.0, 810.0)
        );
        assert_eq!(
            lookup_cid_vertical(12, 1000.0, &dict),
            (-1000.0, 500.0, 880.0)
        );
    }

    #[test]
    fn lookup_cid_vertical_w2_range_form() {
        let mut dict = PdfDictionary::empty();
        dict.insert(
            "W2",
            PdfObject::Array(vec![
                PdfObject::Integer(100),
                PdfObject::Integer(200),
                PdfObject::Integer(-880),
                PdfObject::Integer(500),
                PdfObject::Integer(880),
            ]),
        );
        assert_eq!(
            lookup_cid_vertical(150, 1000.0, &dict),
            (-880.0, 500.0, 880.0)
        );
        assert_eq!(
            lookup_cid_vertical(50, 1000.0, &dict),
            (-1000.0, 500.0, 880.0)
        );
    }

    #[test]
    fn detect_font_subtype_identifies_truetype() {
        let mut dict = PdfDictionary::empty();
        dict.insert("Subtype", PdfObject::Name("TrueType".to_string()));
        assert_eq!(detect_font_subtype(&dict), FontSubtype::TrueType);
    }

    #[test]
    fn font_subtype_enum_covers_common_pdf_subtypes() {
        let mut type1 = PdfDictionary::empty();
        type1.insert("Subtype", PdfObject::Name("Type1".to_string()));
        assert_eq!(detect_font_subtype(&type1), FontSubtype::Type1);

        let mut cid2 = PdfDictionary::empty();
        cid2.insert("Subtype", PdfObject::Name("CIDFontType2".to_string()));
        assert_eq!(detect_font_subtype(&cid2), FontSubtype::CIDFontType2);

        assert_eq!(
            detect_font_subtype(&PdfDictionary::empty()),
            FontSubtype::Unknown
        );
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn dict(entries: &[(&str, PdfObject)]) -> PdfDictionary {
        PdfDictionary::new(
            entries
                .iter()
                .map(|(key, value)| ((*key).to_string(), value.clone()))
                .collect::<BTreeMap<_, _>>(),
        )
    }

    #[test]
    fn type1_standard_encoding_decodes_strings() {
        let font = dict(&[
            ("Type", PdfObject::Name("Font".to_string())),
            ("Subtype", PdfObject::Name("Type1".to_string())),
            ("Encoding", PdfObject::Name("StandardEncoding".to_string())),
            ("FirstChar", PdfObject::Integer(65)),
            ("LastChar", PdfObject::Integer(67)),
            (
                "Widths",
                PdfObject::Array(vec![
                    PdfObject::Integer(600),
                    PdfObject::Integer(600),
                    PdfObject::Integer(600),
                ]),
            ),
        ]);
        let resolver = FontResolver::new_from_dict_only(&font);
        assert_eq!(resolver.decode_string(b"ABC"), "ABC");
        assert_eq!(resolver.decode_string(b"\xAE"), "fi");
    }

    #[test]
    fn win_ansi_encoding_decodes_strings() {
        let font = dict(&[
            ("Subtype", PdfObject::Name("Type1".to_string())),
            ("Encoding", PdfObject::Name("WinAnsiEncoding".to_string())),
        ]);
        let resolver = FontResolver::new_from_dict_only(&font);
        assert_eq!(resolver.decode_string(&[0x80]), "€");
        assert_eq!(resolver.decode_string(&[0x96]), "–");
    }

    #[test]
    fn to_unicode_overrides_encoding() {
        let cmap = b"
        begincmap
        1 beginbfchar
        <41> <4E2D>
        endbfchar
        endcmap
        ";
        let font = dict(&[
            ("Subtype", PdfObject::Name("Type1".to_string())),
            ("Encoding", PdfObject::Name("StandardEncoding".to_string())),
            (
                "ToUnicode",
                PdfObject::Stream {
                    dict: PdfDictionary::empty(),
                    raw: cmap.to_vec(),
                },
            ),
        ]);
        let resolver = FontResolver::new_from_dict_only(&font);
        assert_eq!(resolver.decode_string(b"A"), "中");
    }

    #[test]
    fn partial_to_unicode_falls_back_to_glyph_names_for_missing_codes() {
        let cmap = b"
        begincmap
        1 beginbfchar
        <42> <4E2D>
        endbfchar
        endcmap
        ";
        let encoding = PdfDictionary::new(
            [
                (
                    "BaseEncoding".to_string(),
                    PdfObject::Name("WinAnsiEncoding".to_string()),
                ),
                (
                    "Differences".to_string(),
                    PdfObject::Array(vec![
                        PdfObject::Integer(65),
                        PdfObject::Name("Euro".to_string()),
                        PdfObject::Name("A".to_string()),
                    ]),
                ),
            ]
            .into_iter()
            .collect(),
        );
        let font = dict(&[
            ("Subtype", PdfObject::Name("Type1".to_string())),
            ("Encoding", PdfObject::Dictionary(encoding)),
            (
                "ToUnicode",
                PdfObject::Stream {
                    dict: PdfDictionary::empty(),
                    raw: cmap.to_vec(),
                },
            ),
        ]);

        let resolver = FontResolver::new_from_dict_only(&font);
        assert_eq!(resolver.decode_string(b"AB"), "\u{20AC}\u{4E2D}");
    }
}
