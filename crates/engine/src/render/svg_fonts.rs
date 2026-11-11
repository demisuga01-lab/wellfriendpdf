//! Embedded fonts for SVG text ([`super::SvgTextMode::EmbeddedFonts`]).
//!
//! Every font program whose glyphs are drawn as `<text>` is embedded as an
//! `@font-face` data URI holding only the glyphs used:
//!
//! - TrueType and OpenType-CFF programs are subset with `subsetter`; bare
//!   CFF programs (FontFile3 /Type1C, /CIDFontType0C) are wrapped in an
//!   OpenType-CFF container first.
//! - Programs that cannot be subset that way (Type 1, CFF2, a CFF program
//!   with a non-uniform FontMatrix, anything the subsetter rejects) get an
//!   OpenType-CFF font synthesised from the very glyph outlines the outline
//!   mode draws.
//!
//! Each glyph is addressed by its own private-use code point (U+E000
//! upwards) through a rebuilt `cmap`, so the browser draws exactly the PDF's
//! glyph with no shaping, whatever the PDF's encoding; the decoded Unicode
//! stays in the invisible text layer. A glyph a subset program would not draw
//! the way the outline renderer does returns `None`, and the caller draws it
//! as an outline.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use crate::render::font_rasterizer::cff_support;
use crate::render::path::{Path, PathSegment};
use crate::render::text_decode::DecodedGlyph;

/// First private-use code point handed out per font.
const FIRST_CHAR: u32 = 0xE000;
/// Last BMP private-use code point (6400 glyphs per font).
const LAST_CHAR: u32 = 0xF8FF;

/// Fonts referenced by `<text>` runs of one SVG document.
#[derive(Default)]
pub(super) struct FontSet {
    fonts: Vec<EmbeddedFont>,
    /// Font per program allocation (kept alive so the address is never
    /// reused for another program) and per program content.
    by_program: HashMap<usize, (Arc<[u8]>, Option<usize>)>,
    by_content: HashMap<u64, usize>,
}

struct EmbeddedFont {
    kind: FontKind,
    /// Glyph key (source glyph id, or outline hash) -> character.
    chars: HashMap<u64, char>,
    /// The glyph of each character, in character order.
    glyphs: Vec<FontGlyph>,
}

enum FontKind {
    /// A subset of `source`: the program itself or its OpenType-CFF wrapper.
    Program { source: Vec<u8>, cff: bool },
    /// An OpenType-CFF font built from glyph outlines in `upem` font units.
    Outlines { upem: u16 },
}

enum FontGlyph {
    Gid(u16),
    Outline(Path),
}

/// How the glyphs of a program are addressed.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ProgramKind {
    Sfnt,
    BareCff,
}

fn program_kind(program: &[u8]) -> Option<ProgramKind> {
    if let Ok(face) = ttf_parser::Face::parse(program, 0) {
        let tables = face.tables();
        return (tables.glyf.is_some() || tables.cff.is_some()).then_some(ProgramKind::Sfnt);
    }
    cff_support::is_bare_cff(program).then_some(ProgramKind::BareCff)
}

/// Glyph id `glyph` draws in `program`, by the same lookup the outline
/// renderer uses; `None` when the program is not subset directly or the
/// glyph is not drawn the same way by a standard consumer of the program.
fn glyph_id(program: &[u8], glyph: &DecodedGlyph) -> Option<u16> {
    match program_kind(program)? {
        ProgramKind::Sfnt => {
            if glyph.is_gid {
                let face = ttf_parser::Face::parse(program, 0).ok()?;
                (glyph.code < face.number_of_glyphs()).then_some(glyph.code)
            } else {
                crate::render::glyph_outline::simple_glyph_sfnt_gid(
                    program,
                    glyph.code,
                    glyph.unicode,
                    glyph.glyph_name.as_deref(),
                )
            }
        }
        ProgramKind::BareCff => {
            let gid = if glyph.is_gid {
                cff_support::cid_keyed_gid_for_cid(program, glyph.code).unwrap_or(glyph.code)
            } else {
                cff_support::simple_glyph_gid(program, glyph.code, glyph.glyph_name.as_deref())?
            };
            cff_support::native_outline_ok(program, gid).then_some(gid)
        }
    }
}

fn outline_key(path: &Path) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for segment in &path.segments {
        match *segment {
            PathSegment::MoveTo(x, y) => (0u8, x.to_bits(), y.to_bits()).hash(&mut hasher),
            PathSegment::LineTo(x, y) => (1u8, x.to_bits(), y.to_bits()).hash(&mut hasher),
            PathSegment::CubicTo {
                cp1x,
                cp1y,
                cp2x,
                cp2y,
                x,
                y,
            } => (
                2u8,
                [cp1x, cp1y, cp2x, cp2y, x, y].map(f64::to_bits),
            )
                .hash(&mut hasher),
            PathSegment::ClosePath => 3u8.hash(&mut hasher),
        }
    }
    hasher.finish()
}

impl FontSet {
    /// Index of the embedded font for `program` (whose outlines are in
    /// `upem` units per em), registering it on first use; `None` when it
    /// cannot be embedded at all.
    pub(super) fn font_for(&mut self, program: &Arc<[u8]>, upem: f64) -> Option<usize> {
        let address = program.as_ptr() as usize;
        if let Some((_, index)) = self.by_program.get(&address) {
            return *index;
        }
        let index = self.register(program, upem);
        self.by_program
            .insert(address, (Arc::clone(program), index));
        index
    }

    fn register(&mut self, program: &[u8], upem: f64) -> Option<usize> {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        program.hash(&mut hasher);
        upem.to_bits().hash(&mut hasher);
        let content = hasher.finish();
        if let Some(index) = self.by_content.get(&content) {
            return Some(*index);
        }
        let kind = match subset_source(program) {
            Some((source, cff)) => FontKind::Program { source, cff },
            None => {
                let upem = upem.round();
                if !(16.0..=16384.0).contains(&upem) {
                    return None;
                }
                FontKind::Outlines { upem: upem as u16 }
            }
        };
        let index = self.fonts.len();
        self.fonts.push(EmbeddedFont {
            kind,
            chars: HashMap::new(),
            glyphs: Vec::new(),
        });
        self.by_content.insert(content, index);
        Some(index)
    }

    /// The character addressing `glyph` (drawn as `outline`) in font `font`;
    /// `None` when the glyph must stay an outline.
    pub(super) fn glyph_char(
        &mut self,
        font: usize,
        program: &[u8],
        glyph: &DecodedGlyph,
        outline: &Path,
    ) -> Option<char> {
        let font = self.fonts.get_mut(font)?;
        let (key, entry) = match font.kind {
            FontKind::Program { .. } => {
                let gid = glyph_id(program, glyph)?;
                (u64::from(gid), FontGlyph::Gid(gid))
            }
            FontKind::Outlines { .. } => (outline_key(outline), FontGlyph::Outline(outline.clone())),
        };
        if let Some(ch) = font.chars.get(&key) {
            return Some(*ch);
        }
        let next = FIRST_CHAR + u32::try_from(font.glyphs.len()).ok()?;
        if next > LAST_CHAR {
            return None;
        }
        let ch = char::from_u32(next)?;
        font.chars.insert(key, ch);
        font.glyphs.push(entry);
        Some(ch)
    }

    pub(super) fn family(font: usize) -> String {
        format!("wf-font-{font}")
    }

    /// `<style>` element with one `@font-face` per font that has glyphs.
    pub(super) fn style_element(&self) -> String {
        let mut css = String::new();
        for (index, font) in self.fonts.iter().enumerate() {
            if font.glyphs.is_empty() {
                continue;
            }
            let family = Self::family(index);
            match build_font(font, &family) {
                Ok((bytes, cff)) => {
                    let (mime, format) = if cff {
                        ("font/otf", "opentype")
                    } else {
                        ("font/ttf", "truetype")
                    };
                    css.push_str(&format!(
                        "@font-face{{font-family:\"{family}\";src:url(data:{mime};base64,{}) format(\"{format}\");}}\n",
                        super::base64_encode(&bytes)
                    ));
                }
                Err(reason) => log::warn!("SVG font {family} could not be embedded: {reason}"),
            }
        }
        if css.is_empty() {
            return String::new();
        }
        format!("<style type=\"text/css\"><![CDATA[\n{css}]]></style>\n")
    }

    #[cfg(test)]
    pub(super) fn font_bytes(&self, font: usize) -> Option<Vec<u8>> {
        build_font(self.fonts.get(font)?, &Self::family(font))
            .ok()
            .map(|(bytes, _)| bytes)
    }
}

/// The sfnt to subset for `program` (itself, or its OpenType-CFF wrapper)
/// and whether it has CFF outlines, when the subsetter accepts it (a trial
/// subset with only `.notdef`).
fn subset_source(program: &[u8]) -> Option<(Vec<u8>, bool)> {
    let (source, cff) = match program_kind(program)? {
        ProgramKind::Sfnt => {
            let face = ttf_parser::Face::parse(program, 0).ok()?;
            (program.to_vec(), face.tables().glyf.is_none())
        }
        ProgramKind::BareCff => {
            let (count, widths, upem) = cff_support::wrap_metrics(program)?;
            let tables = otf_tables(program.to_vec(), count, &widths, upem)?;
            (crate::fonts::sfnt_subset::build_sfnt(*b"OTTO", tables).ok()?, true)
        }
    };
    let mut remapper = subsetter::GlyphRemapper::new();
    remapper.remap(0);
    match subsetter::subset(&source, 0, &remapper) {
        Ok(_) => Some((source, cff)),
        Err(err) => {
            log::debug!("SVG font subsetting refused ({err:?}); using outlines");
            None
        }
    }
}

/// The font file for `font`, and whether it is OpenType-CFF.
fn build_font(font: &EmbeddedFont, family: &str) -> Result<(Vec<u8>, bool), String> {
    let (mut tables, mapping, cff) = match &font.kind {
        FontKind::Program { source, cff } => {
            let mut remapper = subsetter::GlyphRemapper::new();
            remapper.remap(0);
            for glyph in &font.glyphs {
                if let FontGlyph::Gid(gid) = glyph {
                    remapper.remap(*gid);
                }
            }
            let subset = subsetter::subset(source, 0, &remapper)
                .map_err(|err| format!("subsetting failed: {err:?}"))?;
            let mapping = font
                .glyphs
                .iter()
                .map(|glyph| match glyph {
                    FontGlyph::Gid(gid) => remapper.get(*gid).ok_or("subset lost a glyph"),
                    FontGlyph::Outline(_) => Err("outline glyph in a program font"),
                })
                .collect::<Result<Vec<u16>, &str>>()?;
            (parse_tables(&subset)?, mapping, *cff)
        }
        FontKind::Outlines { upem } => {
            let outlines: Vec<&Path> = font
                .glyphs
                .iter()
                .filter_map(|glyph| match glyph {
                    FontGlyph::Outline(path) => Some(path),
                    FontGlyph::Gid(_) => None,
                })
                .collect();
            let count = u16::try_from(outlines.len() + 1).map_err(|_| "too many glyphs")?;
            let cff = outline_cff(family, &outlines, *upem)?;
            let widths = vec![*upem / 2; usize::from(count)];
            let tables =
                otf_tables(cff, count, &widths, *upem).ok_or("font tables could not be built")?;
            (tables, (1..count).collect(), true)
        }
    };
    let mut mapping: Vec<(u16, u16)> = mapping
        .iter()
        .enumerate()
        .map(|(index, gid)| (FIRST_CHAR as u16 + index as u16, *gid))
        .collect();
    mapping.sort_unstable();
    let (first, last) = (
        mapping.first().map_or(0xE000, |m| m.0),
        mapping.last().map_or(0xE000, |m| m.0),
    );
    let upem = read_u16(tables.get(b"head").ok_or("font has no head")?, 18)?;
    let hhea = tables.get(b"hhea").ok_or("font has no hhea")?;
    let ascender = read_u16(hhea, 4)? as i16;
    let descender = read_u16(hhea, 6)? as i16;
    let line_gap = read_u16(hhea, 8)? as i16;
    tables.insert(*b"cmap", cmap_table(&mapping));
    tables.insert(
        *b"OS/2",
        os2_table(upem, ascender, descender, line_gap, first, last),
    );
    tables.insert(*b"name", name_table(family));
    tables.insert(*b"post", post_table());
    let version = if cff { *b"OTTO" } else { [0, 1, 0, 0] };
    let bytes = crate::fonts::sfnt_subset::build_sfnt(version, tables)
        .map_err(|err| format!("font rebuild failed: {err}"))?;
    Ok((bytes, cff))
}

/// `head`, `hhea`, `hmtx` and `maxp` around a CFF table with `count`
/// glyphs; the remaining required tables are added by [`build_font`].
fn otf_tables(
    cff: Vec<u8>,
    count: u16,
    widths: &[u16],
    upem: u16,
) -> Option<BTreeMap<[u8; 4], Vec<u8>>> {
    let em = i16::try_from(upem).ok()?;
    let mut head = Vec::with_capacity(54);
    head.extend_from_slice(&0x0001_0000u32.to_be_bytes()); // version
    head.extend_from_slice(&0x0001_0000u32.to_be_bytes()); // fontRevision
    head.extend_from_slice(&0u32.to_be_bytes()); // checkSumAdjustment
    head.extend_from_slice(&0x5F0F_3CF5u32.to_be_bytes()); // magicNumber
    head.extend_from_slice(&0x0003u16.to_be_bytes()); // flags
    head.extend_from_slice(&upem.to_be_bytes());
    head.extend_from_slice(&[0; 16]); // created, modified
    for value in [-em, -em, em.saturating_mul(2), em.saturating_mul(2)] {
        head.extend_from_slice(&value.to_be_bytes()); // xMin, yMin, xMax, yMax
    }
    head.extend_from_slice(&0u16.to_be_bytes()); // macStyle
    head.extend_from_slice(&3u16.to_be_bytes()); // lowestRecPPEM
    head.extend_from_slice(&2i16.to_be_bytes()); // fontDirectionHint
    head.extend_from_slice(&0i16.to_be_bytes()); // indexToLocFormat
    head.extend_from_slice(&0i16.to_be_bytes()); // glyphDataFormat

    let max_advance = widths.iter().copied().max().unwrap_or(upem);
    let mut hhea = Vec::with_capacity(36);
    hhea.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    hhea.extend_from_slice(&((i32::from(em) * 4 / 5) as i16).to_be_bytes()); // ascender
    hhea.extend_from_slice(&(-(i32::from(em) / 5) as i16).to_be_bytes()); // descender
    hhea.extend_from_slice(&0i16.to_be_bytes()); // lineGap
    hhea.extend_from_slice(&max_advance.to_be_bytes());
    hhea.extend_from_slice(&[0; 6]); // minLSB, minRSB, xMaxExtent
    hhea.extend_from_slice(&1i16.to_be_bytes()); // caretSlopeRise
    hhea.extend_from_slice(&[0; 4]); // caretSlopeRun, caretOffset
    hhea.extend_from_slice(&[0; 8]); // reserved
    hhea.extend_from_slice(&0i16.to_be_bytes()); // metricDataFormat
    hhea.extend_from_slice(&count.to_be_bytes()); // numberOfHMetrics

    let mut hmtx = Vec::with_capacity(usize::from(count) * 4);
    for index in 0..usize::from(count) {
        hmtx.extend_from_slice(&widths.get(index).copied().unwrap_or(0).to_be_bytes());
        hmtx.extend_from_slice(&0i16.to_be_bytes());
    }
    let mut maxp = 0x0000_5000u32.to_be_bytes().to_vec();
    maxp.extend_from_slice(&count.to_be_bytes());

    let mut tables = BTreeMap::new();
    tables.insert(*b"CFF ", cff);
    tables.insert(*b"head", head);
    tables.insert(*b"hhea", hhea);
    tables.insert(*b"hmtx", hmtx);
    tables.insert(*b"maxp", maxp);
    Some(tables)
}

/// A bare CFF font (Type 2 charstrings, no hints) holding `.notdef` and one
/// glyph per outline, in `upem` units per em.
fn outline_cff(name: &str, outlines: &[&Path], upem: u16) -> Result<Vec<u8>, String> {
    let count = outlines.len() + 1;
    let mut charstrings = vec![vec![14u8]]; // .notdef: endchar
    for outline in outlines {
        charstrings.push(type2_charstring(outline)?);
    }
    let strings: Vec<Vec<u8>> = (1..count).map(|gid| format!("g{gid}").into_bytes()).collect();
    let mut charset = Vec::new();
    if count > 1 {
        // Format 2, one range: glyphs 1.. are the custom strings (SID 391..).
        charset.push(2);
        charset.extend_from_slice(&391u16.to_be_bytes());
        charset.extend_from_slice(&u16::try_from(count - 2).map_err(|_| "too many glyphs")?.to_be_bytes());
    } else {
        charset.push(0);
    }
    // Private DICT: defaultWidthX 0, nominalWidthX 0.
    let private = vec![139, 20, 139, 21];
    let name_index = cff_index(&[name.as_bytes().to_vec()]);
    let string_index = cff_index(&strings);
    let global_subrs = cff_index(&[]);
    let charstrings_index = cff_index(&charstrings);

    let top_dict = |charset_at: i32, charstrings_at: i32, private_at: i32| {
        let mut dict = Vec::new();
        if upem != 1000 {
            let scale = 1.0 / f64::from(upem);
            for value in [scale, 0.0, 0.0, scale, 0.0, 0.0] {
                dict.extend_from_slice(&cff_real(value));
            }
            dict.extend_from_slice(&[12, 7]); // FontMatrix
        }
        dict.extend_from_slice(&cff_int5(charset_at));
        dict.push(15); // charset
        dict.extend_from_slice(&cff_int5(charstrings_at));
        dict.push(17); // CharStrings
        dict.extend_from_slice(&cff_int5(private.len() as i32));
        dict.extend_from_slice(&cff_int5(private_at));
        dict.push(18); // Private
        dict
    };
    let top_len = cff_index(&[top_dict(0, 0, 0)]).len();
    let charset_at = 4 + name_index.len() + top_len + string_index.len() + global_subrs.len();
    let charstrings_at = charset_at + charset.len();
    let private_at = charstrings_at + charstrings_index.len();
    let offset = |value: usize| i32::try_from(value).map_err(|_| "CFF too large".to_string());
    let top_index = cff_index(&[top_dict(
        offset(charset_at)?,
        offset(charstrings_at)?,
        offset(private_at)?,
    )]);
    let mut out = vec![1, 0, 4, 4]; // major, minor, hdrSize, offSize
    for part in [
        &name_index,
        &top_index,
        &string_index,
        &global_subrs,
        &charset,
        &charstrings_index,
        &private,
    ] {
        out.extend_from_slice(part);
    }
    Ok(out)
}

fn cff_index(items: &[Vec<u8>]) -> Vec<u8> {
    let mut out = (items.len() as u16).to_be_bytes().to_vec();
    if items.is_empty() {
        return out;
    }
    let total: usize = items.iter().map(Vec::len).sum::<usize>() + 1;
    let off_size: usize = match total {
        0..=0xFF => 1,
        0x100..=0xFFFF => 2,
        0x1_0000..=0xFF_FFFF => 3,
        _ => 4,
    };
    out.push(off_size as u8);
    let mut offset = 1usize;
    let push_offset = |out: &mut Vec<u8>, value: usize| {
        out.extend_from_slice(&(value as u32).to_be_bytes()[4 - off_size..]);
    };
    push_offset(&mut out, offset);
    for item in items {
        offset += item.len();
        push_offset(&mut out, offset);
    }
    for item in items {
        out.extend_from_slice(item);
    }
    out
}

/// DICT integer in the fixed five-byte form (so offsets can be patched).
fn cff_int5(value: i32) -> [u8; 5] {
    let [a, b, c, d] = value.to_be_bytes();
    [29, a, b, c, d]
}

/// DICT real number (nibble-encoded decimal).
fn cff_real(value: f64) -> Vec<u8> {
    let text = format!("{value:.10e}");
    let mut nibbles = Vec::new();
    let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
    let mantissa = mantissa.trim_end_matches('0').trim_end_matches('.');
    for ch in mantissa.chars() {
        nibbles.push(match ch {
            '0'..='9' => ch as u8 - b'0',
            '.' => 0xa,
            _ => 0xe, // '-'
        });
    }
    let exponent: i32 = exponent.parse().unwrap_or(0);
    if exponent != 0 {
        nibbles.push(if exponent < 0 { 0xc } else { 0xb });
        for ch in exponent.unsigned_abs().to_string().chars() {
            nibbles.push(ch as u8 - b'0');
        }
    }
    nibbles.push(0xf);
    if nibbles.len() % 2 == 1 {
        nibbles.push(0xf);
    }
    let mut out = vec![30];
    out.extend(nibbles.chunks(2).map(|pair| (pair[0] << 4) | pair[1]));
    out
}

/// Type 2 charstring of an outline: `rmoveto`/`rlineto`/`rrcurveto` with
/// 16.16 fixed operands where the coordinates are not integral, `endchar`.
fn type2_charstring(path: &Path) -> Result<Vec<u8>, String> {
    fn fixed(value: f64) -> Result<i64, String> {
        if !value.is_finite() || value.abs() > 16000.0 {
            return Err("glyph coordinate out of range".to_string());
        }
        Ok((value * 65536.0).round() as i64)
    }
    fn push_number(out: &mut Vec<u8>, delta: i64) {
        if delta % 65536 == 0 {
            let value = delta / 65536;
            match value {
                -107..=107 => out.push((value + 139) as u8),
                108..=1131 => {
                    let v = value - 108;
                    out.extend_from_slice(&[(v / 256 + 247) as u8, (v % 256) as u8]);
                }
                -1131..=-108 => {
                    let v = -value - 108;
                    out.extend_from_slice(&[(v / 256 + 251) as u8, (v % 256) as u8]);
                }
                _ => {
                    out.push(28);
                    out.extend_from_slice(&(value as i16).to_be_bytes());
                }
            }
        } else {
            out.push(255);
            out.extend_from_slice(&(delta as i32).to_be_bytes());
        }
    }
    let mut out = Vec::new();
    let mut current = (0i64, 0i64);
    let mut start = (0i64, 0i64);
    let mut open = false;
    let mut pending_close = false;
    for segment in &path.segments {
        if pending_close && !matches!(segment, PathSegment::MoveTo(..)) {
            // A segment after closepath starts from the subpath's start.
            push_number(&mut out, start.0 - current.0);
            push_number(&mut out, start.1 - current.1);
            out.push(21);
            current = start;
        }
        pending_close = false;
        match *segment {
            PathSegment::MoveTo(x, y) => {
                let point = (fixed(x)?, fixed(y)?);
                push_number(&mut out, point.0 - current.0);
                push_number(&mut out, point.1 - current.1);
                out.push(21); // rmoveto
                current = point;
                start = point;
                open = true;
            }
            PathSegment::LineTo(x, y) => {
                if !open {
                    return Err("line before moveto".to_string());
                }
                let point = (fixed(x)?, fixed(y)?);
                push_number(&mut out, point.0 - current.0);
                push_number(&mut out, point.1 - current.1);
                out.push(5); // rlineto
                current = point;
            }
            PathSegment::CubicTo {
                cp1x,
                cp1y,
                cp2x,
                cp2y,
                x,
                y,
            } => {
                if !open {
                    return Err("curve before moveto".to_string());
                }
                for (px, py) in [(cp1x, cp1y), (cp2x, cp2y), (x, y)] {
                    let point = (fixed(px)?, fixed(py)?);
                    push_number(&mut out, point.0 - current.0);
                    push_number(&mut out, point.1 - current.1);
                    current = point;
                }
                out.push(8); // rrcurveto
            }
            PathSegment::ClosePath => pending_close = true,
        }
    }
    out.push(14); // endchar
    Ok(out)
}

fn read_u16(data: &[u8], at: usize) -> Result<u16, String> {
    data.get(at..at + 2)
        .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
        .ok_or_else(|| "truncated table".to_string())
}

fn parse_tables(font: &[u8]) -> Result<BTreeMap<[u8; 4], Vec<u8>>, String> {
    let count = usize::from(read_u16(font, 4)?);
    let mut out = BTreeMap::new();
    for index in 0..count {
        let record = font
            .get(12 + index * 16..12 + index * 16 + 16)
            .ok_or("truncated table directory")?;
        let tag: [u8; 4] = record[0..4].try_into().map_err(|_| "bad tag")?;
        let offset = u32::from_be_bytes([record[8], record[9], record[10], record[11]]) as usize;
        let length = u32::from_be_bytes([record[12], record[13], record[14], record[15]]) as usize;
        let data = font
            .get(offset..offset.saturating_add(length))
            .ok_or("table outside font")?;
        out.insert(tag, data.to_vec());
    }
    Ok(out)
}

/// `cmap` with one Windows Unicode BMP (3, 1) format 4 subtable mapping
/// each character to its glyph; runs of consecutive characters and glyphs
/// share a segment.
fn cmap_table(mapping: &[(u16, u16)]) -> Vec<u8> {
    let mut segments: Vec<(u16, u16, u16)> = Vec::new(); // start, end, first gid
    for &(code, gid) in mapping {
        if let Some(last) = segments.last_mut() {
            if u32::from(last.1) + 1 == u32::from(code)
                && u32::from(last.2) + u32::from(last.1 - last.0) + 1 == u32::from(gid)
            {
                last.1 = code;
                continue;
            }
        }
        segments.push((code, code, gid));
    }
    segments.push((0xFFFF, 0xFFFF, 0));
    let seg_count = segments.len() as u16;
    let entry_selector = 15 - seg_count.leading_zeros() as u16;
    let search_range = 2 * (1u16 << entry_selector);
    let mut sub = Vec::new();
    sub.extend_from_slice(&4u16.to_be_bytes());
    sub.extend_from_slice(&0u16.to_be_bytes()); // length, patched below
    sub.extend_from_slice(&0u16.to_be_bytes()); // language
    sub.extend_from_slice(&(seg_count * 2).to_be_bytes());
    sub.extend_from_slice(&search_range.to_be_bytes());
    sub.extend_from_slice(&entry_selector.to_be_bytes());
    sub.extend_from_slice(&(seg_count * 2 - search_range).to_be_bytes());
    for segment in &segments {
        sub.extend_from_slice(&segment.1.to_be_bytes());
    }
    sub.extend_from_slice(&0u16.to_be_bytes()); // reservedPad
    for segment in &segments {
        sub.extend_from_slice(&segment.0.to_be_bytes());
    }
    for segment in &segments {
        let delta = if segment.0 == 0xFFFF {
            1
        } else {
            segment.2.wrapping_sub(segment.0)
        };
        sub.extend_from_slice(&delta.to_be_bytes());
    }
    for _ in &segments {
        sub.extend_from_slice(&0u16.to_be_bytes()); // idRangeOffset
    }
    let length = sub.len() as u16;
    sub[2..4].copy_from_slice(&length.to_be_bytes());
    let mut cmap = Vec::new();
    cmap.extend_from_slice(&0u16.to_be_bytes()); // version
    cmap.extend_from_slice(&1u16.to_be_bytes()); // numTables
    cmap.extend_from_slice(&3u16.to_be_bytes()); // Windows
    cmap.extend_from_slice(&1u16.to_be_bytes()); // Unicode BMP
    cmap.extend_from_slice(&12u32.to_be_bytes());
    cmap.extend_from_slice(&sub);
    cmap
}

/// `OS/2` version 4 with metrics from `hhea`.
fn os2_table(upem: u16, ascender: i16, descender: i16, line_gap: i16, first: u16, last: u16) -> Vec<u8> {
    let em = i32::from(upem);
    let scaled = |numerator: i32| ((em * numerator) / 1000).clamp(-32768, 32767) as i16;
    let mut os2 = Vec::with_capacity(96);
    let mut push16 = |value: i16| os2.extend_from_slice(&value.to_be_bytes());
    push16(4); // version
    push16(scaled(500)); // xAvgCharWidth
    push16(400); // usWeightClass
    push16(5); // usWidthClass
    push16(0); // fsType: installable
    for value in [650, 600, 0, 75, 650, 600, 0, 350, 50, 250] {
        push16(scaled(value)); // sub/superscript sizes and offsets, strikeout
    }
    push16(0); // sFamilyClass
    os2.extend_from_slice(&[0; 10]); // panose
    // ulUnicodeRange: bit 60 is the Private Use Area.
    os2.extend_from_slice(&0u32.to_be_bytes());
    os2.extend_from_slice(&(1u32 << 28).to_be_bytes());
    os2.extend_from_slice(&0u32.to_be_bytes());
    os2.extend_from_slice(&0u32.to_be_bytes());
    os2.extend_from_slice(b"WFPD"); // achVendID
    os2.extend_from_slice(&0x0040u16.to_be_bytes()); // fsSelection: REGULAR
    os2.extend_from_slice(&first.to_be_bytes());
    os2.extend_from_slice(&last.to_be_bytes());
    let win_ascent = ascender.max(0) as u16;
    let win_descent = descender.min(0).unsigned_abs();
    for value in [ascender, descender, line_gap] {
        os2.extend_from_slice(&value.to_be_bytes()); // sTypo*
    }
    os2.extend_from_slice(&win_ascent.to_be_bytes());
    os2.extend_from_slice(&win_descent.to_be_bytes());
    os2.extend_from_slice(&1u32.to_be_bytes()); // ulCodePageRange1: Latin 1
    os2.extend_from_slice(&0u32.to_be_bytes());
    os2.extend_from_slice(&scaled(500).to_be_bytes()); // sxHeight
    os2.extend_from_slice(&scaled(700).to_be_bytes()); // sCapHeight
    os2.extend_from_slice(&0u16.to_be_bytes()); // usDefaultChar
    os2.extend_from_slice(&first.to_be_bytes()); // usBreakChar
    os2.extend_from_slice(&0u16.to_be_bytes()); // usMaxContext
    os2
}

/// `name` with family, subfamily, full and PostScript names (Windows,
/// Unicode BMP, en-US).
fn name_table(family: &str) -> Vec<u8> {
    let records: [(u16, &str); 4] = [(1, family), (2, "Regular"), (4, family), (6, family)];
    let mut strings = Vec::new();
    let mut table = Vec::new();
    table.extend_from_slice(&0u16.to_be_bytes()); // format
    table.extend_from_slice(&(records.len() as u16).to_be_bytes());
    table.extend_from_slice(&(6 + 12 * records.len() as u16).to_be_bytes());
    for (name_id, text) in records {
        let encoded: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
        for value in [3u16, 1, 0x0409, name_id, encoded.len() as u16, strings.len() as u16] {
            table.extend_from_slice(&value.to_be_bytes());
        }
        strings.extend_from_slice(&encoded);
    }
    table.extend_from_slice(&strings);
    table
}

/// `post` version 3 (no glyph names).
fn post_table() -> Vec<u8> {
    let mut post = 0x0003_0000u32.to_be_bytes().to_vec();
    post.extend_from_slice(&[0; 28]);
    post
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmap_maps_private_use_characters_to_glyphs() {
        let cmap = cmap_table(&[(0xE000, 1), (0xE001, 2), (0xE002, 5)]);
        let mut tables = BTreeMap::new();
        tables.insert(*b"cmap", cmap);
        let parsed = ttf_parser::cmap::Table::parse(&tables[b"cmap"]).unwrap();
        let subtable = parsed.subtables.into_iter().next().unwrap();
        assert_eq!(subtable.glyph_index(0xE000), Some(ttf_parser::GlyphId(1)));
        assert_eq!(subtable.glyph_index(0xE001), Some(ttf_parser::GlyphId(2)));
        assert_eq!(subtable.glyph_index(0xE002), Some(ttf_parser::GlyphId(5)));
        assert_eq!(subtable.glyph_index(0xE003), None);
    }

    fn gid_glyph(gid: u16) -> DecodedGlyph {
        DecodedGlyph {
            code: gid,
            unicode: '\u{FFFD}',
            text: String::new(),
            glyph_name: None,
            is_space: false,
            width: None,
            is_gid: true,
            is_vertical: false,
            vertical_advance: None,
            vertical_origin: None,
        }
    }

    /// Embed `glyphs` (glyph and outline) of `program`; the font bytes and
    /// each glyph's character.
    fn embedded(
        program: &[u8],
        upem: f64,
        glyphs: &[(DecodedGlyph, Path)],
    ) -> (Vec<u8>, Vec<char>) {
        let program: Arc<[u8]> = Arc::from(program);
        let mut set = FontSet::default();
        let font = set.font_for(&program, upem).expect("embeddable");
        assert_eq!(set.font_for(&program, upem), Some(font), "registered once");
        let chars: Vec<char> = glyphs
            .iter()
            .map(|(glyph, outline)| set.glyph_char(font, &program, glyph, outline).unwrap())
            .collect();
        let (glyph, outline) = &glyphs[0];
        assert_eq!(
            set.glyph_char(font, &program, glyph, outline),
            Some(chars[0]),
            "stable"
        );
        assert!(set
            .style_element()
            .contains("@font-face{font-family:\"wf-font-0\""));
        (set.font_bytes(font).unwrap(), chars)
    }

    fn assert_glyph_box(font: &[u8], ch: char, expected: ttf_parser::Rect) {
        let face = ttf_parser::Face::parse(font, 0).expect("embedded font parses");
        for tag in [
            b"cmap", b"OS/2", b"name", b"post", b"head", b"hhea", b"hmtx", b"maxp",
        ] {
            assert!(
                face.raw_face()
                    .table(ttf_parser::Tag::from_bytes(tag))
                    .is_some(),
                "{} present",
                String::from_utf8_lossy(tag)
            );
        }
        let gid = face.glyph_index(ch).expect("cmap maps the character");
        assert_eq!(face.glyph_bounding_box(gid), Some(expected));
    }

    fn sfnt_box(program: &[u8], gid: u16) -> ttf_parser::Rect {
        ttf_parser::Face::parse(program, 0)
            .unwrap()
            .glyph_bounding_box(ttf_parser::GlyphId(gid))
            .unwrap()
    }

    #[test]
    fn truetype_programs_are_subset_with_a_private_use_cmap() {
        let source = include_bytes!("../../fonts/LiberationSans-Regular.ttf");
        let face = ttf_parser::Face::parse(source, 0).unwrap();
        let w = face.glyph_index('W').unwrap().0;
        let a = face.glyph_index('a').unwrap().0;
        let (font, chars) = embedded(
            source,
            1000.0,
            &[(gid_glyph(w), Path::new()), (gid_glyph(a), Path::new())],
        );
        assert!(font.starts_with(&[0, 1, 0, 0]));
        assert!(font.len() < source.len() / 10, "subset: {} bytes", font.len());
        assert_eq!(chars, vec!['\u{E000}', '\u{E001}']);
        assert_glyph_box(&font, chars[0], sfnt_box(source, w));
        assert_glyph_box(&font, chars[1], sfnt_box(source, a));
    }

    #[test]
    fn bare_cff_programs_are_wrapped_as_opentype_cff() {
        let source = include_bytes!("../../fonts/FoxitSymbol.cff");
        let table = ttf_parser::cff::Table::parse(source).unwrap();
        let alpha = table.glyph_index_by_name("alpha").unwrap().0;
        let mut sink = crate::render::font_rasterizer::GlyphToPath::new();
        let expected = table
            .outline(ttf_parser::GlyphId(alpha), &mut sink)
            .unwrap();
        let (font, chars) = embedded(source, 1000.0, &[(gid_glyph(alpha), Path::new())]);
        assert!(font.starts_with(b"OTTO"));
        assert_glyph_box(&font, chars[0], expected);
    }

    #[test]
    fn other_programs_embed_a_font_built_from_the_outlines() {
        // Not an sfnt or CFF program (a Type 1 font, say): the font is
        // synthesised from the outlines the renderer draws.
        let program = b"%!PS-AdobeFont-1.0: Example".to_vec();
        let mut square = Path::new();
        square.move_to(100.0, 0.0);
        square.line_to(600.0, 0.0);
        square.line_to(600.0, 500.0);
        square.close();
        let mut curve = Path::new();
        curve.move_to(50.0, -200.0);
        curve.curve_to(50.0, 700.0, 900.0, 700.0, 900.0, -200.0);
        curve.close();
        let glyphs = [
            (gid_glyph(1), square.clone()),
            (gid_glyph(2), curve),
            (gid_glyph(3), square),
        ];
        let (font, chars) = embedded(&program, 2048.0, &glyphs);
        assert!(font.starts_with(b"OTTO"));
        assert_eq!(chars[0], chars[2], "an outline is embedded once");
        let face = ttf_parser::Face::parse(&font, 0).unwrap();
        assert_eq!(face.units_per_em(), 2048);
        let rect = |x_min, y_min, x_max, y_max| ttf_parser::Rect {
            x_min,
            y_min,
            x_max,
            y_max,
        };
        assert_glyph_box(&font, chars[0], rect(100, 0, 600, 500));
        let curve_box = face
            .glyph_bounding_box(face.glyph_index(chars[1]).unwrap())
            .unwrap();
        // ttf-parser bounds CFF curves by their control points.
        assert_eq!(curve_box, rect(50, -200, 900, 700));
    }

    #[test]
    fn type2_numbers_and_dict_reals_encode() {
        let mut path = Path::new();
        path.move_to(0.5, -1200.0);
        path.line_to(32.25, 40.0);
        let charstring = type2_charstring(&path).unwrap();
        assert_eq!(charstring.first(), Some(&255));
        assert_eq!(charstring.last(), Some(&14));
        assert_eq!(
            cff_real(0.00048828125),
            vec![30, 0x4a, 0x88, 0x28, 0x12, 0x5c, 0x4f]
        );
        assert_eq!(cff_real(0.0), vec![30, 0x0f]);
    }
}
