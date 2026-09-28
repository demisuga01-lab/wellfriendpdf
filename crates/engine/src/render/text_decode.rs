//! Shared text decoding for the renderers.
//!
//! Turns a content-stream text string into a sequence of [`DecodedGlyph`]s
//! (code, unicode, advance width, is-gid, is-space), exactly as the raster
//! renderer does, so raster and vector consumers resolve the same character
//! codes, native CIDs, glyph indexes and advances through the cached resolver.

use crate::engine::PageResources;
use crate::fonts::cid::{cid_font_has_embedded_program, cid_to_gid};
use crate::fonts::resolver::{
    detect_font_subtype, get_descendant_font, validate_visual_font_metrics, FontSubtype,
};
use crate::fonts::FontResolver;
use crate::object::PdfDictionary;
use crate::reader::PdfReader;
use crate::render::font_rasterizer::{get_fallback_font, FontRasterizer};
use crate::render::glyph_outline::{
    extract_glyph_path_by_gid_mapped_outline, extract_glyph_path_by_gid_required_advance,
    extract_glyph_path_for_simple_mapped_outline, extract_glyph_path_for_simple_required_advance,
};
use crate::render::path::Path;

/// One decoded glyph ready to be shown.
#[derive(Debug, Clone)]
pub struct DecodedGlyph {
    /// Character code (simple fonts) or glyph id (CID fonts, when `is_gid`).
    pub code: u16,
    /// Unicode character (for fallback/lookup).
    pub unicode: char,
    /// PDF simple-font glyph name resolved from Encoding/Differences.
    pub glyph_name: Option<String>,
    /// Whether this is encoded single-byte 0x20 (PDF Tw applies). Unicode
    /// whitespace alone does not enable word spacing, notably for Type0 fonts.
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
    match try_decode_text_bytes(bytes, font_name, resources, reader) {
        Ok(glyphs) => glyphs,
        Err(reason) => {
            log::debug!("visual text decode refused: {reason}");
            Vec::new()
        }
    }
}

/// Decode a visual text string under `font_name`'s font, refusing malformed
/// length-aware character-code sequences instead of synthesizing padded glyphs.
pub fn try_decode_text_bytes(
    bytes: &[u8],
    font_name: &str,
    resources: &PageResources,
    reader: &PdfReader,
) -> std::result::Result<Vec<DecodedGlyph>, String> {
    let Some(font_dict) = resources.fonts.get(font_name) else {
        return Ok(latin1_glyphs(bytes));
    };
    let resolver = FontResolver::new(font_dict, reader);
    try_decode_text_bytes_with_resolver(bytes, font_dict, &resolver, reader)
}

/// Decode a text string using a caller-owned resolver cache.
pub fn decode_text_bytes_with_resolver(
    bytes: &[u8],
    font_dict: &PdfDictionary,
    resolver: &FontResolver,
    reader: &PdfReader,
) -> Vec<DecodedGlyph> {
    match try_decode_text_bytes_with_resolver(bytes, font_dict, resolver, reader) {
        Ok(glyphs) => glyphs,
        Err(reason) => {
            log::debug!("visual text decode refused: {reason}");
            Vec::new()
        }
    }
}

/// Decode a text string using a caller-owned resolver cache.
pub fn try_decode_text_bytes_with_resolver(
    bytes: &[u8],
    font_dict: &PdfDictionary,
    resolver: &FontResolver,
    reader: &PdfReader,
) -> std::result::Result<Vec<DecodedGlyph>, String> {
    validate_visual_font_metrics(font_dict, Some(reader))?;
    if detect_font_subtype(font_dict) == FontSubtype::Type0 {
        return decode_type0_text_with_resolver(bytes, font_dict, resolver, reader);
    }

    // /Widths may be an indirect array. validate_visual_font_metrics has
    // already resolved and validated it, and FontResolver::build resolved the
    // values into its width table. Testing only get_array() here incorrectly
    // discarded valid metrics from many Type1/TrueType fonts.
    let has_pdf_widths = font_dict.get("Widths").is_some() || resolver.has_standard14_metrics();
    let mut glyphs = Vec::new();
    for decoded in resolver.codes(bytes) {
        let char_code = decoded?.code;
        let code = u16::try_from(char_code.value())
            .map_err(|_| "simple font character code exceeds 16 bits")?;
        let text = resolver.decode_code(char_code);
        let ch = text.chars().next().unwrap_or('\u{FFFD}');
        let glyph_name = resolver.glyph_name(code).map(str::to_string);
        let width = if has_pdf_widths {
            let width = resolver.width_for_code(char_code).max(0.0);
            Some(width)
        } else {
            None
        };
        glyphs.push(DecodedGlyph {
            code,
            unicode: ch,
            glyph_name,
            is_space: char_code.is_word_space(),
            width,
            is_gid: false,
            is_vertical: false,
            vertical_advance: None,
            vertical_origin: None,
        });
    }
    Ok(glyphs)
}

fn decode_type0_text_with_resolver(
    bytes: &[u8],
    font_dict: &PdfDictionary,
    resolver: &FontResolver,
    reader: &PdfReader,
) -> std::result::Result<Vec<DecodedGlyph>, String> {
    resolver.validate_encoding()?;
    let descendant_font = get_descendant_font(font_dict, reader);
    let render_as_gid = cid_font_has_embedded_program(descendant_font.as_ref(), reader);
    let mut glyphs = Vec::new();
    for decoded in resolver.codes(bytes) {
        let char_code = match decoded {
            Ok(decoded) => decoded.code,
            Err(reason) if reason.starts_with("truncated PDF character code") => {
                // A few widely consumed PDFs end a composite-font string with
                // an incomplete final code. Preserve and paint the completely
                // decoded prefix, then ignore only the impossible suffix. This
                // matches the recovery posture of mature viewers without
                // padding the byte into a different CID or weakening the
                // strict CodeSpace API used by editing and validation.
                log::warn!(
                    "visual Type0 text ignored an incomplete trailing character code: {reason}"
                );
                break;
            }
            Err(reason) => return Err(reason),
        };
        let cid = resolver.cid_for_character(char_code)?;

        let text = resolver.decode_code(char_code);
        let unicode = text.chars().next().unwrap_or('\u{FFFD}');
        // Text advance is defined by the PDF descendant font metrics whether
        // glyph outlines come from the embedded program or a visual fallback.
        // A present descendant has /DW=1000 by specification even when /DW is
        // omitted, so this remains an exact PDF metric rather than a guess.
        let width = if descendant_font.is_some() {
            Some(resolver.width_for_code(char_code)).filter(|width| *width >= 0.0)
        } else {
            None
        };
        let code = if render_as_gid {
            match resolver.sfnt_cff_gid(cid)? {
                Some(gid) => gid,
                None => cid_to_gid(cid, descendant_font.as_ref(), reader),
            }
        } else {
            u16::try_from(unicode as u32).unwrap_or(cid)
        };
        let (vertical_advance, vertical_origin) = if resolver.is_vertical() {
            let (w1y, vx, vy) = resolver.vertical_metrics_for_code(char_code);
            (Some(w1y), Some((vx, vy)))
        } else {
            (None, None)
        };

        glyphs.push(DecodedGlyph {
            code,
            unicode,
            glyph_name: None,
            is_space: char_code.is_word_space(),
            width,
            is_gid: render_as_gid,
            is_vertical: resolver.is_vertical(),
            vertical_advance,
            vertical_origin,
        });
    }
    Ok(glyphs)
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

/// Return a glyph's horizontal advance only when it is backed by a real font
/// metric table entry. Vector sinks use this to avoid inventing movement when
/// the PDF did not provide widths and the font program cannot provide them.
pub(crate) fn decoded_glyph_strict_horizontal_advance(
    glyph: &DecodedGlyph,
    font_bytes: &[u8],
) -> Option<f64> {
    let advance = if glyph.is_gid {
        extract_glyph_path_by_gid_required_advance(font_bytes, glyph.code)?.1
    } else {
        extract_glyph_path_for_simple_required_advance(
            font_bytes,
            glyph.code,
            glyph.unicode,
            glyph.glyph_name.as_deref(),
        )?
        .1
    };
    advance.is_finite().then_some(advance)
}

/// Return a glyph outline only when vector output can map the glyph through the
/// font program without inventing a compatibility glyph id.
pub(crate) fn decoded_glyph_strict_outline(
    glyph: &DecodedGlyph,
    font_bytes: &[u8],
) -> Option<Option<Path>> {
    if glyph.is_gid {
        extract_glyph_path_by_gid_mapped_outline(font_bytes, glyph.code)
    } else {
        extract_glyph_path_for_simple_mapped_outline(
            font_bytes,
            glyph.code,
            glyph.unicode,
            glyph.glyph_name.as_deref(),
        )
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::PdfObject;

    fn type0_identity_font() -> PdfDictionary {
        let mut font = PdfDictionary::empty();
        font.insert("Type", PdfObject::Name("Font".to_string()));
        font.insert("Subtype", PdfObject::Name("Type0".to_string()));
        font.insert("BaseFont", PdfObject::Name("TestCID".to_string()));
        font.insert("Encoding", PdfObject::Name("Identity-H".to_string()));
        font
    }

    #[test]
    fn type0_visual_text_decodes_complete_two_byte_codes() {
        let reader = PdfReader::from_bytes(crate::render::shading::tests_minimal_pdf()).unwrap();
        let font = type0_identity_font();
        let resolver = FontResolver::new_from_dict_only(&font);
        let glyphs = try_decode_text_bytes_with_resolver(
            &[0x00, 0x48, 0x00, 0x69],
            &font,
            &resolver,
            &reader,
        )
        .expect("complete Type0 text decodes");

        let chars: Vec<char> = glyphs.iter().map(|glyph| glyph.unicode).collect();
        assert_eq!(chars, vec!['H', 'i']);
    }

    #[test]
    fn type0_visual_text_keeps_complete_prefix_without_padding_odd_suffix() {
        let reader = PdfReader::from_bytes(crate::render::shading::tests_minimal_pdf()).unwrap();
        let font = type0_identity_font();
        let resolver = FontResolver::new_from_dict_only(&font);
        let glyphs =
            try_decode_text_bytes_with_resolver(&[0x00, 0x48, 0x56], &font, &resolver, &reader)
                .expect("the complete code must render while the trailing byte is ignored");

        assert_eq!(glyphs.len(), 1);
        assert_eq!(glyphs[0].unicode, 'H');
        assert_eq!(glyphs[0].code, u16::from(b'H'));
    }
}
