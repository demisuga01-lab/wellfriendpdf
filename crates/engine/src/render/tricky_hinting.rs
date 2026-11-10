//! Outlines for "tricky" TrueType fonts.
//!
//! Some old CJK TrueType fonts (MingLiU, PMingLiU, DFKai-SB, HuaTian, the
//! DynaLab "DF" families and others on FreeType's list) store each glyph as
//! loose strokes that only their bytecode instructions move into place. Drawn
//! without running the instructions, their glyphs come out scrambled. FreeType
//! flags these fonts "tricky" and always hints them; PDFium follows
//! (`CFX_Face::IsTricky`, `LoadGlyphPath`: tricky fonts are loaded hinted at
//! 64 pixels per em).
//!
//! The crate's own TrueType program modules (`fonts::tt_bytecode`,
//! `fonts::tt_hint_instance`) only inspect instruction streams and freeze
//! variation data; they do not execute glyph programs. The full TrueType
//! interpreter of `skrifa` (fontations, MIT/Apache-2.0, already in the
//! dependency graph through `subsetter`) does, including FreeType's tricky
//! font detection by family name and by checksum. This module uses it for
//! tricky fonts only; every other font keeps the unhinted outlines.

use crate::render::path::Path;
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, Engine, HintingInstance, HintingOptions, OutlinePen, Target};
use skrifa::{FontRef, GlyphId, MetadataProvider};
use std::cell::RefCell;
use std::sync::Arc;

/// PDFium loads hinted glyph paths at 64 pixels per em.
const HINTING_PPEM: f32 = 64.0;
/// Hinting instances kept per thread (fpgm/prep run once per font).
const HINTER_CACHE_ENTRIES: usize = 8;

thread_local! {
    static HINTERS: RefCell<Vec<(u64, Option<Arc<HintingInstance>>)>> =
        const { RefCell::new(Vec::new()) };
}

/// Whether the TrueType program needs its bytecode run to draw correctly.
pub(crate) fn is_tricky_font(font_bytes: &[u8]) -> bool {
    let Ok(font) = FontRef::new(font_bytes) else {
        return false;
    };
    let outlines = font.outline_glyphs();
    outlines.require_interpreter()
}

fn font_key(font_bytes: &[u8]) -> u64 {
    let mut hash = crate::render::glyph_cache::GlyphCache::hash_font_bytes(font_bytes);
    hash ^= (font_bytes.len() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    for byte in font_bytes.iter().rev().take(256) {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01B3);
    }
    hash
}

fn hinter_for(font_bytes: &[u8], font: &FontRef<'_>) -> Option<Arc<HintingInstance>> {
    let key = font_key(font_bytes);
    if let Some(found) = HINTERS.with(|cache| {
        cache
            .borrow()
            .iter()
            .find(|(cached, _)| *cached == key)
            .map(|(_, hinter)| hinter.clone())
    }) {
        return found;
    }
    let outlines = font.outline_glyphs();
    let hinter = HintingInstance::new(
        &outlines,
        Size::new(HINTING_PPEM),
        LocationRef::default(),
        HintingOptions {
            engine: Engine::Interpreter,
            target: Target::Mono,
        },
    )
    .ok()
    .map(Arc::new);
    HINTERS.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() >= HINTER_CACHE_ENTRIES {
            cache.remove(0);
        }
        cache.push((key, hinter.clone()));
    });
    hinter
}

struct FontUnitPen {
    path: Path,
    scale: f64,
}

impl OutlinePen for FontUnitPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.path
            .move_to(f64::from(x) * self.scale, f64::from(y) * self.scale);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.path
            .line_to(f64::from(x) * self.scale, f64::from(y) * self.scale);
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        let Some((x0, y0)) = self.path.current_point else {
            self.line_to(x, y);
            return;
        };
        let (cx, cy) = (f64::from(cx0) * self.scale, f64::from(cy0) * self.scale);
        let (x, y) = (f64::from(x) * self.scale, f64::from(y) * self.scale);
        self.path.curve_to(
            x0 + 2.0 / 3.0 * (cx - x0),
            y0 + 2.0 / 3.0 * (cy - y0),
            x + 2.0 / 3.0 * (cx - x),
            y + 2.0 / 3.0 * (cy - y),
            x,
            y,
        );
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        let s = self.scale;
        self.path.curve_to(
            f64::from(cx0) * s,
            f64::from(cy0) * s,
            f64::from(cx1) * s,
            f64::from(cy1) * s,
            f64::from(x) * s,
            f64::from(y) * s,
        );
    }

    fn close(&mut self) {
        self.path.close();
    }
}

/// The outline of glyph `gid` of a tricky font with its instructions run (at
/// 64 pixels per em, like PDFium), in font units. `None` when the font is not
/// an sfnt, the glyph is missing, or the interpreter fails; callers then keep
/// the unhinted outline.
pub(crate) fn hinted_outline(font_bytes: &[u8], gid: u16) -> Option<Path> {
    let font = FontRef::new(font_bytes).ok()?;
    let upem = f64::from(
        font.metrics(Size::unscaled(), LocationRef::default())
            .units_per_em,
    );
    if !(upem.is_finite() && upem > 0.0) {
        return None;
    }
    let hinter = hinter_for(font_bytes, &font)?;
    let outlines = font.outline_glyphs();
    let glyph = outlines.get(GlyphId::new(u32::from(gid)))?;
    let mut pen = FontUnitPen {
        path: Path::new(),
        scale: upem / f64::from(HINTING_PPEM),
    };
    glyph
        .draw(DrawSettings::hinted(&hinter, false), &mut pen)
        .ok()?;
    Some(pen.path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_fonts_are_not_tricky() {
        let liberation = crate::render::font_rasterizer::fallback_fonts::LIBERATION_SANS_REGULAR;
        assert!(!is_tricky_font(liberation));
        assert!(!is_tricky_font(b"not a font"));
    }

    #[test]
    fn hinted_outlines_stay_in_font_units() {
        // A non-tricky font still runs through the interpreter here; the
        // hinted outline must be the glyph at its normal size and place.
        let liberation = crate::render::font_rasterizer::fallback_fonts::LIBERATION_SANS_REGULAR;
        let face = ttf_parser::Face::parse(liberation, 0).expect("bundled face");
        let gid = face.glyph_index('H').expect("H").0;
        let hinted = hinted_outline(liberation, gid).expect("hinted H");
        let bbox = face
            .glyph_bounding_box(ttf_parser::GlyphId(gid))
            .expect("bbox");
        let (mut min_y, mut max_y) = (f64::MAX, f64::MIN);
        for segment in &hinted.segments {
            if let crate::render::path::PathSegment::MoveTo(_, y)
            | crate::render::path::PathSegment::LineTo(_, y) = *segment
            {
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
        // Grid fitting at 64 ppem moves edges by at most a pixel (upem / 64).
        let tolerance = f64::from(face.units_per_em()) / 64.0 + 1.0;
        assert!(
            (min_y - f64::from(bbox.y_min)).abs() <= tolerance,
            "{min_y}"
        );
        assert!(
            (max_y - f64::from(bbox.y_max)).abs() <= tolerance,
            "{max_y}"
        );
    }
}
