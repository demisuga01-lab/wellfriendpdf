//! Font analysis reporting (`pdffonts`-equivalent).
//!
//! Walks every resource scope a font can hide in — page resources, Form
//! XObject resources, tiling-pattern resources, and Type3 font resources —
//! collects each font's indirect reference, dedupes by object id, and reports
//! the columns `pdffonts` prints: name, type, encoding, embedded, subset,
//! ToUnicode, and object id.
//!
//! This is aggregation + attribute reporting, not new font parsing: it reads
//! fields straight off the already-parsed font dictionaries and reuses the
//! resolver's subtype mapping.

use std::collections::HashSet;

use serde::Serialize;

use crate::document::PdfDocument;
use crate::error::Result;
use crate::fonts::predefined_cmap;
use crate::fonts::provider::is_standard14_name;
use crate::fonts::resolver::predefined_cmap_name;
use crate::fonts::FontResolver;
use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;
use crate::render::font_rasterizer::FontRasterizer;

/// Maximum resource-scope recursion depth (Form XObjects nesting Form
/// XObjects, etc.). Real documents nest only a few levels; this bounds a
/// pathological/cyclic resource graph.
const MAX_SCOPE_DEPTH: usize = 32;

/// One distinct font used in the document.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FontInfo {
    /// `/BaseFont` (e.g. "ABCDEF+Helvetica"), or the Type3 font's name, or
    /// `[none]` when absent.
    pub name: String,
    /// pdffonts-style type label: "Type 1", "TrueType", "Type 0", "Type 3",
    /// "CID TrueType", "CID Type 0", etc.
    pub font_type: String,
    /// Encoding label: "WinAnsi", "MacRoman", "Identity-H", "Custom",
    /// "Builtin", etc.
    pub encoding: String,
    /// Whether a font program is embedded (FontFile/FontFile2/FontFile3).
    pub embedded: bool,
    /// Whether the font is a subset (6-uppercase-letter "+"-prefixed BaseFont).
    pub subset: bool,
    /// Whether the font carries a `/ToUnicode` CMap.
    pub to_unicode: bool,
    /// Raw `/BaseFont` value before `[none]` fallback.
    pub base_font: Option<String>,
    /// Raw PDF `/Subtype` name when present.
    pub subtype: Option<String>,
    /// Whether a FontDescriptor dictionary was present on the font or its
    /// descendant CIDFont.
    pub descriptor_present: bool,
    /// Embedded font program key, if present: `FontFile`, `FontFile2`, or
    /// `FontFile3`.
    pub font_file_kind: Option<String>,
    /// Descendant CIDFont subtype for Type0 fonts.
    pub cid_descendant_type: Option<String>,
    /// Writing mode inferred from CMap/WMode: `horizontal` or `vertical`.
    pub writing_mode: String,
    /// Predefined CMap name, if the font references one.
    pub predefined_cmap: Option<String>,
    /// Whether the referenced predefined CMap is covered by Wellfriend's bounded
    /// built-in CMap metadata.
    pub predefined_cmap_supported: bool,
    /// Detected OpenType color-glyph table tags.
    pub color_font_tables: Vec<String>,
    /// Rendering posture for detected color glyph tables.
    pub color_glyph_status: String,
    /// Color glyph tables rendered natively by the current engine build.
    pub color_glyph_supported_tables: Vec<String>,
    /// Color glyph tables reported as unsupported instead of silently ignored.
    pub color_glyph_unsupported_tables: Vec<String>,
    /// Raster path used by the engine for this font class.
    pub rasterization: String,
    /// Authoring/subsetting posture for this font class.
    pub embedding_policy: String,
    /// Whether rendering requires Standard 14 or bundled/user font fallback.
    pub fallback_required: bool,
    /// Structured font diagnostics for render/extract/write consumers.
    pub diagnostics: Vec<FontDiagnostic>,
    /// Object number of the font dictionary.
    pub object_number: u32,
    /// Generation number of the font dictionary.
    pub generation: u16,
}

/// Structured font diagnostic attached to a [`FontInfo`] entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FontDiagnostic {
    /// Severity: `info`, `warning`, or `error`.
    pub severity: &'static str,
    /// Stable machine-readable diagnostic code.
    pub code: &'static str,
    /// Human-readable explanation.
    pub message: String,
}

/// Enumerate every distinct font used across the document.
pub fn list_fonts(doc: &PdfDocument) -> Result<Vec<FontInfo>> {
    let reader = doc.reader();
    let pages = doc.get_pages()?;

    // Collected font references, in first-seen order, deduped by object id.
    let mut seen: HashSet<(u32, u16)> = HashSet::new();
    let mut font_refs: Vec<(u32, u16)> = Vec::new();
    // Visited resource scopes (by the object number of the resource-carrying
    // object) to stop cycles in the XObject/pattern graph.
    let mut visited_scopes: HashSet<u32> = HashSet::new();

    for page in &pages {
        // page.resources is the resolved /Resources dictionary (inheritance
        // already applied). Walk it and everything it reaches.
        walk_resources(
            &page.resources,
            reader,
            &mut seen,
            &mut font_refs,
            &mut visited_scopes,
            0,
        );

        // Annotation appearance streams are Form XObjects with their own
        // /Resources, reachable only via the page's /Annots array (NOT via the
        // page /Resources). Form fields in particular keep their fonts here, so
        // skipping /Annots silently drops every font used only in widget
        // appearances — a common pdffonts-disagreement bug.
        walk_page_annotations(
            page.object_number,
            page.generation_number,
            reader,
            &mut seen,
            &mut font_refs,
            &mut visited_scopes,
        );
    }

    // Resolve and describe each distinct font.
    let mut fonts = Vec::with_capacity(font_refs.len());
    for (num, gen) in font_refs {
        if let Ok(PdfObject::Dictionary(font_dict)) = reader.get_and_resolve(num, gen) {
            fonts.push(describe_font(&font_dict, reader, num, gen));
        }
    }
    Ok(fonts)
}

/// Walk a `/Resources` dictionary: collect every `/Font` reference, then
/// recurse into Form XObjects' and tiling patterns' own resources.
fn walk_resources(
    resources: &PdfDictionary,
    reader: &PdfReader,
    seen: &mut HashSet<(u32, u16)>,
    font_refs: &mut Vec<(u32, u16)>,
    visited_scopes: &mut HashSet<u32>,
    depth: usize,
) {
    if depth > MAX_SCOPE_DEPTH {
        return;
    }

    // /Font: each entry references a font dictionary. Collect its object id,
    // and (for Type3) recurse into the font's own /Resources used by CharProcs.
    if let Some(font_dict) = resolve_dict(resources.get("Font"), reader) {
        for (_name, value) in font_dict.entries() {
            // Resolve the font dictionary itself (whether the entry is an
            // indirect ref — the common case — or an inline dict).
            let (resolved_font, object_id) = match value {
                PdfObject::Reference { number, generation } => {
                    match reader.get_and_resolve(*number, *generation) {
                        Ok(PdfObject::Dictionary(fd)) => (Some(fd), Some((*number, *generation))),
                        _ => (None, Some((*number, *generation))),
                    }
                }
                PdfObject::Dictionary(fd) => (Some(fd.clone()), None),
                _ => (None, None),
            };

            // Record the font reference (id-keyed listing; inline fonts have no
            // object id to report and are skipped from the listing).
            if let Some((num, gen)) = object_id {
                if seen.insert((num, gen)) {
                    font_refs.push((num, gen));
                }
            }

            // Type3 fonts carry their own /Resources; recurse into them so
            // fonts used only inside Type3 glyph procedures aren't missed.
            if let Some(fd) = resolved_font {
                if fd.get_name("Subtype") == Some("Type3") {
                    let scope_ok = match object_id {
                        Some((num, _)) => visited_scopes.insert(num),
                        None => true,
                    };
                    if scope_ok {
                        if let Some(t3_res) = resolve_dict(fd.get("Resources"), reader) {
                            walk_resources(
                                &t3_res,
                                reader,
                                seen,
                                font_refs,
                                visited_scopes,
                                depth + 1,
                            );
                        }
                    }
                }
            }
        }
    }

    // /XObject: recurse into Form XObjects' /Resources.
    if let Some(xobj_dict) = resolve_dict(resources.get("XObject"), reader) {
        for (_name, value) in xobj_dict.entries() {
            let Some((num, gen)) = value.as_reference() else {
                continue;
            };
            if !visited_scopes.insert(num) {
                continue;
            }
            if let Ok(PdfObject::Stream { dict, .. }) = reader.get_and_resolve(num, gen) {
                if dict.get_name("Subtype") == Some("Form") {
                    if let Some(form_res) = resolve_dict(dict.get("Resources"), reader) {
                        walk_resources(
                            &form_res,
                            reader,
                            seen,
                            font_refs,
                            visited_scopes,
                            depth + 1,
                        );
                    }
                }
            }
        }
    }

    // /Pattern: tiling patterns (PatternType 1) are streams with /Resources.
    if let Some(pat_dict) = resolve_dict(resources.get("Pattern"), reader) {
        for (_name, value) in pat_dict.entries() {
            let Some((num, gen)) = value.as_reference() else {
                continue;
            };
            if !visited_scopes.insert(num) {
                continue;
            }
            if let Ok(obj) = reader.get_and_resolve(num, gen) {
                let pat_resources = match &obj {
                    PdfObject::Stream { dict, .. } => resolve_dict(dict.get("Resources"), reader),
                    PdfObject::Dictionary(dict) => resolve_dict(dict.get("Resources"), reader),
                    _ => None,
                };
                if let Some(pr) = pat_resources {
                    walk_resources(&pr, reader, seen, font_refs, visited_scopes, depth + 1);
                }
            }
        }
    }
}

/// Walk a page's `/Annots`: each annotation's appearance streams (`/AP /N`,
/// `/D`, `/R`) are Form XObjects whose `/Resources` may carry fonts used
/// nowhere else. The `/AP` entry for a state may be a single stream or a
/// sub-dictionary mapping appearance-state names to streams.
fn walk_page_annotations(
    page_num: u32,
    page_gen: u16,
    reader: &PdfReader,
    seen: &mut HashSet<(u32, u16)>,
    font_refs: &mut Vec<(u32, u16)>,
    visited_scopes: &mut HashSet<u32>,
) {
    let Ok(PdfObject::Dictionary(page_dict)) = reader.get_and_resolve(page_num, page_gen) else {
        return;
    };
    let annots = match page_dict.get("Annots") {
        Some(obj) => match reader.resolve(obj.clone()) {
            Ok(PdfObject::Array(items)) => items,
            _ => return,
        },
        None => return,
    };

    for annot in &annots {
        let Ok(PdfObject::Dictionary(annot_dict)) = reader.resolve(annot.clone()) else {
            continue;
        };
        let Some(ap) = annot_dict.get("AP") else {
            continue;
        };
        let Ok(PdfObject::Dictionary(ap_dict)) = reader.resolve(ap.clone()) else {
            continue;
        };
        // Each appearance type (/N normal, /D down, /R rollover).
        for (_state, value) in ap_dict.entries() {
            collect_appearance_resources(value, reader, seen, font_refs, visited_scopes);
        }
    }
}

/// Resolve an appearance entry — either a Form XObject stream directly, or a
/// sub-dictionary of appearance-state → stream — and walk its resources.
fn collect_appearance_resources(
    value: &PdfObject,
    reader: &PdfReader,
    seen: &mut HashSet<(u32, u16)>,
    font_refs: &mut Vec<(u32, u16)>,
    visited_scopes: &mut HashSet<u32>,
) {
    // Track the scope object id for cycle protection when it's an indirect ref.
    let scope_id = value.as_reference().map(|(n, _)| n);
    if let Some(num) = scope_id {
        if !visited_scopes.insert(num) {
            return;
        }
    }
    let Ok(resolved) = reader.resolve(value.clone()) else {
        return;
    };
    match resolved {
        PdfObject::Stream { dict, .. } => {
            if let Some(res) = resolve_dict(dict.get("Resources"), reader) {
                walk_resources(&res, reader, seen, font_refs, visited_scopes, 1);
            }
        }
        PdfObject::Dictionary(state_dict) => {
            // Sub-dictionary: appearance-state name → stream.
            for (_name, stream_ref) in state_dict.entries() {
                if let Ok(PdfObject::Stream { dict, .. }) = reader.resolve(stream_ref.clone()) {
                    if let Some(res) = resolve_dict(dict.get("Resources"), reader) {
                        walk_resources(&res, reader, seen, font_refs, visited_scopes, 1);
                    }
                }
            }
        }
        _ => {}
    }
}

/// Build a [`FontInfo`] from a resolved font dictionary.
fn describe_font(font_dict: &PdfDictionary, reader: &PdfReader, num: u32, gen: u16) -> FontInfo {
    let base_font = font_dict.get_name("BaseFont").unwrap_or("");
    let name = if base_font.is_empty() {
        font_dict
            .get_name("Name")
            .map(str::to_string)
            .unwrap_or_else(|| "[none]".to_string())
    } else {
        base_font.to_string()
    };

    let subtype = font_dict.get_name("Subtype").unwrap_or("");
    let descendant = descendant_font(font_dict, reader);
    let font_type = font_type_label(subtype, descendant.as_ref());

    let encoding = encoding_label(font_dict, reader, subtype);

    // Embedded: a font program present in the relevant FontDescriptor.
    let embedded = is_embedded(font_dict, descendant.as_ref(), reader);

    // Subset: BaseFont starts with "XXXXXX+" (6 uppercase letters + '+').
    let subset = is_subset_name(base_font);

    let to_unicode = font_dict.contains_key("ToUnicode");
    let descriptor_present = primary_descriptor(font_dict, descendant.as_ref(), reader).is_some();
    let font_file_kind = font_file_kind(font_dict, descendant.as_ref(), reader).map(str::to_string);
    let cid_descendant_type = descendant
        .as_ref()
        .and_then(|dict| dict.get_name("Subtype"))
        .map(str::to_string);
    let writing_mode =
        if subtype == "Type0" && FontResolver::new(font_dict, reader).is_vertical() {
            "vertical"
        } else {
            "horizontal"
        }
        .to_string();
    let predefined_cmap = predefined_cmap_name(font_dict, Some(reader));
    let predefined_cmap_supported = predefined_cmap
        .as_deref()
        .is_some_and(predefined_cmap::is_supported_name);
    let font_bytes = font_program_bytes(font_dict, descendant.as_ref(), reader);
    let color_table_summary = font_bytes
        .as_deref()
        .map(crate::render::color_glyph::color_font_table_summary)
        .unwrap_or_default();
    let color_font_tables = font_bytes
        .as_deref()
        .map(detect_color_font_tables)
        .unwrap_or_default();
    let color_glyph_supported_tables =
        supported_color_glyph_tables(&color_font_tables, &color_table_summary);
    let color_glyph_unsupported_tables =
        unsupported_color_glyph_tables(&color_font_tables, &color_table_summary);
    let color_glyph_status = color_glyph_status(
        &color_font_tables,
        &color_glyph_supported_tables,
        &color_glyph_unsupported_tables,
    )
    .to_string();
    let rasterization = rasterization_label(subtype, &font_type);
    let embedding_policy = embedding_policy_label(embedded, &font_file_kind, subset);
    let fallback_required = !embedded;
    let diagnostics = font_diagnostics(FontDiagnosticInput {
        name: &name,
        subtype,
        encoding: &encoding,
        embedded,
        to_unicode,
        descriptor_present,
        descendant_present: descendant.is_some(),
        writing_mode: &writing_mode,
        predefined_cmap: predefined_cmap.as_deref(),
        predefined_cmap_supported,
        color_font_tables: &color_font_tables,
        color_glyph_supported_tables: &color_glyph_supported_tables,
        color_glyph_unsupported_tables: &color_glyph_unsupported_tables,
        embedding_policy: &embedding_policy,
    });

    FontInfo {
        name,
        font_type,
        encoding,
        embedded,
        subset,
        to_unicode,
        base_font: (!base_font.is_empty()).then(|| base_font.to_string()),
        subtype: (!subtype.is_empty()).then(|| subtype.to_string()),
        descriptor_present,
        font_file_kind,
        cid_descendant_type,
        writing_mode,
        predefined_cmap,
        predefined_cmap_supported,
        color_font_tables,
        color_glyph_status,
        color_glyph_supported_tables,
        color_glyph_unsupported_tables,
        rasterization,
        embedding_policy,
        fallback_required,
        diagnostics,
        object_number: num,
        generation: gen,
    }
}

/// Map a PDF font subtype (+ descendant CIDFont subtype for Type0) to a
/// pdffonts-style type label.
fn font_type_label(subtype: &str, descendant: Option<&PdfDictionary>) -> String {
    match subtype {
        "Type1" => "Type 1".to_string(),
        "MMType1" => "Type 1 (Multiple Master)".to_string(),
        "TrueType" => "TrueType".to_string(),
        "Type3" => "Type 3".to_string(),
        "Type0" => match descendant.and_then(|d| d.get_name("Subtype")) {
            Some("CIDFontType0") => "CID Type 0".to_string(),
            Some("CIDFontType2") => "CID TrueType".to_string(),
            _ => "Type 0".to_string(),
        },
        "CIDFontType0" => "CID Type 0".to_string(),
        "CIDFontType2" => "CID TrueType".to_string(),
        "" => "[unknown]".to_string(),
        other => other.to_string(),
    }
}

/// Normalize an `/Encoding` name to the short label `pdffonts` prints
/// ("WinAnsi", "MacRoman", "Standard", "PDFDoc", or the name verbatim for CMap
/// names like "Identity-H").
fn normalize_encoding_name(name: &str) -> String {
    match name {
        "WinAnsiEncoding" => "WinAnsi".to_string(),
        "MacRomanEncoding" => "MacRoman".to_string(),
        "StandardEncoding" => "Standard".to_string(),
        "PDFDocEncoding" => "PDFDoc".to_string(),
        "MacExpertEncoding" => "MacExpert".to_string(),
        other => other.to_string(),
    }
}

/// Encoding label, matched to `pdffonts`' output.
///
/// - A name `/Encoding` is normalized (WinAnsiEncoding → "WinAnsi", etc.);
///   CMap names (Identity-H, …) pass through verbatim.
/// - An encoding dictionary with `/Differences` is "Custom"; otherwise its
///   normalized `/BaseEncoding`.
/// - An embedded CMap stream (Type0) is "Custom".
/// - When `/Encoding` is **absent**, Poppler reports the *implicit* encoding a
///   simple non-symbolic font would use rather than "Builtin": WinAnsi/Standard.
///   We mirror that for simple fonts (using the FontDescriptor symbolic flag),
///   and report "Identity" / "Builtin" for composite fonts that genuinely have
///   no encoding name.
fn encoding_label(font_dict: &PdfDictionary, reader: &PdfReader, subtype: &str) -> String {
    let Some(encoding) = font_dict.get("Encoding") else {
        return implicit_encoding_label(font_dict, reader, subtype);
    };
    let resolved = reader
        .resolve(encoding.clone())
        .unwrap_or_else(|_| encoding.clone());
    match resolved {
        PdfObject::Name(name) => normalize_encoding_name(&name),
        PdfObject::Dictionary(dict) => {
            // Simple-font encoding dict: /Differences ⇒ Custom; else the base.
            if dict.contains_key("Differences") {
                "Custom".to_string()
            } else {
                match dict.get_name("BaseEncoding") {
                    Some(base) => normalize_encoding_name(base),
                    None => implicit_encoding_label(font_dict, reader, subtype),
                }
            }
        }
        // Embedded CMap (Type0) ⇒ a custom mapping.
        PdfObject::Stream { .. } => "Custom".to_string(),
        _ => implicit_encoding_label(font_dict, reader, subtype),
    }
}

/// The implicit encoding label for a font with no explicit `/Encoding`,
/// following pdffonts' behaviour: composite fonts report "Identity"; simple
/// fonts report their standard encoding unless flagged symbolic (then
/// "Builtin"). Matches Poppler closely so the cross-check agrees.
fn implicit_encoding_label(font_dict: &PdfDictionary, reader: &PdfReader, subtype: &str) -> String {
    if subtype == "Type0" {
        return "Identity".to_string();
    }
    // Simple font. pdffonts treats embedded TrueType subset fonts as using a
    // standard (WinAnsi) encoding even when the symbolic flag is set, so we key
    // on subtype: TrueType ⇒ WinAnsi, Type1/others ⇒ Standard. A genuinely
    // symbolic Type1 with a built-in encoding and no FontFile is "Builtin".
    let symbolic = font_descriptor_symbolic(font_dict, reader);
    match subtype {
        "TrueType" => "WinAnsi".to_string(),
        "Type1" | "MMType1" => {
            if symbolic {
                "Builtin".to_string()
            } else {
                "Standard".to_string()
            }
        }
        _ => "Builtin".to_string(),
    }
}

/// True if the font's FontDescriptor `/Flags` has the Symbolic bit (bit 3, i.e.
/// value 4) set. Returns false when there is no descriptor.
fn font_descriptor_symbolic(font_dict: &PdfDictionary, reader: &PdfReader) -> bool {
    let Some(descriptor) = resolve_dict(font_dict.get("FontDescriptor"), reader) else {
        return false;
    };
    let flags = descriptor.get_integer("Flags").unwrap_or(0);
    (flags & 0b100) != 0
}

/// Whether the font has an embedded program. For simple fonts this is a
/// FontFile/FontFile2/FontFile3 in the font's own /FontDescriptor; for Type0
/// it is in the descendant CIDFont's /FontDescriptor.
fn is_embedded(
    font_dict: &PdfDictionary,
    descendant: Option<&PdfDictionary>,
    reader: &PdfReader,
) -> bool {
    if descriptor_has_fontfile(font_dict, reader) {
        return true;
    }
    if let Some(desc) = descendant {
        if descriptor_has_fontfile(desc, reader) {
            return true;
        }
    }
    false
}

fn descriptor_has_fontfile(font_dict: &PdfDictionary, reader: &PdfReader) -> bool {
    let Some(descriptor) = font_descriptor(font_dict, reader) else {
        return false;
    };
    descriptor.contains_key("FontFile")
        || descriptor.contains_key("FontFile2")
        || descriptor.contains_key("FontFile3")
}

fn primary_descriptor(
    font_dict: &PdfDictionary,
    descendant: Option<&PdfDictionary>,
    reader: &PdfReader,
) -> Option<PdfDictionary> {
    font_descriptor(font_dict, reader)
        .or_else(|| descendant.and_then(|d| font_descriptor(d, reader)))
}

fn font_descriptor(font_dict: &PdfDictionary, reader: &PdfReader) -> Option<PdfDictionary> {
    resolve_dict(font_dict.get("FontDescriptor"), reader)
}

fn font_file_kind(
    font_dict: &PdfDictionary,
    descendant: Option<&PdfDictionary>,
    reader: &PdfReader,
) -> Option<&'static str> {
    let descriptor = primary_descriptor(font_dict, descendant, reader)?;
    if descriptor.contains_key("FontFile") {
        Some("FontFile")
    } else if descriptor.contains_key("FontFile2") {
        Some("FontFile2")
    } else if descriptor.contains_key("FontFile3") {
        Some("FontFile3")
    } else {
        None
    }
}

fn font_program_bytes(
    font_dict: &PdfDictionary,
    descendant: Option<&PdfDictionary>,
    reader: &PdfReader,
) -> Option<Vec<u8>> {
    FontRasterizer::extract_font_bytes(font_dict, reader)
        .or_else(|| descendant.and_then(|desc| FontRasterizer::extract_font_bytes(desc, reader)))
}

fn detect_color_font_tables(bytes: &[u8]) -> Vec<String> {
    if bytes.len() < 12 {
        return Vec::new();
    }
    let num_tables = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
    let mut tags = Vec::new();
    for idx in 0..num_tables {
        let offset = 12usize.saturating_add(idx.saturating_mul(16));
        let Some(tag) = bytes.get(offset..offset + 4) else {
            break;
        };
        if matches!(tag, b"COLR" | b"CPAL" | b"CBDT" | b"CBLC" | b"sbix") || tag == b"SVG " {
            tags.push(String::from_utf8_lossy(tag).trim().to_string());
        }
    }
    tags.sort();
    tags.dedup();
    tags
}

fn color_glyph_status(
    tables: &[String],
    supported: &[String],
    unsupported: &[String],
) -> &'static str {
    if tables.is_empty() {
        "no_color_glyph_tables"
    } else if !supported.is_empty() && unsupported.is_empty() {
        "supported_color_glyph_rendering"
    } else if !supported.is_empty() {
        "supported_color_glyph_rendering_with_precise_unsupported_tables"
    } else {
        "unsupported_color_tables_reported"
    }
}

fn supported_color_glyph_tables(
    tables: &[String],
    summary: &crate::render::color_glyph::ColorFontTableSummary,
) -> Vec<String> {
    let mut supported = Vec::new();
    if summary.supports_colr_cpal_v0()
        && tables.iter().any(|table| table == "COLR")
        && tables.iter().any(|table| table == "CPAL")
    {
        supported.push("COLR/CPAL v0 solid layered glyphs".to_string());
    }
    if summary.supports_colr_cpal_v1_subset()
        && tables.iter().any(|table| table == "COLR")
        && tables.iter().any(|table| table == "CPAL")
    {
        supported.push(
            "COLR/CPAL v1 PaintSolid/PaintColrGlyph/transforms/gradients/clips/PDF blend composites"
                .to_string(),
        );
    }
    if summary.has_cbdt && summary.has_cblc {
        supported.push("CBDT/CBLC PNG and bounded bitmap strikes".to_string());
    }
    if summary.has_sbix {
        supported.push("sbix PNG/JPEG strikes".to_string());
    }
    supported
}

fn unsupported_color_glyph_tables(
    tables: &[String],
    summary: &crate::render::color_glyph::ColorFontTableSummary,
) -> Vec<String> {
    let mut unsupported = Vec::new();
    let has_colr = tables.iter().any(|table| table == "COLR");
    let has_cpal = tables.iter().any(|table| table == "CPAL");
    if has_colr && (!has_cpal || summary.colr_version.is_none()) {
        unsupported.push("COLR/CPAL malformed or unsupported palette pairing".to_string());
    }
    if summary.has_cbdt && summary.has_cblc {
        unsupported.push("CBDT/CBLC malformed or oversized bitmap payloads".to_string());
    } else if summary.has_cbdt || summary.has_cblc {
        unsupported.push("CBDT/CBLC incomplete table pair".to_string());
    }
    if summary.has_sbix {
        unsupported.push("sbix TIFF/PDF/mask or unknown graphicType payloads".to_string());
    }
    if summary.has_svg || tables.iter().any(|table| table == "SVG") {
        unsupported.push("SVG-in-OpenType blocked by security policy".to_string());
    }
    unsupported.sort();
    unsupported.dedup();
    unsupported
}

fn rasterization_label(subtype: &str, font_type: &str) -> String {
    match subtype {
        "Type3" => "Type3 CharProc through PDF content interpreter".to_string(),
        "Type0" if font_type.contains("TrueType") => {
            "pure-rust ttf-parser outlines with light grid fitting".to_string()
        }
        "Type0" if font_type.contains("Type 0") => {
            "pure-rust CFF/Type2 outlines where available".to_string()
        }
        "TrueType" => "pure-rust ttf-parser outlines with light grid fitting".to_string(),
        "Type1" | "MMType1" => "pure-rust Type1/CFF outline interpreter".to_string(),
        _ => "pure-rust fallback outline path".to_string(),
    }
}

fn embedding_policy_label(embedded: bool, font_file_kind: &Option<String>, subset: bool) -> String {
    if !embedded {
        return "not embedded in source; deterministic fallback/substitution at render time"
            .to_string();
    }
    let program = font_file_kind.as_deref().unwrap_or("FontFile");
    if subset {
        format!("source subset {program}; preserves embedded subset program")
    } else {
        format!("source full-font {program}; generated output uses Type0 subset maps with full sfnt fallback")
    }
}

struct FontDiagnosticInput<'a> {
    name: &'a str,
    subtype: &'a str,
    encoding: &'a str,
    embedded: bool,
    to_unicode: bool,
    descriptor_present: bool,
    descendant_present: bool,
    writing_mode: &'a str,
    predefined_cmap: Option<&'a str>,
    predefined_cmap_supported: bool,
    color_font_tables: &'a [String],
    color_glyph_supported_tables: &'a [String],
    color_glyph_unsupported_tables: &'a [String],
    embedding_policy: &'a str,
}

fn font_diagnostics(input: FontDiagnosticInput<'_>) -> Vec<FontDiagnostic> {
    let mut diagnostics = Vec::new();
    if input.subtype == "Type0" && !input.descendant_present {
        diagnostics.push(FontDiagnostic {
            severity: "error",
            code: "font.type0.descendant_missing",
            message: "Type0 font has no resolvable descendant CIDFont".to_string(),
        });
    }
    if input.subtype.is_empty() {
        diagnostics.push(FontDiagnostic {
            severity: "warning",
            code: "font.subtype.missing",
            message: "Font dictionary is missing /Subtype".to_string(),
        });
    }
    if !input.descriptor_present && !is_standard14_name(input.name) && input.subtype != "Type3" {
        diagnostics.push(FontDiagnostic {
            severity: "warning",
            code: "font.descriptor.missing",
            message: "Font has no FontDescriptor for metrics/substitution scoring".to_string(),
        });
    }
    if !input.embedded {
        if is_standard14_name(input.name) {
            diagnostics.push(FontDiagnostic {
                severity: "info",
                code: "font.standard14.substitution",
                message: "Standard 14 font will use deterministic bundled metrics/fallback"
                    .to_string(),
            });
        } else {
            diagnostics.push(FontDiagnostic {
                severity: "warning",
                code: "font.substitution.required",
                message: "Font program is not embedded; rendering depends on fallback substitution"
                    .to_string(),
            });
        }
    }
    if !input.to_unicode {
        let code = if input.subtype == "Type0" {
            "font.tounicode.missing_type0"
        } else {
            "font.tounicode.missing"
        };
        diagnostics.push(FontDiagnostic {
            severity: "warning",
            code,
            message: "Font has no ToUnicode CMap; extraction uses encoding/glyph-name fallback"
                .to_string(),
        });
    }
    if input.encoding == "Custom" && !input.to_unicode {
        diagnostics.push(FontDiagnostic {
            severity: "warning",
            code: "font.custom_encoding.no_tounicode",
            message: "Custom encoding without ToUnicode may not round-trip text accurately"
                .to_string(),
        });
    }
    if input.subtype == "Type3" {
        diagnostics.push(FontDiagnostic {
            severity: "info",
            code: "font.type3.charprocs",
            message: "Type3 glyphs render through PDF CharProc content streams".to_string(),
        });
    }
    if input.writing_mode == "vertical" {
        diagnostics.push(FontDiagnostic {
            severity: "info",
            code: "font.vertical.detected",
            message: "Vertical writing mode detected; full vertical layout fidelity is bounded"
                .to_string(),
        });
    }
    if let Some(name) = input.predefined_cmap {
        if input.predefined_cmap_supported {
            diagnostics.push(FontDiagnostic {
                severity: "info",
                code: "font.cmap.predefined.used",
                message: format!("Predefined CMap {name} is covered by bounded built-in metadata"),
            });
        } else if predefined_cmap::looks_like_predefined_name(name) {
            diagnostics.push(FontDiagnostic {
                severity: "warning",
                code: "font.cmap.predefined.unsupported",
                message: format!(
                    "Predefined CMap {name} is not bundled; extraction/rendering use fallback mapping"
                ),
            });
        }
    }
    if input
        .predefined_cmap
        .is_some_and(|name| name.ends_with("-V") || name == "Identity-V")
    {
        diagnostics.push(FontDiagnostic {
            severity: "info",
            code: "font.cmap.vertical",
            message: "Predefined vertical CMap name was detected".to_string(),
        });
    }
    if input
        .predefined_cmap
        .is_some_and(|name| name == "Identity-H" || name == "Identity-V")
    {
        diagnostics.push(FontDiagnostic {
            severity: "info",
            code: "font.cmap.identity",
            message: "Identity CMap is used for CID-to-character code mapping".to_string(),
        });
    }
    if !input.color_font_tables.is_empty() {
        diagnostics.push(FontDiagnostic {
            severity: if input.color_glyph_supported_tables.is_empty() {
                "warning"
            } else {
                "info"
            },
            code: "font.color_glyphs.detected",
            message: format!(
                "Color glyph tables detected ({}); supported formats are rendered and unsupported formats remain explicitly reported",
                input.color_font_tables.join(", ")
            ),
        });
        if input
            .color_glyph_supported_tables
            .iter()
            .any(|table| table.contains("COLR/CPAL v0"))
        {
            diagnostics.push(FontDiagnostic {
                severity: "info",
                code: "font.color_glyphs.colr_cpal_v0.supported",
                message: "COLR/CPAL v0 solid layered vector glyphs render through the pure-rust outline painter".to_string(),
            });
        }
        if input
            .color_glyph_supported_tables
            .iter()
            .any(|table| table.contains("COLR/CPAL v1"))
        {
            diagnostics.push(FontDiagnostic {
                severity: "info",
                code: "font.color_glyphs.colr_cpal_v1_subset.supported",
                message: "COLR/CPAL v1 PaintSolid/PaintColrGlyph transform and SourceOver graphs render through the bounded pure-rust outline painter".to_string(),
            });
        }
        if input
            .color_glyph_supported_tables
            .iter()
            .any(|table| table.contains("CBDT/CBLC"))
        {
            diagnostics.push(FontDiagnostic {
                severity: "info",
                code: "font.color_glyphs.cbdt_cblc.supported",
                message: "CBDT/CBLC PNG and bounded bitmap strikes render through safe image decode paths".to_string(),
            });
        }
        if input
            .color_glyph_supported_tables
            .iter()
            .any(|table| table.contains("sbix PNG"))
        {
            diagnostics.push(FontDiagnostic {
                severity: "info",
                code: "font.color_glyphs.sbix_png.supported",
                message: "sbix PNG strikes render through safe image decode paths".to_string(),
            });
        }
        if input
            .color_glyph_unsupported_tables
            .iter()
            .any(|table| table.contains("COLR/CPAL"))
        {
            diagnostics.push(FontDiagnostic {
                severity: "warning",
                code: "font.color_glyphs.colr_cpal.unsupported_exotic",
                message: "COLR/CPAL v1 gradients, clip operators, and non-SourceOver composites are reported at operator level instead of silently flattened"
                    .to_string(),
            });
        }
        if input
            .color_glyph_unsupported_tables
            .iter()
            .any(|table| table.contains("CBDT/CBLC"))
        {
            diagnostics.push(FontDiagnostic {
                severity: "warning",
                code: "font.color_glyphs.cbdt_cblc.unsupported_payload",
                message: "Malformed, incomplete, or oversized CBDT/CBLC bitmap payloads fail closed with diagnostics"
                    .to_string(),
            });
        }
        if input
            .color_glyph_unsupported_tables
            .iter()
            .any(|table| table.contains("sbix"))
        {
            diagnostics.push(FontDiagnostic {
                severity: "warning",
                code: "font.color_glyphs.sbix.unsupported_payload",
                message: "sbix PNG/JPEG strikes are supported; TIFF, PDF, mask, unknown, malformed, or oversized payloads fail closed"
                    .to_string(),
            });
        }
        if input
            .color_glyph_unsupported_tables
            .iter()
            .any(|table| table.contains("SVG"))
        {
            diagnostics.push(FontDiagnostic {
                severity: "warning",
                code: "font.color_glyphs.svg_unsupported_security",
                message: "SVG-in-OpenType glyph documents are classified by the static-subset policy; active, external, or dynamic SVG remains blocked and is never executed or dereferenced"
                    .to_string(),
            });
        }
    }
    if input.embedding_policy.contains("full-font") {
        diagnostics.push(FontDiagnostic {
            severity: "info",
            code: "font.subset.sfnt_deferred",
            message: "Authoring uses Type0 subset maps with full sfnt embedding unless a future safe subset writer is selected".to_string(),
        });
    }
    diagnostics
}

/// Resolve a Type0 font's first descendant CIDFont dictionary.
fn descendant_font(font_dict: &PdfDictionary, reader: &PdfReader) -> Option<PdfDictionary> {
    if font_dict.get_name("Subtype") != Some("Type0") {
        return None;
    }
    let descendants = match font_dict.get("DescendantFonts")? {
        PdfObject::Array(items) => items.clone(),
        obj @ PdfObject::Reference { .. } => match reader.resolve(obj.clone()).ok()? {
            PdfObject::Array(items) => items,
            _ => return None,
        },
        _ => return None,
    };
    match descendants.first()? {
        PdfObject::Dictionary(dict) => Some(dict.clone()),
        obj @ PdfObject::Reference { .. } => match reader.resolve(obj.clone()).ok()? {
            PdfObject::Dictionary(dict) => Some(dict),
            _ => None,
        },
        _ => None,
    }
}

/// True if `name` carries the 6-uppercase-letter subset prefix `XXXXXX+`.
pub fn is_subset_name(name: &str) -> bool {
    let Some(plus) = name.find('+') else {
        return false;
    };
    let prefix = &name[..plus];
    prefix.len() == 6 && prefix.bytes().all(|b| b.is_ascii_uppercase())
}

/// Resolve an optional object to a dictionary (following one indirect ref).
fn resolve_dict(obj: Option<&PdfObject>, reader: &PdfReader) -> Option<PdfDictionary> {
    match obj? {
        PdfObject::Dictionary(dict) => Some(dict.clone()),
        r @ PdfObject::Reference { .. } => match reader.resolve(r.clone()).ok()? {
            PdfObject::Dictionary(dict) => Some(dict),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subset_prefix_detection() {
        assert!(is_subset_name("ABCDEF+Helvetica"));
        assert!(is_subset_name("WXYZAB+Arial-Bold"));
        assert!(!is_subset_name("Helvetica"));
        assert!(!is_subset_name("ABC+Helvetica")); // too short
        assert!(!is_subset_name("abcdef+Helvetica")); // lowercase
        assert!(!is_subset_name("ABCDE1+Helvetica")); // digit
    }

    #[test]
    fn type_label_mapping() {
        assert_eq!(font_type_label("Type1", None), "Type 1");
        assert_eq!(font_type_label("TrueType", None), "TrueType");
        assert_eq!(font_type_label("Type3", None), "Type 3");
        assert_eq!(font_type_label("Type0", None), "Type 0");

        let mut cid0 = PdfDictionary::empty();
        cid0.insert("Subtype", PdfObject::Name("CIDFontType0".to_string()));
        assert_eq!(font_type_label("Type0", Some(&cid0)), "CID Type 0");

        let mut cid2 = PdfDictionary::empty();
        cid2.insert("Subtype", PdfObject::Name("CIDFontType2".to_string()));
        assert_eq!(font_type_label("Type0", Some(&cid2)), "CID TrueType");
    }

    #[test]
    fn diagnostics_report_standard14_substitution_without_error() {
        let diagnostics = font_diagnostics(FontDiagnosticInput {
            name: "Helvetica",
            subtype: "Type1",
            encoding: "WinAnsi",
            embedded: false,
            to_unicode: false,
            descriptor_present: false,
            descendant_present: false,
            writing_mode: "horizontal",
            predefined_cmap: None,
            predefined_cmap_supported: false,
            color_font_tables: &[],
            color_glyph_supported_tables: &[],
            color_glyph_unsupported_tables: &[],
            embedding_policy: "not embedded in source",
        });

        assert!(diagnostics
            .iter()
            .any(|diag| diag.code == "font.standard14.substitution"));
        assert!(diagnostics
            .iter()
            .any(|diag| diag.code == "font.tounicode.missing"));
        assert!(!diagnostics
            .iter()
            .any(|diag| diag.code == "font.descriptor.missing"));
    }

    #[test]
    fn diagnostics_report_type0_missing_descendant_and_vertical_mode() {
        let diagnostics = font_diagnostics(FontDiagnosticInput {
            name: "ABCDEE+NotoSansCJK",
            subtype: "Type0",
            encoding: "Identity-V",
            embedded: true,
            to_unicode: true,
            descriptor_present: true,
            descendant_present: false,
            writing_mode: "vertical",
            predefined_cmap: Some("Identity-V"),
            predefined_cmap_supported: true,
            color_font_tables: &[],
            color_glyph_supported_tables: &[],
            color_glyph_unsupported_tables: &[],
            embedding_policy: "source subset FontFile2",
        });

        assert!(diagnostics
            .iter()
            .any(|diag| diag.code == "font.type0.descendant_missing"));
        assert!(diagnostics
            .iter()
            .any(|diag| diag.code == "font.vertical.detected"));
        assert!(diagnostics
            .iter()
            .any(|diag| diag.code == "font.cmap.identity"));
    }

    #[test]
    fn diagnostics_report_predefined_cmap_support_and_color_tables() {
        let color_tables = vec!["COLR".to_string(), "CPAL".to_string()];
        let summary = crate::render::color_glyph::ColorFontTableSummary {
            colr_version: Some(0),
            has_cpal: true,
            ..Default::default()
        };
        let supported_tables = supported_color_glyph_tables(&color_tables, &summary);
        let unsupported_tables = unsupported_color_glyph_tables(&color_tables, &summary);
        let diagnostics = font_diagnostics(FontDiagnosticInput {
            name: "ABCDEE+ColorCjk",
            subtype: "Type0",
            encoding: "UniJIS-UTF16-H",
            embedded: true,
            to_unicode: false,
            descriptor_present: true,
            descendant_present: true,
            writing_mode: "horizontal",
            predefined_cmap: Some("UniJIS-UTF16-H"),
            predefined_cmap_supported: true,
            color_font_tables: &color_tables,
            color_glyph_supported_tables: &supported_tables,
            color_glyph_unsupported_tables: &unsupported_tables,
            embedding_policy: "source full-font FontFile2",
        });

        assert!(diagnostics
            .iter()
            .any(|diag| diag.code == "font.cmap.predefined.used"));
        assert!(diagnostics
            .iter()
            .any(|diag| diag.code == "font.color_glyphs.detected"));
        assert!(diagnostics
            .iter()
            .any(|diag| diag.code == "font.color_glyphs.colr_cpal_v0.supported"));
        assert!(diagnostics
            .iter()
            .any(|diag| diag.code == "font.subset.sfnt_deferred"));
    }

    #[test]
    fn color_glyph_posture_reports_supported_and_precise_unsupported_tables() {
        let color_tables = vec![
            "CBDT".to_string(),
            "CBLC".to_string(),
            "SVG".to_string(),
            "sbix".to_string(),
        ];
        let summary = crate::render::color_glyph::ColorFontTableSummary {
            has_cbdt: true,
            has_cblc: true,
            has_sbix: true,
            has_svg: true,
            ..Default::default()
        };
        let supported_tables = supported_color_glyph_tables(&color_tables, &summary);
        let unsupported_tables = unsupported_color_glyph_tables(&color_tables, &summary);

        assert_eq!(
            color_glyph_status(&color_tables, &supported_tables, &unsupported_tables),
            "supported_color_glyph_rendering_with_precise_unsupported_tables"
        );
        assert!(supported_tables
            .iter()
            .any(|table| table == "CBDT/CBLC PNG and bounded bitmap strikes"));
        assert!(supported_tables
            .iter()
            .any(|table| table == "sbix PNG/JPEG strikes"));
        assert!(unsupported_tables
            .iter()
            .any(|table| table == "SVG-in-OpenType blocked by security policy"));
        assert!(unsupported_tables
            .iter()
            .any(|table| table == "sbix TIFF/PDF/mask or unknown graphicType payloads"));
    }

    #[test]
    fn color_font_table_detection_reads_sfnt_directory() {
        let mut bytes = vec![0u8; 12 + 3 * 16];
        bytes[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        bytes[4..6].copy_from_slice(&3u16.to_be_bytes());
        bytes[12..16].copy_from_slice(b"COLR");
        bytes[28..32].copy_from_slice(b"CPAL");
        bytes[44..48].copy_from_slice(b"name");

        assert_eq!(
            detect_color_font_tables(&bytes),
            vec!["COLR".to_string(), "CPAL".to_string()]
        );
    }
}
