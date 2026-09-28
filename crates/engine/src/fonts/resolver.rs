use crate::content::operation::Operand;
use crate::error::Result;
use crate::filters::decode_stream_lossless;
use crate::fonts::character_code::{CharacterCode, CodeSpace};
use crate::fonts::cmap::ToUnicodeCMap;
#[path = "resolver_codes.rs"]
mod codes;
use crate::fonts::encoding::Encoding;
use crate::fonts::glyph_list::glyph_name_to_unicode;
use crate::fonts::predefined_cmap;
use crate::fonts::type1;
use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;
pub use codes::{CodeIterator, DecodedCode};

/// PDF text-space displacement (PDF Reference 1.6, sections 5.2.1-2, 5.3.3).
/// Tc/Tw are signed additive values, not distances along the writing direction.
/// Positive spacing shortens an ordinary downward vertical advance. Th does
/// not scale this axis. Callers validate their source metrics/state separately.
pub(crate) fn vertical_text_advance(
    w1y: f64,
    font_size: f64,
    char_spacing: f64,
    word_spacing: f64,
    applies_word_spacing: bool,
) -> f64 {
    w1y / 1000.0 * font_size
        + char_spacing
        + if applies_word_spacing {
            word_spacing
        } else {
            0.0
        }
}

/// `adjustment` is already negated from the PDF TJ array operand, matching
/// retained renderer descriptors. No Tc/Tw applies to a numeric TJ adjustment.
pub(crate) fn text_position_adjustment(
    adjustment: f64,
    font_size: f64,
    horizontal_scaling: f64,
    vertical: bool,
) -> [f64; 2] {
    let displacement = adjustment / 1000.0 * font_size;
    if vertical {
        [0.0, displacement]
    } else {
        [displacement * horizontal_scaling / 100.0, 0.0]
    }
}

pub(crate) fn uses_vertical_writing(font: &PdfDictionary, reader: &PdfReader) -> bool {
    font.get_name("Subtype") == Some("Type0") && detect_wmode(font, Some(reader)) == 1
}

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
    unicode_is_predefined: bool,
    standard14_base: Option<String>,
    cid_encoding: std::result::Result<Option<super::cid_encoding::CidEncoding>, String>,
    sfnt_cff_gids: std::result::Result<Option<Vec<u16>>, String>,
    code_space: std::result::Result<std::sync::Arc<CodeSpace>, String>,
    unicode_encoding: std::result::Result<(), String>,
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
        let mut text = String::new();
        for code in self.codes(bytes) {
            match code {
                Ok(code) => text.push_str(&self.decode_code(code.code)),
                Err(_) => {
                    text.push('\u{FFFD}');
                    break;
                }
            }
        }
        text
    }
    pub fn decode_char(&self, code: u16) -> String {
        self.decode_char_with_source(code).0
    }
    pub fn decode_char_with_source(&self, code: u16) -> (String, FontDecodeSource) {
        match CharacterCode::new(u32::from(code), self.code_size) {
            Ok(code) => self.decode_code_with_source(code),
            Err(_) => ("\u{FFFD}".into(), FontDecodeSource::Unknown),
        }
    }
    pub fn decode_code(&self, code: CharacterCode) -> String {
        self.decode_code_with_source(code).0
    }
    pub fn decode_code_with_source(&self, code: CharacterCode) -> (String, FontDecodeSource) {
        if !self
            .code_space
            .as_ref()
            .is_ok_and(|space| space.contains(code))
        {
            return ("\u{FFFD}".into(), FontDecodeSource::Unknown);
        }
        self.decode_code_bytes_with_source(code)
    }

    fn decode_code_bytes_with_source(&self, code: CharacterCode) -> (String, FontDecodeSource) {
        if let Some(text) = self
            .to_unicode
            .as_ref()
            .and_then(|cmap| cmap.lookup_code(code))
        {
            return (
                text.to_string(),
                if self.unicode_is_predefined {
                    FontDecodeSource::PredefinedCMap
                } else {
                    FontDecodeSource::ToUnicode
                },
            );
        }
        // A malformed ToUnicode CMap is fatal for a composite font: without a
        // trustworthy CMap there is no general one-byte encoding fallback and
        // guessing a CID as Unicode would corrupt logical text. Simple fonts
        // are different. Their Encoding/Differences table remains the
        // authoritative code-to-glyph mapping and commonly repairs producer
        // CMaps whose codespace incorrectly declares two-byte codes while its
        // bfchar entries are one byte (for example <E9> in WinAnsi). Refusing
        // that valid simple-font fallback made the renderer drop accented
        // glyphs even though both the embedded program and /Encoding contained
        // them.
        if self.unicode_is_predefined
            || (self.font_type.is_cid()
                && self
                    .to_unicode
                    .as_ref()
                    .is_some_and(|map| map.validate().is_err()))
        {
            return ("\u{FFFD}".into(), FontDecodeSource::Unknown);
        }
        let code = code.value();

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

        if let Some(ch) = char::from_u32(code) {
            if !ch.is_control() || ch.is_whitespace() {
                let source = if self.font_type.is_cid() {
                    FontDecodeSource::IdentityCid
                } else {
                    FontDecodeSource::NativePdfText
                };
                return (ch.to_string(), source);
            }
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

    /// Fixed encoded length, or zero for mixed-length code spaces. Consumers
    /// must use codes()/next_code() rather than chunking source bytes.
    pub fn code_size(&self) -> u8 {
        self.code_size
    }

    /// True when PDF Tw applies, not when the Unicode mapping is whitespace.
    /// Only an encoded single-byte 0x20 receives word spacing (5.2.2).
    pub fn is_space_code(&self, code: u16) -> bool {
        self.code_size == 1 && code == 0x0020
    }

    pub fn has_standard14_metrics(&self) -> bool {
        self.standard14_base.is_some()
    }

    pub fn glyph_width(&self, char_code: u16) -> f64 {
        CharacterCode::new(u32::from(char_code), self.code_size)
            .map(|code| self.width_for_code(code))
            .unwrap_or(self.default_width)
    }
    pub fn width_for_code(&self, char_code: CharacterCode) -> f64 {
        if let Some(descendant_font) = &self.descendant_font {
            return lookup_cid_width(
                self.cid_for_character(char_code)
                    .map(u32::from)
                    .unwrap_or(char_code.value()),
                descendant_font,
            );
        }

        let index = char_code.value();
        let standard_width = u16::try_from(index)
            .ok()
            .and_then(|code| self.standard14_width(code));
        if index >= self.first_char && index <= self.last_char {
            let i = (index - self.first_char) as usize;
            self.widths
                .get(i)
                .copied()
                .or(standard_width)
                .unwrap_or(self.default_width)
        } else {
            standard_width.unwrap_or(self.default_width)
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
        CharacterCode::new(u32::from(char_code), self.code_size)
            .map(|code| self.vertical_metrics_for_code(code))
            .unwrap_or((-1000.0, self.default_width / 2.0, 880.0))
    }
    pub fn vertical_metrics_for_code(&self, char_code: CharacterCode) -> (f64, f64, f64) {
        let cid = self
            .cid_for_character(char_code)
            .map(u32::from)
            .unwrap_or(char_code.value());
        let w0 = self.width_for_code(char_code);
        match &self.descendant_font {
            Some(desc) => lookup_cid_vertical(cid, w0, desc),
            None => (-1000.0, w0 / 2.0, 880.0),
        }
    }

    /// Character codes and native CIDs are distinct for an embedded Encoding
    /// CMap. ToUnicode remains keyed by the original character code.
    pub fn cid_for_code(&self, code: u16) -> std::result::Result<u16, String> {
        self.cid_for_character(CharacterCode::new(u32::from(code), self.code_size)?)
    }
    pub fn cid_for_character(&self, code: CharacterCode) -> std::result::Result<u16, String> {
        if !self
            .code_space
            .as_ref()
            .map_err(Clone::clone)?
            .contains(code)
        {
            return Err("character code outside font Encoding code space".into());
        }
        match self.cid_encoding.as_ref().map_err(Clone::clone)? {
            Some(map) => Ok(map.cid_code(code)),
            None => u16::try_from(code.value())
                .map_err(|_| "character code needs a resolved CID Encoding CMap".into()),
        }
    }

    pub fn validate_encoding(&self) -> std::result::Result<(), String> {
        self.cid_encoding.as_ref().map_err(Clone::clone)?;
        self.sfnt_cff_gids.as_ref().map_err(Clone::clone)?;
        self.code_space.as_ref().map_err(Clone::clone)?;
        Ok(())
    }

    pub(crate) fn sfnt_cff_gid(&self, cid: u16) -> std::result::Result<Option<u16>, String> {
        self.sfnt_cff_gids
            .as_ref()
            .map(|map| {
                map.as_ref()
                    .map(|map| map.get(usize::from(cid)).copied().unwrap_or(0))
            })
            .map_err(Clone::clone)
    }

    fn build(font_dict: &PdfDictionary, reader: Option<&PdfReader>) -> Self {
        let font_type = font_dict
            .get_name("Subtype")
            .map(FontType::from_name)
            .unwrap_or_else(|| FontType::Unknown("Unknown".to_string()));
        let mut to_unicode = parse_to_unicode(font_dict, reader);
        let mut unicode_is_predefined = false;
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
        let cid_encoding = if matches!(font_type, FontType::Type0) {
            super::cid_encoding::CidEncoding::load(font_dict, reader).and_then(|map| {
                if let Some(map) = &map {
                    predefined_cmap::validate_font_system(map, descendant_font.as_ref(), reader)?;
                }
                Ok(map)
            })
        } else {
            Ok(None)
        };
        if to_unicode.is_none() {
            if let Ok(Some(map)) = &cid_encoding {
                match predefined_cmap::font_unicode(map, descendant_font.as_ref(), reader) {
                    Ok(Some(map)) => {
                        to_unicode = Some(map);
                        unicode_is_predefined = true;
                    }
                    Err(error) => {
                        to_unicode = Some(ToUnicodeCMap::failed(error));
                        unicode_is_predefined = true;
                    }
                    Ok(None) => {}
                }
            }
        }
        let sfnt_cff_gids = super::cid::sfnt_cff_gid_map(descendant_font.as_ref(), reader);
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
            cid_encoding
                .as_ref()
                .ok()
                .and_then(|map| map.as_ref())
                .and_then(|map| map.program.name.as_deref())
                .and_then(predefined_cmap::lookup)
        } else {
            None
        };
        let standard14_base = font_dict
            .get_name("BaseFont")
            .and_then(standard14_base_name);
        let code_size = if font_type.is_cid() {
            cid_encoding
                .as_ref()
                .ok()
                .and_then(|map| map.as_ref().map(|map| map.code_size))
                .or_else(|| {
                    to_unicode
                        .as_ref()
                        .map(ToUnicodeCMap::code_size)
                        .or_else(|| predefined_cmap.map(|info| info.code_size))
                })
                .unwrap_or(2)
        } else {
            1
        };

        let code_space = if !font_type.is_cid() {
            CodeSpace::fixed(1).map(std::sync::Arc::new)
        } else {
            match &cid_encoding {
                Err(error) => Err(error.clone()),
                Ok(Some(map)) => Ok(map.shared_space()),
                Ok(None) => match to_unicode.as_ref().and_then(ToUnicodeCMap::shared_space) {
                    Some(space) => Ok(space),
                    None => CodeSpace::fixed(code_size.max(1)).map(std::sync::Arc::new),
                },
            }
        };
        let code_size = code_space
            .as_ref()
            .ok()
            .and_then(|space| space.fixed_length())
            .unwrap_or(0);
        let unicode_encoding = match (&code_space, &to_unicode) {
            (Err(error), _) => Err(error.clone()),
            (Ok(space), Some(map)) => map.validate_codes(space),
            (_, None) => Ok(()),
        };
        // Writing mode: only composite (Type0) fonts can be vertical, and only
        // via their /Encoding CMap. Simple fonts are always horizontal.
        let wmode = if matches!(font_type, FontType::Type0) {
            cid_encoding
                .as_ref()
                .ok()
                .and_then(|map| map.as_ref().and_then(|map| map.wmode))
                .unwrap_or(0)
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
            unicode_is_predefined,
            wmode,
            standard14_base,
            cid_encoding,
            sfnt_cff_gids,
            code_space,
            unicode_encoding,
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
        PdfObject::Stream { .. } => {
            super::cmap_stream::read(&resolved, reader, super::cmap_program::Kind::Cid, 0)
                .ok()
                .and_then(|program| program.name)
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

/// Determine writing mode through the same resolved Encoding used by glyph
/// decoding. Comments, strings and guessed name suffixes are not authoritative.
fn detect_wmode(font_dict: &PdfDictionary, reader: Option<&PdfReader>) -> u8 {
    super::cid_encoding::CidEncoding::load(font_dict, reader)
        .ok()
        .flatten()
        .and_then(|map| map.wmode)
        .unwrap_or(0)
}
#[cfg(test)]
fn wmode_from_cmap_name(name: &str) -> u8 {
    predefined_cmap::wmode_from_name(name).unwrap_or(0)
}
#[cfg(test)]
fn wmode_from_cmap_bytes(bytes: &[u8]) -> u8 {
    super::cmap_program::Program::parse(bytes, super::cmap_program::Kind::Cid, None, false)
        .ok()
        .and_then(|program| program.wmode)
        .unwrap_or(0)
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
                if w2.len().saturating_sub(idx) < 3 {
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
    Some(ToUnicodeCMap::load(font_dict.get("ToUnicode")?, reader))
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

pub(crate) fn validate_visual_font_metrics(
    font_dict: &PdfDictionary,
    reader: Option<&PdfReader>,
) -> std::result::Result<(), String> {
    let font_type = font_dict
        .get_name("Subtype")
        .map(FontType::from_name)
        .unwrap_or_else(|| FontType::Unknown("Unknown".to_string()));
    if font_type.is_cid() {
        let Some(descendant) = get_descendant_font_optional(font_dict, reader) else {
            return Ok(());
        };
        validate_cid_horizontal_metrics(&descendant, reader)?;
        validate_cid_vertical_metrics(&descendant, reader)
    } else {
        validate_simple_width_metrics(font_dict, reader)
    }
}

fn validate_simple_width_metrics(
    font_dict: &PdfDictionary,
    reader: Option<&PdfReader>,
) -> std::result::Result<(), String> {
    let Some(widths_obj) = font_dict.get("Widths") else {
        return Ok(());
    };
    let first_char = required_nonnegative_integer(font_dict, "FirstChar", "simple font")?;
    let last_char = required_nonnegative_integer(font_dict, "LastChar", "simple font")?;
    if last_char < first_char {
        return Err("simple font malformed /Widths: /LastChar precedes /FirstChar".to_string());
    }
    let expected_len = last_char
        .checked_sub(first_char)
        .and_then(|value| value.checked_add(1))
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| "simple font malformed /Widths: declared span is too large".to_string())?;
    let widths_obj = resolve_optional(widths_obj, reader)
        .map_err(|_| "simple font malformed /Widths: could not resolve array".to_string())?;
    let PdfObject::Array(widths) = widths_obj else {
        return Err("simple font malformed /Widths: expected number array".to_string());
    };
    if widths.len() != expected_len {
        return Err(format!(
            "simple font malformed /Widths: expected {expected_len} entries, got {}",
            widths.len()
        ));
    }
    for (idx, width_obj) in widths.iter().enumerate() {
        let width = finite_metric_number(width_obj, reader).ok_or_else(|| {
            format!(
                "simple font malformed /Widths: entry {} is not a finite number",
                idx + 1
            )
        })?;
        if width < 0.0 {
            return Err(format!(
                "simple font malformed /Widths: entry {} is negative",
                idx + 1
            ));
        }
    }
    Ok(())
}

fn validate_cid_horizontal_metrics(
    desc_dict: &PdfDictionary,
    reader: Option<&PdfReader>,
) -> std::result::Result<(), String> {
    if let Some(dw) = desc_dict.get("DW") {
        let value = finite_metric_number(dw, reader)
            .ok_or_else(|| "CID font malformed /DW: expected finite number".to_string())?;
        if value < 0.0 {
            return Err("CID font malformed /DW: width is negative".to_string());
        }
    }
    let Some(w_obj) = desc_dict.get("W") else {
        return Ok(());
    };
    let w_obj = resolve_optional(w_obj, reader)
        .map_err(|_| "CID font malformed /W: could not resolve array".to_string())?;
    let PdfObject::Array(widths) = w_obj else {
        return Err("CID font malformed /W: expected array".to_string());
    };
    let mut idx = 0usize;
    while idx < widths.len() {
        let c1 = nonnegative_metric_integer(&widths[idx], reader)
            .ok_or_else(|| "CID font malformed /W: expected starting CID".to_string())?;
        idx += 1;
        if idx >= widths.len() {
            return Err("CID font malformed /W: missing width entry".to_string());
        }
        let entry = resolve_optional(&widths[idx], reader)
            .map_err(|_| "CID font malformed /W: could not resolve width entry".to_string())?;
        match entry {
            PdfObject::Array(values) => {
                for (offset, width_obj) in values.iter().enumerate() {
                    let width = finite_metric_number(width_obj, reader).ok_or_else(|| {
                        format!(
                            "CID font malformed /W: array width entry {} is not finite",
                            offset + 1
                        )
                    })?;
                    if width < 0.0 {
                        return Err(format!(
                            "CID font malformed /W: array width entry {} is negative",
                            offset + 1
                        ));
                    }
                }
                idx += 1;
            }
            other => {
                let c2 = nonnegative_metric_integer(&other, reader)
                    .ok_or_else(|| "CID font malformed /W: expected ending CID".to_string())?;
                if c2 < c1 {
                    return Err("CID font malformed /W: ending CID precedes start".to_string());
                }
                idx += 1;
                if idx >= widths.len() {
                    return Err("CID font malformed /W: missing range width".to_string());
                }
                let width = finite_metric_number(&widths[idx], reader).ok_or_else(|| {
                    "CID font malformed /W: range width is not finite".to_string()
                })?;
                if width < 0.0 {
                    return Err("CID font malformed /W: range width is negative".to_string());
                }
                idx += 1;
            }
        }
    }
    Ok(())
}

fn validate_cid_vertical_metrics(
    desc_dict: &PdfDictionary,
    reader: Option<&PdfReader>,
) -> std::result::Result<(), String> {
    if let Some(dw2_obj) = desc_dict.get("DW2") {
        let dw2_obj = resolve_optional(dw2_obj, reader)
            .map_err(|_| "CID font malformed /DW2: could not resolve array".to_string())?;
        let PdfObject::Array(values) = dw2_obj else {
            return Err("CID font malformed /DW2: expected two-number array".to_string());
        };
        if values.len() != 2 {
            return Err(format!(
                "CID font malformed /DW2: expected 2 entries, got {}",
                values.len()
            ));
        }
        for (idx, value) in values.iter().enumerate() {
            if finite_metric_number(value, reader).is_none() {
                return Err(format!(
                    "CID font malformed /DW2: entry {} is not finite",
                    idx + 1
                ));
            }
        }
    }
    let Some(w2_obj) = desc_dict.get("W2") else {
        return Ok(());
    };
    let w2_obj = resolve_optional(w2_obj, reader)
        .map_err(|_| "CID font malformed /W2: could not resolve array".to_string())?;
    let PdfObject::Array(metrics) = w2_obj else {
        return Err("CID font malformed /W2: expected array".to_string());
    };
    let mut idx = 0usize;
    while idx < metrics.len() {
        let c1 = nonnegative_metric_integer(&metrics[idx], reader)
            .ok_or_else(|| "CID font malformed /W2: expected starting CID".to_string())?;
        idx += 1;
        if idx >= metrics.len() {
            return Err("CID font malformed /W2: missing metric entry".to_string());
        }
        let entry = resolve_optional(&metrics[idx], reader)
            .map_err(|_| "CID font malformed /W2: could not resolve metric entry".to_string())?;
        match entry {
            PdfObject::Array(values) => {
                if !values.len().is_multiple_of(3) {
                    return Err(
                        "CID font malformed /W2: array entry length is not a multiple of 3"
                            .to_string(),
                    );
                }
                for (entry_idx, value) in values.iter().enumerate() {
                    if finite_metric_number(value, reader).is_none() {
                        return Err(format!(
                            "CID font malformed /W2: array metric entry {} is not finite",
                            entry_idx + 1
                        ));
                    }
                }
                idx += 1;
            }
            other => {
                let c2 = nonnegative_metric_integer(&other, reader)
                    .ok_or_else(|| "CID font malformed /W2: expected ending CID".to_string())?;
                if c2 < c1 {
                    return Err("CID font malformed /W2: ending CID precedes start".to_string());
                }
                idx += 1;
                if idx + 2 >= metrics.len() {
                    return Err(
                        "CID font malformed /W2: range metric triple is incomplete".to_string()
                    );
                }
                for offset in 0..3 {
                    if finite_metric_number(&metrics[idx + offset], reader).is_none() {
                        return Err(format!(
                            "CID font malformed /W2: range metric entry {} is not finite",
                            offset + 1
                        ));
                    }
                }
                idx += 3;
            }
        }
    }
    Ok(())
}

fn required_nonnegative_integer(
    dict: &PdfDictionary,
    key: &str,
    label: &str,
) -> std::result::Result<u32, String> {
    let Some(value) = dict.get_integer(key) else {
        return Err(format!("{label} malformed /Widths: missing /{key}"));
    };
    if value < 0 {
        return Err(format!("{label} malformed /Widths: /{key} is negative"));
    }
    u32::try_from(value).map_err(|_| format!("{label} malformed /Widths: /{key} is too large"))
}

fn finite_metric_number(obj: &PdfObject, reader: Option<&PdfReader>) -> Option<f64> {
    let resolved = resolve_optional(obj, reader).ok()?;
    let value = resolved.as_number()?;
    value.is_finite().then_some(value)
}

fn nonnegative_metric_integer(obj: &PdfObject, reader: Option<&PdfReader>) -> Option<u32> {
    let resolved = resolve_optional(obj, reader).ok()?;
    let value = resolved.as_integer()?;
    if value < 0 {
        return None;
    }
    u32::try_from(value).ok()
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

    let descendant = match descendants.first()?.clone() {
        PdfObject::Dictionary(dict) => Some(dict),
        PdfObject::Reference { number, generation } => {
            let reader = reader?;
            match reader.get_and_resolve(number, generation).ok()? {
                PdfObject::Dictionary(dict) => Some(dict),
                _ => None,
            }
        }
        _ => None,
    }?;

    // Width lookup is intentionally reader-free and hot: normalize the four
    // metric entries once while the resolver still owns a reader. Real-world
    // producers routinely store /W and /W2 (and, less often, /DW or /DW2) as
    // indirect objects. Validation already resolved those objects, but keeping
    // the unresolved references here made lookup_cid_width/vertical silently
    // fall back to the defaults. That distorted text cursors, inline-edit
    // placement, and rendering even though the same font had passed validation.
    let mut descendant = descendant;
    if let Some(reader) = reader {
        for key in ["DW", "W", "DW2", "W2"] {
            let Some(value) = descendant.get(key).cloned() else {
                continue;
            };
            if matches!(value, PdfObject::Reference { .. }) {
                if let Ok(resolved) = reader.resolve(value) {
                    descendant.insert(key, resolved);
                }
            }
        }
    }
    Some(descendant)
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
    fn vertical_spacing_is_signed_and_independent_of_advance_direction() {
        assert_eq!(vertical_text_advance(-1000.0, 10.0, 2.0, 3.0, false), -8.0);
        assert_eq!(vertical_text_advance(-1000.0, 10.0, 2.0, 3.0, true), -5.0);
        assert_eq!(
            vertical_text_advance(-1000.0, 10.0, -2.0, -3.0, true),
            -15.0
        );
        assert_eq!(vertical_text_advance(1000.0, 10.0, 2.0, 3.0, true), 15.0);
        assert_eq!(vertical_text_advance(-1000.0, -10.0, 2.0, 3.0, true), 15.0);
    }

    #[test]
    fn tj_adjustment_uses_writing_axis_not_rotated_matrix_or_unicode() {
        assert_eq!(
            text_position_adjustment(-1600.0, 10.0, 50.0, true),
            [0.0, -16.0]
        );
        assert_eq!(
            text_position_adjustment(-1600.0, 10.0, 50.0, false),
            [-8.0, 0.0]
        );
        assert_eq!(
            text_position_adjustment(500.0, 10.0, 200.0, true),
            [0.0, 5.0]
        );
    }

    #[test]
    fn word_spacing_depends_on_encoded_byte_not_unicode_space() {
        let mut encoding = PdfDictionary::empty();
        encoding.insert("BaseEncoding", PdfObject::Name("WinAnsiEncoding".into()));
        encoding.insert(
            "Differences",
            PdfObject::Array(vec![
                PdfObject::Integer(32),
                PdfObject::Name("A".into()),
                PdfObject::Integer(65),
                PdfObject::Name("space".into()),
            ]),
        );
        let mut font = PdfDictionary::empty();
        font.insert("Subtype", PdfObject::Name("Type1".into()));
        font.insert("Encoding", PdfObject::Dictionary(encoding));
        let simple = FontResolver::new_from_dict_only(&font);
        assert_eq!(simple.decode_char(65), " ");
        assert!(!simple.is_space_code(65));
        assert!(simple.is_space_code(32));
        font.insert("Subtype", PdfObject::Name("Type0".into()));
        font.insert("Encoding", PdfObject::Name("Identity-V".into()));
        let composite = FontResolver::new_from_dict_only(&font);
        assert_eq!(composite.code_size(), 2);
        assert!(!composite.is_space_code(32));
    }

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
        assert_eq!(resolver.code_size(), 0);
        assert_eq!(resolver.decode_string(&[0x65, 0xE5]), "日");
        assert!(!resolver.is_vertical());
    }

    #[test]
    fn wmode_from_embedded_cmap_bytes() {
        let space = "1 begincodespacerange <0000> <ffff> endcodespacerange";
        assert_eq!(
            wmode_from_cmap_bytes(format!("/WMode 1 def {space}").as_bytes()),
            1
        );
        assert_eq!(
            wmode_from_cmap_bytes(format!("/WMode 0 def {space}").as_bytes()),
            0
        );
        assert_eq!(
            wmode_from_cmap_bytes(format!("/CMapName /Custom-V def {space}").as_bytes()),
            0
        );
        assert_eq!(
            wmode_from_cmap_bytes(format!("/Notice (/WMode 1 def) def {space}").as_bytes()),
            0
        );
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
    fn visual_font_metrics_reject_malformed_simple_widths() {
        let mut dict = PdfDictionary::empty();
        dict.insert("Subtype", PdfObject::Name("Type1".to_string()));
        dict.insert("FirstChar", PdfObject::Integer(0));
        dict.insert("LastChar", PdfObject::Integer(2));
        dict.insert(
            "Widths",
            PdfObject::Array(vec![
                PdfObject::Integer(250),
                PdfObject::Name("bad".to_string()),
                PdfObject::Real(333.5),
            ]),
        );

        let err = validate_visual_font_metrics(&dict, None)
            .expect_err("visual rendering must not filter malformed width entries");

        assert!(
            err.contains("simple font malformed /Widths"),
            "unexpected width validation error: {err}"
        );
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
    fn visual_font_metrics_reject_malformed_cid_vertical_metrics() {
        let mut descendant = PdfDictionary::empty();
        descendant.insert("Subtype", PdfObject::Name("CIDFontType2".to_string()));
        descendant.insert(
            "W2",
            PdfObject::Array(vec![
                PdfObject::Integer(10),
                PdfObject::Array(vec![
                    PdfObject::Integer(-900),
                    PdfObject::Integer(400),
                    PdfObject::Name("bad".to_string()),
                ]),
            ]),
        );
        let mut dict = PdfDictionary::empty();
        dict.insert("Subtype", PdfObject::Name("Type0".to_string()));
        dict.insert(
            "DescendantFonts",
            PdfObject::Array(vec![PdfObject::Dictionary(descendant)]),
        );

        let err = validate_visual_font_metrics(&dict, None)
            .expect_err("visual rendering must not default malformed W2 triples");

        assert!(
            err.contains("CID font malformed /W2"),
            "unexpected W2 validation error: {err}"
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
