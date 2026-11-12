//! Shared glyph-outline extraction used by both the raster renderer and the
//! vector (SVG) renderer.
//!
//! Both backends must turn a glyph (by Unicode char for simple fonts, or by
//! glyph id for CID fonts) into the SAME outline [`Path`] in font units, so
//! that vector output is visually identical to raster output. These free
//! functions mirror the extraction the raster path uses (`ttf-parser` outlines,
//! with a bare-CFF / Type1C fallback) and are the single source of truth the
//! SVG renderer relies on.

use crate::fonts::variations::{self, VariationRequest};
use crate::render::font_rasterizer::GlyphToPath;
use crate::render::path::Path;
use ttf_parser::GlyphId;

/// Convert a font size (text-space units) and the font's units-per-em into the
/// scale that maps glyph-outline font units to text space. Returns 0 for
/// degenerate inputs (caller skips the glyph).
pub fn font_size_scale(font_size: f64, upem: f64) -> f64 {
    if font_size <= 0.0 || upem <= 0.0 || !font_size.is_finite() || !upem.is_finite() {
        0.0
    } else {
        font_size / upem
    }
}

/// The font's units-per-em. Handles sfnt-wrapped fonts via `ttf-parser` and
/// bare CFF / Type1C programs (which use a 1000-unit em).
pub fn get_upem(font_bytes: &[u8]) -> Option<u16> {
    if let Ok(face) = ttf_parser::Face::parse(font_bytes, 0) {
        return Some(face.units_per_em());
    }
    if crate::render::font_rasterizer::cff_support::is_bare_cff(font_bytes) {
        return Some(
            crate::render::font_rasterizer::cff_support::units_per_em(font_bytes).round() as u16,
        );
    }
    if crate::fonts::type1::Type1Font::is_type1(font_bytes) {
        return Some(crate::fonts::type1::units_per_em() as u16);
    }
    None
}

/// Extract the outline [`Path`] (in font units) and advance width (in 1/1000
/// text units) for a Unicode character from a simple font. Falls back to the
/// standalone CFF parser for bare-CFF (FontFile3 /Type1C) programs.
pub fn extract_glyph_path(font_bytes: &[u8], ch: char) -> (Option<Path>, f64) {
    extract_glyph_path_for_simple(font_bytes, char_code_for_lookup(ch), ch, None)
}

/// Extract the outline [`Path`] and advance width for a simple-font glyph,
/// preserving the original PDF character code and glyph name. This is needed
/// for Type1 programs (glyph-name keyed CharStrings) and subset TrueType fonts
/// whose useful cmap is not Unicode-keyed.
pub fn extract_glyph_path_for_simple(
    font_bytes: &[u8],
    code: u16,
    ch: char,
    glyph_name: Option<&str>,
) -> (Option<Path>, f64) {
    extract_glyph_path_for_simple_var(font_bytes, code, ch, glyph_name, &VariationRequest::none())
}

/// Variable-font aware variant of [`extract_glyph_path_for_simple`]: applies the
/// requested instance coordinates (if the font is variable and exposes the
/// axes) before producing the interpolated outline and HVAR-adjusted advance.
/// With an empty request this is byte-identical to the non-variable path.
pub fn extract_glyph_path_for_simple_var(
    font_bytes: &[u8],
    code: u16,
    ch: char,
    glyph_name: Option<&str>,
    request: &VariationRequest,
) -> (Option<Path>, f64) {
    extract_glyph_path_for_simple_lookup(
        font_bytes,
        code,
        ch,
        glyph_name,
        request,
        SimpleGlyphLookup::default(),
    )
}

/// [`extract_glyph_path_for_simple_var`] with the PDF font's TrueType lookup
/// rules (symbolic flag, TrueType `.notdef` handling).
pub(crate) fn extract_glyph_path_for_simple_lookup(
    font_bytes: &[u8],
    code: u16,
    ch: char,
    glyph_name: Option<&str>,
    request: &VariationRequest,
    lookup: SimpleGlyphLookup,
) -> (Option<Path>, f64) {
    let mut face = match ttf_parser::Face::parse(font_bytes, 0) {
        Ok(face) => face,
        Err(_) => {
            if let Some(glyph_name) = glyph_name {
                if let Some(result) = crate::render::font_rasterizer::cff_support::outline_by_name(
                    font_bytes, glyph_name,
                ) {
                    return result;
                }
            }
            if let Ok(code) = u8::try_from(code) {
                if let Some(result) =
                    crate::render::font_rasterizer::cff_support::outline_by_code(font_bytes, code)
                {
                    return result;
                }
            }
            if let Some(glyph_name) = glyph_name {
                if let Some(result) = crate::fonts::type1::outline_by_name(font_bytes, glyph_name) {
                    return result;
                }
            }
            if let Some(result) =
                crate::render::font_rasterizer::cff_support::outline_by_char(font_bytes, ch)
            {
                return result;
            }
            return (None, 500.0);
        }
    };

    // Apply the variable-font instance (no-op for static fonts / empty request);
    // the crate then interpolates the outline (gvar/CFF2) and the advance (HVAR).
    if variations::apply_request_checked(&mut face, request).is_err() {
        return (None, 500.0);
    }

    let upem = f64::from(face.units_per_em());
    // When no table maps the code, draw nothing rather than guessing a glyph
    // id: a guessed id draws an unrelated glyph, while "no outline" lets the
    // caller's per-glyph fallback font draw the right character.
    let glyph_id = glyph_index_for_simple_with(&face, code, ch, glyph_name, lookup)
        .or_else(|| (lookup.embedded && face.tables().cmap.is_some()).then_some(GlyphId(0)));
    let Some(glyph_id) = glyph_id else {
        let advance = face
            .glyph_hor_advance(ttf_parser::GlyphId(0))
            .map(|width| f64::from(width) / upem * 1000.0)
            .unwrap_or(500.0);
        return (None, advance);
    };
    let advance = face
        .glyph_hor_advance(glyph_id)
        .map(|width| f64::from(width) / upem * 1000.0)
        .unwrap_or(500.0);

    let mut builder = GlyphToPath::new();
    if crate::fonts::sfnt_outline::outline(&face, glyph_id, &mut builder)
        .ok()
        .flatten()
        .is_none()
    {
        return (None, advance);
    }
    (Some(builder.into_path()), advance)
}

/// [`extract_glyph_path_for_simple_var`] for vertical writing with a
/// substitute program: the glyph found for the character is replaced by its
/// vertical form from the font's GSUB `vrt2` or `vert` feature (rotated or
/// repositioned brackets, dashes, small kana and punctuation), the way Poppler
/// maps substituted vertical CID text. Glyphs without a vertical form, and
/// fonts that are not sfnt, are drawn as in horizontal text.
pub(crate) fn extract_vertical_glyph_path_for_simple_var(
    font_bytes: &[u8],
    code: u16,
    ch: char,
    glyph_name: Option<&str>,
    request: &VariationRequest,
) -> (Option<Path>, f64) {
    extract_vertical_glyph_path_for_simple_lookup(
        font_bytes,
        code,
        ch,
        glyph_name,
        request,
        SimpleGlyphLookup::default(),
    )
}

/// [`extract_vertical_glyph_path_for_simple_var`] with the PDF font's
/// TrueType lookup rules.
pub(crate) fn extract_vertical_glyph_path_for_simple_lookup(
    font_bytes: &[u8],
    code: u16,
    ch: char,
    glyph_name: Option<&str>,
    request: &VariationRequest,
    lookup: SimpleGlyphLookup,
) -> (Option<Path>, f64) {
    let Ok(mut face) = ttf_parser::Face::parse(font_bytes, 0) else {
        return extract_glyph_path_for_simple_lookup(
            font_bytes, code, ch, glyph_name, request, lookup,
        );
    };
    if variations::apply_request_checked(&mut face, request).is_err() {
        return (None, 500.0);
    }
    let Some(glyph_id) = glyph_index_for_simple_with(&face, code, ch, glyph_name, lookup) else {
        return extract_glyph_path_for_simple_lookup(
            font_bytes, code, ch, glyph_name, request, lookup,
        );
    };
    let glyph_id = vertical_form_glyph(&face, glyph_id);
    let upem = f64::from(face.units_per_em());
    let advance = face
        .glyph_hor_advance(glyph_id)
        .filter(|_| upem > 0.0)
        .map(|width| f64::from(width) / upem * 1000.0)
        .unwrap_or(1000.0);
    let mut builder = GlyphToPath::new();
    if crate::fonts::sfnt_outline::outline(&face, glyph_id, &mut builder)
        .ok()
        .flatten()
        .is_none()
    {
        return (None, advance);
    }
    (Some(builder.into_path()), advance)
}

/// The vertical alternate of `glyph` from the GSUB `vrt2` feature, else the
/// `vert` feature (single substitutions only, which is all either feature
/// contains), or `glyph` itself when the font has none.
pub(crate) fn vertical_form_glyph(face: &ttf_parser::Face<'_>, glyph: GlyphId) -> GlyphId {
    use ttf_parser::gsub::{SingleSubstitution, SubstitutionSubtable};
    let Some(gsub) = face.tables().gsub else {
        return glyph;
    };
    for feature_tag in [b"vrt2", b"vert"] {
        let feature_tag = ttf_parser::Tag::from_bytes(feature_tag);
        for feature in gsub
            .features
            .into_iter()
            .filter(|feature| feature.tag == feature_tag)
        {
            for lookup_index in feature.lookup_indices {
                let Some(lookup) = gsub.lookups.get(lookup_index) else {
                    continue;
                };
                for subtable in lookup.subtables.into_iter::<SubstitutionSubtable>() {
                    let SubstitutionSubtable::Single(single) = subtable else {
                        continue;
                    };
                    let Some(coverage_index) = single.coverage().get(glyph) else {
                        continue;
                    };
                    let substitute = match single {
                        SingleSubstitution::Format1 { delta, .. } => {
                            Some(GlyphId(glyph.0.wrapping_add(delta as u16)))
                        }
                        SingleSubstitution::Format2 { substitutes, .. } => {
                            substitutes.get(coverage_index)
                        }
                    };
                    if let Some(substitute) =
                        substitute.filter(|substitute| substitute.0 < face.number_of_glyphs())
                    {
                        return substitute;
                    }
                }
            }
        }
    }
    glyph
}

/// Strict vector-output variant of [`extract_glyph_path_for_simple`].
///
/// This refuses to synthesize a glyph advance. It may still return `None` for
/// the outline (for whitespace or fonts without drawable contours), but the
/// advance is present only when it came from a real sfnt/CFF metric source.
pub(crate) fn extract_glyph_path_for_simple_required_advance(
    font_bytes: &[u8],
    code: u16,
    ch: char,
    glyph_name: Option<&str>,
) -> Option<(Option<Path>, f64)> {
    extract_glyph_path_for_simple_required_advance_var(
        font_bytes,
        code,
        ch,
        glyph_name,
        &VariationRequest::none(),
    )
}

/// Variation-aware strict metric variant used by exact raster text rendering.
pub(crate) fn extract_glyph_path_for_simple_required_advance_var(
    font_bytes: &[u8],
    code: u16,
    ch: char,
    glyph_name: Option<&str>,
    request: &VariationRequest,
) -> Option<(Option<Path>, f64)> {
    let mut face = match ttf_parser::Face::parse(font_bytes, 0) {
        Ok(face) => face,
        Err(_) => {
            if let Some(glyph_name) = glyph_name {
                if let Some(result) =
                    crate::render::font_rasterizer::cff_support::outline_by_name_required_width(
                        font_bytes, glyph_name,
                    )
                    .filter(|(_, advance)| advance.is_finite())
                {
                    return Some(result);
                }
            }
            if let Ok(code) = u8::try_from(code) {
                if let Some(result) =
                    crate::render::font_rasterizer::cff_support::outline_by_code_required_width(
                        font_bytes, code,
                    )
                    .filter(|(_, advance)| advance.is_finite())
                {
                    return Some(result);
                }
            }
            return None;
        }
    };

    variations::apply_request_checked(&mut face, request).ok()?;

    let upem = f64::from(face.units_per_em());
    if upem <= 0.0 || !upem.is_finite() {
        return None;
    }
    let glyph_id = glyph_index_for_simple(&face, code, ch, glyph_name)?;
    let advance = face
        .glyph_hor_advance(glyph_id)
        .map(|width| f64::from(width) / upem * 1000.0)
        .filter(|advance| advance.is_finite())?;

    let mut builder = GlyphToPath::new();
    let outline = crate::fonts::sfnt_outline::outline(&face, glyph_id, &mut builder)
        .ok()?
        .map(|_| builder.into_path());
    Some((outline, advance))
}

/// Extract a simple-font glyph outline only when the glyph id comes from a real
/// font mapping. Unlike the compatibility extractor, this never synthesizes a
/// code-point-derived glyph id.
pub(crate) fn extract_glyph_path_for_simple_mapped_outline(
    font_bytes: &[u8],
    code: u16,
    ch: char,
    glyph_name: Option<&str>,
) -> Option<Option<Path>> {
    extract_glyph_path_for_simple_mapped_outline_var(
        font_bytes,
        code,
        ch,
        glyph_name,
        &VariationRequest::none(),
    )
}

pub(crate) fn extract_glyph_path_for_simple_mapped_outline_var(
    font_bytes: &[u8],
    code: u16,
    ch: char,
    glyph_name: Option<&str>,
    request: &VariationRequest,
) -> Option<Option<Path>> {
    let mut face = match ttf_parser::Face::parse(font_bytes, 0) {
        Ok(face) => face,
        Err(_) => {
            if let Some(glyph_name) = glyph_name {
                if let Some((outline, _)) =
                    crate::render::font_rasterizer::cff_support::outline_by_name(
                        font_bytes, glyph_name,
                    )
                {
                    return Some(outline);
                }
            }
            if let Ok(code) = u8::try_from(code) {
                if let Some((outline, _)) =
                    crate::render::font_rasterizer::cff_support::outline_by_code(font_bytes, code)
                {
                    return Some(outline);
                }
            }
            if let Some(glyph_name) = glyph_name {
                if let Some((outline, _)) =
                    crate::fonts::type1::outline_by_name(font_bytes, glyph_name)
                {
                    return Some(outline);
                }
            }
            return None;
        }
    };

    variations::apply_request_checked(&mut face, request).ok()?;

    let glyph_id = glyph_index_for_simple(&face, code, ch, glyph_name)?;
    let mut builder = GlyphToPath::new();
    Some(
        crate::fonts::sfnt_outline::outline(&face, glyph_id, &mut builder)
            .ok()?
            .map(|_| builder.into_path()),
    )
}

/// Glyph id a simple font's code selects in an sfnt program, by the same
/// lookup [`extract_glyph_path_for_simple_mapped_outline`] uses, so an
/// embedded TrueType glyph is the glyph the outline renderers draw.
pub(crate) fn simple_glyph_sfnt_gid(
    font_bytes: &[u8],
    code: u16,
    ch: char,
    glyph_name: Option<&str>,
) -> Option<u16> {
    let face = ttf_parser::Face::parse(font_bytes, 0).ok()?;
    glyph_index_for_simple(&face, code, ch, glyph_name).map(|gid| gid.0)
}

/// Extract the outline [`Path`] and advance width for a glyph id (CID fonts).
/// Falls back to the standalone CFF parser for bare CFF (/CIDFontType0C).
pub fn extract_glyph_path_by_gid(font_bytes: &[u8], gid: u16) -> (Option<Path>, f64) {
    extract_glyph_path_by_gid_var(font_bytes, gid, &VariationRequest::none())
}

/// Variable-font aware variant of [`extract_glyph_path_by_gid`]: applies the
/// requested instance coordinates before producing the interpolated outline and
/// HVAR-adjusted advance. Byte-identical to the non-variable path for an empty
/// request.
pub fn extract_glyph_path_by_gid_var(
    font_bytes: &[u8],
    gid: u16,
    request: &VariationRequest,
) -> (Option<Path>, f64) {
    let mut face = match ttf_parser::Face::parse(font_bytes, 0) {
        Ok(face) => face,
        Err(_) => {
            if let Some(result) =
                crate::render::font_rasterizer::cff_support::outline_by_gid(font_bytes, gid)
            {
                return result;
            }
            return (None, 500.0);
        }
    };

    if variations::apply_request_checked(&mut face, request).is_err() {
        return (None, 500.0);
    }

    let upem = f64::from(face.units_per_em());
    if upem <= 0.0 {
        return (None, 500.0);
    }
    let glyph_id = ttf_parser::GlyphId(gid);
    let advance = face
        .glyph_hor_advance(glyph_id)
        .map(|width| f64::from(width) / upem * 1000.0)
        .unwrap_or(1000.0);

    let mut builder = GlyphToPath::new();
    if crate::fonts::sfnt_outline::outline(&face, glyph_id, &mut builder)
        .ok()
        .flatten()
        .is_none()
    {
        return (None, advance);
    }
    (Some(builder.into_path()), advance)
}

/// Strict vector-output variant of [`extract_glyph_path_by_gid`].
///
/// This requires a real sfnt/CFF advance for the glyph id and refuses the
/// compatibility defaults used by raster fallback paths.
pub(crate) fn extract_glyph_path_by_gid_required_advance(
    font_bytes: &[u8],
    gid: u16,
) -> Option<(Option<Path>, f64)> {
    extract_glyph_path_by_gid_required_advance_var(font_bytes, gid, &VariationRequest::none())
}

/// Variation-aware strict metric variant used by exact raster text rendering.
pub(crate) fn extract_glyph_path_by_gid_required_advance_var(
    font_bytes: &[u8],
    gid: u16,
    request: &VariationRequest,
) -> Option<(Option<Path>, f64)> {
    let mut face = match ttf_parser::Face::parse(font_bytes, 0) {
        Ok(face) => face,
        Err(_) => {
            return crate::render::font_rasterizer::cff_support::outline_by_gid_required_width(
                font_bytes, gid,
            )
            .filter(|(_, advance)| advance.is_finite());
        }
    };

    variations::apply_request_checked(&mut face, request).ok()?;

    let upem = f64::from(face.units_per_em());
    if upem <= 0.0 || !upem.is_finite() {
        return None;
    }
    let glyph_id = ttf_parser::GlyphId(gid);
    let advance = face
        .glyph_hor_advance(glyph_id)
        .map(|width| f64::from(width) / upem * 1000.0)
        .filter(|advance| advance.is_finite())?;

    let mut builder = GlyphToPath::new();
    let outline = crate::fonts::sfnt_outline::outline(&face, glyph_id, &mut builder)
        .ok()?
        .map(|_| builder.into_path());
    Some((outline, advance))
}

/// Extract a CID/GID outline only when the requested glyph id is valid for the
/// font program. This is the vector-output counterpart to the best-effort
/// compatibility extractor above.
pub(crate) fn extract_glyph_path_by_gid_mapped_outline(
    font_bytes: &[u8],
    gid: u16,
) -> Option<Option<Path>> {
    extract_glyph_path_by_gid_mapped_outline_var(font_bytes, gid, &VariationRequest::none())
}

pub(crate) fn extract_glyph_path_by_gid_mapped_outline_var(
    font_bytes: &[u8],
    gid: u16,
    request: &VariationRequest,
) -> Option<Option<Path>> {
    let mut face = match ttf_parser::Face::parse(font_bytes, 0) {
        Ok(face) => face,
        Err(_) => {
            return crate::render::font_rasterizer::cff_support::outline_by_gid(font_bytes, gid)
                .map(|(outline, _)| outline);
        }
    };

    variations::apply_request_checked(&mut face, request).ok()?;

    if gid >= face.number_of_glyphs() {
        return None;
    }
    let glyph_id = ttf_parser::GlyphId(gid);
    let mut builder = GlyphToPath::new();
    Some(
        crate::fonts::sfnt_outline::outline(&face, glyph_id, &mut builder)
            .ok()?
            .map(|_| builder.into_path()),
    )
}

/// Best-effort fallback glyph id for a char with no cmap mapping, mirroring the
/// raster path's heuristic (code-point-derived id, saturating).
/// The simple-font character code to try when only a Unicode value is known.
fn char_code_for_lookup(ch: char) -> u16 {
    u16::try_from(u32::from(ch)).unwrap_or(0)
}

/// How a PDF simple font's codes reach the glyphs of a TrueType (sfnt)
/// program, following PDFium's `CPDF_TrueTypeFont::LoadGlyphMap`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub(crate) struct SimpleGlyphLookup {
    /// The PDF font is symbolic (Symbolic flag without Nonsymbolic, and not
    /// a plain WinAnsi/MacRoman encoding): the Windows Symbol (3,0) cmap is
    /// tried first with the code and the 0xF000, 0xF100 and 0xF200 prefixes,
    /// then the Mac (1,0) cmap with the raw code, before glyph names and
    /// Unicode.
    pub symbolic: bool,
    /// The PDF font is a TrueType font: a code whose /Differences name is
    /// `.notdef` draws the space glyph.
    pub truetype: bool,
    /// The program is the TrueType font embedded in the PDF: a code that no
    /// cmap, name or Unicode lookup reaches draws glyph 0 (`.notdef`), as
    /// PDFium and Poppler do, rather than a fallback face's character.
    pub embedded: bool,
}

fn glyph_index_for_simple(
    face: &ttf_parser::Face<'_>,
    code: u16,
    ch: char,
    glyph_name: Option<&str>,
) -> Option<GlyphId> {
    glyph_index_for_simple_with(face, code, ch, glyph_name, SimpleGlyphLookup::default())
}

/// The cmap subtables of `face` with the given platform and encoding.
fn cmap_subtables<'a>(
    face: &ttf_parser::Face<'a>,
    platform: ttf_parser::PlatformId,
    encoding: u16,
) -> impl Iterator<Item = ttf_parser::cmap::Subtable<'a>> {
    face.tables()
        .cmap
        .into_iter()
        .flat_map(|cmap| cmap.subtables.into_iter())
        .filter(move |subtable| {
            subtable.platform_id == platform && subtable.encoding_id == encoding
        })
}

/// A single-byte code through the Windows Symbol (3,0) cmap, with the
/// prefixes PDFium tries (`GetGlyphIndexForMSSymbol`): the code itself, then
/// 0xF000, 0xF100 and 0xF200 plus the code.
fn ms_symbol_glyph(face: &ttf_parser::Face<'_>, code: u16) -> Option<GlyphId> {
    if code > 0xFF {
        return None;
    }
    let code = u32::from(code);
    for prefix in [0u32, 0xF000, 0xF100, 0xF200] {
        for subtable in cmap_subtables(face, ttf_parser::PlatformId::Windows, 0) {
            if let Some(gid) = subtable.glyph_index(prefix + code).filter(|gid| gid.0 != 0) {
                return Some(gid);
            }
        }
    }
    None
}

/// A code through the Mac Roman (1,0) cmap.
fn mac_roman_cmap_glyph(face: &ttf_parser::Face<'_>, code: u32) -> Option<GlyphId> {
    cmap_subtables(face, ttf_parser::PlatformId::Macintosh, 0)
        .find_map(|subtable| subtable.glyph_index(code).filter(|gid| gid.0 != 0))
}

/// The space glyph, which PDFium draws for a TrueType code named `.notdef`.
fn space_glyph(face: &ttf_parser::Face<'_>) -> Option<GlyphId> {
    face.glyph_index(' ')
        .or_else(|| ms_symbol_glyph(face, 0x20))
        .or_else(|| mac_roman_cmap_glyph(face, 0x20))
}

fn has_unicode_cmap(face: &ttf_parser::Face<'_>) -> bool {
    face.tables().cmap.is_some_and(|cmap| {
        cmap.subtables
            .into_iter()
            .any(|subtable| subtable.is_unicode())
    })
}

/// The Mac OS Roman code of a glyph, from its PDF glyph name (the
/// MacRomanEncoding table, PDF 32000-1 9.6.6.4) or else its Unicode value
/// (including the Apple Roman symbols PDF's table leaves out).
fn mac_roman_code(glyph_name: Option<&str>, ch: char) -> Option<u32> {
    use crate::fonts::encoding::MAC_ROMAN_ENCODING;
    if let Some(name) = glyph_name {
        if let Some(code) = MAC_ROMAN_ENCODING
            .iter()
            .position(|entry| *entry == Some(name))
        {
            return Some(code as u32);
        }
    }
    if (' '..='~').contains(&ch) {
        return Some(ch as u32);
    }
    let apple_extra = match ch {
        '\u{2260}' => Some(0xAD),
        '\u{221E}' => Some(0xB0),
        '\u{2264}' => Some(0xB2),
        '\u{2265}' => Some(0xB3),
        '\u{2202}' => Some(0xB6),
        '\u{2211}' => Some(0xB7),
        '\u{220F}' => Some(0xB8),
        '\u{03C0}' => Some(0xB9),
        '\u{222B}' => Some(0xBA),
        '\u{2126}' | '\u{03A9}' => Some(0xBD),
        '\u{221A}' => Some(0xC3),
        '\u{2248}' => Some(0xC5),
        '\u{2206}' | '\u{0394}' => Some(0xC6),
        '\u{25CA}' => Some(0xD7),
        '\u{20AC}' => Some(0xDB),
        '\u{F8FF}' => Some(0xF0),
        _ => None,
    };
    if apple_extra.is_some() {
        return apple_extra;
    }
    MAC_ROMAN_ENCODING
        .iter()
        .position(|entry| {
            entry.is_some_and(|name| {
                crate::fonts::glyph_list::glyph_name_to_unicode(name) == Some(ch)
            })
        })
        .map(|code| code as u32)
}

pub(crate) fn glyph_index_for_simple_with(
    face: &ttf_parser::Face<'_>,
    code: u16,
    ch: char,
    glyph_name: Option<&str>,
    lookup: SimpleGlyphLookup,
) -> Option<GlyphId> {
    let notdef = glyph_name == Some(".notdef");
    if lookup.symbolic {
        // A symbolic TrueType font keys its glyphs by code in the Windows
        // Symbol cmap (often at U+F0xx), or else by raw code in the Mac cmap.
        if let Some(gid) = ms_symbol_glyph(face, code) {
            return Some(gid);
        }
        if !notdef {
            if let Some(gid) = mac_roman_cmap_glyph(face, u32::from(code)) {
                return Some(gid);
            }
        }
    }
    if notdef && lookup.truetype {
        return space_glyph(face);
    }
    if let Some(name) = glyph_name.filter(|name| *name != ".notdef") {
        if let Some(gid) = face.glyph_index_by_name(name) {
            return Some(gid);
        }
        // "fi" <-> "f_i" and the other standard ligature spellings.
        if let Some(gid) = crate::fonts::glyph_list::ligature_alternate_name(name)
            .and_then(|alternate| face.glyph_index_by_name(alternate))
        {
            return Some(gid);
        }
    }
    if let Some(gid) = face.glyph_index(ch) {
        return Some(gid);
    }
    // A non-symbolic font with a Mac Roman cmap but no Unicode cmap: the
    // glyph's Mac Roman code, not the PDF code, keys the cmap (WinAnsi 0xE9
    // e-acute is Mac Roman 0x8E).
    if !lookup.symbolic && !notdef && !has_unicode_cmap(face) {
        if let Some(gid) =
            mac_roman_code(glyph_name, ch).and_then(|mac| mac_roman_cmap_glyph(face, mac))
        {
            return Some(gid);
        }
    }
    glyph_index_from_non_unicode_cmap(face, u32::from(code))
}

fn glyph_index_from_non_unicode_cmap(face: &ttf_parser::Face<'_>, code: u32) -> Option<GlyphId> {
    let Some(cmap) = face.tables().cmap else {
        // No cmap at all (common in stripped subset fonts): PDF producers
        // that omit the cmap index glyphs by character code, which is also
        // what Poppler and PDF.js do.
        return (code > 0 && code < u32::from(face.number_of_glyphs()))
            .then_some(GlyphId(code as u16));
    };

    // Prefer the simple-font cmap subtables used by PDF producers for 8-bit
    // subset fonts: Macintosh Roman (1,0) and Windows Symbol (3,0). Only use
    // this path as a fallback after Unicode/name lookup to keep common Unicode
    // TrueType rendering unchanged.
    for wanted in [
        (ttf_parser::PlatformId::Macintosh, 0u16),
        (ttf_parser::PlatformId::Windows, 0u16),
    ] {
        for subtable in cmap.subtables {
            if subtable.platform_id == wanted.0 && subtable.encoding_id == wanted.1 {
                if let Some(gid) = subtable.glyph_index(code) {
                    return Some(gid);
                }
            }
        }
    }

    // Symbol fonts (Wingdings, Webdings, Symbol, many Word-generated subsets)
    // carry only a Windows Symbol (3,0) cmap with their glyphs at
    // U+F020..U+F0FF (or U+F1xx, U+F2xx); a single-byte PDF code `c` means
    // 0xF000 + c.
    if let Ok(code) = u16::try_from(code) {
        if let Some(gid) = ms_symbol_glyph(face, code) {
            return Some(gid);
        }
    }

    for subtable in cmap.subtables {
        if subtable.is_unicode() {
            continue;
        }
        if let Some(gid) = subtable.glyph_index(code) {
            return Some(gid);
        }
    }
    None
}

pub(crate) fn resolve_glyph_id_for_simple(
    face: &ttf_parser::Face<'_>,
    code: u16,
    ch: char,
    glyph_name: Option<&str>,
) -> Option<GlyphId> {
    glyph_index_for_simple(face, code, ch, glyph_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_160f_embedded_arial_resolves_accented_winansi_outlines() {
        let reader = crate::reader::PdfReader::from_bytes(
            include_bytes!("../../tests/fixtures/form_160f.pdf").to_vec(),
        )
        .expect("fixture parses");

        for object_number in [403, 482, 484] {
            let font = match reader
                .resolve(crate::object::PdfObject::Reference {
                    number: object_number,
                    generation: 0,
                })
                .expect("font reference resolves")
            {
                crate::object::PdfObject::Dictionary(font) => font,
                other => panic!("object {object_number} is not a font dictionary: {other:?}"),
            };
            let font_bytes =
                crate::render::font_rasterizer::FontRasterizer::extract_font_bytes(&font, &reader)
                    .expect("fixture font is embedded");
            let resolver = crate::fonts::resolver::FontResolver::new(&font, &reader);

            for (code, expected) in [(0xE9, '\u{00E9}'), (0xF4, '\u{00F4}')]
                .into_iter()
                .filter(|(code, _)| resolver.glyph_width(*code) > 0.0)
            {
                let text = resolver.decode_char(code);
                assert_eq!(text.chars().next(), Some(expected));
                let glyph_name = resolver.glyph_name(code);
                let (outline, _) =
                    extract_glyph_path_for_simple(&font_bytes, code, expected, glyph_name);
                assert!(
                    outline.is_some(),
                    "font object {object_number} did not resolve WinAnsi code {code:#04X} ({glyph_name:?})"
                );
            }
        }
    }
    use crate::engine::ContentEngine;
    use crate::fonts::resolver::FontResolver;
    use crate::render::font_rasterizer::FontRasterizer;

    fn repo_fixture(path: &str) -> String {
        format!("{}/../../{path}", env!("CARGO_MANIFEST_DIR"))
    }

    #[test]
    fn simple_truetype_subset_uses_non_unicode_cmap_when_unicode_absent() {
        let engine =
            ContentEngine::open_path(repo_fixture("tests/corpus/pdfs/pdfjs/openoffice.pdf"))
                .expect("open openoffice fixture");
        let resources = engine.get_page_resources(1).expect("page resources");
        let reader = engine.document().reader();
        let font_dict = resources
            .fonts
            .values()
            .find(|dict| {
                dict.get_name("BaseFont")
                    .map(|name| name.contains("Helvetica-Bold"))
                    .unwrap_or(false)
            })
            .expect("openoffice subset font");
        let font_bytes =
            FontRasterizer::extract_font_bytes(font_dict, reader).expect("embedded font bytes");
        let face = ttf_parser::Face::parse(&font_bytes, 0).expect("parse subset font");

        assert!(
            face.glyph_index('A').is_none(),
            "fixture must exercise the non-Unicode cmap fallback"
        );
        let expected_gid = face
            .tables()
            .cmap
            .and_then(|cmap| {
                cmap.subtables
                    .into_iter()
                    .find(|subtable| !subtable.is_unicode())
                    .and_then(|subtable| subtable.glyph_index(65))
            })
            .expect("MacRoman cmap maps code 65");
        let expected_advance = face
            .glyph_hor_advance(expected_gid)
            .map(|width| f64::from(width) / f64::from(face.units_per_em()) * 1000.0)
            .expect("expected glyph advance");

        let (path, advance) = extract_glyph_path_for_simple(&font_bytes, 65, 'A', Some("A"));

        assert!(path.is_some(), "mapped subset glyph should have an outline");
        assert!(
            (advance - expected_advance).abs() < 0.1,
            "advance {advance} should come from cmap-selected gid {expected_gid:?}, expected {expected_advance}"
        );
    }

    #[test]
    fn vector_mapped_outline_refuses_synthetic_simple_gid() {
        let engine =
            ContentEngine::open_path(repo_fixture("tests/corpus/pdfs/pdfjs/openoffice.pdf"))
                .expect("open openoffice fixture");
        let resources = engine.get_page_resources(1).expect("page resources");
        let reader = engine.document().reader();
        let font_dict = resources
            .fonts
            .values()
            .find(|dict| {
                dict.get_name("BaseFont")
                    .map(|name| name.contains("Helvetica-Bold"))
                    .unwrap_or(false)
            })
            .expect("openoffice subset font");
        let font_bytes =
            FontRasterizer::extract_font_bytes(font_dict, reader).expect("embedded font bytes");

        let mapped = extract_glyph_path_for_simple_mapped_outline(&font_bytes, 65, 'A', Some("A"))
            .expect("mapped code should resolve through the subset cmap");
        assert!(
            mapped.is_some(),
            "mapped glyph should still expose an outline"
        );

        let missing =
            extract_glyph_path_for_simple_mapped_outline(&font_bytes, u16::MAX, '\u{10ffff}', None);
        assert!(
            missing.is_none(),
            "strict vector output must not invent a code-point-derived glyph id"
        );
    }

    #[test]
    fn bare_cff_simple_font_uses_pdf_char_code_for_winansi_punctuation() {
        let fixture =
            repo_fixture("renderer-benchmark/corpus/real-world/irs-public-domain/f1040.pdf");
        if !std::path::Path::new(&fixture).exists() {
            eprintln!("NOTE: IRS form fixture missing; skipping bare-CFF punctuation test");
            return;
        }
        let engine = ContentEngine::open_path(fixture).expect("open IRS form fixture");
        let resources = engine.get_page_resources(1).expect("page resources");
        let reader = engine.document().reader();
        let font_dict = resources
            .fonts
            .values()
            .find(|dict| {
                dict.get_name("BaseFont")
                    .map(|name| name.contains("HelveticaNeueLTStd-Roman"))
                    .unwrap_or(false)
            })
            .expect("IRS form should use embedded HelveticaNeue Type1C");
        let font_bytes =
            FontRasterizer::extract_font_bytes(font_dict, reader).expect("embedded font bytes");
        assert!(
            crate::render::font_rasterizer::cff_support::is_bare_cff(&font_bytes),
            "fixture should exercise bare CFF / Type1C simple fonts"
        );

        let resolver = FontResolver::new(font_dict, reader);
        assert_eq!(resolver.decode_char(0x96), "\u{2013}");
        assert_eq!(resolver.glyph_name(0x96), Some("endash"));

        let (notdef_path, _) = extract_glyph_path_for_simple(&font_bytes, 0, '\0', Some(".notdef"));
        let (endash_path, _) =
            extract_glyph_path_for_simple(&font_bytes, 0x96, '\u{2013}', Some("endash"));

        let notdef = path_bounds(&notdef_path.expect(".notdef outline should exist"));
        let endash = path_bounds(&endash_path.expect("endash outline should exist"));

        assert!(
            endash.2 > endash.0 && endash.3 > endash.1,
            "endash outline should have positive bounds"
        );
        assert_ne!(
            quantized_bounds(notdef),
            quantized_bounds(endash),
            "WinAnsi 0x96 must render the endash glyph, not .notdef"
        );
    }

    #[test]
    fn bare_cff_simple_font_uses_pdf_winansi_name_for_accented_glyphs() {
        let fixture =
            repo_fixture("renderer-benchmark/corpus/real-world/pdfjs-full/glyph_accent.pdf");
        if !std::path::Path::new(&fixture).exists() {
            eprintln!("NOTE: pdf.js glyph_accent fixture missing; skipping bare-CFF accent test");
            return;
        }
        let engine = ContentEngine::open_path(fixture).expect("open glyph_accent fixture");
        let resources = engine.get_page_resources(1).expect("page resources");
        let reader = engine.document().reader();
        let font_dict = resources
            .fonts
            .get("F1")
            .expect("glyph_accent should expose /F1");
        let font_bytes =
            FontRasterizer::extract_font_bytes(font_dict, reader).expect("embedded font bytes");
        assert!(
            crate::render::font_rasterizer::cff_support::is_bare_cff(&font_bytes),
            "fixture should exercise bare CFF / Type1C simple fonts"
        );

        let resolver = FontResolver::new(font_dict, reader);
        assert_eq!(resolver.decode_char(0xE3), "\u{00E3}");
        assert_eq!(resolver.glyph_name(0xE3), Some("atilde"));
        assert!(
            crate::render::font_rasterizer::cff_support::outline_by_gid(&font_bytes, 48)
                .and_then(|(path, _)| path)
                .is_some(),
            "fixture atilde GID should resolve to a composed CFF outline"
        );

        let (path, advance) =
            extract_glyph_path_for_simple(&font_bytes, 0xE3, '\u{00E3}', Some("atilde"));
        let path = path.expect("atilde outline should resolve by PDF glyph name");
        let bounds = path_bounds(&path);

        assert!(advance > 0.0, "atilde advance should be positive");
        assert!(
            bounds.2 > bounds.0 && bounds.3 > bounds.1,
            "atilde outline should have positive bounds"
        );
    }

    fn path_bounds(path: &Path) -> (f64, f64, f64, f64) {
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for segment in &path.segments {
            match segment {
                crate::render::path::PathSegment::MoveTo(x, y)
                | crate::render::path::PathSegment::LineTo(x, y) => {
                    min_x = min_x.min(*x);
                    min_y = min_y.min(*y);
                    max_x = max_x.max(*x);
                    max_y = max_y.max(*y);
                }
                crate::render::path::PathSegment::CubicTo {
                    cp1x,
                    cp1y,
                    cp2x,
                    cp2y,
                    x,
                    y,
                } => {
                    for (px, py) in [(*cp1x, *cp1y), (*cp2x, *cp2y), (*x, *y)] {
                        min_x = min_x.min(px);
                        min_y = min_y.min(py);
                        max_x = max_x.max(px);
                        max_y = max_y.max(py);
                    }
                }
                crate::render::path::PathSegment::ClosePath => {}
            }
        }
        (min_x, min_y, max_x, max_y)
    }

    fn quantized_bounds(bounds: (f64, f64, f64, f64)) -> (i32, i32, i32, i32) {
        (
            bounds.0.round() as i32,
            bounds.1.round() as i32,
            bounds.2.round() as i32,
            bounds.3.round() as i32,
        )
    }

    /// A GSUB table with one `vert` feature (DFLT script) holding one single
    /// substitution `from -> to`.
    fn gsub_with_vert_substitution(from: u16, to: u16) -> Vec<u8> {
        let mut out = Vec::new();
        let mut put = |value: u16| out.extend_from_slice(&value.to_be_bytes());
        // Header: version 1.0, ScriptList at 10, FeatureList at 30,
        // LookupList at 44.
        for value in [1, 0, 10, 30, 44] {
            put(value);
        }
        // ScriptList: one DFLT script at +8; Script: default LangSys at +4,
        // no other languages; LangSys: no required feature, feature 0.
        put(1);
        out.extend_from_slice(b"DFLT");
        let mut put = |value: u16| out.extend_from_slice(&value.to_be_bytes());
        for value in [8, 4, 0, 0, 0xFFFF, 1, 0] {
            put(value);
        }
        // FeatureList: one `vert` feature at +8 using lookup 0.
        put(1);
        out.extend_from_slice(b"vert");
        let mut put = |value: u16| out.extend_from_slice(&value.to_be_bytes());
        for value in [8, 0, 1, 0] {
            put(value);
        }
        // LookupList: one lookup at +4; Lookup: type 1 (single), one
        // subtable at +8; SingleSubst format 2 with coverage at +8;
        // Coverage format 1 with one glyph.
        for value in [1, 4, 1, 0, 1, 8, 2, 8, 1, to, 1, 1, from] {
            put(value);
        }
        out
    }

    fn with_gsub(font: &[u8], gsub: Vec<u8>) -> Vec<u8> {
        let face = ttf_parser::RawFace::parse(font, 0).expect("raw face");
        let mut tables = std::collections::BTreeMap::new();
        for record in face.table_records {
            let tag = record.tag.to_bytes();
            if &tag == b"GSUB" {
                continue;
            }
            let start = record.offset as usize;
            let end = start + record.length as usize;
            tables.insert(tag, font[start..end].to_vec());
        }
        tables.insert(*b"GSUB", gsub);
        crate::fonts::sfnt_subset::build_sfnt([0, 1, 0, 0], tables).expect("rebuilt sfnt")
    }

    #[test]
    fn vertical_text_uses_the_gsub_vertical_form() {
        let base = crate::render::font_rasterizer::fallback_fonts::LIBERATION_SANS_REGULAR;
        let face = ttf_parser::Face::parse(base, 0).expect("bundled face");
        let paren = face.glyph_index('(').expect("parenleft");
        let bracket = face.glyph_index('[').expect("bracketleft");
        let font = with_gsub(base, gsub_with_vert_substitution(paren.0, bracket.0));
        let face = ttf_parser::Face::parse(&font, 0).expect("font with vert");

        assert_eq!(vertical_form_glyph(&face, paren), bracket);
        // Glyphs without a vertical form are unchanged.
        let letter = face.glyph_index('A').expect("A");
        assert_eq!(vertical_form_glyph(&face, letter), letter);

        let none = VariationRequest::none();
        let (vertical, _) =
            extract_vertical_glyph_path_for_simple_var(&font, 0x28, '(', None, &none);
        let (bracket_path, _) = extract_glyph_path_for_simple_var(&font, 0x5B, '[', None, &none);
        let (paren_path, _) = extract_glyph_path_for_simple_var(&font, 0x28, '(', None, &none);
        assert_eq!(vertical, bracket_path);
        assert_ne!(vertical, paren_path);
        // Horizontal extraction never applies the vertical form.
        let (horizontal, _) = extract_glyph_path_for_simple_var(&font, 0x28, '(', None, &none);
        assert_eq!(horizontal, paren_path);
    }

    #[test]
    fn ligature_glyphs_are_found_under_either_name() {
        // Liberation Sans names its ligature "fi"; a PDF that says "f_i"
        // must still reach it.
        let font = crate::render::font_rasterizer::fallback_fonts::LIBERATION_SANS_REGULAR;
        let face = ttf_parser::Face::parse(font, 0).expect("bundled face");
        let Some(fi) = face.glyph_index_by_name("fi") else {
            return;
        };
        assert_eq!(face.glyph_index_by_name("f_i"), None);
        assert_eq!(
            resolve_glyph_id_for_simple(&face, 0, '\u{FFFD}', Some("f_i")),
            Some(fi)
        );
    }

    fn with_table(font: &[u8], table_tag: &[u8; 4], data: Vec<u8>) -> Vec<u8> {
        let face = ttf_parser::RawFace::parse(font, 0).expect("raw face");
        let mut tables = std::collections::BTreeMap::new();
        for record in face.table_records {
            let tag = record.tag.to_bytes();
            let start = record.offset as usize;
            let end = start + record.length as usize;
            tables.insert(tag, font[start..end].to_vec());
        }
        tables.insert(*table_tag, data);
        crate::fonts::sfnt_subset::build_sfnt([0, 1, 0, 0], tables).expect("rebuilt sfnt")
    }

    /// A cmap with one Mac Roman (1,0) format 6 subtable covering `codes`.
    fn mac_cmap(map: &[(u8, u16)]) -> Vec<u8> {
        let mut glyphs = [0u16; 256];
        for (code, gid) in map {
            glyphs[usize::from(*code)] = *gid;
        }
        let mut out = Vec::new();
        for value in [0u16, 1, 1, 0] {
            out.extend_from_slice(&value.to_be_bytes());
        }
        out.extend_from_slice(&12u32.to_be_bytes());
        // Format 6: trimmed table of 256 entries starting at code 0.
        for value in [6u16, 10 + 2 * 256, 0, 0, 256] {
            out.extend_from_slice(&value.to_be_bytes());
        }
        for gid in glyphs {
            out.extend_from_slice(&gid.to_be_bytes());
        }
        out
    }

    /// A cmap with one Windows Symbol (3,0) format 4 subtable.
    fn symbol_cmap(map: &[(u16, u16)]) -> Vec<u8> {
        let mut map = map.to_vec();
        map.sort();
        let segments = map.len() + 1;
        let mut ends = Vec::new();
        let mut starts = Vec::new();
        let mut deltas = Vec::new();
        for (code, gid) in &map {
            ends.push(*code);
            starts.push(*code);
            deltas.push(gid.wrapping_sub(*code));
        }
        ends.push(0xFFFF);
        starts.push(0xFFFF);
        deltas.push(1);
        let mut sub = Vec::new();
        let length = 16 + segments * 8;
        for value in [4u16, length as u16, 0, (segments * 2) as u16, 0, 0, 0] {
            sub.extend_from_slice(&value.to_be_bytes());
        }
        for value in &ends {
            sub.extend_from_slice(&value.to_be_bytes());
        }
        sub.extend_from_slice(&0u16.to_be_bytes());
        for value in starts.iter().chain(&deltas) {
            sub.extend_from_slice(&value.to_be_bytes());
        }
        for _ in 0..segments {
            sub.extend_from_slice(&0u16.to_be_bytes());
        }
        let mut out = Vec::new();
        for value in [0u16, 1, 3, 0] {
            out.extend_from_slice(&value.to_be_bytes());
        }
        out.extend_from_slice(&12u32.to_be_bytes());
        out.extend_from_slice(&sub);
        out
    }

    fn liberation_gid(ch: char) -> u16 {
        let font = crate::render::font_rasterizer::fallback_fonts::LIBERATION_SANS_REGULAR;
        ttf_parser::Face::parse(font, 0)
            .expect("bundled face")
            .glyph_index(ch)
            .expect("glyph")
            .0
    }

    #[test]
    fn mac_roman_cmap_is_keyed_by_the_mac_roman_code() {
        let base = crate::render::font_rasterizer::fallback_fonts::LIBERATION_SANS_REGULAR;
        let eacute = liberation_gid('\u{e9}');
        let a = liberation_gid('A');
        // Mac Roman 0x8E is e-acute; WinAnsi puts it at 0xE9.
        let font = with_table(base, b"cmap", mac_cmap(&[(0x8E, eacute), (0x41, a)]));
        let face = ttf_parser::Face::parse(&font, 0).expect("font with mac cmap");
        let lookup = SimpleGlyphLookup::default();
        assert_eq!(
            glyph_index_for_simple_with(&face, 0xE9, '\u{e9}', None, lookup),
            Some(GlyphId(eacute))
        );
        assert_eq!(
            glyph_index_for_simple_with(&face, 0x41, 'A', None, lookup),
            Some(GlyphId(a))
        );
    }

    #[test]
    fn symbolic_fonts_try_the_symbol_cmap_with_every_prefix_first() {
        let base = crate::render::font_rasterizer::fallback_fonts::LIBERATION_SANS_REGULAR;
        let (a, b, c) = (
            liberation_gid('A'),
            liberation_gid('B'),
            liberation_gid('C'),
        );
        let font = with_table(base, b"cmap", symbol_cmap(&[(0xF041, b), (0xF142, c)]));
        let face = ttf_parser::Face::parse(&font, 0).expect("font with symbol cmap");
        let symbolic = SimpleGlyphLookup {
            symbolic: true,
            truetype: true,
            ..Default::default()
        };
        // The (3,0) cmap beats the glyph name for a symbolic font...
        assert_eq!(
            glyph_index_for_simple_with(&face, 0x41, 'A', Some("A"), symbolic),
            Some(GlyphId(b))
        );
        // ...while a non-symbolic font still trusts the name.
        assert_eq!(
            glyph_index_for_simple_with(&face, 0x41, 'A', Some("A"), SimpleGlyphLookup::default()),
            Some(GlyphId(a))
        );
        // The 0xF100 prefix is tried too.
        assert_eq!(
            glyph_index_for_simple_with(&face, 0x42, 'B', None, symbolic),
            Some(GlyphId(c))
        );
    }

    #[test]
    fn truetype_notdef_draws_the_space_glyph() {
        let font = crate::render::font_rasterizer::fallback_fonts::LIBERATION_SANS_REGULAR;
        let face = ttf_parser::Face::parse(font, 0).expect("bundled face");
        let truetype = SimpleGlyphLookup {
            symbolic: false,
            truetype: true,
            ..Default::default()
        };
        assert_eq!(
            glyph_index_for_simple_with(&face, 0x41, 'A', Some(".notdef"), truetype),
            Some(GlyphId(liberation_gid(' ')))
        );
        // Other font types keep the old behaviour.
        assert_eq!(
            glyph_index_for_simple_with(
                &face,
                0x41,
                'A',
                Some(".notdef"),
                SimpleGlyphLookup::default()
            ),
            Some(GlyphId(liberation_gid('A')))
        );
    }
}
