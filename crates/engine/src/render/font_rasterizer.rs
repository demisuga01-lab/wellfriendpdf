use crate::content::state::Matrix;
use crate::filters::decode_stream_lossless;
use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;
use crate::render::buffer::{PixelBuffer, PixelColor, BLACK};
use crate::render::line::DashState;
use crate::render::path::{FillRule, GlyphHinting, Path, PathPainter};
use crate::render::transform::{Transform2D, Viewport};
use ttf_parser::OutlineBuilder;

mod fallback_fonts {
    pub static LIBERATION_SANS_REGULAR: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fonts/LiberationSans-Regular.ttf"
    ));
    pub static LIBERATION_SANS_BOLD: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fonts/LiberationSans-Bold.ttf"
    ));
    pub static LIBERATION_SANS_ITALIC: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fonts/LiberationSans-Italic.ttf"
    ));
    pub static LIBERATION_SANS_BOLD_ITALIC: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fonts/LiberationSans-BoldItalic.ttf"
    ));

    pub static LIBERATION_SERIF_REGULAR: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fonts/LiberationSerif-Regular.ttf"
    ));
    pub static LIBERATION_SERIF_BOLD: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fonts/LiberationSerif-Bold.ttf"
    ));
    pub static LIBERATION_SERIF_ITALIC: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fonts/LiberationSerif-Italic.ttf"
    ));
    pub static LIBERATION_SERIF_BOLD_ITALIC: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fonts/LiberationSerif-BoldItalic.ttf"
    ));

    pub static LIBERATION_MONO_REGULAR: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fonts/LiberationMono-Regular.ttf"
    ));
    pub static LIBERATION_MONO_BOLD: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fonts/LiberationMono-Bold.ttf"
    ));
    pub static LIBERATION_MONO_ITALIC: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fonts/LiberationMono-Italic.ttf"
    ));
    pub static LIBERATION_MONO_BOLD_ITALIC: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fonts/LiberationMono-BoldItalic.ttf"
    ));

    /// DejaVu Sans — symbolic-font fallback for Symbol / ZapfDingbats /
    /// Wingdings. Chosen because it has broad Unicode coverage:
    /// the Greek block and Mathematical Operators (for Symbol), and the Dingbats
    /// / Miscellaneous Symbols blocks (for ZapfDingbats and Wingdings, which are
    /// mapped through Unicode). Licence: Bitstream Vera / DejaVu (Bitstream
    /// portions © Bitstream Inc.; DejaVu changes are public domain) — a
    /// permissive free licence, at least as permissive as the OFL used by the
    /// bundled Liberation fonts. Source: http://dejavu.sourceforge.net/.
    pub static DEJAVU_SANS: &[u8] =
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/fonts/DejaVuSans.ttf"));
}

/// Map a PDF font name to a bundled fallback font byte slice.
pub fn get_fallback_font(font_name: &str) -> Option<&'static [u8]> {
    let raw = font_name.trim_start_matches('/');
    let raw = raw.find('+').map_or(raw, |idx| &raw[idx + 1..]);
    let name = raw.to_lowercase();

    let is_bold = name.contains("bold")
        || name.contains("-b")
        || name.ends_with('b')
        || name.contains("heavy")
        || name.contains("black");
    let is_italic = name.contains("italic")
        || name.contains("oblique")
        || name.contains("slant")
        || name.ends_with("-i")
        || name.ends_with("-o");

    // Symbolic fonts (Symbol, ZapfDingbats, Wingdings, Webdings) have glyph sets
    // that the Latin Liberation fonts can't represent. Route them to DejaVu Sans,
    // which covers the Greek/math (Symbol) and Dingbats/Misc-Symbols (ZapfDingbats
    // / Wingdings via Unicode) ranges. The char-code → glyph mapping uses the
    // built-in Symbol/ZapfDingbats encodings (Appendix D) → Unicode → DejaVu cmap.
    if name.contains("symbol")
        || name.contains("dingbat")
        || name.contains("wingding")
        || name.contains("webding")
    {
        return Some(fallback_fonts::DEJAVU_SANS);
    }

    if name.contains("courier")
        || name.contains("mono")
        || name.contains("typewriter")
        || name.contains("consolas")
        || name.contains("inconsolata")
        || name.contains("sourcecodemono")
        || name.contains("lucidaconsole")
    {
        return Some(match (is_bold, is_italic) {
            (true, true) => fallback_fonts::LIBERATION_MONO_BOLD_ITALIC,
            (true, false) => fallback_fonts::LIBERATION_MONO_BOLD,
            (false, true) => fallback_fonts::LIBERATION_MONO_ITALIC,
            (false, false) => fallback_fonts::LIBERATION_MONO_REGULAR,
        });
    }

    if name.contains("times")
        || name.contains("serif")
        || name.contains("georgia")
        || name.contains("palatino")
        || name.contains("bookman")
        || name.contains("garamond")
        || name.contains("cambria")
        || name.contains("constantia")
        || name == "trmn"
    {
        return Some(match (is_bold, is_italic) {
            (true, true) => fallback_fonts::LIBERATION_SERIF_BOLD_ITALIC,
            (true, false) => fallback_fonts::LIBERATION_SERIF_BOLD,
            (false, true) => fallback_fonts::LIBERATION_SERIF_ITALIC,
            (false, false) => fallback_fonts::LIBERATION_SERIF_REGULAR,
        });
    }

    Some(match (is_bold, is_italic) {
        (true, true) => fallback_fonts::LIBERATION_SANS_BOLD_ITALIC,
        (true, false) => fallback_fonts::LIBERATION_SANS_BOLD,
        (false, true) => fallback_fonts::LIBERATION_SANS_ITALIC,
        (false, false) => fallback_fonts::LIBERATION_SANS_REGULAR,
    })
}

pub(crate) struct GlyphToPath {
    path: Path,
    current_x: f32,
    current_y: f32,
}

impl GlyphToPath {
    pub(crate) fn new() -> Self {
        Self {
            path: Path::new(),
            current_x: 0.0,
            current_y: 0.0,
        }
    }

    pub(crate) fn into_path(self) -> Path {
        self.path
    }
}

impl OutlineBuilder for GlyphToPath {
    fn move_to(&mut self, x: f32, y: f32) {
        self.path.move_to(x as f64, y as f64);
        self.current_x = x;
        self.current_y = y;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.path.line_to(x as f64, y as f64);
        self.current_x = x;
        self.current_y = y;
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let p0x = self.current_x as f64;
        let p0y = self.current_y as f64;
        let p1x = x1 as f64;
        let p1y = y1 as f64;
        let p2x = x as f64;
        let p2y = y as f64;

        let cp1x = p0x + 2.0 / 3.0 * (p1x - p0x);
        let cp1y = p0y + 2.0 / 3.0 * (p1y - p0y);
        let cp2x = p2x + 2.0 / 3.0 * (p1x - p2x);
        let cp2y = p2y + 2.0 / 3.0 * (p1y - p2y);

        self.path.curve_to(cp1x, cp1y, cp2x, cp2y, p2x, p2y);
        self.current_x = x;
        self.current_y = y;
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.path.curve_to(
            x1 as f64, y1 as f64, x2 as f64, y2 as f64, x as f64, y as f64,
        );
        self.current_x = x;
        self.current_y = y;
    }

    fn close(&mut self) {
        self.path.close();
    }
}

/// Bare-CFF (Compact Font Format) glyph support.
///
/// PDF embeds CFF/Type2 fonts via `/FontFile3` with `/Subtype /Type1C` (simple
/// fonts) or `/CIDFontType0C` (CID-keyed, used by Type0 composite fonts, common
/// in CJK). These are a *raw* `CFF ` table, NOT wrapped in an sfnt/OpenType
/// container, so [`ttf_parser::Face::parse`] rejects them (it requires `head`,
/// `hhea`, and `maxp` tables and an sfnt magic). `ttf_parser` does, however,
/// expose a standalone [`ttf_parser::cff::Table`] parser for exactly this case.
///
/// These helpers are a *fallback*: callers try `Face::parse` first (handling
/// TrueType and CFF-flavoured OpenType, which are sfnt-wrapped) and only reach
/// for the bare-CFF path when that fails. This keeps the existing, working font
/// paths completely untouched (minimal blast radius).
///
/// CFF charstring coordinates are already in a 1000-unit em by convention (the
/// FontMatrix default is `0.001`), so we report a units-per-em of `1000` and
/// scale advances accordingly, matching the glyf path's `/1000` convention.
pub(crate) mod cff_support {
    use super::{GlyphToPath, Path};
    use ttf_parser::GlyphId;

    /// Parse a bare CFF table, returning `None` if the bytes are not a usable
    /// standalone CFF font (e.g. they are actually an sfnt — handled elsewhere).
    fn parse(font_bytes: &[u8]) -> Option<ttf_parser::cff::Table<'_>> {
        ttf_parser::cff::Table::parse(font_bytes)
    }

    /// True if the bytes parse as a bare CFF table.
    pub(crate) fn is_bare_cff(font_bytes: &[u8]) -> bool {
        parse(font_bytes).is_some()
    }

    /// The effective units-per-em for a bare CFF font. CFF charstrings use the
    /// FontMatrix (default 0.001 → a 1000-unit em); we normalise everything to
    /// 1000 so the renderer's existing `/1000` advance math applies unchanged.
    pub(crate) fn units_per_em() -> f64 {
        1000.0
    }

    /// Extract a glyph outline and advance width (in 1000-unit em) for a glyph
    /// index. Returns `(None, advance)` when the glyph has no outline (e.g.
    /// whitespace) and `None` entirely when the font is not bare CFF.
    pub(crate) fn outline_by_gid(font_bytes: &[u8], gid: u16) -> Option<(Option<Path>, f64)> {
        let table = parse(font_bytes)?;
        let glyph_id = GlyphId(gid);
        let advance = table
            .glyph_width(glyph_id)
            .map(f64::from)
            // CID-keyed CFF returns None for glyph_width; the descendant font's
            // /W array (handled by the caller) supplies the real advance, so a
            // neutral 1000 here is only a fallback.
            .unwrap_or(1000.0);
        Some((outline_table_path(&table, font_bytes, glyph_id, 0), advance))
    }

    /// Extract a glyph outline and advance for an original 8-bit PDF character
    /// code in a *simple* (SID-keyed) CFF font, mapping the code through the
    /// CFF encoding + charset. Returns `None` when the font is not bare CFF.
    pub(crate) fn outline_by_code(font_bytes: &[u8], code: u8) -> Option<(Option<Path>, f64)> {
        let table = parse(font_bytes)?;
        let glyph_id = table.glyph_index(code).unwrap_or(GlyphId(0));
        let advance = table.glyph_width(glyph_id).map(f64::from).unwrap_or(1000.0);
        Some((outline_table_path(&table, font_bytes, glyph_id, 0), advance))
    }

    /// Extract a glyph outline and advance by Adobe glyph name from a
    /// SID-keyed CFF font. PDF `/Encoding /Differences` entries are glyph-name
    /// based and override the CFF program's own 8-bit encoding, so this is the
    /// preferred simple-font lookup whenever the PDF resolved a glyph name.
    pub(crate) fn outline_by_name(
        font_bytes: &[u8],
        glyph_name: &str,
    ) -> Option<(Option<Path>, f64)> {
        let table = parse(font_bytes)?;
        let glyph_id = glyph_index_by_name(&table, font_bytes, glyph_name)?;
        let advance = table.glyph_width(glyph_id).map(f64::from).unwrap_or(1000.0);
        Some((outline_table_path(&table, font_bytes, glyph_id, 0), advance))
    }

    fn outline_table_path(
        table: &ttf_parser::cff::Table<'_>,
        font_bytes: &[u8],
        glyph_id: GlyphId,
        depth: u8,
    ) -> Option<Path> {
        let mut builder = GlyphToPath::new();
        if table.outline(glyph_id, &mut builder).is_ok() {
            return Some(builder.into_path());
        }
        compose_seac_outline(table, font_bytes, glyph_id, depth)
            .or_else(|| outline_simple_type2_for_gid(font_bytes, glyph_id))
    }

    fn glyph_index_by_name(
        table: &ttf_parser::cff::Table<'_>,
        font_bytes: &[u8],
        glyph_name: &str,
    ) -> Option<GlyphId> {
        if glyph_name == ".notdef" {
            return Some(GlyphId(0));
        }
        if let Some(glyph_id) = table.glyph_index_by_name(glyph_name) {
            return Some(glyph_id);
        }
        (0..table.number_of_glyphs())
            .map(GlyphId)
            .find(|glyph_id| table.glyph_name(*glyph_id) == Some(glyph_name))
            .or_else(|| cff_charset_gid_by_name(font_bytes, glyph_name).map(GlyphId))
    }

    fn compose_seac_outline(
        table: &ttf_parser::cff::Table<'_>,
        font_bytes: &[u8],
        glyph_id: GlyphId,
        depth: u8,
    ) -> Option<Path> {
        if depth >= 8 {
            return None;
        }
        let (charset_offset, charstrings_index) = cff_metadata(font_bytes)?;
        let charstring = cff_index_object(font_bytes, &charstrings_index, glyph_id.0)?;
        let (dx, dy, base_code, accent_code) = parse_seac_charstring(charstring)?;
        let base_sid = cff_standard_encoding_sid(base_code)?;
        let accent_sid = cff_standard_encoding_sid(accent_code)?;
        let base_gid = cff_charset_gid_for_sid(
            font_bytes,
            charset_offset,
            charstrings_index.count,
            base_sid,
        )?;
        let accent_gid = cff_charset_gid_for_sid(
            font_bytes,
            charset_offset,
            charstrings_index.count,
            accent_sid,
        )?;
        let mut path = outline_table_path(table, font_bytes, GlyphId(base_gid), depth + 1)?;
        let accent = outline_table_path(table, font_bytes, GlyphId(accent_gid), depth + 1)?;
        append_translated_path(&mut path, &accent, dx, dy);
        Some(path)
    }

    fn outline_simple_type2_for_gid(font_bytes: &[u8], glyph_id: GlyphId) -> Option<Path> {
        let (_, charstrings_index) = cff_metadata(font_bytes)?;
        let charstring = cff_index_object(font_bytes, &charstrings_index, glyph_id.0)?;
        outline_simple_type2_charstring(charstring)
    }

    fn outline_simple_type2_charstring(data: &[u8]) -> Option<Path> {
        let mut stack: Vec<f64> = Vec::with_capacity(32);
        let mut stems = 0usize;
        let mut path = Path::new();
        let mut x = 0.0;
        let mut y = 0.0;
        let mut pos = 0usize;
        while pos < data.len() {
            let byte = data[pos];
            pos += 1;
            match byte {
                1 | 3 | 18 | 23 => {
                    if stack.len() % 2 == 1 {
                        stack.remove(0);
                    }
                    stems = stems.saturating_add(stack.len() / 2);
                    stack.clear();
                }
                4 => {
                    if stack.len() == 2 {
                        stack.remove(0);
                    }
                    if stack.len() != 1 {
                        return None;
                    }
                    y += stack[0];
                    path.move_to(x, y);
                    stack.clear();
                }
                5 => {
                    if stack.len() < 2 || !stack.len().is_multiple_of(2) {
                        return None;
                    }
                    for chunk in stack.chunks_exact(2) {
                        x += chunk[0];
                        y += chunk[1];
                        path.line_to(x, y);
                    }
                    stack.clear();
                }
                6 => {
                    type2_line_to(&mut path, &mut x, &mut y, &stack, true)?;
                    stack.clear();
                }
                7 => {
                    type2_line_to(&mut path, &mut x, &mut y, &stack, false)?;
                    stack.clear();
                }
                8 => {
                    if stack.len() < 6 || !stack.len().is_multiple_of(6) {
                        return None;
                    }
                    for chunk in stack.chunks_exact(6) {
                        type2_rrcurve_to(&mut path, &mut x, &mut y, chunk);
                    }
                    stack.clear();
                }
                12 => {
                    let escaped = *data.get(pos)?;
                    pos += 1;
                    if escaped == 0 {
                        stack.clear();
                    } else {
                        return None;
                    }
                }
                14 => {
                    path.close();
                    return (!path.is_empty()).then_some(path);
                }
                19 | 20 => {
                    if stack.len() % 2 == 1 {
                        stack.remove(0);
                    }
                    stems = stems.saturating_add(stack.len() / 2);
                    stack.clear();
                    let mask_bytes = stems.saturating_add(7) / 8;
                    pos = pos.checked_add(mask_bytes)?;
                    if pos > data.len() {
                        return None;
                    }
                }
                21 => {
                    if stack.len() == 3 {
                        stack.remove(0);
                    }
                    if stack.len() != 2 {
                        return None;
                    }
                    x += stack[0];
                    y += stack[1];
                    path.move_to(x, y);
                    stack.clear();
                }
                22 => {
                    if stack.len() == 2 {
                        stack.remove(0);
                    }
                    if stack.len() != 1 {
                        return None;
                    }
                    x += stack[0];
                    path.move_to(x, y);
                    stack.clear();
                }
                24 => {
                    if stack.len() < 8 || !(stack.len() - 6).is_multiple_of(2) {
                        return None;
                    }
                    let line_len = stack.len() - 6;
                    for chunk in stack[..line_len].chunks_exact(2) {
                        x += chunk[0];
                        y += chunk[1];
                        path.line_to(x, y);
                    }
                    type2_rrcurve_to(&mut path, &mut x, &mut y, &stack[line_len..]);
                    stack.clear();
                }
                25 => {
                    if stack.len() < 8 || !(stack.len() - 6).is_multiple_of(2) {
                        return None;
                    }
                    let curve_len = stack.len() - 2;
                    for chunk in stack[..curve_len].chunks_exact(6) {
                        type2_rrcurve_to(&mut path, &mut x, &mut y, chunk);
                    }
                    x += stack[curve_len];
                    y += stack[curve_len + 1];
                    path.line_to(x, y);
                    stack.clear();
                }
                26 => {
                    type2_vvcurve_to(&mut path, &mut x, &mut y, &stack)?;
                    stack.clear();
                }
                27 => {
                    type2_hhcurve_to(&mut path, &mut x, &mut y, &stack)?;
                    stack.clear();
                }
                28 => {
                    let value = read_i16(data, pos)?;
                    pos = pos.checked_add(2)?;
                    push_type2_operand(&mut stack, f64::from(value));
                }
                30 => {
                    type2_hvcurve_to(&mut path, &mut x, &mut y, &stack, false)?;
                    stack.clear();
                }
                31 => {
                    type2_hvcurve_to(&mut path, &mut x, &mut y, &stack, true)?;
                    stack.clear();
                }
                32..=246 => push_type2_operand(&mut stack, f64::from(i32::from(byte) - 139)),
                247..=250 => {
                    let next = f64::from(*data.get(pos)?);
                    pos += 1;
                    push_type2_operand(
                        &mut stack,
                        f64::from(i32::from(byte) - 247) * 256.0 + next + 108.0,
                    );
                }
                251..=254 => {
                    let next = f64::from(*data.get(pos)?);
                    pos += 1;
                    push_type2_operand(
                        &mut stack,
                        -(f64::from(i32::from(byte) - 251) * 256.0) - next - 108.0,
                    );
                }
                255 => {
                    let value = read_i32(data, pos)?;
                    pos = pos.checked_add(4)?;
                    push_type2_operand(&mut stack, f64::from(value) / 65536.0);
                }
                _ => return None,
            }
        }
        None
    }

    fn push_type2_operand(stack: &mut Vec<f64>, value: f64) {
        if stack.len() < 96 {
            stack.push(value);
        } else {
            stack.clear();
        }
    }

    fn type2_line_to(
        path: &mut Path,
        x: &mut f64,
        y: &mut f64,
        args: &[f64],
        horizontal_first: bool,
    ) -> Option<()> {
        if args.is_empty() {
            return None;
        }
        let mut horizontal = horizontal_first;
        for delta in args {
            if horizontal {
                *x += *delta;
            } else {
                *y += *delta;
            }
            path.line_to(*x, *y);
            horizontal = !horizontal;
        }
        Some(())
    }

    fn type2_rrcurve_to(path: &mut Path, x: &mut f64, y: &mut f64, args: &[f64]) {
        let cp1x = *x + args[0];
        let cp1y = *y + args[1];
        let cp2x = cp1x + args[2];
        let cp2y = cp1y + args[3];
        *x = cp2x + args[4];
        *y = cp2y + args[5];
        path.curve_to(cp1x, cp1y, cp2x, cp2y, *x, *y);
    }

    fn type2_hhcurve_to(path: &mut Path, x: &mut f64, y: &mut f64, args: &[f64]) -> Option<()> {
        let mut idx = 0usize;
        let mut dy1 = 0.0;
        if args.len() % 4 == 1 {
            dy1 = args[0];
            idx = 1;
        }
        while idx + 3 < args.len() {
            let cp1x = *x + args[idx];
            let cp1y = *y + dy1;
            let cp2x = cp1x + args[idx + 1];
            let cp2y = cp1y + args[idx + 2];
            *x = cp2x + args[idx + 3];
            *y = cp2y;
            path.curve_to(cp1x, cp1y, cp2x, cp2y, *x, *y);
            dy1 = 0.0;
            idx += 4;
        }
        (idx == args.len()).then_some(())
    }

    fn type2_vvcurve_to(path: &mut Path, x: &mut f64, y: &mut f64, args: &[f64]) -> Option<()> {
        let mut idx = 0usize;
        let mut dx1 = 0.0;
        if args.len() % 4 == 1 {
            dx1 = args[0];
            idx = 1;
        }
        while idx + 3 < args.len() {
            let cp1x = *x + dx1;
            let cp1y = *y + args[idx];
            let cp2x = cp1x + args[idx + 1];
            let cp2y = cp1y + args[idx + 2];
            *x = cp2x;
            *y = cp2y + args[idx + 3];
            path.curve_to(cp1x, cp1y, cp2x, cp2y, *x, *y);
            dx1 = 0.0;
            idx += 4;
        }
        (idx == args.len()).then_some(())
    }

    fn type2_hvcurve_to(
        path: &mut Path,
        x: &mut f64,
        y: &mut f64,
        args: &[f64],
        horizontal_first: bool,
    ) -> Option<()> {
        let mut idx = 0usize;
        let mut horizontal = horizontal_first;
        while idx + 3 < args.len() {
            let remaining = args.len() - idx;
            let has_extra = remaining == 5;
            if horizontal {
                let cp1x = *x + args[idx];
                let cp1y = *y;
                let cp2x = cp1x + args[idx + 1];
                let cp2y = cp1y + args[idx + 2];
                *x = cp2x + if has_extra { args[idx + 4] } else { 0.0 };
                *y = cp2y + args[idx + 3];
                path.curve_to(cp1x, cp1y, cp2x, cp2y, *x, *y);
            } else {
                let cp1x = *x;
                let cp1y = *y + args[idx];
                let cp2x = cp1x + args[idx + 1];
                let cp2y = cp1y + args[idx + 2];
                *x = cp2x + args[idx + 3];
                *y = cp2y + if has_extra { args[idx + 4] } else { 0.0 };
                path.curve_to(cp1x, cp1y, cp2x, cp2y, *x, *y);
            }
            idx += if has_extra { 5 } else { 4 };
            horizontal = !horizontal;
        }
        (idx == args.len()).then_some(())
    }

    fn append_translated_path(target: &mut Path, source: &Path, dx: f64, dy: f64) {
        for segment in &source.segments {
            match *segment {
                crate::render::path::PathSegment::MoveTo(x, y) => target.move_to(x + dx, y + dy),
                crate::render::path::PathSegment::LineTo(x, y) => target.line_to(x + dx, y + dy),
                crate::render::path::PathSegment::CubicTo {
                    cp1x,
                    cp1y,
                    cp2x,
                    cp2y,
                    x,
                    y,
                } => target.curve_to(cp1x + dx, cp1y + dy, cp2x + dx, cp2y + dy, x + dx, y + dy),
                crate::render::path::PathSegment::ClosePath => target.close(),
            }
        }
    }

    fn parse_seac_charstring(data: &[u8]) -> Option<(f64, f64, u8, u8)> {
        let mut stack: Vec<f64> = Vec::with_capacity(5);
        let mut pos = 0usize;
        while pos < data.len() {
            let byte = data[pos];
            pos += 1;
            match byte {
                14 => {
                    let operands = match stack.len() {
                        4 => &stack[..],
                        5 => &stack[1..],
                        _ => return None,
                    };
                    let base_code = seac_code(operands[2])?;
                    let accent_code = seac_code(operands[3])?;
                    return Some((operands[0], operands[1], base_code, accent_code));
                }
                28 => {
                    let value = read_i16(data, pos)?;
                    pos = pos.checked_add(2)?;
                    push_seac_operand(&mut stack, f64::from(value));
                }
                29 => {
                    let value = read_i32(data, pos)?;
                    pos = pos.checked_add(4)?;
                    push_seac_operand(&mut stack, f64::from(value));
                }
                32..=246 => push_seac_operand(&mut stack, f64::from(i32::from(byte) - 139)),
                247..=250 => {
                    let next = f64::from(*data.get(pos)?);
                    pos += 1;
                    push_seac_operand(
                        &mut stack,
                        f64::from(i32::from(byte) - 247) * 256.0 + next + 108.0,
                    );
                }
                251..=254 => {
                    let next = f64::from(*data.get(pos)?);
                    pos += 1;
                    push_seac_operand(
                        &mut stack,
                        -(f64::from(i32::from(byte) - 251) * 256.0) - next - 108.0,
                    );
                }
                255 => {
                    let value = read_i32(data, pos)?;
                    pos = pos.checked_add(4)?;
                    push_seac_operand(&mut stack, f64::from(value) / 65536.0);
                }
                _ => return None,
            }
        }
        None
    }

    fn push_seac_operand(stack: &mut Vec<f64>, value: f64) {
        if stack.len() < 8 {
            stack.push(value);
        } else {
            stack.clear();
        }
    }

    fn seac_code(value: f64) -> Option<u8> {
        if !value.is_finite() {
            return None;
        }
        let rounded = value.round();
        if (value - rounded).abs() > 0.01 || !(0.0..=255.0).contains(&rounded) {
            return None;
        }
        Some(rounded as u8)
    }

    fn cff_standard_encoding_sid(code: u8) -> Option<u16> {
        let name = crate::fonts::encoding::Encoding::lookup("StandardEncoding", code);
        cff_standard_sid(name)
    }

    #[derive(Debug, Clone, Copy)]
    struct CffIndex {
        count: u16,
        off_size: usize,
        offsets_start: usize,
        data_start: usize,
        end: usize,
    }

    fn cff_charset_gid_by_name(font_bytes: &[u8], glyph_name: &str) -> Option<u16> {
        let (charset_offset, charstrings_index) = cff_metadata(font_bytes)?;
        let strings_index = cff_strings_index(font_bytes)?;
        let target_sid = cff_standard_sid(glyph_name)
            .or_else(|| cff_custom_sid(font_bytes, &strings_index, glyph_name))?;
        cff_charset_gid_for_sid(
            font_bytes,
            charset_offset,
            charstrings_index.count,
            target_sid,
        )
    }

    fn cff_metadata(font_bytes: &[u8]) -> Option<(usize, CffIndex)> {
        let header_size = usize::from(*font_bytes.get(2)?);
        if header_size < 4 || header_size > font_bytes.len() {
            return None;
        }

        let name_index = cff_index_at(font_bytes, header_size)?;
        let top_index = cff_index_at(font_bytes, name_index.end)?;
        let top_dict = cff_index_object(font_bytes, &top_index, 0)?;
        let (charset_offset, charstrings_offset) = cff_top_dict_offsets(top_dict)?;
        let charstrings_index = cff_index_at(font_bytes, charstrings_offset)?;
        Some((charset_offset, charstrings_index))
    }

    fn cff_strings_index(font_bytes: &[u8]) -> Option<CffIndex> {
        let header_size = usize::from(*font_bytes.get(2)?);
        if header_size < 4 || header_size > font_bytes.len() {
            return None;
        }
        let name_index = cff_index_at(font_bytes, header_size)?;
        let top_index = cff_index_at(font_bytes, name_index.end)?;
        cff_index_at(font_bytes, top_index.end)
    }

    fn cff_index_at(data: &[u8], offset: usize) -> Option<CffIndex> {
        let count = read_u16(data, offset)?;
        let mut pos = offset.checked_add(2)?;
        if count == 0 {
            return Some(CffIndex {
                count,
                off_size: 0,
                offsets_start: pos,
                data_start: pos,
                end: pos,
            });
        }
        let off_size = usize::from(*data.get(pos)?);
        if !(1..=4).contains(&off_size) {
            return None;
        }
        pos = pos.checked_add(1)?;
        let offset_count = usize::from(count).checked_add(1)?;
        let offsets_bytes = offset_count.checked_mul(off_size)?;
        let offsets_start = pos;
        let data_start = offsets_start.checked_add(offsets_bytes)?;
        let last = read_cff_offset(
            data,
            offsets_start.checked_add(usize::from(count) * off_size)?,
            off_size,
        )?;
        if last == 0 {
            return None;
        }
        let end = data_start.checked_add(last.checked_sub(1)?)?;
        if end > data.len() {
            return None;
        }
        Some(CffIndex {
            count,
            off_size,
            offsets_start,
            data_start,
            end,
        })
    }

    fn cff_index_object<'a>(data: &'a [u8], index: &CffIndex, n: u16) -> Option<&'a [u8]> {
        if n >= index.count || index.count == 0 {
            return None;
        }
        let n = usize::from(n);
        let start_offset = read_cff_offset(
            data,
            index
                .offsets_start
                .checked_add(n.checked_mul(index.off_size)?)?,
            index.off_size,
        )?;
        let end_offset = read_cff_offset(
            data,
            index
                .offsets_start
                .checked_add((n + 1).checked_mul(index.off_size)?)?,
            index.off_size,
        )?;
        if start_offset == 0 || end_offset < start_offset {
            return None;
        }
        let start = index.data_start.checked_add(start_offset.checked_sub(1)?)?;
        let end = index.data_start.checked_add(end_offset.checked_sub(1)?)?;
        data.get(start..end)
    }

    fn cff_custom_sid(data: &[u8], strings_index: &CffIndex, glyph_name: &str) -> Option<u16> {
        let wanted = glyph_name.as_bytes();
        for idx in 0..strings_index.count {
            if cff_index_object(data, strings_index, idx)? == wanted {
                return 391u16.checked_add(idx);
            }
        }
        None
    }

    fn cff_top_dict_offsets(dict: &[u8]) -> Option<(usize, usize)> {
        let mut charset_offset = Some(0usize);
        let mut charstrings_offset = None;
        let mut stack: Vec<i32> = Vec::with_capacity(16);
        let mut pos = 0usize;
        while pos < dict.len() {
            let byte = dict[pos];
            pos += 1;
            match byte {
                0..=21 if byte != 12 => {
                    let value = stack.last().copied().filter(|v| *v >= 0);
                    match byte {
                        15 => charset_offset = value.and_then(|v| usize::try_from(v).ok()),
                        17 => charstrings_offset = value.and_then(|v| usize::try_from(v).ok()),
                        _ => {}
                    }
                    stack.clear();
                }
                12 => {
                    pos = pos.checked_add(1)?;
                    stack.clear();
                }
                28 => {
                    let value = read_i16(dict, pos)?;
                    pos = pos.checked_add(2)?;
                    push_cff_operand(&mut stack, i32::from(value));
                }
                29 => {
                    let value = read_i32(dict, pos)?;
                    pos = pos.checked_add(4)?;
                    push_cff_operand(&mut stack, value);
                }
                30 => {
                    while pos < dict.len() {
                        let nibbles = dict[pos];
                        pos += 1;
                        if nibbles >> 4 == 0x0F || (nibbles & 0x0F) == 0x0F {
                            break;
                        }
                    }
                    push_cff_operand(&mut stack, 0);
                }
                32..=246 => push_cff_operand(&mut stack, i32::from(byte) - 139),
                247..=250 => {
                    let next = i32::from(*dict.get(pos)?);
                    pos += 1;
                    push_cff_operand(&mut stack, (i32::from(byte) - 247) * 256 + next + 108);
                }
                251..=254 => {
                    let next = i32::from(*dict.get(pos)?);
                    pos += 1;
                    push_cff_operand(&mut stack, -((i32::from(byte) - 251) * 256) - next - 108);
                }
                255 => {
                    let value = read_i32(dict, pos)?;
                    pos = pos.checked_add(4)?;
                    push_cff_operand(&mut stack, value);
                }
                _ => {}
            }
        }
        Some((charset_offset?, charstrings_offset?))
    }

    fn push_cff_operand(stack: &mut Vec<i32>, value: i32) {
        if stack.len() < 64 {
            stack.push(value);
        } else {
            stack.clear();
        }
    }

    fn cff_charset_gid_for_sid(
        data: &[u8],
        charset_offset: usize,
        glyph_count: u16,
        target_sid: u16,
    ) -> Option<u16> {
        if target_sid == 0 {
            return Some(0);
        }
        if glyph_count < 2 {
            return None;
        }
        match charset_offset {
            0 => return (target_sid <= 228).then_some(target_sid),
            1 | 2 => return None,
            _ => {}
        }

        let mut pos = charset_offset;
        let format = *data.get(pos)?;
        pos = pos.checked_add(1)?;
        match format {
            0 => {
                for gid in 1..glyph_count {
                    let sid = read_u16(data, pos)?;
                    pos = pos.checked_add(2)?;
                    if sid == target_sid {
                        return Some(gid);
                    }
                }
            }
            1 => {
                let mut gid = 1u16;
                while gid < glyph_count {
                    let first = read_u16(data, pos)?;
                    let left = u16::from(*data.get(pos.checked_add(2)?)?);
                    pos = pos.checked_add(3)?;
                    let count = left.checked_add(1)?;
                    for delta in 0..count {
                        if first.checked_add(delta)? == target_sid {
                            return gid.checked_add(delta);
                        }
                    }
                    gid = gid.checked_add(count)?;
                }
            }
            2 => {
                let mut gid = 1u16;
                while gid < glyph_count {
                    let first = read_u16(data, pos)?;
                    let left = read_u16(data, pos.checked_add(2)?)?;
                    pos = pos.checked_add(4)?;
                    let count = left.checked_add(1)?;
                    for delta in 0..count {
                        if first.checked_add(delta)? == target_sid {
                            return gid.checked_add(delta);
                        }
                    }
                    gid = gid.checked_add(count)?;
                }
            }
            _ => return None,
        }
        None
    }

    fn read_cff_offset(data: &[u8], offset: usize, size: usize) -> Option<usize> {
        if !(1..=4).contains(&size) {
            return None;
        }
        let mut value = 0usize;
        for idx in 0..size {
            value = value.checked_mul(256)?;
            value = value.checked_add(usize::from(*data.get(offset.checked_add(idx)?)?))?;
        }
        Some(value)
    }

    fn read_u16(data: &[u8], offset: usize) -> Option<u16> {
        let hi = u16::from(*data.get(offset)?);
        let lo = u16::from(*data.get(offset.checked_add(1)?)?);
        Some((hi << 8) | lo)
    }

    fn read_i16(data: &[u8], offset: usize) -> Option<i16> {
        Some(i16::from_be_bytes([
            *data.get(offset)?,
            *data.get(offset.checked_add(1)?)?,
        ]))
    }

    fn read_i32(data: &[u8], offset: usize) -> Option<i32> {
        Some(i32::from_be_bytes([
            *data.get(offset)?,
            *data.get(offset.checked_add(1)?)?,
            *data.get(offset.checked_add(2)?)?,
            *data.get(offset.checked_add(3)?)?,
        ]))
    }

    fn cff_standard_sid(name: &str) -> Option<u16> {
        match name {
            "A" => Some(34),
            "AE" => Some(138),
            "Aacute" => Some(171),
            "Acircumflex" => Some(172),
            "Adieresis" => Some(173),
            "Agrave" => Some(174),
            "Aring" => Some(175),
            "Atilde" => Some(176),
            "B" => Some(35),
            "C" => Some(36),
            "Ccedilla" => Some(177),
            "D" => Some(37),
            "E" => Some(38),
            "Eacute" => Some(178),
            "Ecircumflex" => Some(179),
            "Edieresis" => Some(180),
            "Egrave" => Some(181),
            "Eth" => Some(154),
            "F" => Some(39),
            "G" => Some(40),
            "H" => Some(41),
            "I" => Some(42),
            "Iacute" => Some(182),
            "Icircumflex" => Some(183),
            "Idieresis" => Some(184),
            "Igrave" => Some(185),
            "J" => Some(43),
            "K" => Some(44),
            "L" => Some(45),
            "M" => Some(46),
            "N" => Some(47),
            "Ntilde" => Some(186),
            "O" => Some(48),
            "OE" => Some(142),
            "Oacute" => Some(187),
            "Ocircumflex" => Some(188),
            "Odieresis" => Some(189),
            "Ograve" => Some(190),
            "Oslash" => Some(141),
            "Otilde" => Some(191),
            "P" => Some(49),
            "Q" => Some(50),
            "R" => Some(51),
            "S" => Some(52),
            "Scaron" => Some(192),
            "T" => Some(53),
            "Thorn" => Some(157),
            "U" => Some(54),
            "Uacute" => Some(193),
            "Ucircumflex" => Some(194),
            "Udieresis" => Some(195),
            "Ugrave" => Some(196),
            "V" => Some(55),
            "W" => Some(56),
            "X" => Some(57),
            "Y" => Some(58),
            "Yacute" => Some(197),
            "Ydieresis" => Some(198),
            "Z" => Some(59),
            "Zcaron" => Some(199),
            "a" => Some(66),
            "aacute" => Some(200),
            "acircumflex" => Some(201),
            "acute" => Some(125),
            "adieresis" => Some(202),
            "ae" => Some(144),
            "agrave" => Some(203),
            "aring" => Some(204),
            "atilde" => Some(205),
            "b" => Some(67),
            "bar" => Some(93),
            "bullet" => Some(116),
            "c" => Some(68),
            "ccedilla" => Some(206),
            "cedilla" => Some(133),
            "cent" => Some(97),
            "circumflex" => Some(126),
            "copyright" => Some(170),
            "currency" => Some(103),
            "d" => Some(69),
            "dagger" => Some(112),
            "daggerdbl" => Some(113),
            "degree" => Some(161),
            "dieresis" => Some(131),
            "divide" => Some(159),
            "e" => Some(70),
            "eacute" => Some(207),
            "ecircumflex" => Some(208),
            "edieresis" => Some(209),
            "egrave" => Some(210),
            "ellipsis" => Some(121),
            "emdash" => Some(137),
            "endash" => Some(111),
            "eth" => Some(167),
            "exclamdown" => Some(96),
            "f" => Some(71),
            "fi" => Some(109),
            "florin" => Some(101),
            "germandbls" => Some(149),
            "guillemotleft" => Some(106),
            "guillemotright" => Some(120),
            "guilsinglleft" => Some(107),
            "guilsinglright" => Some(108),
            "i" => Some(74),
            "iacute" => Some(211),
            "icircumflex" => Some(212),
            "idieresis" => Some(213),
            "igrave" => Some(214),
            "l" => Some(77),
            "logicalnot" => Some(151),
            "m" => Some(78),
            "macron" => Some(128),
            "mu" => Some(152),
            "multiply" => Some(168),
            "n" => Some(79),
            "ntilde" => Some(215),
            "o" => Some(80),
            "oacute" => Some(216),
            "ocircumflex" => Some(217),
            "odieresis" => Some(218),
            "oe" => Some(148),
            "ograve" => Some(219),
            "one" => Some(18),
            "onehalf" => Some(155),
            "onequarter" => Some(158),
            "onesuperior" => Some(150),
            "ordfeminine" => Some(139),
            "ordmasculine" => Some(143),
            "oslash" => Some(147),
            "otilde" => Some(220),
            "p" => Some(81),
            "paragraph" => Some(115),
            "periodcentered" => Some(114),
            "perthousand" => Some(122),
            "plusminus" => Some(156),
            "questiondown" => Some(123),
            "quotedblbase" => Some(118),
            "quotedblleft" => Some(105),
            "quotedblright" => Some(119),
            "quoteleft" => Some(65),
            "quoteright" => Some(8),
            "quotesinglbase" => Some(117),
            "r" => Some(83),
            "registered" => Some(165),
            "s" => Some(84),
            "scaron" => Some(221),
            "section" => Some(102),
            "space" => Some(1),
            "sterling" => Some(98),
            "t" => Some(85),
            "thorn" => Some(162),
            "threequarters" => Some(163),
            "threesuperior" => Some(169),
            "tilde" => Some(127),
            "trademark" => Some(153),
            "two" => Some(19),
            "twosuperior" => Some(164),
            "u" => Some(86),
            "uacute" => Some(222),
            "ucircumflex" => Some(223),
            "udieresis" => Some(224),
            "ugrave" => Some(225),
            "v" => Some(87),
            "yacute" => Some(226),
            "ydieresis" => Some(227),
            "yen" => Some(100),
            "zcaron" => Some(228),
            "zero" => Some(17),
            _ => None,
        }
    }

    /// Extract a glyph outline and advance for a Unicode scalar in a *simple*
    /// (SID-keyed) CFF font. This is a fallback for callers that no longer have
    /// the original PDF character code; prefer [`outline_by_code`] for PDF
    /// simple fonts so high-byte WinAnsi punctuation does not collapse to
    /// `.notdef`.
    pub(crate) fn outline_by_char(font_bytes: &[u8], ch: char) -> Option<(Option<Path>, f64)> {
        let code = u32::from(ch);
        if code <= 0xFF {
            outline_by_code(font_bytes, code as u8)
        } else {
            let table = parse(font_bytes)?;
            let glyph_id = GlyphId(0);
            let advance = table.glyph_width(glyph_id).map(f64::from).unwrap_or(1000.0);
            let mut builder = GlyphToPath::new();
            match table.outline(glyph_id, &mut builder) {
                Ok(_) => Some((Some(builder.into_path()), advance)),
                Err(_) => Some((None, advance)),
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::render::path::PathSegment;

        fn op(value: i32) -> u8 {
            assert!((-107..=107).contains(&value));
            (value + 139) as u8
        }

        fn bounds(path: &Path) -> (f64, f64, f64, f64) {
            let mut min_x = f64::INFINITY;
            let mut min_y = f64::INFINITY;
            let mut max_x = f64::NEG_INFINITY;
            let mut max_y = f64::NEG_INFINITY;
            for segment in &path.segments {
                match *segment {
                    PathSegment::MoveTo(x, y) | PathSegment::LineTo(x, y) => {
                        min_x = min_x.min(x);
                        min_y = min_y.min(y);
                        max_x = max_x.max(x);
                        max_y = max_y.max(y);
                    }
                    PathSegment::CubicTo {
                        cp1x,
                        cp1y,
                        cp2x,
                        cp2y,
                        x,
                        y,
                    } => {
                        for (px, py) in [(cp1x, cp1y), (cp2x, cp2y), (x, y)] {
                            min_x = min_x.min(px);
                            min_y = min_y.min(py);
                            max_x = max_x.max(px);
                            max_y = max_y.max(py);
                        }
                    }
                    PathSegment::ClosePath => {}
                }
            }
            (min_x, min_y, max_x, max_y)
        }

        #[test]
        fn type2_width_operand_is_not_outline_geometry() {
            let without_width = [op(20), 22, op(50), op(0), 5, 14];
            let with_width = [28, 0x02, 0x58, op(20), 22, op(50), op(0), 5, 14];

            let plain = outline_simple_type2_charstring(&without_width).expect("plain outline");
            let width_prefixed =
                outline_simple_type2_charstring(&with_width).expect("width-prefixed outline");

            assert_eq!(bounds(&plain), bounds(&width_prefixed));
            assert_eq!(plain.segments.len(), width_prefixed.segments.len());
        }

        #[test]
        fn type2_stem_width_operand_and_hintmask_bytes_are_consumed() {
            let charstring = [
                28,
                0x02,
                0x58, // explicit width 600
                op(10),
                op(20),
                1, // hstem: one stem pair after dropping width
                19,
                0x00, // hintmask for one stem consumes one byte
                op(20),
                22, // hmoveto
                op(50),
                op(0),
                5, // rlineto
                14,
            ];

            let path = outline_simple_type2_charstring(&charstring).expect("hinted outline");
            let (min_x, min_y, max_x, max_y) = bounds(&path);
            assert_eq!((min_x, min_y, max_x, max_y), (20.0, 0.0, 70.0, 0.0));
        }

        #[test]
        fn type2_fallback_rejects_subroutines_instead_of_guessing_bias() {
            let charstring = [op(0), 10, 14]; // callsubr, then endchar
            assert!(
                outline_simple_type2_charstring(&charstring).is_none(),
                "the bounded fallback must not guess subr bias or execute subrs"
            );
        }

        #[test]
        fn seac_parser_accepts_optional_width_but_reports_base_and_accent() {
            let charstring = [
                28,
                0x02,
                0x58,    // optional width
                op(-51), // adx
                247,
                40,     // ady = 148
                op(0),  // bchar
                op(97), // achar
                14,
            ];
            let parsed = parse_seac_charstring(&charstring).expect("seac operands");
            assert_eq!(parsed, (-51.0, 148.0, 0, 97));
        }
    }
}

pub struct FontRasterizer;

impl FontRasterizer {
    /// Rasterize a single glyph onto the pixel buffer.
    #[allow(clippy::too_many_arguments)]
    pub fn rasterize_glyph(
        buf: &mut PixelBuffer,
        char_code: u16,
        font_bytes: &[u8],
        font_size: f64,
        tm: &Matrix,
        ctm: &Transform2D,
        viewport: &Viewport,
        color: PixelColor,
        render_mode: i32,
        stroke_color: PixelColor,
        stroke_width: f64,
    ) -> bool {
        if render_mode == 3 {
            return true;
        }

        let face = match ttf_parser::Face::parse(font_bytes, 0) {
            Ok(face) => face,
            Err(err) => {
                log::warn!("FontRasterizer: failed to parse font: {:?}", err);
                return false;
            }
        };

        let ch = char::from_u32(char_code as u32).unwrap_or('\u{FFFD}');
        let glyph_id = face
            .glyph_index(ch)
            .unwrap_or_else(|| ttf_parser::GlyphId(char_code.saturating_sub(1)));

        let mut builder = GlyphToPath::new();
        if face.outline_glyph(glyph_id, &mut builder).is_none() {
            return true;
        }

        let glyph_path = builder.into_path();
        let upem = face.units_per_em() as f64;
        if upem <= 0.0 || font_size <= 0.0 {
            return true;
        }

        let scale_t = Transform2D::scale(font_size / upem, font_size / upem);
        let tm_t = Transform2D::from(*tm);
        let glyph_ctm = scale_t.concat(&tm_t).concat(ctm);

        // Keep production glyph rendering on the non-distorting coverage path.
        // Light grid-fitting is available to test but is deferred for default
        // rendering until it improves Poppler comparisons instead of regressing.
        let glyph_hinting = GlyphHinting::disabled();

        match render_mode {
            0 | 4 => PathPainter::fill_glyph(
                buf,
                &glyph_path,
                &glyph_ctm,
                viewport,
                color,
                FillRule::NonZero,
                glyph_hinting,
            ),
            1 | 5 => PathPainter::stroke(
                buf,
                &glyph_path,
                &glyph_ctm,
                viewport,
                stroke_color,
                stroke_width,
                &DashState::solid(),
            ),
            2 | 6 => {
                PathPainter::fill_glyph(
                    buf,
                    &glyph_path,
                    &glyph_ctm,
                    viewport,
                    color,
                    FillRule::NonZero,
                    glyph_hinting,
                );
                PathPainter::stroke(
                    buf,
                    &glyph_path,
                    &glyph_ctm,
                    viewport,
                    stroke_color,
                    stroke_width,
                    &DashState::solid(),
                );
            }
            3 | 7 => {}
            _ => log::warn!("FontRasterizer: unknown render_mode {}", render_mode),
        }

        true
    }

    /// Rasterize a run of decoded Unicode text.
    #[allow(clippy::too_many_arguments)]
    pub fn rasterize_text(
        buf: &mut PixelBuffer,
        text: &str,
        font_bytes: &[u8],
        font_size: f64,
        tm: &Matrix,
        ctm: &Transform2D,
        viewport: &Viewport,
        color: PixelColor,
        render_mode: i32,
    ) {
        if ttf_parser::Face::parse(font_bytes, 0).is_err() {
            log::warn!("FontRasterizer::rasterize_text: font parse failed");
            return;
        }

        for c in text.chars() {
            let char_code = if (c as u32) <= u16::MAX as u32 {
                c as u16
            } else {
                0xFFFD
            };
            let _ = Self::rasterize_glyph(
                buf,
                char_code,
                font_bytes,
                font_size,
                tm,
                ctm,
                viewport,
                color,
                render_mode,
                BLACK,
                1.0,
            );
        }
    }

    /// Extract embedded raw font bytes from a PDF font dictionary.
    pub fn extract_font_bytes(font_dict: &PdfDictionary, reader: &PdfReader) -> Option<Vec<u8>> {
        let descriptor = match font_dict.get("FontDescriptor") {
            Some(value) => match reader.resolve(value.clone()).ok()? {
                PdfObject::Dictionary(dict) => dict,
                _ => return None,
            },
            None => return None,
        };

        for key in ["FontFile3", "FontFile2", "FontFile"] {
            if let Some(font_file) = descriptor.get(key) {
                if let PdfObject::Stream { dict, raw } = reader.resolve(font_file.clone()).ok()? {
                    if !raw.is_empty() {
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
                }
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::buffer::{BLACK, WHITE};
    use crate::render::path::PathSegment;

    #[test]
    fn glyph_to_path_converts_line_to_correctly() {
        let mut builder = GlyphToPath::new();
        builder.move_to(0.0, 0.0);
        builder.line_to(100.0, 0.0);
        let path = builder.into_path();
        assert_eq!(path.segments.len(), 2);
        assert!(matches!(path.segments[0], PathSegment::MoveTo(0.0, 0.0)));
        assert!(matches!(path.segments[1], PathSegment::LineTo(100.0, 0.0)));
    }

    #[test]
    fn glyph_to_path_quad_to_produces_cubic() {
        let mut builder = GlyphToPath::new();
        builder.move_to(0.0, 0.0);
        builder.quad_to(50.0, 100.0, 100.0, 0.0);
        let path = builder.into_path();
        assert_eq!(path.segments.len(), 2);
        assert!(matches!(path.segments[1], PathSegment::CubicTo { .. }));
    }

    #[test]
    fn glyph_to_path_quad_to_cubic_endpoint_is_correct() {
        let mut builder = GlyphToPath::new();
        builder.move_to(0.0, 0.0);
        builder.quad_to(50.0, 100.0, 100.0, 0.0);
        let path = builder.into_path();
        match path.segments.get(1) {
            Some(PathSegment::CubicTo { x, y, .. }) => {
                assert!((*x - 100.0).abs() < 0.001);
                assert!((*y - 0.0).abs() < 0.001);
            }
            other => panic!("expected CubicTo, got {other:?}"),
        }
    }

    #[test]
    fn get_fallback_font_returns_some_or_none_for_known_names() {
        assert!(get_fallback_font("Helvetica")
            .map(|bytes| !bytes.is_empty())
            .unwrap_or(true));
        let _ = get_fallback_font("UnknownFont12345");
    }

    #[test]
    fn fallback_font_returns_real_bytes_for_standard_fonts() {
        for name in ["Helvetica", "Times-Roman", "Courier"] {
            let font = get_fallback_font(name).expect("standard font should have fallback");
            assert!(
                font.len() > 10_000,
                "{name} should return a real TTF, got {} bytes",
                font.len()
            );
            assert!(
                font.starts_with(b"\x00\x01\x00\x00")
                    || font.starts_with(b"true")
                    || font.starts_with(b"OTTO"),
                "{name} should start with a valid TTF/OTF header"
            );
        }
    }

    #[test]
    fn fallback_font_selects_weight_and_style_variants() {
        let regular = get_fallback_font("Helvetica").expect("regular fallback");
        let bold = get_fallback_font("Helvetica-Bold").expect("bold fallback");
        let italic = get_fallback_font("Helvetica-Oblique").expect("italic fallback");
        let bold_italic = get_fallback_font("Helvetica-BoldOblique").expect("bold italic fallback");

        assert_ne!(regular.as_ptr(), bold.as_ptr());
        assert_ne!(regular.as_ptr(), italic.as_ptr());
        assert_ne!(regular.as_ptr(), bold_italic.as_ptr());
        assert!(!bold_italic.is_empty());
    }

    #[test]
    fn fallback_font_handles_subset_prefix_and_aliases() {
        assert!(get_fallback_font("ABCDEF+Helvetica").is_some());

        let arial = get_fallback_font("Arial").expect("Arial fallback");
        let helvetica = get_fallback_font("Helvetica").expect("Helvetica fallback");
        assert_eq!(arial.as_ptr(), helvetica.as_ptr());

        let courier_new = get_fallback_font("CourierNew").expect("CourierNew fallback");
        assert!(courier_new.len() > 10_000);
    }

    #[test]
    fn fallback_font_routes_symbolic_fonts_to_dejavu() {
        // Symbolic fonts now get the DejaVu Sans fallback rather
        // than rendering as nothing.
        for name in ["Symbol", "ZapfDingbats", "Wingdings", "ABCDEF+Symbol"] {
            let font = get_fallback_font(name)
                .unwrap_or_else(|| panic!("{name} should get a symbolic fallback"));
            assert!(font.len() > 100_000, "{name} -> DejaVu (large TTF)");
        }
        // Symbolic fallback is a different font than the Latin Liberation Sans.
        let symbol = get_fallback_font("Symbol").unwrap();
        let helv = get_fallback_font("Helvetica").unwrap();
        assert_ne!(symbol.as_ptr(), helv.as_ptr());
    }

    #[test]
    fn symbolic_fallback_font_has_greek_math_and_dingbat_glyphs() {
        let font = get_fallback_font("Symbol").expect("symbol fallback");
        let face = ttf_parser::Face::parse(font, 0).expect("DejaVu should parse");
        // Greek alpha (Symbol), summation/integral (math), check mark + black
        // circle (ZapfDingbats), right arrow (Wingdings-ish).
        for ch in [
            '\u{03B1}', '\u{2211}', '\u{222B}', '\u{2713}', '\u{25CF}', '\u{2192}',
        ] {
            assert!(
                face.glyph_index(ch).is_some(),
                "DejaVu should cover U+{:04X}",
                ch as u32
            );
        }
    }

    #[test]
    fn fallback_font_is_parseable_and_contains_common_glyphs() {
        let font = get_fallback_font("Helvetica").expect("Helvetica fallback");
        let parsed = ttf_parser::Face::parse(font, 0);
        assert!(parsed.is_ok(), "Liberation Sans should parse: {parsed:?}");
        let Ok(face) = parsed else {
            return;
        };
        assert!(face.units_per_em() > 0);
        assert!(face.glyph_index('H').is_some(), "should have glyph for H");
    }

    #[test]
    fn fallback_font_can_extract_glyph_outline() {
        let font = get_fallback_font("Helvetica").expect("Helvetica fallback");
        let parsed = ttf_parser::Face::parse(font, 0);
        assert!(parsed.is_ok(), "Liberation Sans should parse: {parsed:?}");
        let Ok(face) = parsed else {
            return;
        };
        let glyph_id = face.glyph_index('A');
        assert!(
            glyph_id.is_some(),
            "Liberation Sans should have glyph for A"
        );
        let Some(glyph_id) = glyph_id else {
            return;
        };
        let mut builder = GlyphToPath::new();
        assert!(face.outline_glyph(glyph_id, &mut builder).is_some());
        let path = builder.into_path();
        assert!(
            !path.segments.is_empty(),
            "glyph A should have path segments"
        );
    }

    #[test]
    fn rasterize_glyph_with_fallback_font_renders_visible_pixels() {
        let font_bytes = match get_fallback_font("Helvetica") {
            Some(bytes) if !bytes.is_empty() => bytes,
            _ => {
                println!("SKIP: fallback fonts not bundled, skipping glyph render test");
                return;
            }
        };
        let vp = Viewport::new([0.0, 0.0, 200.0, 200.0], 72);
        let ctm = Transform2D::identity();
        let tm: Matrix = [1.0, 0.0, 0.0, 1.0, 50.0, 100.0];
        let mut buf = PixelBuffer::new_filled(200, 200, WHITE);

        let success = FontRasterizer::rasterize_glyph(
            &mut buf, 'A' as u16, font_bytes, 24.0, &tm, &ctm, &vp, BLACK, 0, BLACK, 1.0,
        );
        assert!(success);

        let darkened_count = (0..200i32)
            .flat_map(|y| (0..200i32).map(move |x| (x, y)))
            .filter(|&(x, y)| buf.get_pixel(x, y)[0] < 200)
            .count();
        println!("glyph A darkened pixels: {darkened_count}");
        assert!(darkened_count > 0);
    }

    #[test]
    fn rasterize_glyph_invisible_mode_does_not_paint() {
        let font_bytes = match get_fallback_font("Helvetica") {
            Some(bytes) if !bytes.is_empty() => bytes,
            _ => return,
        };
        let vp = Viewport::new([0.0, 0.0, 200.0, 200.0], 72);
        let ctm = Transform2D::identity();
        let tm: Matrix = [1.0, 0.0, 0.0, 1.0, 50.0, 100.0];
        let mut buf = PixelBuffer::new_filled(200, 200, WHITE);

        FontRasterizer::rasterize_glyph(
            &mut buf, 'A' as u16, font_bytes, 24.0, &tm, &ctm, &vp, BLACK, 3, BLACK, 1.0,
        );

        let changed = (0..200i32)
            .flat_map(|y| (0..200i32).map(move |x| (x, y)))
            .any(|(x, y)| buf.get_pixel(x, y) != WHITE);
        assert!(!changed);
    }

    #[test]
    fn rasterize_glyph_text_clip_mode_does_not_paint() {
        let font_bytes = match get_fallback_font("Helvetica") {
            Some(bytes) if !bytes.is_empty() => bytes,
            _ => return,
        };
        let vp = Viewport::new([0.0, 0.0, 200.0, 200.0], 72);
        let ctm = Transform2D::identity();
        let tm: Matrix = [1.0, 0.0, 0.0, 1.0, 50.0, 100.0];
        let mut buf = PixelBuffer::new_filled(200, 200, WHITE);

        FontRasterizer::rasterize_glyph(
            &mut buf, 'A' as u16, font_bytes, 24.0, &tm, &ctm, &vp, BLACK, 7, BLACK, 1.0,
        );

        let changed = (0..200i32)
            .flat_map(|y| (0..200i32).map(move |x| (x, y)))
            .any(|(x, y)| buf.get_pixel(x, y) != WHITE);
        assert!(!changed);
    }

    #[test]
    fn rasterize_glyph_fill_clip_mode_still_paints_fill() {
        let font_bytes = match get_fallback_font("Helvetica") {
            Some(bytes) if !bytes.is_empty() => bytes,
            _ => return,
        };
        let vp = Viewport::new([0.0, 0.0, 200.0, 200.0], 72);
        let ctm = Transform2D::identity();
        let tm: Matrix = [1.0, 0.0, 0.0, 1.0, 50.0, 100.0];
        let mut buf = PixelBuffer::new_filled(200, 200, WHITE);

        FontRasterizer::rasterize_glyph(
            &mut buf, 'A' as u16, font_bytes, 24.0, &tm, &ctm, &vp, BLACK, 4, BLACK, 1.0,
        );

        let darkened_count = (0..200i32)
            .flat_map(|y| (0..200i32).map(move |x| (x, y)))
            .filter(|&(x, y)| buf.get_pixel(x, y)[0] < 200)
            .count();
        assert!(darkened_count > 0);
    }

    // ── Bare CFF (OpenType-CFF / Type1C) support ────────────────────────────
    //
    // `sample_type1c.cff` is a real `/FontFile3 /Subtype /Type1C` program
    // extracted from the `freeculture.pdf` corpus fixture (a bare CFF table,
    // header 01 00 04 02). It exercises the standalone-CFF fallback that the
    // sfnt-based `Face::parse` cannot handle.
    const SAMPLE_CFF: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/sample_type1c.cff"
    ));

    #[test]
    fn bare_cff_is_not_accepted_by_sfnt_parser() {
        // Confirms the gap this code closes: ttf_parser::Face::parse (sfnt-only)
        // rejects a bare CFF table, so the CFF fallback is genuinely needed.
        assert!(
            ttf_parser::Face::parse(SAMPLE_CFF, 0).is_err(),
            "bare CFF must NOT parse as an sfnt face"
        );
    }

    #[test]
    fn bare_cff_is_detected() {
        assert!(cff_support::is_bare_cff(SAMPLE_CFF));
        // A non-CFF blob is not misdetected.
        assert!(!cff_support::is_bare_cff(b"not a font at all"));
    }

    #[test]
    fn bare_cff_extracts_a_glyph_outline_by_gid() {
        // Glyph 0 is .notdef; scan for the first glyph index that yields a
        // non-empty outline, proving the CFF charstring interpreter runs.
        let mut found_outline = false;
        for gid in 0..64u16 {
            if let Some((Some(path), advance)) = cff_support::outline_by_gid(SAMPLE_CFF, gid) {
                if !path.segments.is_empty() {
                    assert!(advance >= 0.0, "advance should be non-negative");
                    found_outline = true;
                    break;
                }
            }
        }
        assert!(
            found_outline,
            "at least one CFF glyph should produce outline segments"
        );
    }

    #[test]
    fn bare_cff_reports_1000_unit_em() {
        assert_eq!(cff_support::units_per_em(), 1000.0);
    }

    #[test]
    fn bare_cff_helpers_return_none_for_non_cff() {
        assert!(cff_support::outline_by_gid(b"garbage", 1).is_none());
        assert!(cff_support::outline_by_char(b"garbage", 'A').is_none());
    }
}
