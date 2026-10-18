//! Content usage scanning and font embedding for PDF/A conversion.
//!
//! PDF/A requires every font program used for rendering to be embedded. When a
//! source font is not embedded (Standard 14 or a referenced system font), the
//! converter substitutes a metric-compatible, freely licensed program
//! (bundled Liberation/DejaVu, or an embeddable system font when enabled),
//! subsets it with `subsetter`, and rewrites its advance widths to the source
//! PDF widths so glyph positions — and therefore layout — do not move. Each
//! substituted font gets a ToUnicode CMap generated from the SDK's own decoder
//! so text extraction is unchanged.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::Serialize;

use crate::content::{ContentParser, Operand};
use crate::document::PdfDocument;
use crate::filters::decode_stream;
use crate::fonts::character_code::CharacterCode;
use crate::fonts::glyph_list::glyph_name_to_unicode;
use crate::fonts::resolver::FontResolver;
use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;

type ObjectId = (u32, u16);

const MAX_SCAN_DEPTH: usize = 24;

/// What a content scan observed: per-font shown strings and device colour use.
#[derive(Debug, Default)]
pub(crate) struct ContentUsage {
    pub font_strings: HashMap<ObjectId, HashSet<Vec<u8>>>,
    pub device_gray: bool,
    pub device_rgb: bool,
    pub device_cmyk: bool,
    /// Streams that could not be decoded/parsed (usage may be incomplete).
    pub incomplete: Vec<String>,
}

struct Scanner<'a> {
    reader: &'a PdfReader,
    usage: ContentUsage,
    visited: HashSet<ObjectId>,
}

/// Scan every page, Form XObject, tiling pattern, Type3 glyph procedure and
/// annotation appearance for text-showing strings and device colour use.
pub(crate) fn scan_content_usage(doc: &PdfDocument) -> crate::error::Result<ContentUsage> {
    let reader = doc.reader();
    let mut scanner = Scanner {
        reader,
        usage: ContentUsage::default(),
        visited: HashSet::new(),
    };
    for page in doc.get_pages()? {
        match doc.get_page_content_bytes(page.page_number) {
            Ok(bytes) => scanner.scan_stream(&bytes, &page.resources, 0),
            Err(err) => scanner
                .usage
                .incomplete
                .push(format!("page {}: {err}", page.page_number)),
        }
        if let Ok(PdfObject::Dictionary(page_dict)) =
            reader.get_and_resolve(page.object_number, page.generation_number)
        {
            if let Some(annots) = resolve(reader, page_dict.get("Annots")) {
                for annot in annots.as_array().unwrap_or(&[]) {
                    if let Some(PdfObject::Dictionary(annot)) = resolve(reader, Some(annot)) {
                        scanner.scan_appearance(&annot, &page.resources);
                    }
                }
            }
        }
    }
    scan_object_colour_usage(reader, &mut scanner.usage);
    Ok(scanner.usage)
}

fn resolve(reader: &PdfReader, object: Option<&PdfObject>) -> Option<PdfObject> {
    object.and_then(|obj| reader.resolve(obj.clone()).ok())
}

fn resolve_dict(reader: &PdfReader, object: Option<&PdfObject>) -> Option<PdfDictionary> {
    match resolve(reader, object)? {
        PdfObject::Dictionary(dict) => Some(dict),
        PdfObject::Stream { dict, .. } => Some(dict),
        _ => None,
    }
}

impl Scanner<'_> {
    fn scan_appearance(&mut self, annot: &PdfDictionary, page_resources: &PdfDictionary) {
        let Some(ap) = resolve_dict(self.reader, annot.get("AP")) else {
            return;
        };
        for key in ["N", "R", "D"] {
            let Some(value) = ap.get(key) else { continue };
            let mut streams = Vec::new();
            match value {
                PdfObject::Reference { number, generation } => {
                    match self.reader.get_and_resolve(*number, *generation) {
                        Ok(PdfObject::Dictionary(states)) => {
                            streams.extend(states.entries().map(|(_, v)| v.clone()))
                        }
                        Ok(_) => streams.push(value.clone()),
                        Err(_) => {}
                    }
                }
                PdfObject::Dictionary(states) => {
                    streams.extend(states.entries().map(|(_, v)| v.clone()))
                }
                _ => {}
            }
            for stream in streams {
                self.scan_form(&stream, page_resources, 0);
            }
        }
    }

    fn scan_form(&mut self, object: &PdfObject, inherited: &PdfDictionary, depth: usize) {
        if depth > MAX_SCAN_DEPTH {
            return;
        }
        if let Some(id) = object.as_reference() {
            if !self.visited.insert(id) {
                return;
            }
        }
        let Ok(resolved) = self.reader.resolve(object.clone()) else {
            return;
        };
        let PdfObject::Stream { dict, .. } = &resolved else {
            return;
        };
        if dict.get_name("Subtype").is_some_and(|s| s != "Form") {
            return;
        }
        let resources =
            resolve_dict(self.reader, dict.get("Resources")).unwrap_or_else(|| inherited.clone());
        match decode_stream(&resolved, self.reader) {
            Ok(bytes) => self.scan_stream(&bytes, &resources, depth + 1),
            Err(err) => self.usage.incomplete.push(format!("form: {err}")),
        }
    }

    fn scan_stream(&mut self, data: &[u8], resources: &PdfDictionary, depth: usize) {
        if depth > MAX_SCAN_DEPTH {
            return;
        }
        // Tiling patterns live in the resource scope and paint their own content.
        if let Some(patterns) = resolve_dict(self.reader, resources.get("Pattern")) {
            for (_, pattern) in patterns.entries() {
                if let Some(id) = pattern.as_reference() {
                    if self.visited.contains(&id) {
                        continue;
                    }
                }
                if let Some(dict) = resolve_dict(self.reader, Some(pattern)) {
                    if dict.get_integer("PatternType") == Some(1) {
                        let mut wrapper = pattern.clone();
                        if let PdfObject::Reference { .. } = pattern {
                            wrapper = pattern.clone();
                        }
                        self.scan_pattern(&wrapper, resources, depth + 1);
                    }
                }
            }
        }
        let operations = match ContentParser::parse(data) {
            Ok(ops) => ops,
            Err(err) => {
                self.usage.incomplete.push(format!("content: {err}"));
                return;
            }
        };
        let fonts = resolve_dict(self.reader, resources.get("Font"));
        let mut current_font: Option<ObjectId> = None;
        for op in &operations {
            match op.operator.as_str() {
                "Tf" => {
                    current_font = None;
                    if let (Some(name), Some(fonts)) = (op.name(0), fonts.as_ref()) {
                        if let Some(PdfObject::Reference { number, generation }) = fonts.get(name) {
                            current_font = Some((*number, *generation));
                            self.scan_type3((*number, *generation), resources, depth);
                        }
                    }
                }
                "Tj" | "'" => {
                    if let (Some(font), Some(Operand::String(bytes))) =
                        (current_font, op.operands.last())
                    {
                        self.record(font, bytes);
                    }
                }
                "\"" => {
                    if let (Some(font), Some(Operand::String(bytes))) =
                        (current_font, op.operands.get(2))
                    {
                        self.record(font, bytes);
                    }
                }
                "TJ" => {
                    if let (Some(font), Some(Operand::Array(items))) =
                        (current_font, op.operands.first())
                    {
                        for item in items {
                            if let Operand::String(bytes) = item {
                                self.record(font, bytes);
                            }
                        }
                    }
                }
                "g" | "G" => self.usage.device_gray = true,
                "rg" | "RG" => self.usage.device_rgb = true,
                "k" | "K" => self.usage.device_cmyk = true,
                "cs" | "CS" => {
                    if let Some(name) = op.name(0) {
                        if !self.classify_device_name(name) {
                            if let Some(spaces) =
                                resolve_dict(self.reader, resources.get("ColorSpace"))
                            {
                                if let Some(space) = spaces.get(name) {
                                    let space = space.clone();
                                    self.classify_colour_space(&space, 0);
                                }
                            }
                        }
                    }
                }
                "ID" => {
                    let mut inline_cs: Option<Operand> = None;
                    let mut iter = op.operands.iter();
                    while let (Some(key), Some(value)) = (iter.next(), iter.next()) {
                        if key.as_name() == Some("ColorSpace") {
                            inline_cs = Some(value.clone());
                        }
                    }
                    if let Some(cs) = inline_cs.take() {
                        self.classify_inline_cs(&cs, resources);
                    }
                }
                "Do" => {
                    if let Some(name) = op.name(0) {
                        if let Some(xobjects) = resolve_dict(self.reader, resources.get("XObject"))
                        {
                            if let Some(xobject) = xobjects.get(name) {
                                let xobject = xobject.clone();
                                self.scan_form(&xobject, resources, depth + 1);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn scan_pattern(&mut self, pattern: &PdfObject, inherited: &PdfDictionary, depth: usize) {
        if let Some(id) = pattern.as_reference() {
            if !self.visited.insert(id) {
                return;
            }
        }
        let Ok(resolved) = self.reader.resolve(pattern.clone()) else {
            return;
        };
        let PdfObject::Stream { dict, .. } = &resolved else {
            return;
        };
        let resources =
            resolve_dict(self.reader, dict.get("Resources")).unwrap_or_else(|| inherited.clone());
        if let Ok(bytes) = decode_stream(&resolved, self.reader) {
            self.scan_stream(&bytes, &resources, depth + 1);
        }
    }

    fn scan_type3(&mut self, font: ObjectId, inherited: &PdfDictionary, depth: usize) {
        if self.visited.contains(&font) {
            return;
        }
        let Ok(PdfObject::Dictionary(dict)) = self.reader.get_and_resolve(font.0, font.1) else {
            return;
        };
        if dict.get_name("Subtype") != Some("Type3") {
            return;
        }
        self.visited.insert(font);
        let resources =
            resolve_dict(self.reader, dict.get("Resources")).unwrap_or_else(|| inherited.clone());
        if let Some(procs) = resolve_dict(self.reader, dict.get("CharProcs")) {
            for (_, proc_ref) in procs.entries() {
                if let Some(stream) = resolve(self.reader, Some(proc_ref)) {
                    if let Ok(bytes) = decode_stream(&stream, self.reader) {
                        self.scan_stream(&bytes, &resources, depth + 1);
                    }
                }
            }
        }
    }

    fn record(&mut self, font: ObjectId, bytes: &[u8]) {
        let entry = self.usage.font_strings.entry(font).or_default();
        if entry.len() < 200_000 {
            entry.insert(bytes.to_vec());
        }
    }

    fn classify_device_name(&mut self, name: &str) -> bool {
        classify_device_name(&mut self.usage, name)
    }

    fn classify_colour_space(&mut self, space: &PdfObject, depth: usize) {
        classify_colour_space(self.reader, &mut self.usage, space, depth);
    }

    fn classify_inline_cs(&mut self, cs: &Operand, resources: &PdfDictionary) {
        match cs {
            Operand::Name(name) => {
                if !classify_device_name(&mut self.usage, name) {
                    if let Some(spaces) = resolve_dict(self.reader, resources.get("ColorSpace")) {
                        if let Some(space) = spaces.get(name) {
                            let space = space.clone();
                            self.classify_colour_space(&space, 0);
                        }
                    }
                }
            }
            Operand::Array(items) => {
                for item in items {
                    if let Operand::Name(name) = item {
                        classify_device_name(&mut self.usage, name);
                    }
                }
            }
            _ => {}
        }
    }
}

fn classify_device_name(usage: &mut ContentUsage, name: &str) -> bool {
    match name {
        "DeviceGray" | "G" => usage.device_gray = true,
        "DeviceRGB" | "RGB" => usage.device_rgb = true,
        "DeviceCMYK" | "CMYK" => usage.device_cmyk = true,
        _ => return false,
    }
    true
}

pub(crate) fn classify_colour_space(
    reader: &PdfReader,
    usage: &mut ContentUsage,
    space: &PdfObject,
    depth: usize,
) {
    if depth > 8 {
        return;
    }
    let Ok(space) = reader.resolve(space.clone()) else {
        return;
    };
    match &space {
        PdfObject::Name(name) => {
            classify_device_name(usage, name);
        }
        PdfObject::Array(items) => match items.first().and_then(PdfObject::as_name) {
            Some("ICCBased") | Some("CalRGB") | Some("CalGray") | Some("Lab") => {}
            Some("Indexed") => {
                if let Some(base) = items.get(1) {
                    classify_colour_space(reader, usage, base, depth + 1);
                }
            }
            Some("Pattern") => {
                if let Some(base) = items.get(1) {
                    classify_colour_space(reader, usage, base, depth + 1);
                }
            }
            Some("Separation") => {
                if let Some(alt) = items.get(2) {
                    classify_colour_space(reader, usage, alt, depth + 1);
                }
            }
            Some("DeviceN") => {
                if let Some(alt) = items.get(2) {
                    classify_colour_space(reader, usage, alt, depth + 1);
                }
            }
            Some(name) => {
                classify_device_name(usage, name);
            }
            None => {}
        },
        _ => {}
    }
}

/// Device colour spaces named by images, shadings, groups and colour space
/// resources anywhere in the object graph.
fn scan_object_colour_usage(reader: &PdfReader, usage: &mut ContentUsage) {
    for (number, generation) in reader.object_ids() {
        let Ok(object) = reader.get_object(number, generation) else {
            continue;
        };
        let dict = match &object {
            PdfObject::Dictionary(dict) => dict,
            PdfObject::Stream { dict, .. } => dict,
            _ => continue,
        };
        scan_dict_colour(reader, usage, dict, 0);
    }
}

fn scan_dict_colour(
    reader: &PdfReader,
    usage: &mut ContentUsage,
    dict: &PdfDictionary,
    depth: usize,
) {
    if depth > 6 {
        return;
    }
    for (key, value) in dict.entries() {
        match key.as_str() {
            "ColorSpace" | "CS" => match value {
                PdfObject::Dictionary(spaces) => {
                    for (name, space) in spaces.entries() {
                        if !name.starts_with("Default") {
                            classify_colour_space(reader, usage, space, 0);
                        }
                    }
                }
                PdfObject::Reference { .. } => match reader.resolve(value.clone()) {
                    Ok(PdfObject::Dictionary(spaces)) => {
                        for (name, space) in spaces.entries() {
                            if !name.starts_with("Default") {
                                classify_colour_space(reader, usage, space, 0);
                            }
                        }
                    }
                    Ok(other) => classify_colour_space(reader, usage, &other, 0),
                    Err(_) => {}
                },
                other => classify_colour_space(reader, usage, other, 0),
            },
            "Group" | "Shading" => {
                if let PdfObject::Dictionary(inner) = value {
                    scan_dict_colour(reader, usage, inner, depth + 1);
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Font embedding
// ---------------------------------------------------------------------------

/// Per-font embedding outcome in the conversion report.
#[derive(Debug, Clone, Serialize)]
pub struct PdfAFontEmbedding {
    pub object_number: u32,
    pub base_font: String,
    pub font_type: String,
    pub substitute: Option<String>,
    pub glyphs: usize,
    pub unmapped_codes: usize,
    pub duplicated_glyphs: usize,
    pub embedded: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
}

/// New objects and replacement dictionaries produced for one font. Object
/// references inside use placeholder numbers `u32::MAX - n` that the caller
/// rewrites to allocated numbers.
pub(crate) struct FontReplacement {
    pub font: ObjectId,
    pub font_dict: PdfDictionary,
    /// Replacement for an indirect descendant CIDFont, if any.
    pub descendant: Option<(ObjectId, PdfDictionary)>,
    pub new_objects: Vec<PdfObject>,
}

pub(crate) const PLACEHOLDER_BASE: u32 = u32::MAX - 1024;

fn placeholder(index: usize) -> PdfObject {
    PdfObject::Reference {
        number: PLACEHOLDER_BASE + index as u32,
        generation: 0,
    }
}

#[derive(Clone)]
struct Candidate {
    label: String,
    data: std::sync::Arc<Vec<u8>>,
    index: u32,
}

impl Candidate {
    fn bundled(label: &str, data: &'static [u8]) -> Self {
        Self {
            label: label.to_string(),
            data: std::sync::Arc::new(data.to_vec()),
            index: 0,
        }
    }
}

/// Font embedding options.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FontEmbedOptions {
    pub system_fonts: bool,
    pub write_cid_set: bool,
}

pub(crate) fn font_is_embedded(reader: &PdfReader, font: &PdfDictionary) -> bool {
    match font.get_name("Subtype") {
        Some("Type3") => true,
        Some("Type0") => {
            let Some(desc) = descendant_font(reader, font) else {
                return false;
            };
            descriptor_has_program(reader, &desc)
        }
        _ => descriptor_has_program(reader, font),
    }
}

fn descriptor_has_program(reader: &PdfReader, font: &PdfDictionary) -> bool {
    resolve_dict(reader, font.get("FontDescriptor")).is_some_and(|fd| {
        ["FontFile", "FontFile2", "FontFile3"]
            .iter()
            .any(|key| fd.get(key).is_some_and(|v| !v.is_null()))
    })
}

fn descendant_font(reader: &PdfReader, font: &PdfDictionary) -> Option<PdfDictionary> {
    let array = resolve(reader, font.get("DescendantFonts"))?;
    let first = array.as_array()?.first()?.clone();
    resolve_dict(reader, Some(&first))
}

/// Build a replacement for a non-embedded font. Returns the report row and,
/// on success, the replacement objects.
pub(crate) fn embed_font(
    doc: &PdfDocument,
    font_id: ObjectId,
    usage: &ContentUsage,
    options: FontEmbedOptions,
) -> (PdfAFontEmbedding, Option<FontReplacement>) {
    let reader = doc.reader();
    let font = match reader.get_and_resolve(font_id.0, font_id.1) {
        Ok(PdfObject::Dictionary(dict)) => dict,
        _ => {
            return (
                failed_row(font_id, "", "", "font dictionary could not be resolved"),
                None,
            )
        }
    };
    let base = font
        .get_name("BaseFont")
        .map(str::to_string)
        .unwrap_or_else(|| "Unnamed".to_string());
    let subtype = font.get_name("Subtype").unwrap_or("Type1").to_string();
    let result = if subtype == "Type0" {
        embed_composite(reader, font_id, &font, &base, usage, options)
    } else {
        embed_simple(reader, font_id, &font, &base, usage, options)
    };
    match result {
        Ok((row, replacement)) => (row, Some(replacement)),
        Err(message) => (failed_row(font_id, &base, &subtype, &message), None),
    }
}

fn failed_row(font_id: ObjectId, base: &str, subtype: &str, message: &str) -> PdfAFontEmbedding {
    PdfAFontEmbedding {
        object_number: font_id.0,
        base_font: base.to_string(),
        font_type: subtype.to_string(),
        substitute: None,
        glyphs: 0,
        unmapped_codes: 0,
        duplicated_glyphs: 0,
        embedded: false,
        failure: Some(message.to_string()),
    }
}

/// One glyph request: an encoded code (simple) or CID (composite), the text it
/// decodes to, an optional glyph name and its PDF advance width (1/1000 em).
struct GlyphRequest {
    key: u32,
    text: String,
    glyph_name: Option<String>,
    width: f64,
}

fn embed_simple(
    reader: &PdfReader,
    font_id: ObjectId,
    font: &PdfDictionary,
    base: &str,
    usage: &ContentUsage,
    options: FontEmbedOptions,
) -> std::result::Result<(PdfAFontEmbedding, FontReplacement), String> {
    let resolver = FontResolver::new(font, reader);
    let descriptor = resolve_dict(reader, font.get("FontDescriptor"));
    let has_widths = font.get("Widths").is_some();
    let mut used: BTreeSet<u32> = BTreeSet::new();
    if let Some(strings) = usage.font_strings.get(&font_id) {
        for bytes in strings {
            used.extend(bytes.iter().map(|b| u32::from(*b)));
        }
    }
    let mut requests = Vec::new();
    for code in 0u32..=255 {
        let (text, _) = resolver.decode_char_with_source(code as u16);
        let glyph_name = resolver.glyph_name(code as u16).map(str::to_string);
        let decodable = !text.is_empty() && text != "\u{FFFD}";
        if !used.contains(&code) && glyph_name.is_none() && !(decodable && code >= 32) {
            continue;
        }
        let width = if has_widths || resolver.has_standard14_metrics() {
            resolver.glyph_width(code as u16)
        } else {
            f64::NAN
        };
        requests.push(GlyphRequest {
            key: code,
            text: if decodable { text } else { String::new() },
            glyph_name,
            width,
        });
    }
    if requests.is_empty() {
        return Err("font has no encodable codes to embed".to_string());
    }
    let flags = descriptor
        .as_ref()
        .and_then(|d| d.get_integer("Flags"))
        .unwrap_or(0);
    let candidates = candidate_fonts(base, flags, options.system_fonts);
    let chosen = choose_candidate(&candidates, &requests, &used, false)?;
    let built = build_font_program(&chosen, &requests, true)?;

    let tag = subset_tag(font_id, base);
    let font_name = format!("{tag}+{}", sanitize_font_name(base));
    let first = requests.iter().map(|r| r.key).min().unwrap_or(0);
    let last = requests.iter().map(|r| r.key).max().unwrap_or(0);
    let mut widths = Vec::new();
    for code in first..=last {
        let width = built.widths_by_key.get(&code).copied().unwrap_or(0.0);
        widths.push(number(width));
    }
    let mut new_objects = Vec::new();
    new_objects.push(flate_stream(
        dict_of(&[("Length1", PdfObject::Integer(built.program.len() as i64))]),
        &built.program,
    ));
    let font_file = placeholder(0);
    let descriptor_dict = font_descriptor(
        &font_name,
        descriptor.as_ref(),
        &chosen,
        (flags as u32 & !32) | 4,
        ("FontFile2", font_file),
    );
    new_objects.push(PdfObject::Dictionary(descriptor_dict));
    let to_unicode = simple_to_unicode(&resolver);
    new_objects.push(flate_stream(PdfDictionary::empty(), to_unicode.as_bytes()));

    let mut dict = dict_of(&[
        ("Type", PdfObject::Name("Font".to_string())),
        ("Subtype", PdfObject::Name("TrueType".to_string())),
        ("BaseFont", PdfObject::Name(font_name)),
        ("FirstChar", PdfObject::Integer(i64::from(first))),
        ("LastChar", PdfObject::Integer(i64::from(last))),
        ("Widths", PdfObject::Array(widths)),
        ("FontDescriptor", placeholder(1)),
        ("ToUnicode", placeholder(2)),
    ]);
    if let Some(name) = font.get("Name") {
        dict.insert("Name", name.clone());
    }
    let row = PdfAFontEmbedding {
        object_number: font_id.0,
        base_font: base.to_string(),
        font_type: font.get_name("Subtype").unwrap_or("Type1").to_string(),
        substitute: Some(chosen.label.clone()),
        glyphs: built.glyph_count,
        unmapped_codes: built.unmapped.iter().filter(|k| used.contains(k)).count(),
        duplicated_glyphs: built.duplicates,
        embedded: true,
        failure: None,
    };
    Ok((
        row,
        FontReplacement {
            font: font_id,
            font_dict: dict,
            descendant: None,
            new_objects,
        },
    ))
}

fn embed_composite(
    reader: &PdfReader,
    font_id: ObjectId,
    font: &PdfDictionary,
    base: &str,
    usage: &ContentUsage,
    options: FontEmbedOptions,
) -> std::result::Result<(PdfAFontEmbedding, FontReplacement), String> {
    let resolver = FontResolver::new(font, reader);
    resolver
        .validate_encoding()
        .map_err(|err| format!("composite font encoding cannot be resolved: {err}"))?;
    let descendant = descendant_font(reader, font).ok_or("Type0 font has no descendant")?;
    let descriptor = resolve_dict(reader, descendant.get("FontDescriptor"));
    let mut codes: BTreeMap<(u32, u8), CharacterCode> = BTreeMap::new();
    if let Some(strings) = usage.font_strings.get(&font_id) {
        for bytes in strings {
            for code in resolver.codes(bytes).flatten() {
                codes.insert((code.code.value(), code.code.len()), code.code);
            }
        }
    }
    if codes.is_empty() {
        return Err("composite font has no observed glyph usage to embed".to_string());
    }
    let mut requests: BTreeMap<u32, GlyphRequest> = BTreeMap::new();
    let mut to_unicode_entries: Vec<(CharacterCode, String)> = Vec::new();
    let mut undecodable = 0usize;
    for code in codes.values() {
        let cid = resolver
            .cid_for_character(*code)
            .map_err(|err| format!("code without CID: {err}"))?;
        // A CID decoded only as "CID value == code point" (Identity ordering
        // without ToUnicode) says nothing about the glyph; never substitute
        // from it.
        let (text, source) = resolver.decode_code_with_source(*code);
        let decodable = !text.is_empty()
            && text != "\u{FFFD}"
            && !matches!(
                source,
                crate::fonts::resolver::FontDecodeSource::IdentityCid
                    | crate::fonts::resolver::FontDecodeSource::Unknown
            );
        if decodable {
            to_unicode_entries.push((*code, text.clone()));
        } else {
            undecodable += 1;
        }
        requests.entry(u32::from(cid)).or_insert(GlyphRequest {
            key: u32::from(cid),
            text: if decodable { text } else { String::new() },
            glyph_name: None,
            width: resolver.width_for_code(*code),
        });
    }
    let requests: Vec<GlyphRequest> = requests.into_values().collect();
    if undecodable == codes.len() {
        return Err(
            "composite font has no Unicode mapping (no ToUnicode and no predefined CMap); glyphs cannot be substituted"
                .to_string(),
        );
    }
    let flags = descriptor
        .as_ref()
        .and_then(|d| d.get_integer("Flags"))
        .unwrap_or(4);
    let all_keys: BTreeSet<u32> = requests.iter().map(|r| r.key).collect();
    let candidates = candidate_fonts(base, flags, options.system_fonts);
    let chosen = choose_candidate(&candidates, &requests, &all_keys, true)?;
    let built = build_font_program(&chosen, &requests, false)?;

    let tag = subset_tag(font_id, base);
    let font_name = format!("{tag}+{}", sanitize_font_name(base));
    let max_cid = requests.iter().map(|r| r.key).max().unwrap_or(0) as usize;
    let mut cid_to_gid = vec![0u8; (max_cid + 1) * 2];
    for (cid, gid) in &built.gid_by_key {
        let at = *cid as usize * 2;
        cid_to_gid[at..at + 2].copy_from_slice(&gid.to_be_bytes());
    }
    let mut new_objects = vec![
        flate_stream(
            dict_of(&[("Length1", PdfObject::Integer(built.program.len() as i64))]),
            &built.program,
        ),
        PdfObject::Null, // descriptor, filled below
        flate_stream(PdfDictionary::empty(), &cid_to_gid),
    ];
    let mut descriptor_dict = font_descriptor(
        &font_name,
        descriptor.as_ref(),
        &chosen,
        (flags as u32 & !32) | 4,
        ("FontFile2", placeholder(0)),
    );
    if options.write_cid_set {
        let mut bits = vec![0u8; max_cid / 8 + 1];
        bits[0] |= 0x80; // CID 0
        for cid in built.gid_by_key.keys() {
            bits[*cid as usize / 8] |= 0x80 >> (*cid % 8);
        }
        new_objects.push(flate_stream(PdfDictionary::empty(), &bits));
        descriptor_dict.insert("CIDSet", placeholder(new_objects.len() - 1));
    }
    new_objects[1] = PdfObject::Dictionary(descriptor_dict);

    let mut new_descendant = descendant.clone();
    new_descendant.insert("Subtype", PdfObject::Name("CIDFontType2".to_string()));
    new_descendant.insert("BaseFont", PdfObject::Name(font_name.clone()));
    new_descendant.insert("FontDescriptor", placeholder(1));
    new_descendant.insert("CIDToGIDMap", placeholder(2));
    if !new_descendant.contains_key("DW") && !new_descendant.contains_key("W") {
        new_descendant.insert("DW", PdfObject::Integer(1000));
    }

    let mut new_font = font.clone();
    new_font.insert("BaseFont", PdfObject::Name(font_name));
    if font.get("ToUnicode").is_none() && !to_unicode_entries.is_empty() {
        let cmap = composite_to_unicode(&to_unicode_entries);
        new_objects.push(flate_stream(PdfDictionary::empty(), cmap.as_bytes()));
        new_font.insert("ToUnicode", placeholder(new_objects.len() - 1));
    }
    let descendant_ref = resolve(reader, font.get("DescendantFonts"))
        .and_then(|arr| arr.as_array().and_then(|a| a.first().cloned()))
        .and_then(|first| first.as_reference());
    let descendant_replacement = match descendant_ref {
        Some(id) => Some((id, new_descendant)),
        None => {
            new_font.insert(
                "DescendantFonts",
                PdfObject::Array(vec![PdfObject::Dictionary(new_descendant)]),
            );
            None
        }
    };
    let used_keys: BTreeSet<u32> = all_keys;
    let row = PdfAFontEmbedding {
        object_number: font_id.0,
        base_font: base.to_string(),
        font_type: "Type0".to_string(),
        substitute: Some(chosen.label.clone()),
        glyphs: built.glyph_count,
        unmapped_codes: built
            .unmapped
            .iter()
            .filter(|k| used_keys.contains(k))
            .count(),
        duplicated_glyphs: built.duplicates,
        embedded: true,
        failure: None,
    };
    Ok((
        row,
        FontReplacement {
            font: font_id,
            font_dict: new_font,
            descendant: descendant_replacement,
            new_objects,
        },
    ))
}

// ---------------------------------------------------------------------------
// Candidate selection
// ---------------------------------------------------------------------------

fn candidate_fonts(base: &str, flags: i64, system_fonts: bool) -> Vec<Candidate> {
    let mut out = Vec::new();
    if system_fonts {
        if let Some(exact) = system_font_exact(base) {
            out.push(exact);
        }
    }
    let (label, data) = bundled_substitute(base, flags);
    out.push(Candidate::bundled(label, data));
    if label != "DejaVuSans" {
        out.push(Candidate::bundled(
            "DejaVuSans",
            crate::render::font_rasterizer::fallback_fonts::DEJAVU_SANS,
        ));
    }
    out
}

fn bundled_substitute(base: &str, flags: i64) -> (&'static str, &'static [u8]) {
    use crate::render::font_rasterizer::fallback_fonts as f;
    let raw = base.rsplit('+').next().unwrap_or(base).to_lowercase();
    let bold = raw.contains("bold")
        || raw.contains("black")
        || raw.contains("heavy")
        || raw.contains("semibold")
        || flags & (1 << 18) != 0;
    let italic =
        raw.contains("italic") || raw.contains("oblique") || raw.ends_with(",i") || flags & 64 != 0;
    if raw.contains("symbol")
        || raw.contains("dingbat")
        || raw.contains("wingding")
        || raw.contains("webding")
    {
        return ("DejaVuSans", f::DEJAVU_SANS);
    }
    let mono = raw.contains("courier")
        || raw.contains("mono")
        || raw.contains("consol")
        || raw.contains("typewriter")
        || (flags & 1 != 0);
    let serif = !mono
        && (raw.contains("times")
            || raw.contains("serif") && !raw.contains("sans")
            || raw.contains("roman")
            || raw.contains("georgia")
            || raw.contains("garamond")
            || raw.contains("cambria")
            || raw.contains("book")
            || raw.contains("palatino")
            || raw.contains("minion")
            || (flags & 2 != 0
                && !raw.contains("arial")
                && !raw.contains("helvetica")
                && !raw.contains("sans")));
    match (mono, serif, bold, italic) {
        (true, _, true, true) => ("LiberationMono-BoldItalic", f::LIBERATION_MONO_BOLD_ITALIC),
        (true, _, true, false) => ("LiberationMono-Bold", f::LIBERATION_MONO_BOLD),
        (true, _, false, true) => ("LiberationMono-Italic", f::LIBERATION_MONO_ITALIC),
        (true, _, false, false) => ("LiberationMono-Regular", f::LIBERATION_MONO_REGULAR),
        (_, true, true, true) => (
            "LiberationSerif-BoldItalic",
            f::LIBERATION_SERIF_BOLD_ITALIC,
        ),
        (_, true, true, false) => ("LiberationSerif-Bold", f::LIBERATION_SERIF_BOLD),
        (_, true, false, true) => ("LiberationSerif-Italic", f::LIBERATION_SERIF_ITALIC),
        (_, true, false, false) => ("LiberationSerif-Regular", f::LIBERATION_SERIF_REGULAR),
        (_, _, true, true) => ("LiberationSans-BoldItalic", f::LIBERATION_SANS_BOLD_ITALIC),
        (_, _, true, false) => ("LiberationSans-Bold", f::LIBERATION_SANS_BOLD),
        (_, _, false, true) => ("LiberationSans-Italic", f::LIBERATION_SANS_ITALIC),
        _ => ("LiberationSans-Regular", f::LIBERATION_SANS_REGULAR),
    }
}

fn face_embeddable(face: &ttf_parser::Face<'_>) -> bool {
    let fs_type = crate::fonts::pdf_embedding::serialized_fs_type(face).unwrap_or(0);
    // Installable (0), preview & print (4) and editable (8) embedding all
    // permit embedding in a static archival document; restricted (2),
    // bitmap-only (0x200) and no-subsetting (0x100) do not fit this path.
    matches!(fs_type & 0x000f, 0 | 4 | 8)
        && fs_type & 0x0200 == 0
        && fs_type & 0x0100 == 0
        && face.tables().glyf.is_some()
}

#[cfg(not(target_arch = "wasm32"))]
fn system_database() -> Option<&'static fontdb::Database> {
    static DB: std::sync::OnceLock<Option<fontdb::Database>> = std::sync::OnceLock::new();
    DB.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        (!db.is_empty()).then_some(db)
    })
    .as_ref()
}

fn normalize_font_key(name: &str) -> String {
    name.rsplit('+')
        .next()
        .unwrap_or(name)
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .replace("regular", "")
        .replace("roman", "")
        .replace("mt", "")
        .replace("ps", "")
}

#[cfg(not(target_arch = "wasm32"))]
fn system_font_exact(base: &str) -> Option<Candidate> {
    let db = system_database()?;
    let wanted = normalize_font_key(base);
    if wanted.is_empty() {
        return None;
    }
    for face in db.faces() {
        if normalize_font_key(&face.post_script_name) != wanted {
            continue;
        }
        let candidate = db.with_face_data(face.id, |data, index| {
            let parsed = ttf_parser::Face::parse(data, index).ok()?;
            face_embeddable(&parsed).then(|| Candidate {
                label: format!("system:{}", face.post_script_name),
                data: std::sync::Arc::new(data.to_vec()),
                index,
            })
        })??;
        return Some(candidate);
    }
    None
}

#[cfg(target_arch = "wasm32")]
fn system_font_exact(_base: &str) -> Option<Candidate> {
    None
}

#[cfg(not(target_arch = "wasm32"))]
fn system_font_with_coverage(requests: &[GlyphRequest]) -> Option<Candidate> {
    let db = system_database()?;
    let chars: Vec<char> = requests
        .iter()
        .filter_map(|r| r.text.chars().next())
        .collect();
    if chars.is_empty() {
        return None;
    }
    let mut best: Option<(usize, fontdb::ID)> = None;
    for face in db.faces() {
        let covered = db
            .with_face_data(face.id, |data, index| {
                let parsed = ttf_parser::Face::parse(data, index).ok()?;
                if !face_embeddable(&parsed) {
                    return None;
                }
                Some(
                    chars
                        .iter()
                        .filter(|ch| parsed.glyph_index(**ch).is_some())
                        .count(),
                )
            })
            .flatten()
            .unwrap_or(0);
        if covered > best.map(|b| b.0).unwrap_or(0) {
            best = Some((covered, face.id));
            if covered == chars.len() {
                break;
            }
        }
    }
    let (_, id) = best?;
    db.with_face_data(id, |data, index| Candidate {
        label: format!(
            "system:{}",
            db.face(id)
                .map(|f| f.post_script_name.clone())
                .unwrap_or_default()
        ),
        data: std::sync::Arc::new(data.to_vec()),
        index,
    })
}

#[cfg(target_arch = "wasm32")]
fn system_font_with_coverage(_requests: &[GlyphRequest]) -> Option<Candidate> {
    None
}

fn glyph_for_request(face: &ttf_parser::Face<'_>, request: &GlyphRequest) -> Option<u16> {
    if let Some(name) = request.glyph_name.as_deref() {
        if let Some(gid) = face.glyph_index_by_name(name) {
            return Some(gid.0);
        }
        if let Some(ch) = glyph_name_to_unicode(name) {
            if let Some(gid) = face.glyph_index(ch) {
                return Some(gid.0);
            }
        }
    }
    let mut chars = request.text.chars();
    let first = chars.next()?;
    if chars.next().is_some() {
        let ligature = match request.text.as_str() {
            "ff" => Some('\u{FB00}'),
            "fi" => Some('\u{FB01}'),
            "fl" => Some('\u{FB02}'),
            "ffi" => Some('\u{FB03}'),
            "ffl" => Some('\u{FB04}'),
            _ => None,
        };
        if let Some(gid) = ligature.and_then(|ch| face.glyph_index(ch)) {
            return Some(gid.0);
        }
    }
    face.glyph_index(first).map(|gid| gid.0)
}

fn coverage(candidate: &Candidate, requests: &[GlyphRequest], used: &BTreeSet<u32>) -> usize {
    let Ok(face) = ttf_parser::Face::parse(&candidate.data, candidate.index) else {
        return 0;
    };
    requests
        .iter()
        .filter(|r| used.contains(&r.key) && glyph_for_request(&face, r).is_some())
        .count()
}

fn choose_candidate(
    candidates: &[Candidate],
    requests: &[GlyphRequest],
    used: &BTreeSet<u32>,
    allow_system_coverage: bool,
) -> std::result::Result<Candidate, String> {
    let needed = requests
        .iter()
        .filter(|r| used.contains(&r.key) && (!r.text.is_empty() || r.glyph_name.is_some()))
        .count();
    let mut best: Option<(usize, &Candidate)> = None;
    for candidate in candidates {
        let covered = coverage(candidate, requests, used);
        if covered >= needed {
            return Ok(candidate.clone());
        }
        if best.map(|b| covered > b.0).unwrap_or(true) {
            best = Some((covered, candidate));
        }
    }
    if allow_system_coverage || needed > 0 {
        if let Some(system) = system_font_with_coverage(
            &requests
                .iter()
                .filter(|r| used.contains(&r.key))
                .map(|r| GlyphRequest {
                    key: r.key,
                    text: r.text.clone(),
                    glyph_name: r.glyph_name.clone(),
                    width: r.width,
                })
                .collect::<Vec<_>>(),
        ) {
            let covered = coverage(&system, requests, used);
            if covered >= needed {
                return Ok(system);
            }
            if best.map(|b| covered > b.0).unwrap_or(true) {
                return if covered * 10 >= needed * 9 || needed == 0 {
                    Ok(system)
                } else {
                    Err(format!(
                        "no substitute font covers the used glyphs ({covered}/{needed} best)"
                    ))
                };
            }
        }
    }
    match best {
        // Accept a near-complete match; the uncovered codes are drawn with a
        // blank glyph and counted in the report as unmapped.
        Some((covered, candidate)) if needed == 0 || covered * 10 >= needed * 9 => {
            Ok(candidate.clone())
        }
        Some((covered, _)) => Err(format!(
            "no substitute font covers the used glyphs ({covered}/{needed} best)"
        )),
        None => Err("no substitute font available".to_string()),
    }
}

// ---------------------------------------------------------------------------
// Font program construction
// ---------------------------------------------------------------------------

struct BuiltProgram {
    program: Vec<u8>,
    gid_by_key: BTreeMap<u32, u16>,
    widths_by_key: BTreeMap<u32, f64>,
    unmapped: Vec<u32>,
    duplicates: usize,
    glyph_count: usize,
}

fn build_font_program(
    candidate: &Candidate,
    requests: &[GlyphRequest],
    simple_cmap: bool,
) -> std::result::Result<BuiltProgram, String> {
    let face = ttf_parser::Face::parse(&candidate.data, candidate.index)
        .map_err(|err| format!("substitute font unreadable: {err}"))?;
    let upem = f64::from(face.units_per_em());
    let blank = face.glyph_index(' ').map(|g| g.0).unwrap_or(0);
    let mut remapper = subsetter::GlyphRemapper::new();
    let mut source_gid: Vec<(u32, u16, f64)> = Vec::new();
    let mut unmapped = Vec::new();
    for request in requests {
        let gid = match glyph_for_request(&face, request) {
            Some(gid) => gid,
            None => {
                unmapped.push(request.key);
                blank
            }
        };
        let width = if request.width.is_finite() {
            request.width
        } else {
            f64::from(
                face.glyph_hor_advance(ttf_parser::GlyphId(gid))
                    .unwrap_or(0),
            ) * 1000.0
                / upem
        };
        remapper.remap(gid);
        source_gid.push((request.key, gid, width));
    }
    let subset = subsetter::subset(&candidate.data, candidate.index, &remapper)
        .map_err(|err| format!("font subsetting failed: {err:?}"))?;
    let mut tables = parse_tables(&subset)?;
    let maxp = tables.get(b"maxp").ok_or("subset font has no maxp")?;
    let base_glyphs = usize::from(u16::from_be_bytes([maxp[4], maxp[5]]));

    // Advance per new gid; duplicate a glyph when two codes/CIDs share a
    // glyph but declare different PDF widths.
    let mut advance: HashMap<u16, i32> = HashMap::new();
    let mut duplicates: Vec<u16> = Vec::new(); // dup index -> source new gid
    let mut gid_by_key = BTreeMap::new();
    let mut widths_by_key = BTreeMap::new();
    let mut dup_lookup: HashMap<(u16, i32), u16> = HashMap::new();
    for (key, gid, width) in &source_gid {
        let new_gid = remapper.get(*gid).ok_or("subset lost a requested glyph")?;
        let units = (width * upem / 1000.0).round() as i32;
        let final_gid = match advance.get(&new_gid) {
            None => {
                advance.insert(new_gid, units);
                new_gid
            }
            Some(existing) if *existing == units => new_gid,
            Some(_) => {
                if let Some(dup) = dup_lookup.get(&(new_gid, units)) {
                    *dup
                } else {
                    let dup = u16::try_from(base_glyphs + duplicates.len())
                        .map_err(|_| "too many glyphs")?;
                    duplicates.push(new_gid);
                    advance.insert(dup, units);
                    dup_lookup.insert((new_gid, units), dup);
                    dup
                }
            }
        };
        gid_by_key.insert(*key, final_gid);
        widths_by_key.insert(*key, f64::from(units) * 1000.0 / upem);
    }
    let total = base_glyphs + duplicates.len();
    if !duplicates.is_empty() {
        append_duplicate_glyphs(&mut tables, base_glyphs, &duplicates)?;
    }
    rewrite_horizontal_metrics(&mut tables, base_glyphs, total, &advance)?;
    if let Some(post) = tables.get_mut(b"post") {
        if post.len() >= 32 {
            post.truncate(32);
            post[0..4].copy_from_slice(&0x0003_0000u32.to_be_bytes());
        }
    }
    if simple_cmap {
        tables.insert(*b"cmap", symbol_cmap(&gid_by_key));
    } else {
        tables.remove(b"cmap");
    }
    let program = crate::fonts::sfnt_subset::build_sfnt([0, 1, 0, 0], tables)
        .map_err(|err| format!("font rebuild failed: {err}"))?;
    Ok(BuiltProgram {
        program,
        gid_by_key,
        widths_by_key,
        unmapped,
        duplicates: duplicates.len(),
        glyph_count: total,
    })
}

fn parse_tables(font: &[u8]) -> std::result::Result<BTreeMap<[u8; 4], Vec<u8>>, String> {
    let read16 = |at: usize| -> std::result::Result<u16, String> {
        font.get(at..at + 2)
            .map(|b| u16::from_be_bytes([b[0], b[1]]))
            .ok_or_else(|| "truncated sfnt".to_string())
    };
    let read32 = |at: usize| -> std::result::Result<u32, String> {
        font.get(at..at + 4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
            .ok_or_else(|| "truncated sfnt".to_string())
    };
    let count = usize::from(read16(4)?);
    let mut out = BTreeMap::new();
    for idx in 0..count {
        let rec = 12 + idx * 16;
        let tag: [u8; 4] = font
            .get(rec..rec + 4)
            .ok_or("truncated table directory")?
            .try_into()
            .map_err(|_| "bad tag")?;
        let offset = read32(rec + 8)? as usize;
        let len = read32(rec + 12)? as usize;
        let data = font
            .get(offset..offset + len)
            .ok_or("table outside font")?
            .to_vec();
        out.insert(tag, data);
    }
    Ok(out)
}

fn read_loca(
    tables: &BTreeMap<[u8; 4], Vec<u8>>,
    glyphs: usize,
) -> std::result::Result<Vec<usize>, String> {
    let head = tables.get(b"head").ok_or("no head")?;
    let loca = tables.get(b"loca").ok_or("no loca")?;
    let long = i16::from_be_bytes([head[50], head[51]]) == 1;
    let mut offsets = Vec::with_capacity(glyphs + 1);
    for idx in 0..=glyphs {
        let value = if long {
            loca.get(idx * 4..idx * 4 + 4)
                .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize)
        } else {
            loca.get(idx * 2..idx * 2 + 2)
                .map(|b| usize::from(u16::from_be_bytes([b[0], b[1]])) * 2)
        };
        offsets.push(value.ok_or("truncated loca")?);
    }
    Ok(offsets)
}

fn append_duplicate_glyphs(
    tables: &mut BTreeMap<[u8; 4], Vec<u8>>,
    base_glyphs: usize,
    duplicates: &[u16],
) -> std::result::Result<(), String> {
    let offsets = read_loca(tables, base_glyphs)?;
    let mut glyf = tables.get(b"glyf").ok_or("no glyf")?.clone();
    glyf.truncate(offsets[base_glyphs]);
    let mut new_offsets: Vec<u32> = offsets.iter().map(|o| *o as u32).collect();
    for source in duplicates {
        let start = offsets[usize::from(*source)];
        let end = offsets[usize::from(*source) + 1];
        if end > start && end - start >= 10 {
            // Composite glyph with a single component at offset (0,0).
            let header = tables.get(b"glyf").unwrap()[start + 2..start + 10].to_vec();
            glyf.extend_from_slice(&(-1i16).to_be_bytes());
            glyf.extend_from_slice(&header);
            glyf.extend_from_slice(&0x0003u16.to_be_bytes()); // ARG_1_AND_2_ARE_WORDS | ARGS_ARE_XY_VALUES
            glyf.extend_from_slice(&source.to_be_bytes());
            glyf.extend_from_slice(&0i16.to_be_bytes());
            glyf.extend_from_slice(&0i16.to_be_bytes());
            while glyf.len() % 4 != 0 {
                glyf.push(0);
            }
        }
        new_offsets.push(glyf.len() as u32);
    }
    let mut loca = Vec::with_capacity(new_offsets.len() * 4);
    for offset in &new_offsets {
        loca.extend_from_slice(&offset.to_be_bytes());
    }
    tables.insert(*b"glyf", glyf);
    tables.insert(*b"loca", loca);
    let head = tables.get_mut(b"head").ok_or("no head")?;
    head[50..52].copy_from_slice(&1i16.to_be_bytes());
    let total = (base_glyphs + duplicates.len()) as u16;
    let maxp = tables.get_mut(b"maxp").ok_or("no maxp")?;
    maxp[4..6].copy_from_slice(&total.to_be_bytes());
    if maxp.len() >= 32 {
        let elements = u16::from_be_bytes([maxp[28], maxp[29]]).max(1);
        maxp[28..30].copy_from_slice(&elements.to_be_bytes());
        let depth = u16::from_be_bytes([maxp[30], maxp[31]]).max(1);
        maxp[30..32].copy_from_slice(&depth.to_be_bytes());
    }
    Ok(())
}

fn rewrite_horizontal_metrics(
    tables: &mut BTreeMap<[u8; 4], Vec<u8>>,
    base_glyphs: usize,
    total: usize,
    advance: &HashMap<u16, i32>,
) -> std::result::Result<(), String> {
    let hhea = tables.get(b"hhea").ok_or("no hhea")?;
    let metrics = usize::from(u16::from_be_bytes([hhea[34], hhea[35]]));
    let hmtx = tables.get(b"hmtx").ok_or("no hmtx")?;
    let mut old_adv = Vec::with_capacity(base_glyphs);
    let mut old_lsb = Vec::with_capacity(base_glyphs);
    let mut last_adv = 0u16;
    for gid in 0..base_glyphs {
        if gid < metrics {
            let at = gid * 4;
            last_adv = hmtx
                .get(at..at + 2)
                .map(|b| u16::from_be_bytes([b[0], b[1]]))
                .unwrap_or(0);
            old_adv.push(last_adv);
            old_lsb.push(
                hmtx.get(at + 2..at + 4)
                    .map(|b| i16::from_be_bytes([b[0], b[1]]))
                    .unwrap_or(0),
            );
        } else {
            let at = metrics * 4 + (gid - metrics) * 2;
            old_adv.push(last_adv);
            old_lsb.push(
                hmtx.get(at..at + 2)
                    .map(|b| i16::from_be_bytes([b[0], b[1]]))
                    .unwrap_or(0),
            );
        }
    }
    let mut new_hmtx = Vec::with_capacity(total * 4);
    let mut max_adv = 0u16;
    for gid in 0..total {
        let source = if gid < base_glyphs { gid } else { 0 };
        let adv = advance
            .get(&(gid as u16))
            .map(|v| (*v).clamp(0, i32::from(u16::MAX)) as u16)
            .unwrap_or_else(|| old_adv.get(source).copied().unwrap_or(0));
        let lsb = old_lsb.get(source).copied().unwrap_or(0);
        max_adv = max_adv.max(adv);
        new_hmtx.extend_from_slice(&adv.to_be_bytes());
        new_hmtx.extend_from_slice(&lsb.to_be_bytes());
    }
    tables.insert(*b"hmtx", new_hmtx);
    let hhea = tables.get_mut(b"hhea").ok_or("no hhea")?;
    hhea[10..12].copy_from_slice(&max_adv.to_be_bytes());
    hhea[34..36].copy_from_slice(&(total as u16).to_be_bytes());
    Ok(())
}

/// cmap with (3,0) symbol (0xF000 + code) and (1,0) Mac Roman (code) subtables
/// for a symbolic simple TrueType font addressed by single-byte codes.
fn symbol_cmap(gid_by_code: &BTreeMap<u32, u16>) -> Vec<u8> {
    // Format 4: one segment per code plus the 0xFFFF terminator.
    let mut seg_ends = Vec::new();
    let mut seg_starts = Vec::new();
    let mut deltas = Vec::new();
    for (code, gid) in gid_by_code {
        let unicode = 0xF000u16 + *code as u16;
        seg_starts.push(unicode);
        seg_ends.push(unicode);
        deltas.push(gid.wrapping_sub(unicode));
    }
    seg_starts.push(0xFFFF);
    seg_ends.push(0xFFFF);
    deltas.push(1);
    let seg_count = seg_starts.len() as u16;
    let mut entry_selector = 0u16;
    while (1u32 << (entry_selector + 1)) <= u32::from(seg_count) {
        entry_selector += 1;
    }
    let search_range = 2 * (1u16 << entry_selector);
    let mut f4 = Vec::new();
    f4.extend_from_slice(&4u16.to_be_bytes());
    f4.extend_from_slice(&0u16.to_be_bytes()); // length placeholder
    f4.extend_from_slice(&0u16.to_be_bytes());
    f4.extend_from_slice(&(seg_count * 2).to_be_bytes());
    f4.extend_from_slice(&search_range.to_be_bytes());
    f4.extend_from_slice(&entry_selector.to_be_bytes());
    f4.extend_from_slice(&(seg_count * 2 - search_range).to_be_bytes());
    for v in &seg_ends {
        f4.extend_from_slice(&v.to_be_bytes());
    }
    f4.extend_from_slice(&0u16.to_be_bytes());
    for v in &seg_starts {
        f4.extend_from_slice(&v.to_be_bytes());
    }
    for v in &deltas {
        f4.extend_from_slice(&v.to_be_bytes());
    }
    for _ in 0..seg_count {
        f4.extend_from_slice(&0u16.to_be_bytes());
    }
    let len = f4.len() as u16;
    f4[2..4].copy_from_slice(&len.to_be_bytes());

    // Format 6 trimmed table over the code range.
    let first = gid_by_code.keys().next().copied().unwrap_or(0) as u16;
    let last = gid_by_code.keys().last().copied().unwrap_or(0) as u16;
    let mut f6 = Vec::new();
    f6.extend_from_slice(&6u16.to_be_bytes());
    let count = last - first + 1;
    f6.extend_from_slice(&(10 + count * 2).to_be_bytes());
    f6.extend_from_slice(&0u16.to_be_bytes());
    f6.extend_from_slice(&first.to_be_bytes());
    f6.extend_from_slice(&count.to_be_bytes());
    for code in first..=last {
        let gid = gid_by_code.get(&u32::from(code)).copied().unwrap_or(0);
        f6.extend_from_slice(&gid.to_be_bytes());
    }

    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&2u16.to_be_bytes());
    let header = 4 + 2 * 8;
    // (1,0) must precede (3,0) in platform order.
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&(header as u32).to_be_bytes());
    out.extend_from_slice(&3u16.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&((header + f6.len()) as u32).to_be_bytes());
    out.extend_from_slice(&f6);
    out.extend_from_slice(&f4);
    out
}

// ---------------------------------------------------------------------------
// PDF objects
// ---------------------------------------------------------------------------

fn font_descriptor(
    font_name: &str,
    original: Option<&PdfDictionary>,
    candidate: &Candidate,
    flags: u32,
    program: (&str, PdfObject),
) -> PdfDictionary {
    let face = ttf_parser::Face::parse(&candidate.data, candidate.index).ok();
    let scale = face
        .as_ref()
        .map(|f| 1000.0 / f64::from(f.units_per_em()))
        .unwrap_or(1.0);
    let metric = |key: &str, fallback: f64| -> PdfObject {
        original
            .and_then(|d| d.get(key))
            .filter(|v| v.as_number().is_some())
            .cloned()
            .unwrap_or_else(|| number(fallback))
    };
    let (ascent, descent, cap, bbox, italic) = match &face {
        Some(f) => {
            let bb = f.global_bounding_box();
            (
                f64::from(f.ascender()) * scale,
                f64::from(f.descender()) * scale,
                f64::from(f.capital_height().unwrap_or(f.ascender())) * scale,
                [
                    f64::from(bb.x_min) * scale,
                    f64::from(bb.y_min) * scale,
                    f64::from(bb.x_max) * scale,
                    f64::from(bb.y_max) * scale,
                ],
                f64::from(f.italic_angle()),
            )
        }
        None => (800.0, -200.0, 700.0, [0.0, -200.0, 1000.0, 800.0], 0.0),
    };
    let mut dict = dict_of(&[
        ("Type", PdfObject::Name("FontDescriptor".to_string())),
        ("FontName", PdfObject::Name(font_name.to_string())),
        ("Flags", PdfObject::Integer(i64::from(flags))),
        (
            "FontBBox",
            PdfObject::Array(bbox.iter().map(|v| number(*v)).collect()),
        ),
        ("ItalicAngle", metric("ItalicAngle", italic)),
        ("Ascent", number(ascent)),
        ("Descent", number(descent)),
        ("CapHeight", metric("CapHeight", cap)),
        ("StemV", metric("StemV", 80.0)),
    ]);
    dict.insert(program.0, program.1);
    dict
}

fn simple_to_unicode(resolver: &FontResolver) -> String {
    let mut entries = Vec::new();
    for code in 0u16..=255 {
        let text = resolver.decode_char(code);
        if text.is_empty() || text == "\u{FFFD}" {
            continue;
        }
        entries.push((format!("{code:02X}"), utf16_hex(&text)));
    }
    to_unicode_cmap(&[(1, 0x00, 0xFF)], &entries)
}

fn composite_to_unicode(entries: &[(CharacterCode, String)]) -> String {
    let mut ranges: BTreeSet<(u8, u32, u32)> = BTreeSet::new();
    let mut rows = Vec::new();
    for (code, text) in entries {
        let len = code.len();
        let value = code.value();
        let hex = match len {
            1 => format!("{value:02X}"),
            2 => format!("{value:04X}"),
            3 => format!("{value:06X}"),
            _ => format!("{value:08X}"),
        };
        let shift = 8 * u32::from(len.saturating_sub(1));
        let lead = value >> shift;
        let low = lead << shift;
        let high = low | ((1u32 << shift).wrapping_sub(1));
        ranges.insert((len, low, high));
        rows.push((hex, utf16_hex(text)));
    }
    to_unicode_cmap(&ranges.into_iter().collect::<Vec<_>>(), &rows)
}

fn to_unicode_cmap(ranges: &[(u8, u32, u32)], entries: &[(String, String)]) -> String {
    let mut out = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n",
    );
    out.push_str(&format!("{} begincodespacerange\n", ranges.len()));
    for (len, low, high) in ranges {
        let width = usize::from(*len) * 2;
        out.push_str(&format!("<{low:0width$X}> <{high:0width$X}>\n"));
    }
    out.push_str("endcodespacerange\n");
    for chunk in entries.chunks(100) {
        out.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for (code, text) in chunk {
            out.push_str(&format!("<{code}> <{text}>\n"));
        }
        out.push_str("endbfchar\n");
    }
    out.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    out
}

fn utf16_hex(text: &str) -> String {
    text.encode_utf16()
        .map(|unit| format!("{unit:04X}"))
        .collect()
}

pub(crate) fn flate_stream(mut dict: PdfDictionary, data: &[u8]) -> PdfObject {
    use std::io::Write;
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = encoder.write_all(data);
    let raw = encoder.finish().unwrap_or_default();
    dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
    PdfObject::Stream { dict, raw }
}

fn number(value: f64) -> PdfObject {
    if (value - value.round()).abs() < 1e-6 {
        PdfObject::Integer(value.round() as i64)
    } else {
        PdfObject::Real((value * 1000.0).round() / 1000.0)
    }
}

pub(crate) fn dict_of(entries: &[(&str, PdfObject)]) -> PdfDictionary {
    let mut out = PdfDictionary::empty();
    for (key, value) in entries {
        out.insert(*key, value.clone());
    }
    out
}

fn subset_tag(font_id: ObjectId, base: &str) -> String {
    let mut seed = format!("{}:{}:{}", font_id.0, font_id.1, base).into_bytes();
    seed.extend_from_slice(b"wellfriend-pdfa");
    let digest = crate::crypto::md5(&seed);
    digest[..6]
        .iter()
        .map(|b| char::from(b'A' + b % 26))
        .collect()
}

fn sanitize_font_name(base: &str) -> String {
    let raw = base.rsplit('+').next().unwrap_or(base);
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_graphic() && !"()<>[]{}/%#".contains(*c))
        .collect();
    if cleaned.is_empty() {
        "Font".to_string()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_cmap_maps_codes_through_both_subtables() {
        let mut map = BTreeMap::new();
        map.insert(0x41u32, 5u16);
        map.insert(0x42u32, 6u16);
        let cmap = symbol_cmap(&map);
        let mut font = BTreeMap::new();
        font.insert(*b"cmap", cmap);
        // Parse with ttf-parser through a minimal table wrapper.
        let table = ttf_parser::cmap::Table::parse(font.get(b"cmap").unwrap()).unwrap();
        let mut seen = 0;
        for subtable in table.subtables {
            match (subtable.platform_id, subtable.encoding_id) {
                (ttf_parser::PlatformId::Macintosh, 0) => {
                    assert_eq!(subtable.glyph_index(0x41).map(|g| g.0), Some(5));
                    seen += 1;
                }
                (ttf_parser::PlatformId::Windows, 0) => {
                    assert_eq!(subtable.glyph_index(0xF042).map(|g| g.0), Some(6));
                    seen += 1;
                }
                _ => {}
            }
        }
        assert_eq!(seen, 2);
    }

    #[test]
    fn bundled_substitutes_are_metric_families() {
        assert_eq!(
            bundled_substitute("Helvetica-Bold", 0).0,
            "LiberationSans-Bold"
        );
        assert_eq!(
            bundled_substitute("Times-Italic", 0).0,
            "LiberationSerif-Italic"
        );
        assert_eq!(bundled_substitute("Courier", 0).0, "LiberationMono-Regular");
        assert_eq!(bundled_substitute("ABCDEF+Symbol", 0).0, "DejaVuSans");
        assert_eq!(
            bundled_substitute("Unknown", 2).0,
            "LiberationSerif-Regular"
        );
    }

    #[test]
    fn subset_rewrites_advances_to_pdf_widths() {
        let candidate = Candidate::bundled(
            "LiberationSans-Regular",
            crate::render::font_rasterizer::fallback_fonts::LIBERATION_SANS_REGULAR,
        );
        let requests = vec![
            GlyphRequest {
                key: 0x41,
                text: "A".into(),
                glyph_name: Some("A".into()),
                width: 700.0,
            },
            // Same glyph, different width: forces a duplicate glyph.
            GlyphRequest {
                key: 0x42,
                text: "A".into(),
                glyph_name: Some("A".into()),
                width: 500.0,
            },
        ];
        let built = build_font_program(&candidate, &requests, true).unwrap();
        assert_eq!(built.duplicates, 1);
        let face = ttf_parser::Face::parse(&built.program, 0).unwrap();
        let upem = f64::from(face.units_per_em());
        for (code, expected) in [(0x41u32, 700.0), (0x42, 500.0)] {
            let gid = built.gid_by_key[&code];
            let adv = f64::from(face.glyph_hor_advance(ttf_parser::GlyphId(gid)).unwrap());
            assert!((adv * 1000.0 / upem - expected).abs() <= 1.0);
        }
        assert_ne!(built.gid_by_key[&0x41], built.gid_by_key[&0x42]);
    }
}
