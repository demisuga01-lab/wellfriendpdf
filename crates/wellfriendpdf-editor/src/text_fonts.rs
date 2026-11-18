//! Page fonts as the object reader needs them (QF-5211): how a shown string
//! splits into codes, what each code reads as and how wide and tall its
//! glyph is, for simple (Type 1, TrueType), composite (Type 0) and Type 3
//! fonts; and a library font embedded as a composite font for text an
//! object edit sets.
//!
//! Widths are in text space per unit of font size (the `/Widths` value
//! divided by 1000, or a Type 3 width times its font matrix), so a glyph's
//! advance is `width × Tfs`.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use lopdf::{Dictionary, Document, Object, ObjectId, Stream, StringFormat};

use crate::error::EditorError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FontKind {
    Simple,
    Type0,
    Type3,
}

#[derive(Debug, Clone)]
pub(crate) struct FontInfo {
    pub kind: FontKind,
    pub two_byte: bool,
    widths: HashMap<u32, f64>,
    default_width: f64,
    unicode: HashMap<u32, String>,
    /// Codes a Type 3 font has a glyph procedure for.
    procs: Option<HashSet<u32>>,
    pub ascent: f64,
    pub descent: f64,
    pub base_font: String,
    pub subtype: String,
    pub subset: bool,
    pub embedded: bool,
    pub bold: bool,
    pub italic: bool,
    pub flags: i64,
    /// Type 3 glyphs that set their own colour (`d0`): a recolour cannot reach them.
    pub coloured_glyphs: bool,
    /// A Type 3 font without `/Resources` (its glyphs use the page's).
    pub borrows_resources: bool,
}

fn resolve<'a>(document: &'a Document, object: &'a Object) -> Option<&'a Object> {
    match object {
        Object::Reference(id) => document.get_object(*id).ok(),
        other => Some(other),
    }
}

fn dict<'a>(document: &'a Document, object: &'a Object) -> Option<&'a Dictionary> {
    match resolve(document, object)? {
        Object::Dictionary(d) => Some(d),
        Object::Stream(s) => Some(&s.dict),
        _ => None,
    }
}

fn num(object: &Object) -> Option<f64> {
    match object {
        Object::Integer(v) => Some(*v as f64),
        Object::Real(v) => Some(f64::from(*v)),
        _ => None,
    }
}

fn array<'a>(document: &'a Document, object: &'a Object) -> Option<&'a Vec<Object>> {
    resolve(document, object)?.as_array().ok()
}

fn name_of(d: &Dictionary, key: &[u8]) -> Option<String> {
    d.get(key).ok().and_then(|v| v.as_name().ok()).map(|v| String::from_utf8_lossy(v).into_owned())
}

fn stream_bytes(document: &Document, object: &Object) -> Option<Vec<u8>> {
    let stream = resolve(document, object)?.as_stream().ok()?;
    if stream.dict.has(b"Filter") {
        stream.decompressed_content().ok()
    } else {
        Some(stream.content.clone())
    }
}

/// Windows-1252 (`/WinAnsiEncoding`) code to text.
fn win_ansi(code: u32) -> Option<char> {
    const HIGH: [u32; 32] = [
        0x20AC, 0, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0, 0x017D, 0,
        0, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0, 0x017E, 0x0178,
    ];
    match code {
        0x20..=0x7E | 0xA0..=0xFF => char::from_u32(code),
        0x80..=0x9F => char::from_u32(HIGH[(code - 0x80) as usize]).filter(|c| *c != '\0'),
        _ => None,
    }
}

/// `/StandardEncoding` and `/MacRomanEncoding` read as ASCII (their
/// printable ASCII range; the curly quotes of the standard encoding).
fn ascii_base(code: u32, standard: bool) -> Option<char> {
    match code {
        0x27 if standard => Some('\u{2019}'),
        0x60 if standard => Some('\u{2018}'),
        0x20..=0x7E => char::from_u32(code),
        _ => None,
    }
}

/// The Unicode text of a glyph name (the Adobe Glyph List's common names,
/// `uniXXXX`, `uXXXX[XX]`, single letters and digits).
pub(crate) fn glyph_unicode(name: &str) -> Option<String> {
    let base = name.split('.').next().unwrap_or(name);
    if base.len() == 1 && base.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Some(base.to_owned());
    }
    if let Some(hex) = base.strip_prefix("uni") {
        if hex.len() >= 4 && hex.len() % 4 == 0 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            let units: Vec<u16> = hex.as_bytes().chunks(4).filter_map(|c| u16::from_str_radix(std::str::from_utf8(c).ok()?, 16).ok()).collect();
            return String::from_utf16(&units).ok();
        }
    }
    if let Some(hex) = base.strip_prefix('u') {
        if (4..=6).contains(&hex.len()) && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return u32::from_str_radix(hex, 16).ok().and_then(char::from_u32).map(String::from);
        }
    }
    let text = match base {
        "space" | "nbspace" => " ",
        "exclam" => "!",
        "quotedbl" => "\"",
        "numbersign" => "#",
        "dollar" => "$",
        "percent" => "%",
        "ampersand" => "&",
        "quotesingle" => "'",
        "parenleft" => "(",
        "parenright" => ")",
        "asterisk" => "*",
        "plus" => "+",
        "comma" => ",",
        "hyphen" | "minus" => "-",
        "period" => ".",
        "slash" => "/",
        "zero" => "0",
        "one" => "1",
        "two" => "2",
        "three" => "3",
        "four" => "4",
        "five" => "5",
        "six" => "6",
        "seven" => "7",
        "eight" => "8",
        "nine" => "9",
        "colon" => ":",
        "semicolon" => ";",
        "less" => "<",
        "equal" => "=",
        "greater" => ">",
        "question" => "?",
        "at" => "@",
        "bracketleft" => "[",
        "backslash" => "\\",
        "bracketright" => "]",
        "asciicircum" => "^",
        "underscore" => "_",
        "grave" => "`",
        "braceleft" => "{",
        "bar" => "|",
        "braceright" => "}",
        "asciitilde" => "~",
        "quoteleft" => "\u{2018}",
        "quoteright" => "\u{2019}",
        "quotedblleft" => "\u{201C}",
        "quotedblright" => "\u{201D}",
        "endash" => "\u{2013}",
        "emdash" => "\u{2014}",
        "bullet" => "\u{2022}",
        "ellipsis" => "\u{2026}",
        "fi" => "fi",
        "fl" => "fl",
        "ff" => "ff",
        "ffi" => "ffi",
        "ffl" => "ffl",
        "copyright" => "\u{00A9}",
        "registered" => "\u{00AE}",
        "trademark" => "\u{2122}",
        "degree" => "\u{00B0}",
        _ => return None,
    };
    Some(text.to_owned())
}

// ---- ToUnicode CMaps ----

enum Token {
    Hex(Vec<u8>),
    Open,
    Close,
    Word(String),
}

fn tokens(data: &[u8]) -> Vec<Token> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let c = data[i];
        match c {
            b'%' => {
                while i < data.len() && data[i] != b'\n' && data[i] != b'\r' {
                    i += 1;
                }
            }
            b'<' if data.get(i + 1) == Some(&b'<') => {
                out.push(Token::Word("<<".into()));
                i += 2;
                continue;
            }
            b'>' if data.get(i + 1) == Some(&b'>') => {
                out.push(Token::Word(">>".into()));
                i += 2;
                continue;
            }
            b'<' => {
                let mut hex = Vec::new();
                i += 1;
                while i < data.len() && data[i] != b'>' {
                    if data[i].is_ascii_hexdigit() {
                        hex.push(data[i]);
                    }
                    i += 1;
                }
                if hex.len() % 2 == 1 {
                    hex.push(b'0');
                }
                let bytes = hex
                    .chunks(2)
                    .filter_map(|p| u8::from_str_radix(std::str::from_utf8(p).ok()?, 16).ok())
                    .collect();
                out.push(Token::Hex(bytes));
            }
            b'[' => out.push(Token::Open),
            b']' => out.push(Token::Close),
            b'(' => {
                // A literal string (rare in CMaps): skipped with its nesting.
                let mut depth = 1;
                i += 1;
                while i < data.len() && depth > 0 {
                    match data[i] {
                        b'\\' => i += 1,
                        b'(' => depth += 1,
                        b')' => depth -= 1,
                        _ => {}
                    }
                    i += 1;
                }
                continue;
            }
            c if c.is_ascii_whitespace() => {}
            _ => {
                let start = i;
                while i < data.len() && !data[i].is_ascii_whitespace() && !b"<>[]()%/".contains(&data[i]) {
                    i += 1;
                }
                if i == start {
                    i += 1;
                } else {
                    out.push(Token::Word(String::from_utf8_lossy(&data[start..i]).into_owned()));
                }
                continue;
            }
        }
        i += 1;
    }
    out
}

fn code_of(bytes: &[u8]) -> u32 {
    bytes.iter().take(4).fold(0u32, |acc, b| (acc << 8) | u32::from(*b))
}

fn utf16(bytes: &[u8]) -> Option<String> {
    let units: Vec<u16> = bytes.chunks(2).map(|p| if p.len() == 2 { u16::from_be_bytes([p[0], p[1]]) } else { u16::from(p[0]) }).collect();
    String::from_utf16(&units).ok()
}

/// Codes to text from a ToUnicode CMap, and the byte width of its codespace.
fn parse_to_unicode(data: &[u8]) -> (HashMap<u32, String>, Option<usize>) {
    let tokens = tokens(data);
    let mut map = HashMap::new();
    let mut code_bytes = None;
    let mut i = 0;
    let mut budget = 70_000usize;
    while i < tokens.len() {
        match &tokens[i] {
            Token::Word(w) if w == "begincodespacerange" => {
                if let Some(Token::Hex(lo)) = tokens.get(i + 1) {
                    code_bytes.get_or_insert(lo.len());
                }
            }
            Token::Word(w) if w == "beginbfchar" => {
                i += 1;
                while i + 1 < tokens.len() {
                    match (&tokens[i], &tokens[i + 1]) {
                        (Token::Hex(src), Token::Hex(dst)) => {
                            if let Some(text) = utf16(dst) {
                                map.insert(code_of(src), text);
                            }
                            i += 2;
                        }
                        _ => break,
                    }
                }
                continue;
            }
            Token::Word(w) if w == "beginbfrange" => {
                i += 1;
                while i + 2 < tokens.len() {
                    let (Token::Hex(lo), Token::Hex(hi)) = (&tokens[i], &tokens[i + 1]) else { break };
                    let (lo, hi) = (code_of(lo), code_of(hi));
                    match &tokens[i + 2] {
                        Token::Hex(dst) => {
                            for (offset, code) in (lo..=hi.max(lo)).enumerate() {
                                if budget == 0 {
                                    break;
                                }
                                budget -= 1;
                                let mut units = dst.clone();
                                if let Some(last) = units.len().checked_sub(2) {
                                    let value = u16::from_be_bytes([units[last], units[last + 1]]).wrapping_add(offset as u16);
                                    units[last..].copy_from_slice(&value.to_be_bytes());
                                }
                                if let Some(text) = utf16(&units) {
                                    map.insert(code, text);
                                }
                            }
                            i += 3;
                        }
                        Token::Open => {
                            let mut j = i + 3;
                            let mut code = lo;
                            while j < tokens.len() {
                                match &tokens[j] {
                                    Token::Hex(dst) => {
                                        if let Some(text) = utf16(dst) {
                                            map.insert(code, text);
                                        }
                                        code += 1;
                                    }
                                    Token::Close => break,
                                    _ => {}
                                }
                                j += 1;
                            }
                            i = j + 1;
                        }
                        _ => break,
                    }
                }
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    (map, code_bytes)
}

/// What the codes of a simple TrueType font read as, from its embedded
/// program: the code's glyph (through the font's symbol `(3,0)` subtable at
/// `0xF000 + code` or `code`, or its Macintosh `(1,0)` subtable), then that
/// glyph's `post` name or the Unicode the font's own Unicode subtables give
/// it. Subset fonts written without an encoding or a ToUnicode map (common
/// for symbolic-flagged subsets) read back this way.
fn program_unicode(program: &[u8]) -> HashMap<u32, String> {
    let mut out = HashMap::new();
    let Ok(face) = ttf_parser::Face::parse(program, 0) else { return out };
    let Some(cmap) = face.tables().cmap else { return out };
    let mut by_glyph: HashMap<u16, char> = HashMap::new();
    let mut budget = 70_000usize;
    for subtable in cmap.subtables {
        if !subtable.is_unicode() {
            continue;
        }
        subtable.codepoints(|cp| {
            if budget == 0 {
                return;
            }
            budget -= 1;
            if let (Some(gid), Some(c)) = (subtable.glyph_index(cp), char::from_u32(cp)) {
                by_glyph.entry(gid.0).or_insert(c);
            }
        });
    }
    for code in 0u32..=255 {
        let mut gid = None;
        for subtable in cmap.subtables {
            let found = match (subtable.platform_id, subtable.encoding_id) {
                (ttf_parser::PlatformId::Windows, 0) => subtable.glyph_index(0xF000 + code).or_else(|| subtable.glyph_index(code)),
                (ttf_parser::PlatformId::Macintosh, 0) => subtable.glyph_index(code),
                _ => None,
            };
            if let Some(found) = found.filter(|g| g.0 != 0) {
                gid = Some(found);
                break;
            }
        }
        let Some(gid) = gid else { continue };
        let text = face
            .glyph_name(gid)
            .and_then(glyph_unicode)
            .or_else(|| by_glyph.get(&gid.0).map(|c| c.to_string()));
        if let Some(text) = text {
            out.insert(code, text);
        }
    }
    out
}

/// Fonts whose codes are pictures, not letters.
fn picture_font(base_font: &str) -> bool {
    let lower = base_font.to_ascii_lowercase();
    ["symbol", "dingbat", "wingding", "webding"].iter().any(|w| lower.contains(w))
}

// ---- loading ----

/// What `font` (a font dictionary) needs for reading shown strings.
pub(crate) fn load(document: &Document, font: &Dictionary) -> FontInfo {
    let subtype = name_of(font, b"Subtype").unwrap_or_default();
    let base_font = name_of(font, b"BaseFont").unwrap_or_default();
    let kind = match subtype.as_str() {
        "Type0" => FontKind::Type0,
        "Type3" => FontKind::Type3,
        _ => FontKind::Simple,
    };
    let descendant = if kind == FontKind::Type0 {
        font.get(b"DescendantFonts").ok().and_then(|d| array(document, d)).and_then(|a| a.first()).and_then(|d| dict(document, d))
    } else {
        None
    };
    let descriptor = descendant
        .unwrap_or(font)
        .get(b"FontDescriptor")
        .ok()
        .and_then(|d| dict(document, d));
    let flags = descriptor.and_then(|d| d.get(b"Flags").ok()).and_then(num).unwrap_or(0.0) as i64;
    let embedded = descriptor.is_some_and(|d| d.has(b"FontFile") || d.has(b"FontFile2") || d.has(b"FontFile3"));
    let subset = base_font.len() > 7 && base_font.as_bytes()[6] == b'+' && base_font.as_bytes()[..6].iter().all(u8::is_ascii_uppercase);
    let lower = base_font.to_ascii_lowercase();
    let bold = ["bold", "black", "heavy", "semibold", "demi", "extrabold"].iter().any(|w| lower.contains(w))
        || descriptor.and_then(|d| d.get(b"FontWeight").ok()).and_then(num).is_some_and(|w| w >= 600.0)
        || flags & (1 << 18) != 0;
    let italic = lower.contains("italic") || lower.contains("oblique") || flags & 64 != 0;

    let font_matrix = if kind == FontKind::Type3 {
        font.get(b"FontMatrix")
            .ok()
            .and_then(|m| array(document, m))
            .map(|a| a.iter().filter_map(num).collect::<Vec<_>>())
            .filter(|v| v.len() == 6)
            .unwrap_or_else(|| vec![0.001, 0.0, 0.0, 0.001, 0.0, 0.0])
    } else {
        vec![0.001, 0.0, 0.0, 0.001, 0.0, 0.0]
    };
    let (sx, sy) = (font_matrix[0].abs().max(1e-9), font_matrix[3].abs().max(1e-9));

    // Byte width of codes and the ToUnicode map.
    let mut two_byte = kind == FontKind::Type0;
    let mut unicode = HashMap::new();
    if let Some(data) = font.get(b"ToUnicode").ok().and_then(|t| stream_bytes(document, t)) {
        let (map, _) = parse_to_unicode(&data);
        unicode = map;
    }
    if kind == FontKind::Type0 {
        if let Some(Ok(stream)) = font.get(b"Encoding").ok().and_then(|e| resolve(document, e)).map(Object::as_stream) {
            let data = if stream.dict.has(b"Filter") { stream.decompressed_content().unwrap_or_default() } else { stream.content.clone() };
            if let (_, Some(bytes)) = parse_to_unicode(&data) {
                two_byte = bytes >= 2;
            }
        }
    }

    // Simple-font encodings (when the ToUnicode map does not name a code).
    let mut procs = None;
    if kind != FontKind::Type0 {
        let encoding = font.get(b"Encoding").ok().and_then(|e| resolve(document, e));
        let (base, differences) = match encoding {
            Some(Object::Name(n)) => (Some(String::from_utf8_lossy(n).into_owned()), None),
            Some(Object::Dictionary(d)) => (name_of(d, b"BaseEncoding"), d.get(b"Differences").ok().and_then(|a| array(document, a))),
            _ => (None, None),
        };
        let symbolic = flags & 4 != 0 && flags & 32 == 0;
        // A symbolic or unencoded TrueType font reads through its own program.
        let program = if kind == FontKind::Simple && (symbolic || base.is_none()) && differences.is_none() {
            descriptor
                .and_then(|d| d.get(b"FontFile2").ok())
                .and_then(|f| stream_bytes(document, f))
                .map(|p| program_unicode(&p))
                .unwrap_or_default()
        } else {
            HashMap::new()
        };
        let mut names: BTreeMap<u32, String> = BTreeMap::new();
        if let Some(items) = differences {
            let mut code = 0u32;
            for item in items {
                match item {
                    Object::Integer(v) => code = (*v).clamp(0, 255) as u32,
                    Object::Name(n) => {
                        names.insert(code, String::from_utf8_lossy(n).into_owned());
                        code += 1;
                    }
                    _ => {}
                }
            }
        }
        for code in 0u32..=255 {
            if unicode.contains_key(&code) {
                continue;
            }
            let from_name = names.get(&code).and_then(|n| glyph_unicode(n));
            let from_base = if names.contains_key(&code) {
                None
            } else {
                match base.as_deref() {
                    Some("WinAnsiEncoding") => win_ansi(code).map(String::from),
                    Some("MacRomanEncoding") => ascii_base(code, false).map(String::from),
                    Some("StandardEncoding") => ascii_base(code, true).map(String::from),
                    _ if kind == FontKind::Type3 => None,
                    _ if program.contains_key(&code) => program.get(&code).cloned(),
                    // A symbolic text font without any map: its codes are
                    // read as the standard encoding, as text extractors do.
                    _ if symbolic && picture_font(&base_font) => None,
                    _ => ascii_base(code, !symbolic).map(String::from),
                }
            };
            if let Some(text) = from_name.or(from_base) {
                unicode.insert(code, text);
            }
        }
        if kind == FontKind::Type3 {
            let char_procs = font.get(b"CharProcs").ok().and_then(|c| dict(document, c));
            let mut codes = HashSet::new();
            let mut coloured = false;
            if let Some(char_procs) = char_procs {
                for (code, name) in &names {
                    if let Ok(proc_object) = char_procs.get(name.as_bytes()) {
                        codes.insert(*code);
                        if !coloured {
                            if let Some(data) = stream_bytes(document, proc_object) {
                                let head = String::from_utf8_lossy(&data[..data.len().min(200)]).into_owned();
                                coloured = head.split_whitespace().take(8).any(|t| t == "d0");
                            }
                        }
                    }
                }
            }
            procs = Some(codes);
            let mut info = base_info(kind, two_byte, unicode, procs, base_font, subtype, subset, embedded, bold, italic, flags);
            info.coloured_glyphs = coloured;
            info.borrows_resources = !font.has(b"Resources");
            let first = font.get(b"FirstChar").ok().and_then(num).unwrap_or(0.0) as u32;
            if let Some(widths) = font.get(b"Widths").ok().and_then(|w| array(document, w)) {
                for (i, w) in widths.iter().enumerate() {
                    if let Some(w) = resolve(document, w).and_then(num) {
                        info.widths.insert(first + i as u32, w * sx);
                    }
                }
            }
            let bbox: Vec<f64> = font.get(b"FontBBox").ok().and_then(|b| array(document, b)).map(|a| a.iter().filter_map(num).collect()).unwrap_or_default();
            if bbox.len() == 4 && (bbox[3] - bbox[1]).abs() > 0.0 {
                info.ascent = bbox[1].max(bbox[3]) * sy;
                info.descent = bbox[1].min(bbox[3]) * sy;
            }
            info.default_width = 0.0;
            return info;
        }
    }

    let mut info = base_info(kind, two_byte, unicode, procs, base_font.clone(), subtype, subset, embedded, bold, italic, flags);
    // Widths.
    if kind == FontKind::Type0 {
        if let Some(d) = descendant {
            info.default_width = d.get(b"DW").ok().and_then(num).unwrap_or(1000.0) / 1000.0;
            if let Some(w) = d.get(b"W").ok().and_then(|w| array(document, w)) {
                let mut i = 0;
                let mut budget = 70_000usize;
                while i < w.len() {
                    let Some(first) = resolve(document, &w[i]).and_then(num) else { break };
                    match w.get(i + 1).and_then(|o| resolve(document, o)) {
                        Some(Object::Array(list)) => {
                            for (k, v) in list.iter().enumerate() {
                                if let Some(v) = resolve(document, v).and_then(num) {
                                    info.widths.insert(first as u32 + k as u32, v / 1000.0);
                                }
                            }
                            i += 2;
                        }
                        Some(last) => {
                            let (Some(last), Some(width)) = (num(last), w.get(i + 2).and_then(|o| resolve(document, o)).and_then(num)) else { break };
                            for code in first as u32..=last.max(first) as u32 {
                                if budget == 0 {
                                    break;
                                }
                                budget -= 1;
                                info.widths.insert(code, width / 1000.0);
                            }
                            i += 3;
                        }
                        None => break,
                    }
                }
            }
        }
    } else {
        let first = font.get(b"FirstChar").ok().and_then(num).unwrap_or(0.0) as u32;
        let missing = descriptor.and_then(|d| d.get(b"MissingWidth").ok()).and_then(num).unwrap_or(0.0);
        match font.get(b"Widths").ok().and_then(|w| array(document, w)) {
            Some(widths) => {
                for (i, w) in widths.iter().enumerate() {
                    if let Some(w) = resolve(document, w).and_then(num) {
                        info.widths.insert(first + i as u32, w / 1000.0);
                    }
                }
                info.default_width = missing / 1000.0;
            }
            None => {
                // A standard font without widths: an average advance.
                info.default_width = if lower.contains("courier") { 0.6 } else { 0.52 };
            }
        }
    }
    // Vertical extent.
    let ascent = descriptor.and_then(|d| d.get(b"Ascent").ok()).and_then(num);
    let descent = descriptor.and_then(|d| d.get(b"Descent").ok()).and_then(num);
    let bbox: Vec<f64> = descriptor
        .and_then(|d| d.get(b"FontBBox").ok())
        .and_then(|b| array(document, b))
        .map(|a| a.iter().filter_map(num).collect())
        .unwrap_or_default();
    info.ascent = ascent.filter(|a| *a > 0.0).or_else(|| (bbox.len() == 4).then(|| bbox[3]).filter(|a| *a > 0.0)).unwrap_or(800.0) / 1000.0;
    info.descent = descent.filter(|d| *d < 0.0).or_else(|| (bbox.len() == 4).then(|| bbox[1]).filter(|d| *d < 0.0)).unwrap_or(-200.0) / 1000.0;
    info.ascent = info.ascent.clamp(0.3, 1.6);
    info.descent = info.descent.clamp(-1.0, 0.0);
    info
}

#[allow(clippy::too_many_arguments)]
fn base_info(
    kind: FontKind,
    two_byte: bool,
    unicode: HashMap<u32, String>,
    procs: Option<HashSet<u32>>,
    base_font: String,
    subtype: String,
    subset: bool,
    embedded: bool,
    bold: bool,
    italic: bool,
    flags: i64,
) -> FontInfo {
    FontInfo {
        kind,
        two_byte,
        widths: HashMap::new(),
        default_width: 0.5,
        unicode,
        procs,
        ascent: 0.8,
        descent: -0.2,
        base_font,
        subtype,
        subset,
        embedded,
        bold,
        italic,
        flags,
        coloured_glyphs: false,
        borrows_resources: false,
    }
}

impl FontInfo {
    /// The codes of a shown string.
    pub fn codes(&self, bytes: &[u8]) -> Vec<u32> {
        if self.two_byte {
            bytes.chunks(2).map(code_of).collect()
        } else {
            bytes.iter().map(|b| u32::from(*b)).collect()
        }
    }

    /// Advance of `code` per unit of font size.
    pub fn width(&self, code: u32) -> f64 {
        self.widths.get(&code).copied().unwrap_or(self.default_width)
    }

    pub fn unicode(&self, code: u32) -> Option<&str> {
        self.unicode.get(&code).map(String::as_str)
    }

    /// Whether the code is a word space (`Tw` applies to the one-byte code 32).
    pub fn word_space(&self, code: u32) -> bool {
        !self.two_byte && code == 32
    }

    /// `text` as a string of this font's codes, using only codes it surely
    /// has glyphs for: the codes `known` (already shown on the page) for a
    /// subset or a Type 3 font, any mapped code otherwise. `None` when a
    /// character has no such code.
    pub fn encode(&self, text: &str, known: &BTreeSet<u32>) -> Option<Vec<u8>> {
        let restricted = self.subset || self.kind == FontKind::Type3 || (self.kind == FontKind::Type0 && !self.embedded);
        let mut reverse: HashMap<&str, u32> = HashMap::new();
        let mut candidates: Vec<(&u32, &String)> = self.unicode.iter().collect();
        candidates.sort_by_key(|(code, _)| **code);
        for (code, text) in candidates {
            if restricted && !known.contains(code) {
                continue;
            }
            if let Some(procs) = &self.procs {
                if !procs.contains(code) {
                    continue;
                }
            }
            if !self.two_byte && *code > 255 {
                continue;
            }
            reverse.entry(text.as_str()).or_insert(*code);
        }
        let mut out = Vec::new();
        let chars: Vec<char> = text.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            // Ligature glyphs read as several characters: the longest match first.
            let mut matched = false;
            for len in (1..=3.min(chars.len() - i)).rev() {
                let piece: String = chars[i..i + len].iter().collect();
                if let Some(code) = reverse.get(piece.as_str()) {
                    if self.two_byte {
                        out.extend_from_slice(&(*code as u16).to_be_bytes());
                    } else {
                        out.push(*code as u8);
                    }
                    i += len;
                    matched = true;
                    break;
                }
            }
            if !matched {
                return None;
            }
        }
        Some(out)
    }

    /// The string object for encoded codes.
    pub fn string(&self, bytes: Vec<u8>) -> Object {
        Object::String(bytes, if self.two_byte { StringFormat::Hexadecimal } else { StringFormat::Literal })
    }
}

// ---- library fonts embedded for edited text ----

/// A library face embedded as a composite font (Identity-H, codes are glyph
/// ids), with widths and a ToUnicode map for the glyphs used. A TrueType
/// face is embedded as a subset of those glyphs (renumbered); other faces
/// in full.
pub(crate) struct EmbeddedFont {
    pub font: ObjectId,
    /// Cap height per unit of size.
    pub cap_height: f64,
    units_per_em: f64,
    face_bytes: Vec<u8>,
    /// Source glyph id to the subset's glyph id (`None`: embedded in full).
    remap: Option<HashMap<u16, u16>>,
}

/// The glyphs `used` of a TrueType face as a subset program, and each source
/// glyph's new id; `None` when the face is not TrueType or cannot be subset.
fn subset_program(bytes: &[u8], used: &BTreeMap<u16, String>) -> Option<(Vec<u8>, HashMap<u16, u16>)> {
    let mut remapper = subsetter::GlyphRemapper::new();
    remapper.remap(0);
    for gid in used.keys() {
        remapper.remap(*gid);
    }
    let program = subsetter::subset(bytes, 0, &remapper).ok()?;
    ttf_parser::Face::parse(&program, 0).ok()?;
    let remap = used.keys().filter_map(|gid| remapper.get(*gid).map(|new| (*gid, new))).collect::<HashMap<_, _>>();
    (remap.len() == used.len()).then_some((program, remap))
}

/// The six-letter subset tag of a subset font (from the glyphs it holds).
fn subset_tag(id: &str, used: &BTreeMap<u16, String>) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(id.as_bytes());
    for gid in used.keys() {
        hasher.update(gid.to_be_bytes());
    }
    hasher.finalize().iter().take(6).map(|b| char::from(b'A' + b % 26)).collect()
}

fn sanitize_name(name: &str) -> String {
    let cleaned: String = name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    if cleaned.is_empty() { "WellfriendFont".into() } else { cleaned }
}

impl EmbeddedFont {
    /// Glyph ids of `text` (big-endian pairs) and its advance per unit size;
    /// the characters the face has no glyph for, when there are any.
    pub fn encode(&self, text: &str) -> Result<(Vec<u8>, f64), Vec<char>> {
        let face = ttf_parser::Face::parse(&self.face_bytes, 0).map_err(|_| text.chars().collect::<Vec<_>>())?;
        let mut out = Vec::new();
        let mut advance = 0.0;
        let mut missing = Vec::new();
        for c in text.chars() {
            let gid = face.glyph_index(c).filter(|g| g.0 != 0).or_else(|| c.is_whitespace().then(|| face.glyph_index(' ')).flatten());
            let Some(gid) = gid else {
                missing.push(c);
                continue;
            };
            let code = match &self.remap {
                Some(remap) => match remap.get(&gid.0) {
                    Some(new) => *new,
                    None => {
                        missing.push(c);
                        continue;
                    }
                },
                None => gid.0,
            };
            out.extend_from_slice(&code.to_be_bytes());
            advance += f64::from(face.glyph_hor_advance(gid).unwrap_or(0)) / self.units_per_em;
        }
        if missing.is_empty() { Ok((out, advance)) } else { Err(missing) }
    }
}

/// Embed `bytes` (a TrueType or OpenType face) for `text` and return the font.
pub(crate) fn embed(document: &mut Document, bytes: &[u8], id: &str, text: &str) -> Result<EmbeddedFont, EditorError> {
    let face = ttf_parser::Face::parse(bytes, 0)
        .map_err(|_| EditorError::new("EDITOR_FONT_UNAVAILABLE", format!("font {id} could not be read")))?;
    let upem = f64::from(face.units_per_em().max(1));
    let scale = |v: f64| (v * 1000.0 / upem).round();
    let missing: Vec<char> = text.chars().filter(|c| !c.is_whitespace() && face.glyph_index(*c).is_none_or(|g| g.0 == 0)).collect();
    if !missing.is_empty() {
        let list: String = missing.iter().take(12).collect();
        return Err(EditorError::new("EDITOR_FONT_GLYPHS_MISSING", format!("font {id} has no glyph for: {list}")));
    }
    let postscript = face
        .names()
        .into_iter()
        .filter(|n| n.name_id == ttf_parser::name_id::POST_SCRIPT_NAME)
        .find_map(|n| n.to_string())
        .filter(|n| !n.is_empty() && n.is_ascii())
        .unwrap_or_else(|| id.to_owned());
    let mut used: BTreeMap<u16, String> = BTreeMap::new();
    for c in text.chars().chain(std::iter::once(' ')) {
        if let Some(gid) = face.glyph_index(c).filter(|g| g.0 != 0) {
            used.entry(gid.0).or_insert_with(|| c.to_string());
        }
    }
    let truetype = face.raw_face().table(ttf_parser::Tag::from_bytes(b"glyf")).is_some();
    // A TrueType face is embedded as a subset of the glyphs the text uses.
    let subset = if truetype { subset_program(bytes, &used) } else { None };
    let base_font = match &subset {
        Some(_) => format!("{}+{}", subset_tag(id, &used), sanitize_name(&postscript)),
        None => sanitize_name(&postscript),
    };
    let cid = |gid: u16| subset.as_ref().map_or(gid, |(_, remap)| remap[&gid]);
    let widths: Vec<Object> = used
        .keys()
        .flat_map(|gid| {
            let w = scale(f64::from(face.glyph_hor_advance(ttf_parser::GlyphId(*gid)).unwrap_or(0)));
            [Object::Integer(i64::from(cid(*gid))), Object::Array(vec![Object::Integer(w as i64)])]
        })
        .collect();
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /Adobe-Identity-UCS def /CMapType 2 def\n1 begincodespacerange <0000> <FFFF> endcodespacerange\n",
    );
    let entries: Vec<_> = used.iter().collect();
    for chunk in entries.chunks(100) {
        cmap.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for (gid, text) in chunk {
            let hex: String = text.encode_utf16().map(|u| format!("{u:04X}")).collect();
            cmap.push_str(&format!("<{:04X}> <{hex}>\n", cid(**gid)));
        }
        cmap.push_str("endbfchar\n");
    }
    cmap.push_str("endcmap CMapName currentdict /CMap defineresource pop end end\n");
    let mut to_unicode = Stream::new(Dictionary::new(), cmap.into_bytes());
    let _ = to_unicode.compress();
    let to_unicode = document.add_object(to_unicode);

    let program_bytes = subset.as_ref().map_or_else(|| bytes.to_vec(), |(program, _)| program.clone());
    let mut program_dict = Dictionary::new();
    if truetype {
        program_dict.set("Length1", Object::Integer(program_bytes.len() as i64));
    } else {
        program_dict.set("Subtype", Object::Name(b"OpenType".to_vec()));
    }
    let mut program = Stream::new(program_dict, program_bytes);
    let _ = program.compress();
    let program = document.add_object(program);

    let bbox = face.global_bounding_box();
    let ascent = f64::from(face.ascender()) / upem;
    let descent = f64::from(face.descender()) / upem;
    let cap = face.capital_height().map(|c| f64::from(c) / upem).filter(|c| *c > 0.1).unwrap_or(ascent * 0.72);
    let serif = id.contains("serif") && !id.contains("sans");
    let italic = face.is_italic();
    let flags = 32 | if serif { 2 } else { 0 } | if italic { 64 } else { 0 };
    let mut descriptor = Dictionary::new();
    descriptor.set("Type", Object::Name(b"FontDescriptor".to_vec()));
    descriptor.set("FontName", Object::Name(base_font.clone().into_bytes()));
    descriptor.set("Flags", Object::Integer(flags));
    descriptor.set(
        "FontBBox",
        Object::Array(
            [bbox.x_min, bbox.y_min, bbox.x_max, bbox.y_max]
                .iter()
                .map(|v| Object::Integer(scale(f64::from(*v)) as i64))
                .collect(),
        ),
    );
    descriptor.set("ItalicAngle", Object::Integer(if italic { -12 } else { 0 }));
    descriptor.set("Ascent", Object::Integer((ascent * 1000.0).round() as i64));
    descriptor.set("Descent", Object::Integer((descent * 1000.0).round() as i64));
    descriptor.set("CapHeight", Object::Integer((cap * 1000.0).round() as i64));
    descriptor.set("StemV", Object::Integer(if face.is_bold() { 140 } else { 80 }));
    descriptor.set(if truetype { "FontFile2" } else { "FontFile3" }, Object::Reference(program));
    let descriptor = document.add_object(descriptor);

    let mut cid = Dictionary::new();
    cid.set("Type", Object::Name(b"Font".to_vec()));
    cid.set("Subtype", Object::Name(if truetype { b"CIDFontType2".to_vec() } else { b"CIDFontType0".to_vec() }));
    cid.set("BaseFont", Object::Name(base_font.clone().into_bytes()));
    let mut info = Dictionary::new();
    info.set("Registry", Object::String(b"Adobe".to_vec(), StringFormat::Literal));
    info.set("Ordering", Object::String(b"Identity".to_vec(), StringFormat::Literal));
    info.set("Supplement", Object::Integer(0));
    cid.set("CIDSystemInfo", Object::Dictionary(info));
    cid.set("FontDescriptor", Object::Reference(descriptor));
    cid.set("DW", Object::Integer(1000));
    cid.set("W", Object::Array(widths));
    if truetype {
        cid.set("CIDToGIDMap", Object::Name(b"Identity".to_vec()));
    }
    let cid = document.add_object(cid);

    let mut font = Dictionary::new();
    font.set("Type", Object::Name(b"Font".to_vec()));
    font.set("Subtype", Object::Name(b"Type0".to_vec()));
    font.set("BaseFont", Object::Name(base_font.into_bytes()));
    font.set("Encoding", Object::Name(b"Identity-H".to_vec()));
    font.set("DescendantFonts", Object::Array(vec![Object::Reference(cid)]));
    font.set("ToUnicode", Object::Reference(to_unicode));
    let font = document.add_object(font);
    Ok(EmbeddedFont {
        font,
        cap_height: cap,
        units_per_em: upem,
        face_bytes: bytes.to_vec(),
        remap: subset.map(|(_, remap)| remap),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_unicode_ranges_and_ligatures_read_back() {
        let cmap = b"1 begincodespacerange <0000> <FFFF> endcodespacerange\n2 beginbfchar <0003> <0020> <00A1> <00660069> endbfchar\n1 beginbfrange <0010> <0012> <0041> endbfrange\n1 beginbfrange <0020> <0021> [<0061> <0062>] endbfrange";
        let (map, bytes) = parse_to_unicode(cmap);
        assert_eq!(bytes, Some(2));
        assert_eq!(map.get(&0x03).map(String::as_str), Some(" "));
        assert_eq!(map.get(&0xA1).map(String::as_str), Some("fi"));
        assert_eq!(map.get(&0x12).map(String::as_str), Some("C"));
        assert_eq!(map.get(&0x21).map(String::as_str), Some("b"));
    }

    #[test]
    fn a_library_face_is_embedded_as_a_subset_of_the_typed_glyphs() {
        const BOLD: &[u8] = include_bytes!("../../wellfriendpdf-edit/fonts/LiberationSans-Bold.ttf");
        let mut document = Document::with_version("1.7");
        let embedded = embed(&mut document, BOLD, "liberation-sans-bold", "WELLREAD").unwrap();
        let (codes, advance) = embedded.encode("READ WELL").unwrap();
        assert_eq!(codes.len(), 18, "two bytes per character");
        assert!(codes.chunks(2).all(|c| u16::from_be_bytes([c[0], c[1]]) < 10), "renumbered glyphs: {codes:?}");
        assert!(advance > 4.0);
        assert!(embedded.encode("Z").is_err(), "a glyph outside the subset is refused");
        let font = document.get_dictionary(embedded.font).unwrap();
        let base = font.get(b"BaseFont").unwrap().as_name().unwrap();
        assert_eq!(base[6], b'+', "a subset tag: {}", String::from_utf8_lossy(base));
        let program = document
            .objects
            .values()
            .find_map(|o| o.as_stream().ok().filter(|s| s.dict.has(b"Length1")))
            .unwrap();
        let length = program.dict.get(b"Length1").unwrap().as_i64().unwrap() as usize;
        assert!(length < BOLD.len() / 5, "the subset is far smaller than the face: {length} of {}", BOLD.len());
        let decoded = program.decompressed_content().unwrap_or_else(|_| program.content.clone());
        assert!(ttf_parser::Face::parse(&decoded, 0).is_ok(), "the subset is a valid TrueType program");
    }

    #[test]
    fn glyph_names_read_as_text() {
        assert_eq!(glyph_unicode("A").as_deref(), Some("A"));
        assert_eq!(glyph_unicode("uni0041").as_deref(), Some("A"));
        assert_eq!(glyph_unicode("fi").as_deref(), Some("fi"));
        assert_eq!(glyph_unicode("a.sc").as_deref(), Some("a"));
        assert_eq!(glyph_unicode("g1"), None);
        assert_eq!(win_ansi(0x93), Some('\u{201C}'));
    }
}
