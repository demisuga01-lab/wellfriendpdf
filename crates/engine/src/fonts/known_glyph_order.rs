//! Glyph orders of well-known TrueType fonts.
//!
//! Some PDFs carry nothing but the glyph indexes of the TrueType font the text
//! was set in:
//!
//! - a non-embedded `CIDFontType2` font with an Identity encoding, whose CIDs
//!   are glyph indexes into the system font (pdf.js keeps a glyph map of the
//!   standard fonts for exactly this case), and
//! - a simple font converted from TrueType (Acrobat Distiller output for the
//!   Office fonts) whose `/Differences` names every glyph `g<index>`, with no
//!   ToUnicode for some of the subsets.
//!
//! The glyph index alone identifies the character once the font family's glyph
//! order is known. The tables come from the fonts' own `cmap`s, restricted to
//! the index range where the regular, bold and italic styles of a family agree.

#[path = "known_glyph_order_data.rs"]
mod data;

/// A font family whose TrueType glyph order is known, valid below `limit`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct KnownGlyphOrder {
    table: &'static [(u16, u16, u16)],
    limit: u16,
}

impl KnownGlyphOrder {
    /// The known glyph order of a PDF `/BaseFont` name, if any. A subset tag
    /// and spaces are ignored; the style suffix (`,Bold`, `-BoldItalicMT`, ...)
    /// must be a plain weight/slant so that a different design such as
    /// "Arial Narrow" or "Calibri Light" is never matched.
    pub(crate) fn for_base_font(name: &str) -> Option<Self> {
        let raw = name.trim_start_matches('/');
        let raw = raw.rsplit('+').next().unwrap_or(raw);
        let compact: String = raw
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .collect::<String>()
            .to_ascii_lowercase();
        let (family, style) = match compact.find([',', '-']) {
            Some(index) => (&compact[..index], &compact[index + 1..]),
            None => (compact.as_str(), ""),
        };
        let family = family
            .strip_suffix("psmt")
            .or_else(|| family.strip_suffix("mt"))
            .or_else(|| family.strip_suffix("ps"))
            .unwrap_or(family);
        let style = style
            .strip_suffix("mt")
            .or_else(|| style.strip_suffix("ps"))
            .unwrap_or(style);
        let italic = match style {
            "" | "regular" | "roman" | "bold" => false,
            "italic" | "oblique" | "bolditalic" | "boldoblique" => true,
            // "Arial-Black" names the Arial Black design.
            "black" if family == "arial" => {
                return Some(Self::new(data::ARIAL_BLACK, u16::MAX));
            }
            _ => return None,
        };
        Some(match family {
            "arialblack" => Self::new(data::ARIAL_BLACK, u16::MAX),
            "arial" | "timesnewroman" | "couriernew" => Self::new(data::WGL4, 774),
            "calibri" => Self::new(data::CALIBRI, 1300),
            "cambria" => Self::new(data::CAMBRIA, if italic { 820 } else { 977 }),
            _ => return None,
        })
    }

    fn new(table: &'static [(u16, u16, u16)], limit: u16) -> Self {
        Self { table, limit }
    }

    /// The character drawn by glyph `gid`, when the glyph order covers it.
    pub(crate) fn unicode(self, gid: u16) -> Option<char> {
        if gid >= self.limit {
            return None;
        }
        let index = match self.table.binary_search_by(|(first, _, _)| first.cmp(&gid)) {
            Ok(index) => index,
            Err(0) => return None,
            Err(index) => index - 1,
        };
        let (first, code, len) = self.table[index];
        let offset = gid - first;
        if offset >= len {
            return None;
        }
        char::from_u32(u32::from(code) + u32::from(offset))
    }
}

/// The glyph index named by a `g<decimal>` glyph name (`g3`, `g486`), the
/// convention of fonts converted from TrueType.
pub(crate) fn glyph_index_name(name: &str) -> Option<u16> {
    let digits = name.strip_prefix('g')?;
    if digits.is_empty() || digits.len() > 5 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// Unicode for a byte of Apple's Mac OS Arabic encoding, the encoding of a
/// TrueType `cmap` subtable for platform 1 (Macintosh), encoding 4 (Arabic).
/// The low half is ASCII.
pub(crate) fn mac_arabic_to_unicode(byte: u8) -> Option<char> {
    if byte < 0x80 {
        return Some(char::from(byte));
    }
    const HIGH: [u16; 128] = [
        0x00C4, 0x00A0, 0x00C7, 0x00C9, 0x00D1, 0x00D6, 0x00DC, 0x00E1, // 0x80
        0x00E0, 0x00E2, 0x00E4, 0x06BA, 0x00AB, 0x00E7, 0x00E9, 0x00E8, // 0x88
        0x00EA, 0x00EB, 0x00ED, 0x2026, 0x00EE, 0x00EF, 0x00F1, 0x00F3, // 0x90
        0x00BB, 0x00F4, 0x00F6, 0x00F7, 0x00FA, 0x00F9, 0x00FB, 0x00FC, // 0x98
        0x0020, 0x0021, 0x0022, 0x0023, 0x0024, 0x066A, 0x0026, 0x0027, // 0xA0
        0x0028, 0x0029, 0x002A, 0x002B, 0x060C, 0x002D, 0x002E, 0x002F, // 0xA8
        0x0660, 0x0661, 0x0662, 0x0663, 0x0664, 0x0665, 0x0666, 0x0667, // 0xB0
        0x0668, 0x0669, 0x003A, 0x061B, 0x003C, 0x003D, 0x003E, 0x061F, // 0xB8
        0x274A, 0x0621, 0x0622, 0x0623, 0x0624, 0x0625, 0x0626, 0x0627, // 0xC0
        0x0628, 0x0629, 0x062A, 0x062B, 0x062C, 0x062D, 0x062E, 0x062F, // 0xC8
        0x0630, 0x0631, 0x0632, 0x0633, 0x0634, 0x0635, 0x0636, 0x0637, // 0xD0
        0x0638, 0x0639, 0x063A, 0x005B, 0x005C, 0x005D, 0x005E, 0x005F, // 0xD8
        0x0640, 0x0641, 0x0642, 0x0643, 0x0644, 0x0645, 0x0646, 0x0647, // 0xE0
        0x0648, 0x0649, 0x064A, 0x064B, 0x064C, 0x064D, 0x064E, 0x064F, // 0xE8
        0x0650, 0x0651, 0x0652, 0x067E, 0x0679, 0x0686, 0x06D5, 0x06A4, // 0xF0
        0x06AF, 0x0688, 0x0691, 0x007B, 0x007C, 0x007D, 0x0698, 0x06D2, // 0xF8
    ];
    char::from_u32(u32::from(HIGH[usize::from(byte - 0x80)]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_font_names_select_their_family_order() {
        let arial = KnownGlyphOrder::for_base_font("/ArialMT").unwrap();
        assert_eq!(arial.unicode(3), Some(' '));
        assert_eq!(arial.unicode(36), Some('A'));
        assert_eq!(
            KnownGlyphOrder::for_base_font("TimesNewRomanPS-BoldItalicMT"),
            Some(arial)
        );
        assert_eq!(
            KnownGlyphOrder::for_base_font("Courier New,Bold"),
            Some(arial)
        );
        // Arial Black has its own order above the ASCII range.
        let black = KnownGlyphOrder::for_base_font("ArialBlack,Bold").unwrap();
        assert_eq!(black.unicode(0x53), Some('p'));
        assert_eq!(black.unicode(264), Some('\u{105}'));
        assert_eq!(black.unicode(291), Some('\u{15A}'));
        assert_eq!(KnownGlyphOrder::for_base_font("Arial-Black"), Some(black));
        let cambria = KnownGlyphOrder::for_base_font("EMMOLK+Cambria").unwrap();
        assert_eq!(cambria.unicode(18), Some('O'));
        assert_eq!(cambria.unicode(152), Some('v'));
        assert_eq!(cambria.unicode(3), Some(' '));
        assert_eq!(cambria.unicode(883), Some('1'));
        // Italic Cambria diverges from 820 on.
        let italic = KnownGlyphOrder::for_base_font("Cambria-Italic").unwrap();
        assert_eq!(italic.unicode(820), None);
        assert_eq!(italic.unicode(152), Some('v'));
        assert!(KnownGlyphOrder::for_base_font("Calibri-Bold").is_some());
        // Different designs and unknown fonts are not matched.
        assert_eq!(KnownGlyphOrder::for_base_font("ArialNarrow"), None);
        assert_eq!(KnownGlyphOrder::for_base_font("Calibri-Light"), None);
        assert_eq!(KnownGlyphOrder::for_base_font("CambriaMath"), None);
        assert_eq!(KnownGlyphOrder::for_base_font("Helvetica"), None);
    }

    #[test]
    fn glyph_index_names_and_mac_arabic() {
        assert_eq!(glyph_index_name("g3"), Some(3));
        assert_eq!(glyph_index_name("g486"), Some(486));
        assert_eq!(glyph_index_name("g"), None);
        assert_eq!(glyph_index_name("gab"), None);
        assert_eq!(glyph_index_name("uni0041"), None);
        assert_eq!(mac_arabic_to_unicode(b'A'), Some('A'));
        assert_eq!(mac_arabic_to_unicode(0xC7), Some('\u{627}'));
        assert_eq!(mac_arabic_to_unicode(0xE4), Some('\u{644}'));
        assert_eq!(mac_arabic_to_unicode(0xFF), Some('\u{6D2}'));
    }
}
