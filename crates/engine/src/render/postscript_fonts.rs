//! Embedded fonts for the PostScript backend (`pdftops` font handling).
//!
//! Text drawn in ordinary fill mode is written as real PostScript text —
//! strings shown with `xyshow` in an embedded font — instead of one filled
//! outline per glyph occurrence. Each glyph is then stored once per document,
//! the file stays small, and the text stays searchable and extractable
//! (glyph names follow the Adobe Glyph List `uniXXXX` convention).
//!
//! | PDF font program | PostScript font |
//! |---|---|
//! | TrueType (`glyf` outlines) | Type 42, subset to the used glyphs, unhinted |
//! | Type 1, CFF / Type1C, OpenType CFF, CID CFF | Type 1, charstrings generated from the glyph outlines |
//! | Type 3, unparsable programs | not embedded; glyphs stay filled outlines |
//!
//! Glyphs are collected per page in a [`PsFontResources`]; the document
//! assembler merges the pages' glyph sets, so every font program is defined
//! once, in the document setup, with the union of the glyphs the pages use.
//! A page then builds its own 256-code re-encodings of those fonts (the
//! `wfre` prolog procedure) and positions every glyph exactly with `xyshow`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Arc;

use crate::render::path::{Path, PathSegment};

use super::{ascii85_encode, ps_number};

/// Fonts one PostScript page draws text with. Filled by the page renderer
/// and merged across pages by [`super::assemble_ps_document`], which defines
/// each font program once in the document setup.
#[derive(Debug, Clone, Default)]
pub struct PsFontResources {
    pub(crate) fonts: Vec<PsFontProgram>,
}

impl PsFontResources {
    /// True when the page draws no text through an embedded font.
    pub fn is_empty(&self) -> bool {
        self.fonts.is_empty()
    }

    /// Number of distinct font programs the page uses.
    pub fn len(&self) -> usize {
        self.fonts.len()
    }

    /// The program with `base_name`, created with `kind` on first use.
    pub(crate) fn program_mut(&mut self, base_name: &str, kind: PsFontKind) -> &mut PsFontProgram {
        let index = match self.fonts.iter().position(|f| f.base_name == base_name) {
            Some(index) => index,
            None => {
                self.fonts.push(PsFontProgram {
                    base_name: base_name.to_string(),
                    kind,
                    glyphs: BTreeMap::new(),
                });
                self.fonts.len() - 1
            }
        };
        &mut self.fonts[index]
    }
}

/// One font program and the glyphs (by PostScript glyph name) drawn with it.
#[derive(Debug, Clone)]
pub(crate) struct PsFontProgram {
    pub(crate) base_name: String,
    pub(crate) kind: PsFontKind,
    pub(crate) glyphs: BTreeMap<String, PsGlyphSource>,
}

#[derive(Debug, Clone)]
pub(crate) enum PsFontKind {
    /// A TrueType (`glyf`) program, embedded as a Type 42 font.
    TrueType(Arc<[u8]>),
    /// Any other outline font, embedded as a Type 1 font built from the
    /// glyph outlines (in font units, `units_per_em` to the em).
    Outline { units_per_em: f64 },
}

#[derive(Debug, Clone)]
pub(crate) enum PsGlyphSource {
    /// Glyph id in the TrueType program.
    Gid(u16),
    /// Glyph outline in font units (`None` for a blank glyph) and its
    /// advance width in font units.
    Outline { path: Option<Path>, advance: f64 },
}

/// Document-unique PostScript font name for a font program, derived from its
/// bytes so the same program used on several pages (or under several PDF
/// font resources) is embedded once.
pub(crate) fn ps_font_base_name(program: &[u8]) -> String {
    // FNV-1a, 64 bit.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in program {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("WF{hash:016X}")
}

/// PostScript glyph name for a glyph identified by `ident` (glyph id or
/// per-font outline number) that represents `unicode`.
///
/// Names follow the Adobe Glyph List convention — `uniXXXX` / `uXXXXX`
/// followed by a `.suffix` that AGL parsers ignore — so text extracted from
/// the PostScript (for example by `ps2pdf` + `pdftotext`) maps back to
/// Unicode. Glyphs without a meaningful Unicode value get `gN`.
pub(crate) fn ps_glyph_name(unicode: char, ident: impl std::fmt::Display) -> String {
    let cp = u32::from(unicode);
    let meaningful =
        cp >= 0x20 && cp != 0x7F && !(0x80..0xA0).contains(&cp) && cp != 0xFFFD && cp != 0xFFFF;
    if !meaningful {
        format!("g{ident}")
    } else if cp <= 0xFFFF {
        format!("uni{cp:04X}.{ident}")
    } else {
        format!("u{cp:X}.{ident}")
    }
}

/// Prolog procedures: `wfre` creates a page-local re-encoding of an embedded
/// font (`/NewName /BaseName [code /glyph code /glyph …] wfre`), `wfsf`
/// selects a font under a full matrix (`/Name [a b c d 0 0] wfsf`).
pub(crate) const PS_FONT_PROLOG: &str = "/wfre { /wfpairs exch def findfont dup length dict begin \
{ 1 index /FID ne { def } { pop pop } ifelse } forall \
/Encoding 256 array def 0 1 255 { Encoding exch /.notdef put } for \
0 2 wfpairs length 1 sub { dup wfpairs exch get exch 1 add wfpairs exch get Encoding 3 1 roll put } for \
currentdict end definefont pop } bind def\n\
/wfsf { exch findfont exch makefont setfont } bind def\n";

/// Merge the fonts of several pages: one program per base name with the
/// union of the glyphs every page uses.
pub(crate) fn merge_page_fonts<'a>(
    pages: impl IntoIterator<Item = &'a PsFontResources>,
) -> Vec<PsFontProgram> {
    let mut merged: BTreeMap<String, PsFontProgram> = BTreeMap::new();
    for page in pages {
        for font in &page.fonts {
            match merged.get_mut(&font.base_name) {
                Some(existing) => {
                    for (name, source) in &font.glyphs {
                        existing
                            .glyphs
                            .entry(name.clone())
                            .or_insert_with(|| source.clone());
                    }
                }
                None => {
                    merged.insert(font.base_name.clone(), font.clone());
                }
            }
        }
    }
    merged.into_values().collect()
}

/// Write the DSC font resources for `fonts` (from [`merge_page_fonts`]).
pub(crate) fn write_font_definitions(fonts: &[PsFontProgram], out: &mut String) {
    for font in fonts {
        let _ = writeln!(out, "%%BeginResource: font {}", font.base_name);
        match &font.kind {
            PsFontKind::TrueType(program) => {
                if !write_type42_font(font, program, out) {
                    // The program could not be rebuilt as a Type 42 font:
                    // embed the same glyphs as outline charstrings instead.
                    let units_per_em = ttf_parser::Face::parse(program, 0)
                        .map(|face| f64::from(face.units_per_em()))
                        .unwrap_or(1000.0);
                    let glyphs = font
                        .glyphs
                        .iter()
                        .map(|(name, source)| {
                            let source = match *source {
                                PsGlyphSource::Gid(gid) => {
                                    let (path, advance) =
                                        crate::render::glyph_outline::extract_glyph_path_by_gid(
                                            program, gid,
                                        );
                                    PsGlyphSource::Outline {
                                        path,
                                        advance: advance * units_per_em / 1000.0,
                                    }
                                }
                                ref other => other.clone(),
                            };
                            (name.clone(), source)
                        })
                        .collect();
                    write_outline_type1_font(&font.base_name, units_per_em, &glyphs, out);
                }
            }
            PsFontKind::Outline { units_per_em } => {
                write_outline_type1_font(&font.base_name, *units_per_em, &font.glyphs, out);
            }
        }
        out.push_str("%%EndResource\n");
    }
}

// ── Type 42 ──────────────────────────────────────────────────────────────────

/// Largest `sfnts` string (PostScript strings are limited to 65535 bytes).
const SFNTS_MAX_STRING: usize = 65_000;

fn write_type42_font(font: &PsFontProgram, program: &[u8], out: &mut String) -> bool {
    let mut gids: Vec<(&str, u16)> = Vec::with_capacity(font.glyphs.len());
    for (name, source) in &font.glyphs {
        match source {
            PsGlyphSource::Gid(gid) => gids.push((name.as_str(), *gid)),
            PsGlyphSource::Outline { .. } => return false,
        }
    }
    let mut remapper = subsetter::GlyphRemapper::new();
    for &(_, gid) in &gids {
        remapper.remap(gid);
    }
    let Ok(subset) = subsetter::subset(program, 0, &remapper) else {
        return false;
    };
    let Some(subset) = unhinted_type42_sfnt(&subset) else {
        return false;
    };
    let Ok(face) = ttf_parser::Face::parse(&subset, 0) else {
        return false;
    };
    let Some(chunks) = sfnts_chunks(&subset) else {
        return false;
    };
    let bbox = face.global_bounding_box();

    let _ = writeln!(out, "11 dict begin\n/FontName /{} def", font.base_name);
    out.push_str("/FontType 42 def\n/PaintType 0 def\n/FontMatrix [1 0 0 1 0 0] def\n");
    let _ = writeln!(
        out,
        "/FontBBox [{} {} {} {}] def",
        bbox.x_min, bbox.y_min, bbox.x_max, bbox.y_max
    );
    out.push_str("/Encoding 256 array def 0 1 255 { Encoding exch /.notdef put } for\n");
    let _ = writeln!(out, "/CharStrings {} dict dup begin", gids.len() + 1);
    out.push_str("/.notdef 0 def\n");
    let mut line_len = 0usize;
    for (name, gid) in gids {
        let Some(new_gid) = remapper.get(gid) else {
            return false;
        };
        let entry = format!("/{name} {new_gid} def");
        if line_len + entry.len() > 200 {
            out.push('\n');
            line_len = 0;
        } else if line_len > 0 {
            out.push(' ');
        }
        line_len += entry.len() + 1;
        out.push_str(&entry);
    }
    out.push_str("\nend def\n/sfnts [\n");
    for chunk in chunks {
        // Each string carries one pad byte, which a Type 42 interpreter
        // ignores in an odd-length string (as `pdftops` writes them).
        let mut padded = Vec::with_capacity(chunk.len() + 1);
        padded.extend_from_slice(chunk);
        padded.push(0);
        out.push_str("<~");
        out.push_str(&ascii85_encode(&padded));
        out.push('\n');
    }
    out.push_str("] def\nFontName currentdict end definefont pop\n");
    true
}

/// Tables a Type 42 font does without: naming (glyphs are reached through
/// CharStrings), the hinting programs and device-metric caches.
const TYPE42_DROPPED_TABLES: [[u8; 4]; 8] = [
    *b"name", *b"post", *b"cvt ", *b"fpgm", *b"prep", *b"hdmx", *b"LTSH", *b"VDMX",
];

/// Rebuild a subset TrueType program for Type 42 embedding without hinting:
/// the hinting tables are dropped and every glyph's instructions removed
/// (`glyf`/`loca` rewritten with long offsets), so the PostScript interpreter
/// draws the plain outlines — the same unhinted shapes the Wellfriend
/// rasterizer and the other vector backends draw — and no glyph program can
/// call a function the font no longer defines. Table offsets, alignment and
/// checksums are recomputed. `None` for a malformed font.
fn unhinted_type42_sfnt(font: &[u8]) -> Option<Vec<u8>> {
    let u16_at =
        |data: &[u8], at: usize| Some(u16::from_be_bytes([*data.get(at)?, *data.get(at + 1)?]));
    let u32_at = |data: &[u8], at: usize| {
        Some(u32::from_be_bytes([
            *data.get(at)?,
            *data.get(at + 1)?,
            *data.get(at + 2)?,
            *data.get(at + 3)?,
        ]))
    };
    let num_tables = usize::from(u16_at(font, 4)?);
    let mut tables: Vec<([u8; 4], std::borrow::Cow<'_, [u8]>)> = Vec::with_capacity(num_tables);
    for index in 0..num_tables {
        let record = 12 + index * 16;
        let tag: [u8; 4] = font.get(record..record + 4)?.try_into().ok()?;
        let offset = u32_at(font, record + 8)? as usize;
        let length = u32_at(font, record + 12)? as usize;
        let data = font.get(offset..offset.checked_add(length)?)?;
        if !TYPE42_DROPPED_TABLES.contains(&tag) {
            tables.push((tag, std::borrow::Cow::Borrowed(data)));
        }
    }
    let table = |tables: &[([u8; 4], std::borrow::Cow<'_, [u8]>)], tag: &[u8; 4]| {
        tables.iter().position(|(t, _)| t == tag)
    };
    let (head_index, loca_index, glyf_index, maxp_index) = (
        table(&tables, b"head")?,
        table(&tables, b"loca")?,
        table(&tables, b"glyf")?,
        table(&tables, b"maxp")?,
    );
    let long_loca = u16_at(&tables[head_index].1, 50)? != 0;
    let glyph_count = usize::from(u16_at(&tables[maxp_index].1, 4)?);
    let (loca, glyf) = (&tables[loca_index].1, &tables[glyf_index].1);
    let offset_of = |glyph: usize| -> Option<usize> {
        if long_loca {
            Some(u32_at(loca, glyph * 4)? as usize)
        } else {
            Some(usize::from(u16_at(loca, glyph * 2)?) * 2)
        }
    };
    let mut new_glyf = Vec::with_capacity(glyf.len());
    let mut new_loca = Vec::with_capacity((glyph_count + 1) * 4);
    for glyph in 0..glyph_count {
        new_loca.extend_from_slice(&(new_glyf.len() as u32).to_be_bytes());
        let (start, end) = (offset_of(glyph)?, offset_of(glyph + 1)?);
        if end <= start {
            continue;
        }
        let data = glyf.get(start..end)?;
        let contours = i16::from_be_bytes([*data.first()?, *data.get(1)?]);
        if contours >= 0 {
            // Simple glyph: header, endPtsOfContours, instructionLength,
            // instructions, then flags and coordinates.
            let length_at = 10 + 2 * contours as usize;
            let instructions = usize::from(u16_at(data, length_at)?);
            let rest = length_at + 2 + instructions;
            new_glyf.extend_from_slice(data.get(..length_at)?);
            new_glyf.extend_from_slice(&[0, 0]);
            new_glyf.extend_from_slice(data.get(rest..)?);
        } else {
            // Composite glyph: walk the components, clear
            // WE_HAVE_INSTRUCTIONS and drop the trailing instructions.
            let mut at = 10usize;
            new_glyf.extend_from_slice(data.get(..10)?);
            loop {
                let flags = u16_at(data, at)?;
                let mut size = 4 + if flags & 0x0001 != 0 { 4 } else { 2 };
                size += if flags & 0x0008 != 0 {
                    2
                } else if flags & 0x0040 != 0 {
                    4
                } else if flags & 0x0080 != 0 {
                    8
                } else {
                    0
                };
                let component = data.get(at..at + size)?;
                let flag_at = new_glyf.len();
                new_glyf.extend_from_slice(component);
                let cleared = (flags & !0x0100).to_be_bytes();
                new_glyf[flag_at..flag_at + 2].copy_from_slice(&cleared);
                at += size;
                if flags & 0x0020 == 0 {
                    break;
                }
            }
        }
        if new_glyf.len() % 2 == 1 {
            new_glyf.push(0);
        }
    }
    new_loca.extend_from_slice(&(new_glyf.len() as u32).to_be_bytes());
    let mut head = tables[head_index].1.to_vec();
    head.get_mut(50..52)?.copy_from_slice(&1u16.to_be_bytes());
    tables[head_index].1 = std::borrow::Cow::Owned(head);
    tables[loca_index].1 = std::borrow::Cow::Owned(new_loca);
    tables[glyf_index].1 = std::borrow::Cow::Owned(new_glyf);
    tables.sort_by_key(|(tag, _)| *tag);

    let count = tables.len() as u16;
    let entry_selector = 15 - count.max(1).leading_zeros() as u16;
    let search_range = (1u16 << entry_selector) * 16;
    let mut out = Vec::with_capacity(font.len());
    out.extend_from_slice(font.get(0..4)?);
    for value in [
        count,
        search_range,
        entry_selector,
        count * 16 - search_range,
    ] {
        out.extend_from_slice(&value.to_be_bytes());
    }
    let mut offset = 12 + tables.len() * 16;
    for (tag, data) in &tables {
        let checksum = data
            .chunks(4)
            .map(|chunk| {
                let mut word = [0u8; 4];
                word[..chunk.len()].copy_from_slice(chunk);
                u32::from_be_bytes(word)
            })
            .fold(0u32, u32::wrapping_add);
        out.extend_from_slice(tag);
        out.extend_from_slice(&checksum.to_be_bytes());
        out.extend_from_slice(&(offset as u32).to_be_bytes());
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        offset += data.len().div_ceil(4) * 4;
    }
    for (_, data) in &tables {
        out.extend_from_slice(data);
        out.resize(out.len().div_ceil(4) * 4, 0);
    }
    Some(out)
}

/// Split an sfnt into `sfnts` strings that each end on a table boundary or,
/// inside `glyf`, on an even glyph boundary.
fn sfnts_chunks(font: &[u8]) -> Option<Vec<&[u8]>> {
    let read_u16 = |at: usize| -> Option<u16> {
        Some(u16::from_be_bytes([*font.get(at)?, *font.get(at + 1)?]))
    };
    let read_u32 = |at: usize| -> Option<u32> {
        Some(u32::from_be_bytes([
            *font.get(at)?,
            *font.get(at + 1)?,
            *font.get(at + 2)?,
            *font.get(at + 3)?,
        ]))
    };
    let num_tables = usize::from(read_u16(4)?);
    let mut tables: Vec<([u8; 4], usize, usize)> = Vec::with_capacity(num_tables);
    for index in 0..num_tables {
        let record = 12 + index * 16;
        let tag = font.get(record..record + 4)?;
        let offset = read_u32(record + 8)? as usize;
        let length = read_u32(record + 12)? as usize;
        if offset.checked_add(length)? > font.len() {
            return None;
        }
        tables.push(([tag[0], tag[1], tag[2], tag[3]], offset, length));
    }
    tables.sort_by_key(|&(_, offset, _)| offset);

    // Candidate break offsets: every table start, plus glyph starts in glyf.
    let mut breaks: Vec<usize> = tables.iter().map(|&(_, offset, _)| offset).collect();
    let find = |tag: &[u8; 4]| tables.iter().find(|(t, _, _)| t == tag).copied();
    if let (Some((_, glyf_offset, glyf_len)), Some((_, loca_offset, loca_len)), Some(head)) =
        (find(b"glyf"), find(b"loca"), find(b"head"))
    {
        if glyf_len > SFNTS_MAX_STRING {
            let long = read_u16(head.1 + 50)? != 0;
            let entries = if long { loca_len / 4 } else { loca_len / 2 };
            for entry in 0..entries {
                let glyph_offset = if long {
                    read_u32(loca_offset + entry * 4)? as usize
                } else {
                    usize::from(read_u16(loca_offset + entry * 2)?) * 2
                };
                if glyph_offset < glyf_len && glyph_offset % 2 == 0 {
                    breaks.push(glyf_offset + glyph_offset);
                }
            }
        }
    }
    breaks.push(font.len());
    breaks.sort_unstable();
    breaks.dedup();

    let mut chunks = Vec::new();
    let mut start = 0usize;
    let mut last_break = 0usize;
    for &candidate in &breaks {
        if candidate - start > SFNTS_MAX_STRING && last_break > start {
            chunks.push(&font[start..last_break]);
            start = last_break;
        }
        if candidate - start > SFNTS_MAX_STRING {
            // A single table or glyph larger than a PostScript string.
            return None;
        }
        last_break = candidate;
    }
    if font.len() > start {
        chunks.push(&font[start..]);
    }
    Some(chunks)
}

// ── Type 1 from outlines ─────────────────────────────────────────────────────

/// Write a Type 1 font whose charstrings draw the given outlines. The font
/// dictionary is plain PostScript (no `eexec` section); charstrings are
/// encrypted with the standard charstring key as every Type 1 interpreter
/// expects (`lenIV` 4).
fn write_outline_type1_font(
    base_name: &str,
    units_per_em: f64,
    glyphs: &BTreeMap<String, PsGlyphSource>,
    out: &mut String,
) {
    let units_per_em = if units_per_em.is_finite() && units_per_em > 0.0 {
        units_per_em
    } else {
        1000.0
    };
    // Charstring numbers are integers: keep at least 1000 units to the em.
    let grid = if units_per_em < 1000.0 {
        (1000.0 / units_per_em).ceil()
    } else {
        1.0
    };
    let mut bbox = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    let mut charstrings = Vec::with_capacity(glyphs.len());
    for (name, source) in glyphs {
        let PsGlyphSource::Outline { path, advance } = source else {
            continue;
        };
        if let Some(path) = path {
            for (x, y) in path_points(path) {
                bbox[0] = bbox[0].min(x * grid);
                bbox[1] = bbox[1].min(y * grid);
                bbox[2] = bbox[2].max(x * grid);
                bbox[3] = bbox[3].max(y * grid);
            }
        }
        charstrings.push((
            name.as_str(),
            type1_charstring(path.as_ref(), *advance, grid),
        ));
    }
    if !bbox.iter().all(|v| v.is_finite()) {
        bbox = [0.0; 4];
    }
    let scale = 1.0 / (units_per_em * grid);

    let _ = writeln!(out, "11 dict begin\n/FontName /{base_name} def");
    out.push_str("/FontType 1 def\n/PaintType 0 def\n");
    let _ = writeln!(
        out,
        "/FontMatrix [{} 0 0 {} 0 0] def",
        ps_number(scale),
        ps_number(scale)
    );
    let _ = writeln!(
        out,
        "/FontBBox [{} {} {} {}] def",
        bbox[0].floor(),
        bbox[1].floor(),
        bbox[2].ceil(),
        bbox[3].ceil()
    );
    out.push_str("/Encoding 256 array def 0 1 255 { Encoding exch /.notdef put } for\n");
    out.push_str(
        "/Private 4 dict dup begin /BlueValues [] def /MinFeature {16 16} def /password 5839 def end def\n",
    );
    let _ = writeln!(out, "/CharStrings {} dict dup begin", charstrings.len() + 1);
    let notdef = type1_charstring(None, 0.0, 1.0);
    let _ = writeln!(out, "/.notdef <~{} def", ascii85_encode(&notdef));
    for (name, charstring) in charstrings {
        let _ = writeln!(out, "/{name} <~{} def", ascii85_encode(&charstring));
    }
    out.push_str("end def\nFontName currentdict end definefont pop\n");
}

fn path_points(path: &Path) -> impl Iterator<Item = (f64, f64)> + '_ {
    path.segments.iter().flat_map(|segment| {
        let points: [Option<(f64, f64)>; 3] = match *segment {
            PathSegment::MoveTo(x, y) | PathSegment::LineTo(x, y) => [Some((x, y)), None, None],
            PathSegment::CubicTo {
                cp1x,
                cp1y,
                cp2x,
                cp2y,
                x,
                y,
            } => [Some((cp1x, cp1y)), Some((cp2x, cp2y)), Some((x, y))],
            PathSegment::ClosePath => [None, None, None],
        };
        points.into_iter().flatten()
    })
}

/// Encrypted Type 1 charstring drawing `path` (font units, scaled by
/// `grid` onto the integer charstring grid) with advance `advance`.
fn type1_charstring(path: Option<&Path>, advance: f64, grid: f64) -> Vec<u8> {
    let mut cs = Vec::new();
    type1_number(&mut cs, 0);
    type1_number(&mut cs, round_unit(advance * grid));
    cs.push(13); // hsbw
    if let Some(path) = path {
        // Absolute position on the integer grid; deltas between rounded
        // absolute points never accumulate rounding drift.
        let mut pen = (0i32, 0i32);
        let mut open = false;
        let to_grid = |x: f64, y: f64| (round_unit(x * grid), round_unit(y * grid));
        for segment in &path.segments {
            match *segment {
                PathSegment::MoveTo(x, y) => {
                    if open {
                        cs.push(9); // closepath
                    }
                    let p = to_grid(x, y);
                    let (dx, dy) = (p.0 - pen.0, p.1 - pen.1);
                    if dy == 0 {
                        type1_number(&mut cs, dx);
                        cs.push(22); // hmoveto
                    } else if dx == 0 {
                        type1_number(&mut cs, dy);
                        cs.push(4); // vmoveto
                    } else {
                        type1_number(&mut cs, dx);
                        type1_number(&mut cs, dy);
                        cs.push(21); // rmoveto
                    }
                    pen = p;
                    open = true;
                }
                PathSegment::LineTo(x, y) => {
                    let p = to_grid(x, y);
                    let (dx, dy) = (p.0 - pen.0, p.1 - pen.1);
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    if dy == 0 {
                        type1_number(&mut cs, dx);
                        cs.push(6); // hlineto
                    } else if dx == 0 {
                        type1_number(&mut cs, dy);
                        cs.push(7); // vlineto
                    } else {
                        type1_number(&mut cs, dx);
                        type1_number(&mut cs, dy);
                        cs.push(5); // rlineto
                    }
                    pen = p;
                }
                PathSegment::CubicTo {
                    cp1x,
                    cp1y,
                    cp2x,
                    cp2y,
                    x,
                    y,
                } => {
                    let a = to_grid(cp1x, cp1y);
                    let b = to_grid(cp2x, cp2y);
                    let c = to_grid(x, y);
                    for value in [
                        a.0 - pen.0,
                        a.1 - pen.1,
                        b.0 - a.0,
                        b.1 - a.1,
                        c.0 - b.0,
                        c.1 - b.1,
                    ] {
                        type1_number(&mut cs, value);
                    }
                    cs.push(8); // rrcurveto
                    pen = c;
                }
                PathSegment::ClosePath => {
                    if open {
                        cs.push(9);
                        open = false;
                    }
                }
            }
        }
        if open {
            cs.push(9);
        }
    }
    cs.push(14); // endchar
    type1_encrypt_charstring(&cs)
}

fn round_unit(value: f64) -> i32 {
    if value.is_finite() {
        value.round().clamp(-1.0e9, 1.0e9) as i32
    } else {
        0
    }
}

/// Type 1 charstring number encoding (Adobe Type 1 Font Format, 6.2).
fn type1_number(out: &mut Vec<u8>, value: i32) {
    match value {
        -107..=107 => out.push((value + 139) as u8),
        108..=1131 => {
            let v = value - 108;
            out.push((v / 256 + 247) as u8);
            out.push((v % 256) as u8);
        }
        -1131..=-108 => {
            let v = -value - 108;
            out.push((v / 256 + 251) as u8);
            out.push((v % 256) as u8);
        }
        _ => {
            out.push(255);
            out.extend_from_slice(&value.to_be_bytes());
        }
    }
}

/// Charstring encryption (key 4330) with the default four leading bytes.
fn type1_encrypt_charstring(plain: &[u8]) -> Vec<u8> {
    let mut r: u16 = 4330;
    let mut out = Vec::with_capacity(plain.len() + 4);
    for &byte in [0u8; 4].iter().chain(plain) {
        let cipher = byte ^ (r >> 8) as u8;
        r = (u16::from(cipher).wrapping_add(r))
            .wrapping_mul(52845)
            .wrapping_add(22719);
        out.push(cipher);
    }
    out
}

/// How a PDF font program is embedded, decided once per page and font.
pub(crate) fn classify_font_program(program: &[u8]) -> Option<PsFontKind> {
    if let Ok(face) = ttf_parser::Face::parse(program, 0) {
        let tables = face.tables();
        if tables.glyf.is_some() && tables.hmtx.is_some() {
            return Some(PsFontKind::TrueType(Arc::from(program)));
        }
    }
    let units_per_em = f64::from(crate::render::glyph_outline::get_upem(program)?);
    (units_per_em > 0.0).then_some(PsFontKind::Outline { units_per_em })
}

/// Name identifying an outline glyph inside its font: a hash of the outline,
/// so the same name always denotes the same shape on every page.
pub(crate) fn outline_glyph_ident(path: Option<&Path>, advance: f64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    advance.to_bits().hash(&mut hasher);
    if let Some(path) = path {
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
                } => (2u8, [cp1x, cp1y, cp2x, cp2y, x, y].map(f64::to_bits)).hash(&mut hasher),
                PathSegment::ClosePath => 3u8.hash(&mut hasher),
            }
        }
    }
    // 48 bits keep the names short; collisions inside one font are
    // vanishingly unlikely.
    hasher.finish() & 0xFFFF_FFFF_FFFF
}

/// A page-local 256-code re-encoding of an embedded font.
#[derive(Debug, Clone)]
struct PsFontInstance {
    base_name: String,
    slots: Vec<Option<String>>,
    used: usize,
}

/// The embedded-font state of one page: the fonts and glyphs it uses, and the
/// page-local re-encodings that give every glyph a single-byte code.
#[derive(Debug, Default)]
pub(crate) struct PsPageText {
    pub(crate) resources: PsFontResources,
    instances: Vec<PsFontInstance>,
    codes: std::collections::HashMap<(String, String), (usize, u8)>,
}

/// Order in which free codes are handed out when the preferred one (the
/// glyph's own Latin-1 value) is taken: printable codes first, so strings
/// stay readable.
fn code_search_order() -> impl Iterator<Item = u8> {
    (33u8..=126)
        .chain(161..=255)
        .chain(128..=160)
        .chain(1..=32)
        .chain([127, 0])
}

impl PsPageText {
    /// Register `glyph_name` of font `base_name` (adding it to the font
    /// resources with `source` on first use) and return the instance index
    /// and code that show it.
    pub(crate) fn code_for(
        &mut self,
        base_name: &str,
        kind: &PsFontKind,
        glyph_name: &str,
        preferred: Option<u8>,
        source: impl FnOnce() -> PsGlyphSource,
    ) -> (usize, u8) {
        let key = (base_name.to_string(), glyph_name.to_string());
        if let Some(&found) = self.codes.get(&key) {
            return found;
        }
        self.resources
            .program_mut(base_name, kind.clone())
            .glyphs
            .entry(glyph_name.to_string())
            .or_insert_with(source);

        if let Some(code) = preferred {
            let free = self.instances.iter().position(|instance| {
                instance.base_name == base_name && instance.slots[usize::from(code)].is_none()
            });
            if let Some(index) = free {
                return self.assign(key, index, code);
            }
        }
        let open = self
            .instances
            .iter()
            .position(|instance| instance.base_name == base_name && instance.used < 256);
        let index = match open {
            Some(index) => index,
            None => {
                self.instances.push(PsFontInstance {
                    base_name: base_name.to_string(),
                    slots: vec![None; 256],
                    used: 0,
                });
                self.instances.len() - 1
            }
        };
        let code = preferred
            .filter(|&code| self.instances[index].slots[usize::from(code)].is_none())
            .or_else(|| {
                code_search_order()
                    .find(|&code| self.instances[index].slots[usize::from(code)].is_none())
            })
            .unwrap_or(0);
        self.assign(key, index, code)
    }

    fn assign(&mut self, key: (String, String), index: usize, code: u8) -> (usize, u8) {
        let instance = &mut self.instances[index];
        instance.slots[usize::from(code)] = Some(key.1.clone());
        instance.used += 1;
        self.codes.insert(key, (index, code));
        (index, code)
    }

    /// PostScript name of a page-local font instance.
    pub(crate) fn instance_name(index: usize) -> String {
        format!("WFp{index}")
    }

    /// Page setup creating every re-encoded font instance the page uses.
    pub(crate) fn setup(&self) -> String {
        let mut out = String::new();
        for (index, instance) in self.instances.iter().enumerate() {
            let _ = write!(
                out,
                "/{} /{} [",
                Self::instance_name(index),
                instance.base_name
            );
            let mut line = 0usize;
            for (code, slot) in instance.slots.iter().enumerate() {
                let Some(name) = slot else { continue };
                if line >= 8 {
                    out.push('\n');
                    line = 0;
                }
                let _ = write!(out, "{code} /{name} ");
                line += 1;
            }
            out.push_str("] wfre\n");
        }
        out
    }
}

/// PostScript literal string for single-byte glyph codes: printable ASCII
/// is written as is, `(`, `)` and `\` escaped, everything else in octal.
pub(crate) fn ps_code_string(codes: &[u8]) -> String {
    let mut out = String::with_capacity(codes.len() + 2);
    out.push('(');
    for &code in codes {
        match code {
            b'(' | b')' | b'\\' => {
                out.push('\\');
                out.push(code as char);
            }
            0x20..=0x7E => out.push(code as char),
            _ => {
                let _ = write!(out, "\\{code:03o}");
            }
        }
    }
    out.push(')');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type1_decrypt(cipher: &[u8]) -> Vec<u8> {
        let mut r: u16 = 4330;
        let mut plain = Vec::new();
        for &c in cipher {
            plain.push(c ^ (r >> 8) as u8);
            r = (u16::from(c).wrapping_add(r))
                .wrapping_mul(52845)
                .wrapping_add(22719);
        }
        plain.split_off(4)
    }

    #[test]
    fn glyph_names_follow_the_adobe_glyph_list_convention() {
        assert_eq!(ps_glyph_name('A', 36), "uni0041.36");
        assert_eq!(ps_glyph_name('\u{1F600}', 7), "u1F600.7");
        assert_eq!(ps_glyph_name('\u{FFFD}', 3), "g3");
        assert_eq!(ps_glyph_name('\u{1}', 9), "g9");
    }

    #[test]
    fn type1_numbers_use_the_compact_encodings() {
        let mut out = Vec::new();
        for value in [0, 107, -107, 108, 1131, -108, -1131, 1132, -40000] {
            type1_number(&mut out, value);
        }
        assert_eq!(
            out,
            vec![
                139, 246, 32, 247, 0, 250, 255, 251, 0, 254, 255, 255, 0, 0, 4, 108, 255, 255, 255,
                99, 192
            ]
        );
    }

    #[test]
    fn type1_charstring_draws_a_closed_square_and_round_trips_encryption() {
        let mut path = Path::new();
        path.rect(10.0, 0.0, 100.0, 200.0);
        let cs = type1_decrypt(&type1_charstring(Some(&path), 500.0, 1.0));
        // 0 500 hsbw, 10 hmoveto, 100 hlineto, 200 vlineto, -100 hlineto,
        // closepath (the final segment back to the start), endchar.
        let mut expected = Vec::new();
        type1_number(&mut expected, 0);
        type1_number(&mut expected, 500);
        expected.push(13);
        type1_number(&mut expected, 10);
        expected.push(22);
        type1_number(&mut expected, 100);
        expected.push(6);
        type1_number(&mut expected, 200);
        expected.push(7);
        type1_number(&mut expected, -100);
        expected.push(6);
        expected.push(9);
        expected.push(14);
        assert_eq!(cs, expected);
    }

    #[test]
    fn unhinted_type42_program_keeps_outlines_and_drops_hinting() {
        let program = crate::render::font_rasterizer::get_fallback_font("Helvetica")
            .expect("bundled Helvetica substitute");
        let original = ttf_parser::Face::parse(program, 0).unwrap();
        if original.tables().glyf.is_none() {
            return; // the bundled substitute is not a TrueType program
        }
        let gids: Vec<u16> = ['H', 'e', 'l', 'o', 'W']
            .iter()
            .filter_map(|&ch| original.glyph_index(ch).map(|g| g.0))
            .collect();
        let mut remapper = subsetter::GlyphRemapper::new();
        for &gid in &gids {
            remapper.remap(gid);
        }
        let subset = subsetter::subset(program, 0, &remapper).unwrap();
        let unhinted = unhinted_type42_sfnt(&subset).expect("rebuilt program");
        let face = ttf_parser::Face::parse(&unhinted, 0).unwrap();
        for tag in [b"fpgm", b"prep", b"cvt ", b"name", b"post"] {
            assert!(
                face.raw_face()
                    .table(ttf_parser::Tag::from_bytes(tag))
                    .is_none(),
                "{tag:?} dropped"
            );
        }
        for &gid in &gids {
            let new_gid = ttf_parser::GlyphId(remapper.get(gid).unwrap());
            assert_eq!(
                face.glyph_bounding_box(new_gid),
                original.glyph_bounding_box(ttf_parser::GlyphId(gid))
            );
        }
        assert!(sfnts_chunks(&unhinted).is_some());
    }

    #[test]
    fn code_strings_escape_delimiters_and_binary_codes() {
        assert_eq!(ps_code_string(b"Hi (x)\\"), "(Hi \\(x\\)\\\\)");
        assert_eq!(ps_code_string(&[0, 200]), "(\\000\\310)");
    }

    #[test]
    fn merging_pages_unions_glyphs_per_font() {
        let mut a = PsFontResources::default();
        a.program_mut(
            "WF1",
            PsFontKind::Outline {
                units_per_em: 1000.0,
            },
        )
        .glyphs
        .insert("g1".into(), PsGlyphSource::Gid(1));
        let mut b = PsFontResources::default();
        b.program_mut(
            "WF1",
            PsFontKind::Outline {
                units_per_em: 1000.0,
            },
        )
        .glyphs
        .insert("g2".into(), PsGlyphSource::Gid(2));
        b.program_mut(
            "WF2",
            PsFontKind::Outline {
                units_per_em: 1000.0,
            },
        );
        let merged = merge_page_fonts([&a, &b]);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].glyphs.len(), 2);
    }
}
