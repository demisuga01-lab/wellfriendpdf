//! Shared text decoding for the renderers.
//!
//! Turns a content-stream text string into a sequence of [`DecodedGlyph`]s
//! (code, unicode, advance width, is-gid, is-space), exactly as the raster
//! renderer does, so the SVG renderer shows the same glyphs with the same
//! advances. The raster renderer keeps its own equivalent code paths untouched
//! (to guarantee no raster regression); this module is the version the vector
//! renderer uses.

use crate::engine::PageResources;
use crate::fonts::cid::{cid_font_has_embedded_program, cid_to_gid};
use crate::fonts::resolver::{detect_font_subtype, get_descendant_font, FontSubtype};
use crate::fonts::FontResolver;
use crate::object::PdfDictionary;
use crate::reader::PdfReader;
use crate::render::font_rasterizer::{get_fallback_font, FontRasterizer};

/// One decoded glyph ready to be shown.
#[derive(Debug, Clone)]
pub struct DecodedGlyph {
    /// Character code (simple fonts) or glyph id (CID fonts, when `is_gid`).
    pub code: u16,
    /// Unicode character (for fallback/lookup).
    pub unicode: char,
    /// PDF simple-font glyph name resolved from Encoding/Differences.
    pub glyph_name: Option<String>,
    /// Whether this is a space code (affects word spacing).
    pub is_space: bool,
    /// Explicit advance width in 1/1000 text units, when known from the PDF.
    pub width: Option<f64>,
    /// Whether `code` is a glyph id (CID fonts) rather than a char code.
    pub is_gid: bool,
    /// Whether the font's CMap declares vertical writing mode.
    pub is_vertical: bool,
    /// Vertical displacement `(w1y)` in 1/1000 text units for Type0 vertical
    /// fonts. Negative values advance downward, per PDF vertical metrics.
    pub vertical_advance: Option<f64>,
    /// Vertical origin vector `(v_x, v_y)` in 1/1000 text units when available.
    pub vertical_origin: Option<(f64, f64)>,
}

/// Decode a text string under `font_name`'s font into glyphs, resolving the
/// font from `resources`.
pub fn decode_text_bytes(
    bytes: &[u8],
    font_name: &str,
    resources: &PageResources,
    reader: &PdfReader,
) -> Vec<DecodedGlyph> {
    let Some(font_dict) = resources.fonts.get(font_name) else {
        return latin1_glyphs(bytes);
    };
    let resolver = FontResolver::new(font_dict, reader);
    decode_text_bytes_with_resolver(bytes, font_dict, &resolver, reader)
}

/// Decode a text string using a caller-owned resolver cache.
pub fn decode_text_bytes_with_resolver(
    bytes: &[u8],
    font_dict: &PdfDictionary,
    resolver: &FontResolver,
    reader: &PdfReader,
) -> Vec<DecodedGlyph> {
    if detect_font_subtype(font_dict) == FontSubtype::Type0 {
        return decode_type0_text_with_resolver(bytes, font_dict, resolver, reader);
    }

    let has_pdf_widths =
        font_dict.get_array("Widths").is_some() || resolver.has_standard14_metrics();
    let mut glyphs = Vec::new();
    let code_size = resolver.code_size().max(1);
    let mut idx = 0usize;
    while idx < bytes.len() {
        let code = if code_size == 2 {
            let high = bytes[idx];
            let low = bytes.get(idx + 1).copied().unwrap_or(0);
            idx = idx.saturating_add(2);
            (u16::from(high) << 8) | u16::from(low)
        } else {
            let code = u16::from(bytes[idx]);
            idx = idx.saturating_add(1);
            code
        };
        let text = resolver.decode_char(code);
        let ch = text.chars().next().unwrap_or('\u{FFFD}');
        let glyph_name = resolver.glyph_name(code).map(str::to_string);
        let width = if has_pdf_widths {
            let width = resolver.glyph_width(code).max(0.0);
            (width > 0.0).then_some(width)
        } else {
            None
        };
        glyphs.push(DecodedGlyph {
            code,
            unicode: ch,
            glyph_name,
            is_space: resolver.is_space_code(code) || ch == ' ',
            width,
            is_gid: false,
            is_vertical: false,
            vertical_advance: None,
            vertical_origin: None,
        });
    }
    glyphs
}

fn decode_type0_text_with_resolver(
    bytes: &[u8],
    font_dict: &PdfDictionary,
    resolver: &FontResolver,
    reader: &PdfReader,
) -> Vec<DecodedGlyph> {
    let descendant_font = get_descendant_font(font_dict, reader);
    let render_as_gid = cid_font_has_embedded_program(descendant_font.as_ref(), reader);
    let mut glyphs = Vec::new();
    let mut idx = 0usize;
    let code_size = resolver.code_size().max(1);

    while idx < bytes.len() {
        let cid = if code_size == 2 {
            let high = bytes[idx];
            let low = bytes.get(idx + 1).copied().unwrap_or(0);
            idx = idx.saturating_add(2);
            (u16::from(high) << 8) | u16::from(low)
        } else {
            let code = u16::from(bytes[idx]);
            idx = idx.saturating_add(1);
            code
        };

        let text = resolver.decode_char(cid);
        let unicode = text.chars().next().unwrap_or('\u{FFFD}');
        let width = if render_as_gid {
            Some(resolver.glyph_width(cid)).filter(|width| *width > 0.0)
        } else {
            None
        };
        let code = if render_as_gid {
            cid_to_gid(cid, descendant_font.as_ref(), reader)
        } else {
            u16::try_from(unicode as u32).unwrap_or(cid)
        };
        let (vertical_advance, vertical_origin) = if resolver.is_vertical() {
            let (w1y, vx, vy) = resolver.vertical_metrics(cid);
            (Some(w1y), Some((vx, vy)))
        } else {
            (None, None)
        };

        glyphs.push(DecodedGlyph {
            code,
            unicode,
            glyph_name: None,
            is_space: resolver.is_space_code(cid) || unicode == ' ',
            width,
            is_gid: render_as_gid,
            is_vertical: resolver.is_vertical(),
            vertical_advance,
            vertical_origin,
        });
    }
    glyphs
}

/// Resolve the embedded (or fallback) font program bytes for a font name.
pub fn get_font_bytes(
    font_name: &str,
    resources: &PageResources,
    reader: &PdfReader,
) -> Option<Vec<u8>> {
    if let Some(font_dict) = resources.fonts.get(font_name) {
        if let Some(bytes) = FontRasterizer::extract_font_bytes(font_dict, reader) {
            if !bytes.is_empty() {
                return Some(bytes);
            }
        }
        if detect_font_subtype(font_dict) == FontSubtype::Type0 {
            if let Some(descendant_font) = get_descendant_font(font_dict, reader) {
                if let Some(bytes) = FontRasterizer::extract_font_bytes(&descendant_font, reader) {
                    if !bytes.is_empty() {
                        return Some(bytes);
                    }
                }
            }
        }
    }
    get_fallback_font(font_name).map(|bytes| bytes.to_vec())
}

fn latin1_glyphs(bytes: &[u8]) -> Vec<DecodedGlyph> {
    bytes
        .iter()
        .map(|byte| DecodedGlyph {
            code: u16::from(*byte),
            unicode: decode_win_ansi(*byte),
            glyph_name: None,
            is_space: *byte == b' ',
            width: None,
            is_gid: false,
            is_vertical: false,
            vertical_advance: None,
            vertical_origin: None,
        })
        .collect()
}

/// WinAnsi high-byte decoding (matches the raster renderer's fallback table for
/// the printable C1 range; other bytes pass through as Latin-1).
fn decode_win_ansi(byte: u8) -> char {
    match byte {
        0x80 => '€',
        0x82 => '‚',
        0x83 => 'ƒ',
        0x84 => '„',
        0x85 => '…',
        0x86 => '†',
        0x87 => '‡',
        0x88 => 'ˆ',
        0x89 => '‰',
        0x8A => 'Š',
        0x8B => '‹',
        0x8C => 'Œ',
        0x8E => 'Ž',
        0x91 => '\u{2018}',
        0x92 => '\u{2019}',
        0x93 => '\u{201C}',
        0x94 => '\u{201D}',
        0x95 => '•',
        0x96 => '–',
        0x97 => '—',
        0x98 => '˜',
        0x99 => '™',
        0x9A => 'š',
        0x9B => '›',
        0x9C => 'œ',
        0x9E => 'ž',
        0x9F => 'Ÿ',
        other => other as char,
    }
}
