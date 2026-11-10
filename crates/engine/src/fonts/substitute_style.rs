//! The style a non-embedded PDF font asks for, and the synthetic bold and
//! italic needed when the substitute face chosen for it lacks that style.
//!
//! A PDF that does not embed its fonts still says what they look like: the
//! font descriptor carries `/FontWeight`, `/StemV`, `/ItalicAngle` and style
//! flags, and the base font name usually ends in a style (`Arial,Bold`,
//! `TimesNewRomanPS-BoldItalicMT`, `ArialBold`). This module reads all of
//! them the way PDFium does (`CPDF_Font::LoadFontDescriptor`,
//! `CPDF_Font::GetFontWeight`, `CFX_FontMapper::FindSubstFace`), then works
//! out how much to embolden and slant a substitute face that does not have
//! the requested weight or slant (`CFX_SubstFont`).

use crate::fonts::installed::FontLanguage;
use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;

/// Font descriptor flag bits (PDF 32000-1 table 123).
pub(crate) const FLAG_FIXED_PITCH: i64 = 1;
pub(crate) const FLAG_SERIF: i64 = 1 << 1;
pub(crate) const FLAG_SYMBOLIC: i64 = 1 << 2;
pub(crate) const FLAG_NONSYMBOLIC: i64 = 1 << 5;
pub(crate) const FLAG_ITALIC: i64 = 1 << 6;
pub(crate) const FLAG_ALL_CAP: i64 = 1 << 16;
pub(crate) const FLAG_FORCE_BOLD: i64 = 1 << 18;

const WEIGHT_NORMAL: u16 = 400;
const WEIGHT_BOLD: u16 = 700;
const WEIGHT_EXTRA_BOLD: u16 = 800;

/// The style a font name announces by itself.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct NameStyle {
    /// The weight the name states (`Bold` 700, `Bold` twice 800, `Black`
    /// 900, `Semibold` 600, `Light` 300...), `None` when it states none.
    pub weight: Option<u16>,
    /// The name says italic or oblique.
    pub italic: bool,
}

impl NameStyle {
    pub(crate) fn is_bold(&self) -> bool {
        self.weight.is_some_and(|weight| weight >= 600)
    }
}

/// Strip a leading `/`, a vertical-font `@`, and a subset tag (`ABCDEF+`).
fn strip_name_decorations(name: &str) -> &str {
    let name = name.trim().trim_start_matches('/').trim_start_matches('@');
    match name.find('+') {
        Some(index) => &name[index + 1..],
        None => name,
    }
}

/// Read the style from a PDF font name, like PDFium's `FindSubstFace`:
/// the part after a comma (`Arial,BoldItalic`) or after a hyphen
/// (`Helvetica-BoldOblique`, `Arial-BoldMT`), or else a style word at the
/// very end of the name (`ArialBold`, `CourierNewItalic`). A name that is
/// bold twice (`ArialBold,Bold`) is extra bold.
///
/// Unlike the old "ends with b" heuristics, a family that merely ends in the
/// letters of a style word (`Kobold`, `DFKai-SB`, `Webdings`) is not styled.
pub(crate) fn parse_name_style(name: &str) -> NameStyle {
    let name = strip_name_decorations(name);
    let (family, style_part) = if let Some(index) = name.find(',') {
        (&name[..index], Some(&name[index + 1..]))
    } else if let Some(index) = name.find('-') {
        (&name[..index], Some(&name[index + 1..]))
    } else {
        (name, None)
    };

    let mut style = NameStyle::default();
    let mut bold_count = 0u8;

    // A style word glued to the end of the family name. Case matters here:
    // `ArialBold` is bold, `Kobold` is not.
    for (suffix, weight, italic) in [
        ("BoldItalic", WEIGHT_BOLD, true),
        ("BoldOblique", WEIGHT_BOLD, true),
        ("Bold", WEIGHT_BOLD, false),
        ("Black", 900, false),
        ("Heavy", 900, false),
        ("Italic", 0, true),
        ("Oblique", 0, true),
    ] {
        if family.len() > suffix.len() && family.ends_with(suffix) {
            if weight == WEIGHT_BOLD {
                bold_count += 1;
            } else if weight > 0 {
                style.weight = Some(weight);
            }
            style.italic |= italic;
            break;
        }
    }

    if let Some(style_part) = style_part {
        for token in style_part
            .split([',', '-', ' ', '_'])
            .filter(|token| !token.is_empty())
        {
            let token_style = style_token(token);
            if let Some(weight) = token_style.weight {
                if weight == WEIGHT_BOLD {
                    bold_count += 1;
                } else {
                    style.weight = Some(style.weight.map_or(weight, |known| known.max(weight)));
                }
            }
            style.italic |= token_style.italic;
        }
    }

    let bold_weight = match bold_count {
        0 => None,
        1 => Some(WEIGHT_BOLD),
        _ => Some(WEIGHT_EXTRA_BOLD),
    };
    style.weight = match (style.weight, bold_weight) {
        (Some(named), Some(bold)) => Some(named.max(bold)),
        (named, bold) => named.or(bold),
    };
    style
}

/// The style of one style token after a comma or hyphen. Tokens such as
/// `BoldItalicMT` or `SemiboldIt` combine several words.
fn style_token(token: &str) -> NameStyle {
    let lower = token.to_ascii_lowercase();
    let mut style = NameStyle::default();
    // Whole-token abbreviations.
    match lower.as_str() {
        "b" | "bd" | "bol" => {
            style.weight = Some(WEIGHT_BOLD);
            return style;
        }
        "i" | "it" | "o" | "obl" | "ital" => {
            style.italic = true;
            return style;
        }
        "bi" | "bdit" | "boldit" => {
            style.weight = Some(WEIGHT_BOLD);
            style.italic = true;
            return style;
        }
        _ => {}
    }
    style.italic = lower.contains("italic")
        || lower.contains("oblique")
        || lower.contains("slanted")
        || lower.contains("inclined")
        || lower.contains("kursiv")
        || (lower.ends_with("it") && lower.len() > 2 && !lower.ends_with("bit"));
    style.weight = if lower.contains("extrabold")
        || lower.contains("ultrabold")
        || lower.contains("heavy")
        || lower.contains("black")
    {
        Some(if lower.contains("heavy") || lower.contains("black") {
            900
        } else {
            WEIGHT_EXTRA_BOLD
        })
    } else if lower.contains("semibold") || lower.contains("demibold") || lower.contains("demi") {
        Some(600)
    } else if lower.contains("bold") {
        Some(WEIGHT_BOLD)
    } else if lower.contains("medium") {
        Some(500)
    } else if lower.contains("extralight") || lower.contains("ultralight") {
        Some(200)
    } else if lower.contains("light") {
        Some(300)
    } else if lower.contains("thin") || lower.contains("hairline") {
        Some(100)
    } else {
        None
    };
    style
}

/// Weight estimated from a descriptor's `/StemV`, as PDFium's
/// `CPDF_Font::GetFontWeight` does: `StemV * 5` below 140, otherwise
/// `StemV * 4 + 140`.
pub(crate) fn weight_from_stem_v(stem_v: i64) -> Option<i64> {
    if stem_v < 140 {
        stem_v.checked_mul(5)
    } else {
        stem_v.checked_mul(4)?.checked_add(140)
    }
}

/// The style a PDF font requests from its descriptor, `/Widths` and name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RequestedFontStyle {
    /// Requested weight, 100..=900 (400 when nothing says otherwise).
    pub weight: u16,
    /// Italic from the Italic flag, a negative `/ItalicAngle`, or the name.
    pub italic: bool,
    /// The descriptor's `/ItalicAngle` in whole degrees when negative,
    /// otherwise 0 (a default slant is then used for italic).
    pub italic_angle: i32,
    /// FixedPitch flag, or every non-zero `/Widths` entry is the same.
    pub fixed_pitch: bool,
    /// Serif flag.
    pub serif: bool,
    /// Symbolic flag without the Nonsymbolic flag.
    pub symbolic: bool,
    /// AllCap flag: lowercase codes draw the uppercase glyphs.
    pub all_cap: bool,
}

impl Default for RequestedFontStyle {
    fn default() -> Self {
        Self {
            weight: WEIGHT_NORMAL,
            italic: false,
            italic_angle: 0,
            fixed_pitch: false,
            serif: false,
            symbolic: false,
            all_cap: false,
        }
    }
}

impl RequestedFontStyle {
    /// Whether a bold face should be chosen for this request.
    pub(crate) fn wants_bold_face(&self) -> bool {
        self.weight >= 600
    }

    /// Read the requested style of `font_dict` (for a Type0 font, its
    /// descendant's descriptor) under the base font name `name`. `cjk` is set
    /// for composite CJK fonts: like PDFium, their weight comes from the name,
    /// the flags and an explicit `/FontWeight`, never from `/StemV`.
    pub(crate) fn from_font(
        name: &str,
        font_dict: Option<&PdfDictionary>,
        reader: Option<&PdfReader>,
        cjk: bool,
    ) -> Self {
        let mut style = Self::default();
        let name_style = parse_name_style(name);
        if let Some(font_dict) = font_dict {
            if let Some(descriptor) = font_descriptor(font_dict, reader) {
                style.apply_descriptor(&descriptor, reader, cjk);
            }
            if !style.fixed_pitch && widths_are_uniform(font_dict, reader) {
                style.fixed_pitch = true;
            }
        }
        if let Some(weight) = name_style.weight.filter(|weight| *weight > WEIGHT_NORMAL) {
            style.weight = style.weight.max(weight);
        }
        style.italic |= name_style.italic;
        style
    }

    fn apply_descriptor(
        &mut self,
        descriptor: &PdfDictionary,
        reader: Option<&PdfReader>,
        cjk: bool,
    ) {
        let number = |key: &str| -> Option<f64> {
            let value = descriptor.get(key)?;
            let number = match value {
                PdfObject::Reference { .. } => reader?.resolve(value.clone()).ok()?.as_number(),
                other => other.as_number(),
            };
            number.filter(|value| value.is_finite())
        };
        let flags = number("Flags").map_or(0, |flags| flags as i64);
        self.fixed_pitch = flags & FLAG_FIXED_PITCH != 0;
        self.serif = flags & FLAG_SERIF != 0;
        self.symbolic = flags & FLAG_SYMBOLIC != 0 && flags & FLAG_NONSYMBOLIC == 0;
        self.all_cap = flags & FLAG_ALL_CAP != 0;
        self.italic = flags & FLAG_ITALIC != 0;

        let italic_angle = number("ItalicAngle");
        if let Some(angle) = italic_angle.filter(|angle| *angle < 0.0) {
            self.italic = true;
            self.italic_angle = angle.round().max(-90.0) as i32;
        }

        let font_weight = number("FontWeight")
            .map(|weight| weight.round() as i64)
            .filter(|weight| *weight > 0);
        let stem_v = number("StemV").map(|stem_v| stem_v.round() as i64);
        // PDFium trusts the descriptor's metrics (and so its StemV) only
        // when it is complete: ItalicAngle, Ascent, Descent, CapHeight and
        // StemV or FontWeight (`kFontUseExternAttr`).
        let complete = italic_angle.is_some()
            && number("Ascent").is_some()
            && number("Descent").is_some()
            && number("CapHeight").is_some()
            && (stem_v.is_some() || font_weight.is_some());
        let weight = font_weight.or_else(|| {
            stem_v
                .filter(|stem_v| *stem_v > 0 && complete && !cjk)
                .and_then(weight_from_stem_v)
        });
        if let Some(weight) = weight.filter(|weight| (100..=900).contains(weight)) {
            self.weight = weight as u16;
        }
        if flags & FLAG_FORCE_BOLD != 0 {
            self.weight = self.weight.max(WEIGHT_BOLD);
        }
    }
}

/// The font descriptor of a simple font, or of a Type0 font's descendant.
fn font_descriptor(font_dict: &PdfDictionary, reader: Option<&PdfReader>) -> Option<PdfDictionary> {
    let resolve_dict = |object: &PdfObject| -> Option<PdfDictionary> {
        match object {
            PdfObject::Dictionary(dict) => Some(dict.clone()),
            PdfObject::Reference { .. } => match reader?.resolve(object.clone()).ok()? {
                PdfObject::Dictionary(dict) => Some(dict),
                _ => None,
            },
            _ => None,
        }
    };
    if font_dict.get_name("Subtype") == Some("Type0") {
        let descendants = match font_dict.get("DescendantFonts")? {
            PdfObject::Array(items) => items.clone(),
            other @ PdfObject::Reference { .. } => match reader?.resolve(other.clone()).ok()? {
                PdfObject::Array(items) => items,
                _ => return None,
            },
            _ => return None,
        };
        let descendant = resolve_dict(descendants.first()?)?;
        return resolve_dict(descendant.get("FontDescriptor")?);
    }
    resolve_dict(font_dict.get("FontDescriptor")?)
}

/// True when a simple font's `/Widths` has at least two non-zero entries and
/// all of them are equal: the font is monospaced whatever its flags say
/// (PDFium's `CPDF_FaceBasedSimpleFont::LoadSubstFont`).
fn widths_are_uniform(font_dict: &PdfDictionary, reader: Option<&PdfReader>) -> bool {
    if font_dict.get_name("Subtype") == Some("Type0") {
        return false;
    }
    let Some(widths) = font_dict.get("Widths") else {
        return false;
    };
    let widths = match widths {
        PdfObject::Array(items) => items.clone(),
        PdfObject::Reference { .. } => {
            match reader.and_then(|reader| reader.resolve(widths.clone()).ok()) {
                Some(PdfObject::Array(items)) => items,
                _ => return false,
            }
        }
        _ => return false,
    };
    let mut first = None;
    let mut count = 0usize;
    for width in &widths {
        let width = match width {
            PdfObject::Reference { .. } => reader
                .and_then(|reader| reader.resolve(width.clone()).ok())
                .and_then(|value| value.as_number()),
            other => other.as_number(),
        };
        let Some(width) = width.filter(|width| width.is_finite() && *width != 0.0) else {
            continue;
        };
        match first {
            None => first = Some(width),
            Some(known) if (known - width).abs() > 0.5 => return false,
            Some(_) => {}
        }
        count += 1;
    }
    count >= 2
}

/// The weight and slant a substitute face actually has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FaceStyle {
    pub weight: u16,
    pub italic: bool,
}

impl FaceStyle {
    pub(crate) const REGULAR: Self = Self {
        weight: WEIGHT_NORMAL,
        italic: false,
    };

    /// Read the style of an sfnt program (OS/2 weight class, OS/2 and `head`
    /// style bits, `post` italic angle). Other programs (bare CFF, Type 1)
    /// are taken as regular upright faces.
    pub(crate) fn of_program(bytes: &[u8]) -> Self {
        let Ok(face) = ttf_parser::Face::parse(bytes, 0) else {
            return Self::REGULAR;
        };
        let mut weight = face.weight().to_number();
        // Some old fonts store the weight class on a 1..9 scale.
        if (1..=9).contains(&weight) {
            weight *= 100;
        }
        if !(100..=1000).contains(&weight) {
            weight = if face.is_bold() {
                WEIGHT_BOLD
            } else {
                WEIGHT_NORMAL
            };
        }
        let italic = face.is_italic()
            || face.is_oblique()
            || face.italic_angle().is_finite() && face.italic_angle() < -0.5;
        Self {
            weight: weight.min(900),
            italic,
        }
    }
}

/// How much to embolden a substitute glyph, from PDFium's
/// `kWeightPow11` (Latin, rendering) and `kWeightPowShiftJis` (Japanese)
/// tables, indexed by `(weight - 400) / 10`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub(crate) enum EmboldenTable {
    #[default]
    Latin,
    ShiftJis,
}

// The two tables below and `ANGLE_SKEW` are PDFium's (core/fxge/cfx_substfont.cpp,
// Copyright 2016 The PDFium Authors, BSD-3-Clause; original code copyright
// 2014 Foxit Software Inc.).
const WEIGHT_POW_11: [u8; 100] = [
    0, 4, 7, 8, 9, 10, 12, 13, 15, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 28, 29, 30, 31, 32, 33,
    34, 35, 36, 37, 39, 39, 40, 40, 41, 41, 41, 42, 42, 42, 43, 43, 43, 44, 44, 44, 45, 45, 45, 46,
    46, 46, 46, 43, 47, 47, 48, 48, 48, 48, 45, 50, 50, 50, 46, 51, 51, 51, 52, 52, 52, 52, 53, 53,
    53, 53, 53, 54, 54, 54, 54, 55, 55, 55, 55, 55, 56, 56, 56, 56, 56, 57, 57, 57, 57, 57, 58, 58,
    58, 58, 58,
];

const WEIGHT_POW_SHIFT_JIS: [u8; 100] = [
    0, 0, 2, 4, 6, 8, 10, 14, 16, 20, 22, 26, 28, 32, 34, 38, 42, 44, 48, 52, 56, 60, 64, 66, 70,
    74, 78, 82, 86, 90, 96, 96, 96, 96, 98, 98, 98, 100, 100, 100, 100, 102, 102, 102, 102, 104,
    104, 104, 104, 104, 106, 106, 106, 106, 106, 108, 108, 108, 108, 108, 110, 110, 110, 110, 110,
    112, 112, 112, 112, 112, 112, 114, 114, 114, 114, 114, 114, 114, 116, 116, 116, 116, 116, 116,
    116, 118, 118, 118, 118, 118, 118, 118, 120, 120, 120, 120, 120, 120, 120, 120,
];

/// Skew (in hundredths) for italic angles of 0..=29 degrees.
const ANGLE_SKEW: [i8; 30] = [
    0, -2, -3, -5, -7, -9, -11, -12, -14, -16, -18, -19, -21, -23, -25, -27, -29, -31, -32, -34,
    -36, -38, -40, -42, -45, -47, -49, -51, -53, -55,
];

/// The default slant of a synthetic italic, in degrees (PDFium's -12).
const DEFAULT_ITALIC_ANGLE: i32 = 12;
/// The fixed slant of a synthetic CJK italic, in degrees.
const CJK_ITALIC_ANGLE: i32 = 15;

/// Skew (hundredths, non-positive) for an italic angle in degrees.
fn skew_for_angle(degrees: i32) -> i8 {
    let index = degrees.unsigned_abs() as usize;
    ANGLE_SKEW.get(index).copied().unwrap_or(-58)
}

/// Synthetic emboldening and slanting applied to a substitute glyph outline.
/// The default value applies nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub(crate) struct SyntheticStyle {
    /// Effective emboldening weight (> 400), or 0 for none.
    pub weight: u16,
    /// Skew in hundredths (PDFium's `GetSkew`), non-positive; 0 for none.
    pub skew: i8,
    pub table: EmboldenTable,
}

impl SyntheticStyle {
    pub(crate) const NONE: Self = Self {
        weight: 0,
        skew: 0,
        table: EmboldenTable::Latin,
    };

    pub(crate) fn is_none(&self) -> bool {
        self.weight <= WEIGHT_NORMAL && self.skew == 0
    }

    /// The synthetic style for a substitute face of style `face` standing in
    /// for a font that requests `requested`. `language` is the CJK language
    /// of a composite font, which gets a fixed 15 degree slant and, for
    /// Japanese, PDFium's stronger Shift-JIS emboldening.
    pub(crate) fn for_substitute(
        requested: &RequestedFontStyle,
        face: FaceStyle,
        language: Option<FontLanguage>,
    ) -> Self {
        // A face lighter than requested is thickened by the difference, so a
        // regular face drawn for weight 700 gets PDFium's weight-700 stroke
        // and a bold face drawn for 900 gets a little more.
        let weight = if requested.weight > face.weight {
            WEIGHT_NORMAL + (requested.weight - face.weight)
        } else {
            0
        };
        let weight = if weight > WEIGHT_NORMAL { weight } else { 0 };
        let skew = if requested.italic && !face.italic {
            if language.is_some() {
                skew_for_angle(CJK_ITALIC_ANGLE)
            } else {
                match requested.italic_angle.unsigned_abs() as i32 {
                    0 => skew_for_angle(DEFAULT_ITALIC_ANGLE),
                    // PDFium draws a font that claims a slant below five
                    // degrees upright.
                    angle if angle < 5 => 0,
                    angle => skew_for_angle(angle),
                }
            }
        } else {
            0
        };
        Self {
            weight,
            skew,
            table: if language == Some(FontLanguage::Japanese) {
                EmboldenTable::ShiftJis
            } else {
                EmboldenTable::Latin
            },
        }
    }

    /// Horizontal shear factor (x += shear * y in glyph space).
    pub(crate) fn shear(&self) -> f64 {
        -f64::from(self.skew) / 100.0
    }

    /// How much wider and taller the glyph becomes, as a fraction of the em.
    /// PDFium thickens the device outline by `level / 64` pixels with
    /// `level = table[(weight - 400) / 10] * (|xx| + |xy|) / 36655` and
    /// `xx = em_pixels * 1024`, so the em fraction is `table * 1024 / 36655 / 64`
    /// times `1 + shear` (the `|xy|` term of a slanted glyph).
    pub(crate) fn embolden_fraction(&self) -> f64 {
        if self.weight <= WEIGHT_NORMAL {
            return 0.0;
        }
        let index = usize::from((self.weight - WEIGHT_NORMAL) / 10).min(99);
        let level = match self.table {
            EmboldenTable::Latin => WEIGHT_POW_11[index],
            EmboldenTable::ShiftJis => WEIGHT_POW_SHIFT_JIS[index],
        };
        f64::from(level) * 1024.0 / 36655.0 / 64.0 * (1.0 + self.shear().abs())
    }

    /// A stable hash input for glyph caches.
    pub(crate) fn cache_bytes(&self) -> [u8; 4] {
        let [w0, w1] = self.weight.to_le_bytes();
        [
            w0,
            w1,
            self.skew as u8,
            match self.table {
                EmboldenTable::Latin => 0,
                EmboldenTable::ShiftJis => 1,
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style(name: &str) -> (Option<u16>, bool) {
        let parsed = parse_name_style(name);
        (parsed.weight, parsed.italic)
    }

    #[test]
    fn styles_are_read_from_comma_hyphen_and_suffix_names() {
        assert_eq!(style("Arial,Bold"), (Some(700), false));
        assert_eq!(style("Arial,BoldItalic"), (Some(700), true));
        assert_eq!(style("Arial,Italic"), (None, true));
        assert_eq!(
            style("ABCDEF+TimesNewRomanPS-BoldItalicMT"),
            (Some(700), true)
        );
        assert_eq!(style("Arial-BoldMT"), (Some(700), false));
        assert_eq!(style("Helvetica-BoldOblique"), (Some(700), true));
        assert_eq!(style("Helvetica-Bold-Italic"), (Some(700), true));
        assert_eq!(style("ArialBold"), (Some(700), false));
        assert_eq!(style("CourierNewItalic"), (None, true));
        assert_eq!(style("ArialBoldItalic"), (Some(700), true));
        assert_eq!(style("ArialBlack"), (Some(900), false));
        assert_eq!(style("Arial-Black"), (Some(900), false));
        assert_eq!(style("MyriadPro-Semibold"), (Some(600), false));
        assert_eq!(style("MyriadPro-SemiboldIt"), (Some(600), true));
        assert_eq!(style("Times-Roman"), (None, false));
        assert_eq!(style("Arial,Regular"), (None, false));
        assert_eq!(style("Helvetica-Light"), (Some(300), false));
    }

    #[test]
    fn bold_twice_is_extra_bold() {
        assert_eq!(style("ArialBold,Bold"), (Some(800), false));
        assert_eq!(style("Arial,Bold,Bold"), (Some(800), false));
    }

    #[test]
    fn family_names_ending_in_style_letters_are_not_styled() {
        for name in [
            "Kobold",
            "DFKai-SB",
            "Webdings",
            "Gill-Book",
            "Arial",
            "Verdanab",
        ] {
            assert_eq!(style(name), (None, false), "{name}");
        }
        // Case matters for a style glued to the family name.
        assert_eq!(style("Arialbold"), (None, false));
    }

    #[test]
    fn stem_v_estimates_weight_like_pdfium() {
        assert_eq!(weight_from_stem_v(80), Some(400));
        assert_eq!(weight_from_stem_v(139), Some(695));
        assert_eq!(weight_from_stem_v(140), Some(700));
        assert_eq!(weight_from_stem_v(165), Some(800));
    }

    fn descriptor(entries: &[(&str, PdfObject)]) -> PdfDictionary {
        let mut dict = PdfDictionary::empty();
        for (key, value) in entries {
            dict.insert(*key, value.clone());
        }
        dict
    }

    fn font_with(descriptor: PdfDictionary, widths: Option<Vec<i64>>) -> PdfDictionary {
        let mut font = PdfDictionary::empty();
        font.insert("Subtype", PdfObject::Name("TrueType".into()));
        font.insert("FontDescriptor", PdfObject::Dictionary(descriptor));
        if let Some(widths) = widths {
            font.insert(
                "Widths",
                PdfObject::Array(widths.into_iter().map(PdfObject::Integer).collect()),
            );
        }
        font
    }

    #[test]
    fn descriptor_italic_angle_weight_and_stem_v_are_read() {
        let complete = |extra: &[(&str, PdfObject)]| {
            let mut entries = vec![
                ("Flags", PdfObject::Integer(32)),
                ("ItalicAngle", PdfObject::Integer(0)),
                ("Ascent", PdfObject::Integer(900)),
                ("Descent", PdfObject::Integer(-200)),
                ("CapHeight", PdfObject::Integer(700)),
            ];
            entries.extend_from_slice(extra);
            font_with(descriptor(&entries), None)
        };
        let stem = complete(&[("StemV", PdfObject::Integer(140))]);
        let requested = RequestedFontStyle::from_font("Foo", Some(&stem), None, false);
        assert_eq!(requested.weight, 700);
        assert!(requested.wants_bold_face());
        assert!(!requested.italic);

        // An explicit FontWeight beats StemV.
        let weighted = complete(&[
            ("StemV", PdfObject::Integer(140)),
            ("FontWeight", PdfObject::Integer(500)),
        ]);
        let requested = RequestedFontStyle::from_font("Foo", Some(&weighted), None, false);
        assert_eq!(requested.weight, 500);

        // CJK fonts ignore StemV.
        let requested = RequestedFontStyle::from_font("Foo", Some(&stem), None, true);
        assert_eq!(requested.weight, 400);

        // StemV of an incomplete descriptor is not trusted.
        let incomplete = font_with(
            descriptor(&[
                ("Flags", PdfObject::Integer(32)),
                ("StemV", PdfObject::Integer(160)),
            ]),
            None,
        );
        let requested = RequestedFontStyle::from_font("Foo", Some(&incomplete), None, false);
        assert_eq!(requested.weight, 400);

        let slanted = complete(&[("ItalicAngle", PdfObject::Real(-11.5))]);
        let requested = RequestedFontStyle::from_font("Foo", Some(&slanted), None, false);
        assert!(requested.italic);
        assert_eq!(requested.italic_angle, -12);

        let named = RequestedFontStyle::from_font("Foo,BoldItalic", None, None, false);
        assert_eq!(named.weight, 700);
        assert!(named.italic);
    }

    #[test]
    fn flags_and_uniform_widths_are_read() {
        let flags = FLAG_SYMBOLIC | FLAG_ALL_CAP | FLAG_FORCE_BOLD | FLAG_SERIF;
        let font = font_with(
            descriptor(&[("Flags", PdfObject::Integer(flags))]),
            Some(vec![600, 0, 600, 600]),
        );
        let requested = RequestedFontStyle::from_font("Foo", Some(&font), None, false);
        assert!(requested.symbolic);
        assert!(requested.all_cap);
        assert!(requested.serif);
        assert!(requested.fixed_pitch);
        assert_eq!(requested.weight, 700);

        let proportional = font_with(
            descriptor(&[("Flags", PdfObject::Integer(32))]),
            Some(vec![600, 500, 600]),
        );
        let requested = RequestedFontStyle::from_font("Foo", Some(&proportional), None, false);
        assert!(!requested.fixed_pitch);
        assert!(!requested.symbolic);

        // A single width says nothing about the pitch.
        let single = font_with(
            descriptor(&[("Flags", PdfObject::Integer(32))]),
            Some(vec![0, 600, 0]),
        );
        assert!(!RequestedFontStyle::from_font("Foo", Some(&single), None, false).fixed_pitch);
    }

    #[test]
    fn synthetic_style_covers_only_what_the_face_lacks() {
        let bold_italic = RequestedFontStyle {
            weight: 700,
            italic: true,
            ..RequestedFontStyle::default()
        };
        let synthetic = SyntheticStyle::for_substitute(&bold_italic, FaceStyle::REGULAR, None);
        assert_eq!(synthetic.weight, 700);
        assert_eq!(synthetic.skew, -21);
        assert!((synthetic.shear() - 0.21).abs() < 1e-9);
        // About 1.7% of the em at weight 700 (upright), 2.0% at 900.
        let upright = SyntheticStyle {
            skew: 0,
            ..synthetic
        };
        assert!((upright.embolden_fraction() - 0.0170).abs() < 0.0005);
        let black = SyntheticStyle {
            weight: 900,
            ..upright
        };
        assert!((black.embolden_fraction() - 0.0201).abs() < 0.0005);

        let bold_face = FaceStyle {
            weight: 700,
            italic: true,
        };
        assert!(SyntheticStyle::for_substitute(&bold_italic, bold_face, None).is_none());

        // CJK: fixed 15 degree slant (skew 27) and Shift-JIS emboldening.
        let cjk = SyntheticStyle::for_substitute(
            &bold_italic,
            FaceStyle::REGULAR,
            Some(FontLanguage::Japanese),
        );
        assert_eq!(cjk.skew, -27);
        assert_eq!(cjk.table, EmboldenTable::ShiftJis);
        assert!(cjk.embolden_fraction() > upright.embolden_fraction() * 2.0);

        // A tiny descriptor slant is drawn upright; a large one is honoured.
        let mut slight = bold_italic;
        slight.weight = 400;
        slight.italic_angle = -3;
        assert!(SyntheticStyle::for_substitute(&slight, FaceStyle::REGULAR, None).is_none());
        slight.italic_angle = -20;
        assert_eq!(
            SyntheticStyle::for_substitute(&slight, FaceStyle::REGULAR, None).skew,
            -36
        );
    }

    #[test]
    fn face_style_is_read_from_the_program() {
        let regular = crate::render::font_rasterizer::fallback_fonts::LIBERATION_SANS_REGULAR;
        let bold_italic =
            crate::render::font_rasterizer::fallback_fonts::LIBERATION_SANS_BOLD_ITALIC;
        assert_eq!(FaceStyle::of_program(regular), FaceStyle::REGULAR);
        assert_eq!(
            FaceStyle::of_program(bold_italic),
            FaceStyle {
                weight: 700,
                italic: true
            }
        );
        assert_eq!(FaceStyle::of_program(b"not a font"), FaceStyle::REGULAR);
    }
}
