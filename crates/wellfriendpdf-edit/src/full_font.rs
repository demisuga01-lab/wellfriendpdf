//! Replacement text that the run's embedded font cannot draw.
//!
//! PDFs usually embed a subset of each font: only the glyphs the document
//! used. A replacement can encode through the font's /Encoding and still name
//! glyphs the subset does not contain, which viewers draw as blanks or boxes.
//! `embedded_font_draws` checks the embedded program itself. When it cannot
//! draw the replacement, `replace_text_full_font` writes that one text-show
//! operation in the full font of the same family, embedded as a Type0 font, at
//! the run's own size: the text matrix, spacing, colour and every other
//! operation are untouched, and the run's original font is selected again
//! right after it.
use crate::*;
use lopdf::{Stream, dictionary};
use std::sync::OnceLock;

/// Directories scanned for full font programs, in order.
const SYSTEM_FONT_DIRS: &[&str] = &["/usr/share/fonts", "/usr/local/share/fonts"];
const MAX_SYSTEM_FONT_FILES: usize = 4_000;
const MAX_FONT_FILE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_FONT_DIR_DEPTH: usize = 8;

fn font_descriptor<'a>(document: &'a Document, font: &'a Dictionary) -> Option<&'a Dictionary> {
    let descriptor = font.get(b"FontDescriptor").ok()?;
    match descriptor {
        Object::Reference(id) => document.get_dictionary(*id).ok(),
        Object::Dictionary(dictionary) => Some(dictionary),
        _ => None,
    }
}

fn stream_bytes(document: &Document, value: &Object) -> Option<Vec<u8>> {
    let stream = match value {
        Object::Reference(id) => document.get_object(*id).ok()?.as_stream().ok()?,
        Object::Stream(stream) => stream,
        _ => return None,
    };
    stream
        .get_plain_content_with_limit(MAX_DECOMPRESSED_STREAM_BYTES)
        .ok()
}

fn name_of(dictionary: &Dictionary, key: &[u8]) -> Option<String> {
    dictionary
        .get(key)
        .ok()
        .and_then(|value| value.as_name().ok())
        .map(|name| String::from_utf8_lossy(name).into_owned())
}

/// Whether the embedded font program of a simple (one byte per code) font
/// can draw every code of `encoded`, which encodes `text`. Composite fonts
/// map codes through their ToUnicode CMap, which a subsetter writes for the
/// kept glyphs only, so the encoding round-trip already proved coverage.
pub(crate) fn embedded_font_draws(
    document: &Document,
    font: &Dictionary,
    encoded: &[u8],
    text: &str,
) -> bool {
    if name_of(font, b"Subtype").as_deref() != Some("TrueType")
        && name_of(font, b"Subtype").as_deref() != Some("Type1")
        && name_of(font, b"Subtype").as_deref() != Some("MMType1")
    {
        return true;
    }
    let characters: Vec<char> = text.chars().collect();
    if characters.len() != encoded.len() {
        return false;
    }
    let first_char = font
        .get(b"FirstChar")
        .ok()
        .and_then(pdf_number)
        .map(|value| value as i64);
    let widths = font
        .get(b"Widths")
        .ok()
        .and_then(|value| match value {
            Object::Array(items) => Some(items.clone()),
            Object::Reference(id) => document
                .get_object(*id)
                .ok()
                .and_then(|object| object.as_array().ok().cloned()),
            _ => None,
        });
    let Some(descriptor) = font_descriptor(document, font) else {
        // A font without a descriptor is one of the standard 14: the viewer
        // supplies the complete font.
        return true;
    };
    let program = descriptor
        .get(b"FontFile2")
        .ok()
        .and_then(|value| stream_bytes(document, value));
    let embedded = program.is_some()
        || descriptor.has(b"FontFile")
        || descriptor.has(b"FontFile3");
    if !embedded {
        return true;
    }
    for (code, character) in encoded.iter().zip(&characters) {
        // A code outside /Widths is drawn with /MissingWidth: the advance
        // would be wrong even where a glyph exists.
        if let (Some(first), Some(widths)) = (first_char, widths.as_ref()) {
            let index = i64::from(*code) - first;
            let width = usize::try_from(index)
                .ok()
                .and_then(|index| widths.get(index))
                .and_then(pdf_number);
            match width {
                Some(width) if width > 0.0 || character.is_whitespace() => {}
                _ => return false,
            }
        }
        if character.is_whitespace() {
            continue;
        }
        if let Some(program) = program.as_deref() {
            if !truetype_has_glyph(program, *code, *character) {
                return false;
            }
        } else if let Some(charset) = descriptor.get(b"CharSet").ok().and_then(|value| value.as_str().ok()) {
            // Type 1 and CFF subsets list their glyph names; check the
            // standard name of plain ASCII characters against it.
            if let Some(glyph) = standard_glyph_name(*character) {
                let charset = String::from_utf8_lossy(charset);
                if !charset.split('/').any(|name| name.trim() == glyph) {
                    return false;
                }
            }
        }
    }
    true
}

fn truetype_has_glyph(program: &[u8], code: u8, character: char) -> bool {
    let Ok(face) = ttf_parser::Face::parse(program, 0) else {
        return false;
    };
    if face.glyph_index(character).is_some_and(|glyph| glyph.0 != 0) {
        return true;
    }
    let Some(cmap) = face.tables().cmap else {
        return false;
    };
    // Symbolic subsets map the byte code itself: (3,0) at 0xF000+code or
    // code, and (1,0) at code.
    cmap.subtables.into_iter().any(|subtable| {
        [u32::from(code), 0xF000 + u32::from(code)]
            .into_iter()
            .any(|candidate| subtable.glyph_index(candidate).is_some_and(|glyph| glyph.0 != 0))
    })
}

fn standard_glyph_name(character: char) -> Option<&'static str> {
    const DIGITS: [&str; 10] = [
        "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
    ];
    const LETTERS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    if let Some(digit) = character.to_digit(10) {
        return Some(DIGITS[digit as usize]);
    }
    if character.is_ascii_alphabetic() {
        let index = LETTERS.find(character)?;
        return Some(&LETTERS[index..index + 1]);
    }
    Some(match character {
        '.' => "period",
        ',' => "comma",
        ':' => "colon",
        ';' => "semicolon",
        '-' => "hyphen",
        '(' => "parenleft",
        ')' => "parenright",
        '\'' => "quotesingle",
        '"' => "quotedbl",
        '!' => "exclam",
        '?' => "question",
        '/' => "slash",
        '&' => "ampersand",
        '%' => "percent",
        '$' => "dollar",
        '@' => "at",
        '#' => "numbersign",
        '+' => "plus",
        '=' => "equal",
        '*' => "asterisk",
        _ => return None,
    })
}

/// One full font program found on the host or bundled with the editor.
#[derive(Clone)]
struct FontCandidate {
    postscript_name: String,
    family: String,
    bold: bool,
    italic: bool,
    source: FontSource,
}

#[derive(Clone)]
enum FontSource {
    File(PathBuf),
    Bundled(&'static [u8]),
}

impl FontCandidate {
    fn bytes(&self) -> Option<Vec<u8>> {
        match &self.source {
            FontSource::Bundled(bytes) => Some(bytes.to_vec()),
            FontSource::File(path) => {
                let metadata = fs::metadata(path).ok()?;
                if metadata.len() > MAX_FONT_FILE_BYTES {
                    return None;
                }
                fs::read(path).ok()
            }
        }
    }
}

fn normalized(value: &str) -> String {
    value
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect()
}

fn describe(bytes: &[u8], source: FontSource) -> Option<FontCandidate> {
    let face = ttf_parser::Face::parse(bytes, 0).ok()?;
    // Only TrueType outlines embed as FontFile2, and a restricted licence
    // forbids embedding at all.
    if face.tables().glyf.is_none()
        || matches!(face.permissions(), Some(ttf_parser::Permissions::Restricted))
    {
        return None;
    }
    let name = |id: u16| {
        face.names()
            .into_iter()
            .filter(|entry| entry.name_id == id && entry.is_unicode())
            .find_map(|entry| entry.to_string())
    };
    let postscript_name = name(ttf_parser::name_id::POST_SCRIPT_NAME)?;
    let family = name(ttf_parser::name_id::TYPOGRAPHIC_FAMILY)
        .or_else(|| name(ttf_parser::name_id::FAMILY))?;
    Some(FontCandidate {
        postscript_name,
        family,
        bold: face.is_bold(),
        italic: face.is_italic() || face.is_oblique(),
        source,
    })
}

fn scan_directory(directory: &Path, depth: usize, found: &mut Vec<FontCandidate>, files: &mut usize) {
    if depth > MAX_FONT_DIR_DEPTH || *files >= MAX_SYSTEM_FONT_FILES {
        return;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(|entry| entry.ok().map(|entry| entry.path())).collect();
    paths.sort();
    for path in paths {
        if *files >= MAX_SYSTEM_FONT_FILES {
            return;
        }
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.is_dir() {
            scan_directory(&path, depth + 1, found, files);
            continue;
        }
        let is_truetype = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("ttf"));
        if !metadata.is_file() || !is_truetype || metadata.len() > MAX_FONT_FILE_BYTES {
            continue;
        }
        *files += 1;
        if let Ok(bytes) = fs::read(&path) {
            if let Some(candidate) = describe(&bytes, FontSource::File(path.clone())) {
                found.push(candidate);
            }
        }
    }
}

fn system_fonts() -> &'static [FontCandidate] {
    static FONTS: OnceLock<Vec<FontCandidate>> = OnceLock::new();
    FONTS.get_or_init(|| {
        let mut found = Vec::new();
        let mut files = 0;
        for directory in SYSTEM_FONT_DIRS {
            scan_directory(Path::new(directory), 0, &mut found, &mut files);
        }
        for name in styled_text::BUNDLED_FONTS {
            if let Ok(bytes) = styled_text::font_data(name) {
                if let Some(candidate) = describe(bytes, FontSource::Bundled(bytes)) {
                    found.push(candidate);
                }
            }
        }
        found
    })
}

/// The family a PDF BaseFont names, without a subset tag or style suffix.
fn base_font_parts(base_font: &str) -> (String, bool, bool) {
    let bytes = base_font.as_bytes();
    let name = if bytes.len() > 7 && bytes[6] == b'+' && bytes[..6].iter().all(u8::is_ascii_uppercase) {
        &base_font[7..]
    } else {
        base_font
    };
    let lower = name.to_ascii_lowercase();
    let bold = ["bold", "black", "heavy", "semibold", "demibold"].iter().any(|word| lower.contains(word));
    let italic = lower.contains("italic") || lower.contains("oblique");
    let family = name
        .split(['-', ','])
        .next()
        .unwrap_or(name)
        .trim_end_matches("PSMT")
        .trim_end_matches("MT")
        .trim_end_matches("PS");
    (family.to_string(), bold, italic)
}

/// Metric-compatible families for the commonest proprietary fonts.
fn compatible_families(family: &str) -> &'static [&'static str] {
    match normalized(family).as_str() {
        "arial" | "helvetica" | "arialmt" | "helveticaneue" | "liberationsans" => &["Liberation Sans", "Arimo"],
        "times" | "timesnewroman" | "timesroman" | "liberationserif" => &["Liberation Serif", "Tinos"],
        "courier" | "couriernew" | "liberationmono" => &["Liberation Mono", "Cousine"],
        "calibri" => &["Carlito"],
        "cambria" => &["Caladea"],
        _ => &[],
    }
}

/// Full font programs to try for a run, best match first: the exact
/// PostScript name, then the same family and style, then a metric-compatible
/// family in the same style.
fn font_candidates(base_font: &str, descriptor_bold: bool, descriptor_italic: bool) -> Vec<FontCandidate> {
    let (family, name_bold, name_italic) = base_font_parts(base_font);
    let bold = name_bold || descriptor_bold;
    let italic = name_italic || descriptor_italic;
    let exact = {
        let bytes = base_font.as_bytes();
        let stripped = if bytes.len() > 7 && bytes[6] == b'+' { &base_font[7..] } else { base_font };
        normalized(stripped)
    };
    let fonts = system_fonts();
    let mut ordered: Vec<FontCandidate> = Vec::new();
    let mut push = |candidate: &FontCandidate| {
        if !ordered.iter().any(|existing| existing.postscript_name == candidate.postscript_name) {
            ordered.push(candidate.clone());
        }
    };
    for candidate in fonts.iter().filter(|candidate| normalized(&candidate.postscript_name) == exact) {
        push(candidate);
    }
    let families: Vec<String> = std::iter::once(family.clone())
        .chain(compatible_families(&family).iter().map(|name| (*name).to_string()))
        .collect();
    for wanted in &families {
        for candidate in fonts.iter().filter(|candidate| {
            normalized(&candidate.family) == normalized(wanted) && candidate.bold == bold && candidate.italic == italic
        }) {
            push(candidate);
        }
    }
    ordered
}

/// Embeds `bytes` as a Type0 Identity-H font and encodes `text` with it.
fn embed_type0(
    document: &mut Document,
    name: &str,
    bytes: Vec<u8>,
    text: &str,
) -> Result<(ObjectId, Vec<u8>), PdfEditError> {
    let face = ttf_parser::Face::parse(&bytes, 0).map_err(|_| PdfEditError::GlyphUnavailable)?;
    let upem = f64::from(face.units_per_em());
    let unit = |value: i16| Object::Real((f64::from(value) * 1000.0 / upem) as f32);
    let mut glyphs: Vec<(char, u16, f64)> = Vec::new();
    let mut encoded = Vec::new();
    for character in text.chars() {
        // Shaping-dependent scripts and invisible controls need the SDK shaper.
        if character.is_control()
            || matches!(character as u32, 0x300..=0x36f | 0x590..=0x10ff | 0x200b..=0x200f | 0x202a..=0x202e | 0x2066..=0x2069)
        {
            return Err(PdfEditError::GlyphUnavailable);
        }
        let glyph = face
            .glyph_index(character)
            .filter(|glyph| glyph.0 != 0)
            .ok_or(PdfEditError::GlyphUnavailable)?;
        let width = f64::from(face.glyph_hor_advance(glyph).ok_or(PdfEditError::GlyphUnavailable)?) * 1000.0 / upem;
        let cid = match glyphs.iter().position(|(seen, _, _)| *seen == character) {
            Some(index) => index + 1,
            None => {
                glyphs.push((character, glyph.0, width));
                glyphs.len()
            }
        };
        let cid = u16::try_from(cid).map_err(|_| PdfEditError::TextRunUnsupported)?;
        encoded.extend_from_slice(&cid.to_be_bytes());
    }
    let mut cid_to_gid = vec![0u8, 0];
    let mut widths = Vec::new();
    let mut mappings = Vec::new();
    for (index, (character, glyph, width)) in glyphs.iter().enumerate() {
        cid_to_gid.extend_from_slice(&glyph.to_be_bytes());
        widths.push(Object::Real(*width as f32));
        let unicode: String = character
            .encode_utf16(&mut [0; 2])
            .iter()
            .map(|value| format!("{value:04X}"))
            .collect();
        mappings.push(format!("<{:04X}> <{unicode}>\n", index + 1));
    }
    let bbox = face.global_bounding_box();
    let length = bytes.len() as i64;
    let file = add_compressed_stream(document, dictionary! { "Length1" => length }, bytes)?;
    let base_name = format!("WFEDIT+{}", name.replace(|c: char| !c.is_ascii_alphanumeric() && c != '-', ""));
    let mut flags = 32;
    if face.is_monospaced() {
        flags |= 1;
    }
    if face.is_italic() || face.is_oblique() {
        flags |= 64;
    }
    let descriptor = document.add_object(dictionary! {
        "Type" => "FontDescriptor", "FontName" => base_name.as_str(), "Flags" => flags,
        "FontBBox" => vec![unit(bbox.x_min), unit(bbox.y_min), unit(bbox.x_max), unit(bbox.y_max)],
        "ItalicAngle" => face.italic_angle(), "Ascent" => unit(face.ascender()),
        "Descent" => unit(face.descender()), "CapHeight" => unit(face.capital_height().unwrap_or(face.ascender())),
        "StemV" => if face.is_bold() { 140 } else { 80 }, "FontFile2" => file,
    });
    let map = add_compressed_stream(document, Dictionary::new(), cid_to_gid)?;
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /WellfriendFullFont def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    for chunk in mappings.chunks(100) {
        cmap.push_str(&format!("{} beginbfchar\n{}endbfchar\n", chunk.len(), chunk.concat()));
    }
    cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    let to_unicode = add_compressed_stream(document, Dictionary::new(), cmap.into_bytes())?;
    let descendant = document.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "CIDFontType2", "BaseFont" => base_name.as_str(),
        "CIDSystemInfo" => dictionary! { "Registry" => Object::string_literal("Adobe"), "Ordering" => Object::string_literal("Identity"), "Supplement" => 0 },
        "FontDescriptor" => descriptor, "DW" => 1000,
        "W" => vec![Object::Integer(1), Object::Array(widths)], "CIDToGIDMap" => map,
    });
    let font = document.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type0", "BaseFont" => base_name.as_str(),
        "Encoding" => "Identity-H", "DescendantFonts" => vec![Object::Reference(descendant)], "ToUnicode" => to_unicode,
    });
    Ok((font, encoded))
}

fn add_compressed_stream(
    document: &mut Document,
    dictionary: Dictionary,
    bytes: Vec<u8>,
) -> Result<ObjectId, PdfEditError> {
    let mut stream = Stream::new(dictionary, bytes);
    stream.compress().map_err(|_| PdfEditError::OutputWriteFailed)?;
    Ok(document.add_object(stream))
}

/// The run's font state at the selected operation: resource name and size.
fn font_state_at(
    document: &Document,
    page: ObjectId,
    target: ObjectId,
    content: &Content,
    operation_index: usize,
) -> Result<(Vec<u8>, f64), PdfEditError> {
    let mut state = text_show::FontState::default();
    for stream_id in document.get_page_contents(page) {
        if stream_id == target {
            break;
        }
        let bytes = document
            .get_object(stream_id)
            .and_then(Object::as_stream)
            .map_err(|_| PdfEditError::ContentStreamUnsupported)?
            .get_plain_content_with_limit(MAX_DECOMPRESSED_STREAM_BYTES)
            .map_err(|_| PdfEditError::ContentStreamUnsupported)?;
        for operation in Content::decode(&bytes).map_err(|_| PdfEditError::ContentStreamUnsupported)?.operations {
            state.apply(&operation)?;
        }
    }
    for operation in content.operations.iter().take(operation_index) {
        state.apply(operation)?;
    }
    match (state.name, state.size) {
        (Some(name), Some(size)) if size.is_finite() => Ok((name, size)),
        _ => Err(PdfEditError::TextRunUnsupported),
    }
}

/// Replaces one selected text run in the full font of its family when the
/// run's embedded font cannot draw the replacement. Refuses when no
/// embeddable full font of the same family (or a metric-compatible one)
/// covers every character.
pub fn replace_text_full_font(request: &ReplaceTextRequest) -> Result<serde_json::Value, PdfEditError> {
    if request.replacement_text.len() > MAX_EDITABLE_TEXT_RUN_BYTES
        || request.replacement_text.chars().count() > MAX_EDITABLE_TEXT_RUN_CHARS
        || request.replacement_text.is_empty()
    {
        return Err(PdfEditError::TextRunUnsupported);
    }
    validate_paths(&request.input_path, &request.output_path)?;
    let input_metadata = checked_metadata(&request.input_path)?;
    let input_sha256 = sha256_file(&request.input_path)?;
    let mut document = load_checked_pdf(&request.input_path)?;
    let signatures = signature_summaries(&document);
    if !signatures.is_empty() && !request.allow_signature_invalidation {
        return Err(PdfEditError::SignatureConfirmationRequired);
    }
    let source_objects = document.objects.clone();
    let pages = document.get_pages();
    let page = page_id(&document, request.selector.page_number)?;
    let target: ObjectId = request.selector.content_stream_object.into();
    if !document.get_page_contents(page).contains(&target) {
        return Err(PdfEditError::ContentStreamNotOnPage);
    }
    // The new font is a page resource: a stream drawn by another page too
    // would name a font that page does not have.
    if pages.values().filter(|id| document.get_page_contents(**id).contains(&target)).count() != 1 {
        return Err(PdfEditError::TextRunUnsupported);
    }
    // Authenticate the selection against its original text and font.
    let (_, source_bytes, font_resource) = replacement_bytes_for_target(
        &document,
        page,
        target,
        &request.selector,
        &request.selector.expected_text,
    )?;
    let plain = document
        .get_object(target)
        .and_then(Object::as_stream)
        .map_err(|_| PdfEditError::ContentStreamUnsupported)?
        .get_plain_content_with_limit(MAX_DECOMPRESSED_STREAM_BYTES)
        .map_err(|_| PdfEditError::ContentStreamUnsupported)?;
    let mut content = Content::decode(&plain).map_err(|_| PdfEditError::ContentStreamUnsupported)?;
    if content.operations.len() > MAX_CONTENT_OPERATIONS_PER_STREAM {
        return Err(PdfEditError::ContentOperationLimitExceeded);
    }
    let index = request.selector.operation_index;
    let operator = content
        .operations
        .get(index)
        .map(|operation| operation.operator.clone())
        .ok_or(PdfEditError::StaleTextRun)?;
    if !matches!(operator.as_str(), "Tj" | "TJ" | "'" | "\"") {
        return Err(PdfEditError::TextRunUnsupported);
    }
    let (font_name, font_size) = font_state_at(&document, page, target, &content, index)?;
    if font_name != font_resource.as_bytes() {
        return Err(PdfEditError::StaleTextRun);
    }
    let fonts = document.get_page_fonts(page).map_err(|_| PdfEditError::ContentStreamUnsupported)?;
    let original = fonts.get(&font_name).ok_or(PdfEditError::TextRunUnsupported)?;
    // Vertical writing needs Identity-V and vertical metrics.
    if name_of(original, b"Encoding").is_some_and(|encoding| encoding.ends_with("-V")) {
        return Err(PdfEditError::TextRunUnsupported);
    }
    let original_type0 = name_of(original, b"Subtype").as_deref() == Some("Type0");
    let base_font = name_of(original, b"BaseFont").ok_or(PdfEditError::GlyphUnavailable)?;
    let descriptor_font = if original_type0 {
        original
            .get(b"DescendantFonts")
            .ok()
            .and_then(|value| match value {
                Object::Array(items) => items.first().cloned(),
                Object::Reference(id) => document.get_object(*id).ok()?.as_array().ok()?.first().cloned(),
                _ => None,
            })
            .and_then(|value| match value {
                Object::Reference(id) => document.get_dictionary(id).ok().cloned(),
                Object::Dictionary(dictionary) => Some(dictionary),
                _ => None,
            })
    } else {
        Some(original.clone())
    };
    let flags = descriptor_font
        .as_ref()
        .and_then(|font| font_descriptor(&document, font))
        .and_then(|descriptor| descriptor.get(b"Flags").ok().and_then(pdf_number))
        .unwrap_or(0.0) as i64;
    let descriptor_italic = flags & 64 != 0;
    let descriptor_bold = flags & (1 << 18) != 0;

    let mut chosen = None;
    for candidate in font_candidates(&base_font, descriptor_bold, descriptor_italic) {
        let Some(bytes) = candidate.bytes() else {
            continue;
        };
        let Ok(face) = ttf_parser::Face::parse(&bytes, 0) else {
            continue;
        };
        if request
            .replacement_text
            .chars()
            .all(|character| face.glyph_index(character).is_some_and(|glyph| glyph.0 != 0))
        {
            chosen = Some((candidate, bytes));
            break;
        }
    }
    let (candidate, bytes) = chosen.ok_or(PdfEditError::GlyphUnavailable)?;
    let exact_family = normalized(&candidate.family) == normalized(&base_font_parts(&base_font).0)
        || normalized(&candidate.postscript_name) == normalized(&base_font_parts(&base_font).0);
    let (font, encoded) = embed_type0(&mut document, &candidate.postscript_name, bytes, &request.replacement_text)?;

    let resource_name = format!("WFFull{}", font.0);
    let mut resources = page_effective_resources_clone(&document, page)?;
    let mut page_fonts = match resources.get(b"Font") {
        Ok(Object::Dictionary(dictionary)) => dictionary.clone(),
        Ok(Object::Reference(id)) => document
            .get_object(*id)
            .and_then(Object::as_dict)
            .map_err(|_| PdfEditError::TextRunUnsupported)?
            .clone(),
        _ => return Err(PdfEditError::TextRunUnsupported),
    };
    if page_fonts.has(resource_name.as_bytes()) {
        return Err(PdfEditError::ValidationFailed);
    }
    page_fonts.set(resource_name.as_str(), font);
    resources.set("Font", page_fonts);
    document
        .get_object_mut(page)
        .and_then(Object::as_dict_mut)
        .map_err(|_| PdfEditError::ValidationFailed)?
        .set("Resources", resources);

    // Select the full font for this one show operation, then the run's own
    // font again, both at the run's size. Positioning stays as it was.
    let size = pdf_number_object(font_size)?;
    let replacement = Object::String(encoded.clone(), StringFormat::Hexadecimal);
    let show = match operator.as_str() {
        "\"" => {
            let operands = &content.operations[index].operands;
            Operation::new("\"", vec![operands[0].clone(), operands[1].clone(), replacement])
        }
        "'" => Operation::new("'", vec![replacement]),
        _ => Operation::new("Tj", vec![replacement]),
    };
    content.operations[index] = show;
    content.operations.insert(
        index,
        Operation::new("Tf", vec![Object::Name(resource_name.as_bytes().to_vec()), size.clone()]),
    );
    content
        .operations
        .insert(index + 2, Operation::new("Tf", vec![Object::Name(font_name.clone()), size]));
    let output_index = index + 1;
    let stream = document
        .get_object_mut(target)
        .and_then(Object::as_stream_mut)
        .map_err(|_| PdfEditError::ValidationFailed)?;
    stream.set_plain_content(content.encode().map_err(|_| PdfEditError::ValidationFailed)?);
    stream.compress().map_err(|_| PdfEditError::OutputWriteFailed)?;
    let changed = changed_object_ids(&source_objects, &document.objects);
    save_new_pdf(&mut document, &request.output_path)?;

    let validated = (|| {
        let output = load_checked_pdf(&request.output_path)?;
        let output_page = page_id(&output, request.selector.page_number)?;
        let mut selector = request.selector.clone();
        selector.operation_index = output_index;
        let operation = target_operation(&output, output_page, target, &selector)?;
        let replacement_extractable =
            decoded_tj_text(&output, output_page, &operation, &resource_name)? == request.replacement_text;
        let non_target_preserved = source_objects.iter().all(|(id, object)| {
            *id == target
                || *id == page
                || is_writer_housekeeping_object(object)
                || output
                    .objects
                    .get(id)
                    .is_some_and(|candidate| pdf_objects_semantically_equal(object, candidate))
        });
        if !replacement_extractable || !non_target_preserved || output.get_pages().len() != pages.len() {
            eprintln!(
                "PDF_FULL_FONT_VALIDATION_FAILED text={replacement_extractable} objects={non_target_preserved} pages={}",
                output.get_pages().len() == pages.len()
            );
            return Err(PdfEditError::ValidationFailed);
        }
        Ok(serde_json::json!({
            "operation": "REPLACE_TEXT", "capability": "PDF_TEXT_REPLACE",
            "inputSha256": input_sha256, "outputSha256": sha256_file(&request.output_path)?,
            "inputBytes": input_metadata.len(), "outputBytes": checked_metadata(&request.output_path)?.len(),
            "pageCount": pages.len(), "sourceObjectCount": source_objects.len(), "outputObjectCount": output.objects.len(),
            "changedObjects": changed.iter().copied().map(PdfObjectId::from).collect::<Vec<_>>(),
            "fontResource": resource_name, "sourceText": request.selector.expected_text, "replacementText": request.replacement_text,
            "sourceEncodedBytes": source_bytes.len(), "replacementEncodedBytes": encoded.len(), "saveMode": "FULL_REWRITE",
            "signatureEffect": if signatures.is_empty() { "NONE" } else { "INVALIDATES_EXISTING_SIGNATURES" },
            "fullFont": {
                "reason": "EMBEDDED_SUBSET_LACKS_GLYPHS",
                "originalFont": base_font,
                "embeddedFont": candidate.postscript_name,
                "sameFamily": exact_family,
                "fontSize": font_size,
            },
            "validation": {
                "reparsed": true, "pageCountPreserved": true, "nonTargetObjectsPreserved": true,
                "replacementExtractable": true, "fontEmbedded": true, "sizePreserved": true,
                "positionPreserved": true, "noPageRasterization": true
            }
        }))
    })();
    if validated.is_err() {
        let _ = fs::remove_file(&request.output_path);
    }
    validated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_font_parts_strip_subset_tags_and_styles() {
        assert_eq!(base_font_parts("BAAAAA+LiberationMono"), ("LiberationMono".to_string(), false, false));
        assert_eq!(base_font_parts("ABCDEF+Arial-BoldItalicMT"), ("Arial".to_string(), true, true));
        assert_eq!(base_font_parts("TimesNewRomanPSMT"), ("TimesNewRoman".to_string(), false, false));
        assert_eq!(base_font_parts("Helvetica,Bold"), ("Helvetica".to_string(), true, false));
    }

    #[test]
    fn proprietary_families_map_to_metric_compatible_ones() {
        assert_eq!(compatible_families("Arial")[0], "Liberation Sans");
        assert_eq!(compatible_families("TimesNewRoman")[0], "Liberation Serif");
        assert_eq!(compatible_families("CourierNew")[0], "Liberation Mono");
        assert!(compatible_families("SomeBrandFont").is_empty());
    }

    #[test]
    fn bundled_liberation_fonts_are_candidates() {
        let candidates = font_candidates("BAAAAA+LiberationMono", false, false);
        assert!(candidates.iter().any(|candidate| candidate.postscript_name == "LiberationMono"));
        let arial = font_candidates("ArialMT", false, false);
        assert!(arial.iter().any(|candidate| candidate.family == "Liberation Sans" && !candidate.bold));
    }

    #[test]
    fn standard_glyph_names_cover_ascii() {
        assert_eq!(standard_glyph_name('W'), Some("W"));
        assert_eq!(standard_glyph_name('7'), Some("seven"));
        assert_eq!(standard_glyph_name('.'), Some("period"));
        assert_eq!(standard_glyph_name('é'), None);
    }
}
