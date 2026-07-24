//! Text shaping facade for generated text.
//!
//! Existing PDF pages usually contain positioned glyph codes and should not be
//! reshaped during rendering. This module is for text Wellfriend creates itself:
//! authoring, page numbers, watermarks, annotations, and future Office-to-PDF
//! output. It provides a HarfBuzz-style shape result with a deterministic Latin
//! fallback and a rustybuzz-backed complex-script path.

use rustybuzz::{script, Direction, Script, UnicodeBuffer};

use crate::error::{Result, WellfriendError};

/// Writing direction requested for generated text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextDirection {
    /// Left-to-right text, used for Latin and most CJK horizontal writing.
    LeftToRight,
    /// Right-to-left text, used for Arabic/Hebrew runs.
    RightToLeft,
}

/// Optional shaping controls for generated text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ShapeOptions {
    /// Direction override. When absent, the shaper chooses from the dominant
    /// supported script.
    pub direction: Option<TextDirection>,
}

/// One shaped glyph in a generated run.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapedGlyph {
    /// Font glyph id.
    pub glyph_id: u16,
    /// Source UTF-8 cluster index.
    pub cluster: u32,
    /// Advance in 1/1000 text units.
    pub advance: f64,
    /// X offset in 1/1000 text units.
    pub offset_x: f64,
    /// Y offset in 1/1000 text units.
    pub offset_y: f64,
}

/// Shaped generated-text run.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapedRun {
    /// Glyphs in visual/backend order.
    pub glyphs: Vec<ShapedGlyph>,
    /// Direction used for shaping.
    pub direction: TextDirection,
    /// Whether the rustybuzz complex-script path was used.
    pub used_complex_shaping: bool,
}

/// Generated-output text shaper.
#[derive(Debug, Default, Clone, Copy)]
pub struct TextShaper;

impl TextShaper {
    /// Shape `text` with `font_bytes`.
    ///
    /// Latin/default text uses a deterministic cmap/advance fallback. Arabic
    /// and Indic scripts use rustybuzz when the supplied font parses as an sfnt
    /// face. Unsupported or malformed font data returns a structured error.
    pub fn shape(font_bytes: &[u8], text: &str, options: ShapeOptions) -> Result<ShapedRun> {
        if text.is_empty() {
            return Ok(ShapedRun {
                glyphs: Vec::new(),
                direction: options.direction.unwrap_or(TextDirection::LeftToRight),
                used_complex_shaping: false,
            });
        }

        let Some(script) = dominant_shaping_script(text) else {
            return shape_latin_fallback(font_bytes, text, options);
        };
        shape_complex(font_bytes, text, script, options)
    }
}

fn shape_latin_fallback(font_bytes: &[u8], text: &str, options: ShapeOptions) -> Result<ShapedRun> {
    let face = ttf_parser::Face::parse(font_bytes, 0)
        .map_err(|_| WellfriendError::UnsupportedFeature("font.shaper.invalid_font".to_string()))?;
    let upem = f64::from(face.units_per_em()).max(1.0);
    let mut byte_index = 0u32;
    let mut glyphs = Vec::with_capacity(text.chars().count());
    for ch in text.chars() {
        let glyph_id = face.glyph_index(ch).map(|gid| gid.0).unwrap_or(0);
        let advance = face
            .glyph_hor_advance(ttf_parser::GlyphId(glyph_id))
            .map(|advance| f64::from(advance) / upem * 1000.0)
            .unwrap_or(0.0);
        glyphs.push(ShapedGlyph {
            glyph_id,
            cluster: byte_index,
            advance,
            offset_x: 0.0,
            offset_y: 0.0,
        });
        byte_index = byte_index.saturating_add(ch.len_utf8() as u32);
    }

    Ok(ShapedRun {
        glyphs,
        direction: options.direction.unwrap_or(TextDirection::LeftToRight),
        used_complex_shaping: false,
    })
}

fn shape_complex(
    font_bytes: &[u8],
    text: &str,
    script: Script,
    options: ShapeOptions,
) -> Result<ShapedRun> {
    let face = rustybuzz::Face::from_slice(font_bytes, 0).ok_or_else(|| {
        WellfriendError::UnsupportedFeature("font.shaper.invalid_font".to_string())
    })?;
    let mut buffer = UnicodeBuffer::new();
    buffer.push_str(text);
    buffer.set_script(script);
    let direction = options
        .direction
        .unwrap_or_else(|| text_direction_for_script(script));
    buffer.set_direction(match direction {
        TextDirection::LeftToRight => Direction::LeftToRight,
        TextDirection::RightToLeft => Direction::RightToLeft,
    });

    let shaped = rustybuzz::shape(&face, &[], buffer);
    let infos = shaped.glyph_infos();
    let positions = shaped.glyph_positions();
    if infos.len() != positions.len() {
        return Err(WellfriendError::ParseError(
            "font.shaper.position_count_mismatch".to_string(),
        ));
    }
    let upem = f64::from(face.units_per_em()).max(1.0);
    let glyphs = infos
        .iter()
        .zip(positions.iter())
        .map(|(info, pos)| ShapedGlyph {
            glyph_id: info.glyph_id.min(u32::from(u16::MAX)) as u16,
            cluster: info.cluster,
            advance: f64::from(pos.x_advance) / upem * 1000.0,
            offset_x: f64::from(pos.x_offset) / upem * 1000.0,
            offset_y: f64::from(pos.y_offset) / upem * 1000.0,
        })
        .collect();

    Ok(ShapedRun {
        glyphs,
        direction,
        used_complex_shaping: true,
    })
}

fn dominant_shaping_script(text: &str) -> Option<Script> {
    text.chars().find_map(script_for_char)
}

fn script_for_char(ch: char) -> Option<Script> {
    let code = ch as u32;
    if is_arabic_codepoint(code) {
        return Some(script::ARABIC);
    }
    if is_hebrew_codepoint(code) {
        return Some(script::HEBREW);
    }
    if is_devanagari_codepoint(code) {
        return Some(script::DEVANAGARI);
    }
    if is_bengali_codepoint(code) {
        return Some(script::BENGALI);
    }
    if is_gurmukhi_codepoint(code) {
        return Some(script::GURMUKHI);
    }
    if is_gujarati_codepoint(code) {
        return Some(script::GUJARATI);
    }
    if is_oriya_codepoint(code) {
        return Some(script::ORIYA);
    }
    if is_tamil_codepoint(code) {
        return Some(script::TAMIL);
    }
    if is_telugu_codepoint(code) {
        return Some(script::TELUGU);
    }
    if is_kannada_codepoint(code) {
        return Some(script::KANNADA);
    }
    if is_malayalam_codepoint(code) {
        return Some(script::MALAYALAM);
    }
    None
}

fn text_direction_for_script(script: Script) -> TextDirection {
    if script == script::ARABIC || script == script::HEBREW {
        TextDirection::RightToLeft
    } else {
        TextDirection::LeftToRight
    }
}

fn is_arabic_codepoint(code: u32) -> bool {
    matches!(
        code,
        0x0600..=0x06FF
            | 0x0750..=0x077F
            | 0x08A0..=0x08FF
            | 0xFB50..=0xFDFF
            | 0xFE70..=0xFEFF
    )
}

fn is_hebrew_codepoint(code: u32) -> bool {
    matches!(code, 0x0590..=0x05FF | 0xFB1D..=0xFB4F)
}

fn is_devanagari_codepoint(code: u32) -> bool {
    matches!(code, 0x0900..=0x097F | 0xA8E0..=0xA8FF)
}

fn is_bengali_codepoint(code: u32) -> bool {
    matches!(code, 0x0980..=0x09FF)
}

fn is_gurmukhi_codepoint(code: u32) -> bool {
    matches!(code, 0x0A00..=0x0A7F)
}

fn is_gujarati_codepoint(code: u32) -> bool {
    matches!(code, 0x0A80..=0x0AFF)
}

fn is_oriya_codepoint(code: u32) -> bool {
    matches!(code, 0x0B00..=0x0B7F)
}

fn is_tamil_codepoint(code: u32) -> bool {
    matches!(code, 0x0B80..=0x0BFF)
}

fn is_telugu_codepoint(code: u32) -> bool {
    matches!(code, 0x0C00..=0x0C7F)
}

fn is_kannada_codepoint(code: u32) -> bool {
    matches!(code, 0x0C80..=0x0CFF)
}

fn is_malayalam_codepoint(code: u32) -> bool {
    matches!(code, 0x0D00..=0x0D7F)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::font_rasterizer::get_fallback_font;

    #[test]
    fn latin_fallback_shapes_stable_glyphs() {
        let font = get_fallback_font("Helvetica").expect("bundled font");
        let run = TextShaper::shape(font, "Hello", ShapeOptions::default()).expect("shape latin");

        assert_eq!(run.glyphs.len(), 5);
        assert_eq!(run.direction, TextDirection::LeftToRight);
        assert!(!run.used_complex_shaping);
        assert!(run.glyphs.iter().all(|glyph| glyph.advance >= 0.0));
    }

    #[test]
    fn arabic_uses_complex_rtl_shaping_when_font_supports_it() {
        let font = get_fallback_font("Symbol").expect("DejaVu fallback");
        let run = TextShaper::shape(
            font,
            "\u{0633}\u{0644}\u{0627}\u{0645}",
            ShapeOptions::default(),
        )
        .expect("shape arabic");

        assert!(run.used_complex_shaping);
        assert_eq!(run.direction, TextDirection::RightToLeft);
        assert!(!run.glyphs.is_empty());
        assert!(run.glyphs.iter().all(|glyph| glyph.glyph_id > 0));
    }

    #[test]
    fn hebrew_uses_complex_rtl_shaping_when_font_supports_it() {
        let font = get_fallback_font("Symbol").expect("DejaVu fallback");
        let run = TextShaper::shape(
            font,
            "\u{05E9}\u{05DC}\u{05D5}\u{05DD}",
            ShapeOptions::default(),
        )
        .expect("shape hebrew");

        assert!(run.used_complex_shaping);
        assert_eq!(run.direction, TextDirection::RightToLeft);
        assert!(!run.glyphs.is_empty());
        assert!(run.glyphs.iter().all(|glyph| glyph.glyph_id > 0));
    }

    #[test]
    fn invalid_font_bytes_fail_cleanly() {
        let err = TextShaper::shape(b"not a font", "Hello", ShapeOptions::default())
            .expect_err("invalid font should fail");
        assert_eq!(err.code(), "unsupported_feature");
    }
}
