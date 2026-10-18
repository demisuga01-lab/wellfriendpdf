//! Native PDF/A conversion (PDF/A-1b, 2b, 2u, 3b, 3u; 2a/3a best effort).
//!
//! The converter rewrites the whole document and repairs, per level:
//!
//! * fonts — every non-embedded font is replaced by an embedded, subset,
//!   metric-locked substitute with a ToUnicode CMap ([`crate::pdfa_fonts`]);
//! * colour — an sRGB OutputIntent is added when none is present, and
//!   `DefaultRGB` / `DefaultCMYK` ICC spaces are injected when device colour
//!   does not match the OutputIntent;
//! * metadata — an XMP packet with `pdfaid`, Dublin Core, XMP basic and PDF
//!   schema properties synchronised with a rebuilt Info dictionary;
//! * forbidden features — JavaScript, Launch and other forbidden actions,
//!   additional actions, embedded files (PDF/A-1/2), encryption, LZW,
//!   external stream data, PostScript/reference XObjects, transfer functions,
//!   forbidden annotation types and flags, missing appearance streams,
//!   NeedAppearances, XFA and non-conforming optional content configurations.
//!   PDF/A-1 transparency is refused with `PDFA1_TRANSPARENCY` rather than
//!   flattened.
//!
//! The result is gated by Wellfriend's own clause-mapped PDF/A validator. A
//! file that does not validate is never returned: the outcome carries
//! `PDFA_CONVERSION_FAILED` and the failing rule list instead.

use std::collections::{BTreeSet, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::compliance::{writer_for_profile, PdfAProfile};
use crate::document::PdfDocument;
use crate::error::Result;
use crate::fonts::resolver::FontResolver;
use crate::fonts_report::list_fonts;
use crate::info::{decode_pdf_text_string, DocumentInfo};
use crate::object::{PdfDictionary, PdfObject};
use crate::pdfa_fonts::{
    dict_of, embed_font, flate_stream, font_is_embedded, scan_content_usage, ContentUsage,
    FontEmbedOptions, PdfAFontEmbedding, PLACEHOLDER_BASE,
};
use crate::pdfa_rules::{
    action_allowed, annotation_allowed, filter_names, ANNOT_HIDDEN, ANNOT_INVISIBLE, ANNOT_NOVIEW,
    ANNOT_PRINT, ANNOT_TOGGLE_NOVIEW,
};
use crate::standards_engine::{validate_pdfa_profile, RuleStatus, StandardsValidationOptions};
use crate::writer::{rewrite_document_objects, OutputObject, PdfWriter};
use crate::ContentEngine;

/// Stable failure code for a conversion the validator did not accept.
pub const PDFA_CONVERSION_FAILED: &str = "PDFA_CONVERSION_FAILED";
/// Stable failure code for PDF/A-1 sources with live transparency.
pub const PDFA1_TRANSPARENCY: &str = "PDFA1_TRANSPARENCY";

const PRODUCER: &str = "Wellfriend PDF SDK";

/// Options for [`convert_pdf_to_pdfa`]. JSON form: `{"profile":"PDF/A-2b"}`
/// or the short alias `{"level":"2b"}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PdfAConvertOptions {
    /// `PDF/A-1b`, `PDF/A-2b`, `PDF/A-2u`, `PDF/A-3b`, `PDF/A-3u` (also
    /// `PDF/A-2a`, `PDF/A-3a`). Takes precedence over `level`.
    pub profile: Option<String>,
    /// Short alias: `1b`, `2b` (default), `2u`, `3b`, `3u`, `2a`, `3a`.
    pub level: String,
    /// Allow embeddable installed system fonts as substitutes (exact name
    /// match first, Unicode coverage last). Bundled Liberation/DejaVu fonts
    /// are always available. Disable for byte-reproducible output.
    pub system_fonts: bool,
}

impl Default for PdfAConvertOptions {
    fn default() -> Self {
        Self {
            profile: None,
            level: "2b".to_string(),
            system_fonts: true,
        }
    }
}

impl PdfAConvertOptions {
    /// Options for one level with bundled fonts only (reproducible output).
    pub fn for_profile(profile: PdfAProfile) -> Self {
        Self {
            profile: None,
            level: profile.code().to_string(),
            system_fonts: false,
        }
    }

    pub fn profile(&self) -> Result<PdfAProfile> {
        let requested = self.profile.as_deref().unwrap_or(&self.level);
        PdfAProfile::parse(requested).ok_or_else(|| {
            crate::error::WellfriendError::invalid_input(format!(
                "invalid_options: unknown PDF/A profile '{requested}'; use PDF/A-1b, PDF/A-2b, PDF/A-2u, PDF/A-3b or PDF/A-3u"
            ))
        })
    }
}

/// One validator rule that blocked the conversion.
#[derive(Debug, Clone, Serialize)]
pub struct PdfAFailedRule {
    pub rule: String,
    pub clause: String,
    pub location: String,
    pub message: String,
}

/// Repairs applied during conversion.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PdfAFixSummary {
    pub output_intent: String,
    pub output_intent_added: bool,
    pub default_color_spaces: Vec<String>,
    pub fonts_embedded: usize,
    pub to_unicode_added: usize,
    pub actions_removed: usize,
    pub javascript_removed: usize,
    pub embedded_files_removed: usize,
    pub embedded_files_associated: usize,
    pub annotations_removed: usize,
    pub annotation_flags_fixed: usize,
    pub appearances_generated: usize,
    pub appearance_dictionaries_trimmed: usize,
    pub lzw_streams_reencoded: usize,
    pub external_references_removed: usize,
    pub transfer_functions_removed: usize,
    pub interpolation_disabled: usize,
    pub transparency_groups_removed: usize,
    pub optional_content_fixed: usize,
    pub form_fixes: usize,
    pub decrypted: bool,
}

/// JSON result contract of `pdfa-convert`.
#[derive(Debug, Clone, Serialize)]
pub struct PdfAConvertReport {
    pub ok: bool,
    /// Profile label, e.g. `PDF/A-2b`.
    pub profile: String,
    pub level: String,
    pub validated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub failed_rules: Vec<PdfAFailedRule>,
    pub validator_rules: usize,
    pub fonts: Vec<PdfAFontEmbedding>,
    pub fixes: PdfAFixSummary,
    pub warnings: Vec<String>,
    pub output_bytes: usize,
}

/// Conversion outcome: `bytes` is `Some` only when the output validated.
#[derive(Debug, Clone)]
pub struct PdfAConversionOutcome {
    pub bytes: Option<Vec<u8>>,
    pub report: PdfAConvertReport,
}

/// Convert `doc` to PDF/A and gate the result with Wellfriend's validator.
pub fn convert_pdf_to_pdfa(
    doc: &PdfDocument,
    options: &PdfAConvertOptions,
) -> Result<PdfAConversionOutcome> {
    let profile = options.profile()?;
    let mut report = PdfAConvertReport {
        ok: false,
        profile: profile.label().to_string(),
        level: profile.code().to_string(),
        validated: false,
        code: None,
        message: None,
        failed_rules: Vec::new(),
        validator_rules: 0,
        fonts: Vec::new(),
        fixes: PdfAFixSummary::default(),
        warnings: Vec::new(),
        output_bytes: 0,
    };

    if profile.is_pdfa1() {
        let blockers = pdfa1_blockers(doc);
        if !blockers.is_empty() {
            let transparency = blockers.iter().any(|b| b.rule == "pdfa1.transparency");
            report.code = Some(
                if transparency {
                    PDFA1_TRANSPARENCY
                } else {
                    PDFA_CONVERSION_FAILED
                }
                .to_string(),
            );
            report.message = Some(if transparency {
                "PDF/A-1 forbids transparency and this document uses it; convert to PDF/A-2b or later instead".to_string()
            } else {
                "the document uses features PDF/A-1 cannot represent".to_string()
            });
            report.failed_rules = blockers;
            return Ok(PdfAConversionOutcome {
                bytes: None,
                report,
            });
        }
    }

    let bytes = build_pdfa(doc, profile, options, &mut report)?;
    gate(bytes, profile, report)
}

fn gate(
    bytes: Vec<u8>,
    profile: PdfAProfile,
    mut report: PdfAConvertReport,
) -> Result<PdfAConversionOutcome> {
    let engine = match ContentEngine::open_bytes(bytes.clone()) {
        Ok(engine) => engine,
        Err(err) => {
            report.code = Some(PDFA_CONVERSION_FAILED.to_string());
            report.message = Some(format!("converted output did not reopen: {err}"));
            return Ok(PdfAConversionOutcome {
                bytes: None,
                report,
            });
        }
    };
    let validation = validate_pdfa_profile(
        &engine,
        &StandardsValidationOptions::with_target(&profile.label().to_uppercase()),
    )?;
    report.validator_rules = validation.rules.len();
    report.failed_rules = validation
        .rules
        .iter()
        .filter(|rule| rule.status.is_failing() || rule.status.is_inconclusive())
        .filter(|rule| rule.status != RuleStatus::Warning)
        .map(|rule| PdfAFailedRule {
            rule: rule.rule_id.clone(),
            clause: format!("{} {}", rule.clause.standard, rule.clause.clause),
            location: rule.context.object.clone().unwrap_or_default(),
            message: rule.diagnostic.clone(),
        })
        .collect();
    if report.failed_rules.is_empty() {
        report.ok = true;
        report.validated = true;
        report.output_bytes = bytes.len();
        Ok(PdfAConversionOutcome {
            bytes: Some(bytes),
            report,
        })
    } else {
        report.code = Some(PDFA_CONVERSION_FAILED.to_string());
        report.message = Some(format!(
            "converted output failed {} PDF/A rule(s); no file was produced",
            report.failed_rules.len()
        ));
        Ok(PdfAConversionOutcome {
            bytes: None,
            report,
        })
    }
}

// ---------------------------------------------------------------------------
// Pre-flight (PDF/A-1)
// ---------------------------------------------------------------------------

fn pdfa1_blockers(doc: &PdfDocument) -> Vec<PdfAFailedRule> {
    let reader = doc.reader();
    let mut out = Vec::new();
    for (number, generation) in reader.object_ids() {
        let Ok(object) = reader.get_object(number, generation) else {
            continue;
        };
        let location = format!("object {number} {generation}");
        let dict = match &object {
            PdfObject::Dictionary(dict) => dict,
            PdfObject::Stream { dict, .. } => dict,
            _ => continue,
        };
        if let PdfObject::Stream { .. } = &object {
            if filter_names(dict).iter().any(|f| f == "JPXDecode") {
                out.push(failed(
                    "pdfa1.jpx",
                    "ISO 19005-1:2005 6.1",
                    &location,
                    "JPEG 2000 images cannot be stored in PDF/A-1 (PDF 1.4)",
                ));
            }
            if dict.get_name("Subtype") == Some("Image")
                && (dict.get("SMask").is_some_and(|v| !v.is_null())
                    || dict.get_integer("SMaskInData").unwrap_or(0) > 0)
            {
                out.push(failed(
                    "pdfa1.transparency",
                    "ISO 19005-1:2005 6.4",
                    &location,
                    "soft-masked image",
                ));
            }
        }
        if let Some(reason) = live_transparency(dict) {
            out.push(failed(
                "pdfa1.transparency",
                "ISO 19005-1:2005 6.4",
                &location,
                reason,
            ));
        }
        if out.len() > 50 {
            break;
        }
    }
    out
}

fn live_transparency(dict: &PdfDictionary) -> Option<&'static str> {
    for key in ["ca", "CA"] {
        if dict
            .get(key)
            .and_then(PdfObject::as_number)
            .is_some_and(|v| v < 1.0)
        {
            return Some("constant alpha below 1.0");
        }
    }
    let is_gstate = dict.get_name("Type") == Some("ExtGState")
        || dict.contains_key("ca")
        || dict.contains_key("CA")
        || dict.contains_key("BM");
    if is_gstate {
        if let Some(smask) = dict.get("SMask") {
            if smask.as_name() != Some("None") {
                return Some("soft mask in graphics state");
            }
        }
    }
    if let Some(bm) = dict.get("BM") {
        let normal = |obj: &PdfObject| matches!(obj.as_name(), Some("Normal" | "Compatible"));
        let ok = match bm {
            PdfObject::Array(items) => items.iter().all(normal),
            other => normal(other),
        };
        if !ok {
            return Some("non-Normal blend mode");
        }
    }
    None
}

fn failed(rule: &str, clause: &str, location: &str, message: &str) -> PdfAFailedRule {
    PdfAFailedRule {
        rule: rule.to_string(),
        clause: clause.to_string(),
        location: location.to_string(),
        message: message.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Object store over the rewritten document
// ---------------------------------------------------------------------------

struct Store {
    objects: Vec<OutputObject>,
    index: HashMap<u32, usize>,
    next: u32,
}

impl Store {
    fn new(objects: Vec<OutputObject>) -> Self {
        let index = objects
            .iter()
            .enumerate()
            .map(|(idx, obj)| (obj.number, idx))
            .collect();
        let next = objects.iter().map(|o| o.number).max().unwrap_or(0) + 1;
        Self {
            objects,
            index,
            next,
        }
    }

    fn get(&self, number: u32) -> Option<&PdfObject> {
        self.index
            .get(&number)
            .map(|idx| &self.objects[*idx].object)
    }

    fn get_mut(&mut self, number: u32) -> Option<&mut PdfObject> {
        let idx = *self.index.get(&number)?;
        Some(&mut self.objects[idx].object)
    }

    fn reserve(&mut self) -> u32 {
        let number = self.next;
        self.next += 1;
        number
    }

    fn set(&mut self, number: u32, object: PdfObject) {
        if let Some(idx) = self.index.get(&number) {
            self.objects[*idx].object = object;
        } else {
            self.index.insert(number, self.objects.len());
            self.objects.push(OutputObject { number, object });
            self.next = self.next.max(number + 1);
        }
    }

    fn add(&mut self, object: PdfObject) -> u32 {
        let number = self.reserve();
        self.set(number, object);
        number
    }

    /// Resolve references (bounded) to a cloned direct object.
    fn resolve(&self, object: &PdfObject) -> Option<PdfObject> {
        let mut current = object.clone();
        for _ in 0..16 {
            match current {
                PdfObject::Reference { number, .. } => current = self.get(number)?.clone(),
                other => return Some(other),
            }
        }
        None
    }

    fn resolve_dict(&self, object: Option<&PdfObject>) -> Option<PdfDictionary> {
        match self.resolve(object?)? {
            PdfObject::Dictionary(dict) => Some(dict),
            PdfObject::Stream { dict, .. } => Some(dict),
            _ => None,
        }
    }

    fn numbers(&self) -> Vec<u32> {
        self.objects.iter().map(|o| o.number).collect()
    }
}

fn reference(number: u32) -> PdfObject {
    PdfObject::Reference {
        number,
        generation: 0,
    }
}

fn object_dict_mut(object: &mut PdfObject) -> Option<&mut PdfDictionary> {
    match object {
        PdfObject::Dictionary(dict) => Some(dict),
        PdfObject::Stream { dict, .. } => Some(dict),
        _ => None,
    }
}

fn object_dict(object: &PdfObject) -> Option<&PdfDictionary> {
    match object {
        PdfObject::Dictionary(dict) => Some(dict),
        PdfObject::Stream { dict, .. } => Some(dict),
        _ => None,
    }
}

/// Mutate the dictionary stored at `value`: in place when direct, through the
/// store when it is a reference.
fn with_dict_value(
    store: &mut Store,
    value: &mut PdfObject,
    mut f: impl FnMut(&mut PdfDictionary),
) -> bool {
    match value {
        PdfObject::Dictionary(dict) => {
            f(dict);
            true
        }
        PdfObject::Reference { number, .. } => {
            let number = *number;
            match store.get_mut(number).and_then(object_dict_mut) {
                Some(dict) => {
                    f(dict);
                    true
                }
                None => false,
            }
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Conversion
// ---------------------------------------------------------------------------

fn build_pdfa(
    doc: &PdfDocument,
    profile: PdfAProfile,
    options: &PdfAConvertOptions,
    report: &mut PdfAConvertReport,
) -> Result<Vec<u8>> {
    let reader = doc.reader();
    let mut usage = scan_content_usage(doc)?;
    for note in usage.incomplete.iter().take(5) {
        report
            .warnings
            .push(format!("content scan incomplete: {note}"));
    }

    // Fonts: plan replacements against the source numbering.
    let font_options = FontEmbedOptions {
        system_fonts: options.system_fonts,
        write_cid_set: profile.is_pdfa1(),
    };
    let mut replacements = Vec::new();
    let mut unicode_additions: Vec<((u32, u16), PdfObject)> = Vec::new();
    let fonts = list_fonts(doc)?;
    for info in &fonts {
        let id = (info.object_number, info.generation);
        let Ok(PdfObject::Dictionary(dict)) = reader.get_and_resolve(id.0, id.1) else {
            continue;
        };
        if !font_is_embedded(reader, &dict) {
            let (row, replacement) = embed_font(doc, id, &usage, font_options);
            if let Some(replacement) = replacement {
                replacements.push(replacement);
                report.fixes.fonts_embedded += 1;
            }
            report.fonts.push(row);
        } else if profile.requires_unicode() && dict.get("ToUnicode").is_none() {
            if let Some(stream) = generate_to_unicode(reader, id, &dict, &usage) {
                unicode_additions.push((id, stream));
            }
        }
    }

    let mut old_numbers = Vec::new();
    let (objects, root, info_number) = rewrite_document_objects(reader, &mut |old, _| {
        old_numbers.push(old);
    })?;
    let old_to_new: HashMap<u32, u32> = old_numbers
        .iter()
        .copied()
        .zip(objects.iter().map(|o| o.number))
        .collect();
    let mut store = Store::new(objects);

    // Encryption: the reader already decrypted every object; drop the
    // Encrypt dictionary and the PDF 1.5 structural objects.
    if let Some((number, _)) = reader
        .trailer()
        .get("Encrypt")
        .and_then(PdfObject::as_reference)
    {
        if let Some(new) = old_to_new.get(&number) {
            store.set(*new, PdfObject::Null);
        }
    }
    report.fixes.decrypted = reader.is_encrypted();
    for number in store.numbers() {
        let artifact = store
            .get(number)
            .and_then(object_dict)
            .and_then(|d| d.get_name("Type"))
            .is_some_and(|t| t == "ObjStm" || t == "XRef");
        if artifact {
            store.set(number, PdfObject::Null);
        }
    }

    apply_font_replacements(&mut store, &old_to_new, replacements);
    for ((old, _), stream) in unicode_additions {
        let Some(new_font) = old_to_new.get(&old).copied() else {
            continue;
        };
        let cmap = store.add(stream);
        if let Some(dict) = store.get_mut(new_font).and_then(object_dict_mut) {
            dict.insert("ToUnicode", reference(cmap));
            report.fixes.to_unicode_added += 1;
        }
    }

    sanitize_objects(&mut store, profile, &mut report.fixes);
    fix_catalog(&mut store, root, profile, &mut report.fixes);
    fix_embedded_files(&mut store, root, profile, &mut report.fixes);
    fix_annotations(&mut store, root, profile, &mut usage, report);
    if profile.is_pdfa1() {
        remove_transparency_groups(&mut store, &mut report.fixes);
    }
    let intent = apply_output_intent(doc, &mut store, root, profile, &usage, &old_to_new, report);
    let _ = intent;

    let document_info = DocumentInfo::gather(doc)?;
    let info_number = rebuild_info(&mut store, info_number, &document_info);
    let metadata = xmp_metadata_stream(profile, &store, info_number);
    let metadata_number = store.add(metadata);
    if let Some(catalog) = store.get_mut(root).and_then(object_dict_mut) {
        catalog.insert("Metadata", reference(metadata_number));
    }
    if profile.is_level_a() {
        add_level_a_structure(&mut store, root);
    }

    let file_id = crate::compliance::pdfa_file_id(doc, profile, &document_info);
    let mut objects = store.objects;
    objects.retain(|o| !matches!(o.object, PdfObject::Null) || o.number == root);
    objects.sort_by_key(|o| o.number);
    let (mode, version) = writer_for_profile(profile);
    PdfWriter::new(objects, root)
        .with_version(version)
        .with_info(Some(info_number))
        .with_id(Some(file_id))
        .with_mode(mode)
        .write()
}

fn generate_to_unicode(
    reader: &crate::reader::PdfReader,
    id: (u32, u16),
    dict: &PdfDictionary,
    usage: &ContentUsage,
) -> Option<PdfObject> {
    if dict.get_name("Subtype") == Some("Type3") {
        return None;
    }
    let resolver = FontResolver::new(dict, reader);
    let mut entries: Vec<(String, String)> = Vec::new();
    let mut ranges: BTreeSet<(u8, u32, u32)> = BTreeSet::new();
    let mut seen = HashSet::new();
    for bytes in usage.font_strings.get(&id)? {
        for code in resolver.codes(bytes) {
            let code = code.ok()?.code;
            if !seen.insert((code.value(), code.len())) {
                continue;
            }
            let text = resolver.decode_code(code);
            if text.is_empty() || text == "\u{FFFD}" {
                return None; // cannot honestly claim a Unicode mapping
            }
            let len = code.len();
            let width = usize::from(len) * 2;
            entries.push((
                format!("{:0width$X}", code.value()),
                text.encode_utf16().map(|u| format!("{u:04X}")).collect(),
            ));
            let shift = 8 * u32::from(len.saturating_sub(1));
            let low = (code.value() >> shift) << shift;
            ranges.insert((len, low, low | ((1u32 << shift).wrapping_sub(1))));
        }
    }
    if entries.is_empty() {
        return None;
    }
    let mut out = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n",
    );
    out.push_str(&format!("{} begincodespacerange\n", ranges.len()));
    for (len, low, high) in &ranges {
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
    Some(flate_stream(PdfDictionary::empty(), out.as_bytes()))
}

fn remap_references(object: &mut PdfObject, old_to_new: &HashMap<u32, u32>, locals: &[u32]) {
    match object {
        PdfObject::Reference { number, generation } => {
            if *number >= PLACEHOLDER_BASE {
                if let Some(local) = locals.get((*number - PLACEHOLDER_BASE) as usize) {
                    *number = *local;
                    *generation = 0;
                }
            } else if let Some(new) = old_to_new.get(number) {
                *number = *new;
                *generation = 0;
            } else {
                *object = PdfObject::Null;
            }
        }
        PdfObject::Array(items) => {
            for item in items {
                remap_references(item, old_to_new, locals);
            }
        }
        PdfObject::Dictionary(dict) => {
            for (_, value) in dict.entries_mut() {
                remap_references(value, old_to_new, locals);
            }
        }
        PdfObject::Stream { dict, .. } => {
            for (_, value) in dict.entries_mut() {
                remap_references(value, old_to_new, locals);
            }
        }
        _ => {}
    }
}

fn apply_font_replacements(
    store: &mut Store,
    old_to_new: &HashMap<u32, u32>,
    replacements: Vec<crate::pdfa_fonts::FontReplacement>,
) {
    for replacement in replacements {
        let locals: Vec<u32> = (0..replacement.new_objects.len())
            .map(|_| store.reserve())
            .collect();
        for (number, mut object) in locals.iter().copied().zip(replacement.new_objects) {
            remap_references(&mut object, old_to_new, &locals);
            store.set(number, object);
        }
        let mut font = PdfObject::Dictionary(replacement.font_dict);
        remap_references(&mut font, old_to_new, &locals);
        if let Some(new) = old_to_new.get(&replacement.font.0) {
            store.set(*new, font);
        }
        if let Some((id, dict)) = replacement.descendant {
            let mut descendant = PdfObject::Dictionary(dict);
            remap_references(&mut descendant, old_to_new, &locals);
            if let Some(new) = old_to_new.get(&id.0) {
                store.set(*new, descendant);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Generic object sanitation
// ---------------------------------------------------------------------------

const ACTION_TYPES: &[&str] = &[
    "GoTo",
    "GoToR",
    "GoToE",
    "Launch",
    "Thread",
    "URI",
    "Sound",
    "Movie",
    "Hide",
    "Named",
    "SubmitForm",
    "ResetForm",
    "ImportData",
    "JavaScript",
    "SetOCGState",
    "Rendition",
    "Trans",
    "GoTo3DView",
    "RichMediaExecute",
];

fn is_forbidden_action(profile: PdfAProfile, dict: &PdfDictionary) -> bool {
    let Some(kind) = dict.get_name("S") else {
        return false;
    };
    let is_action = dict.get_name("Type") == Some("Action")
        || (dict.get_name("Type").is_none() && ACTION_TYPES.contains(&kind));
    is_action && !action_allowed(profile, dict)
}

fn sanitize_objects(store: &mut Store, profile: PdfAProfile, fixes: &mut PdfAFixSummary) {
    // Indirect forbidden actions are dropped by their referrers.
    let forbidden: HashSet<u32> = store
        .objects
        .iter()
        .filter(
            |o| matches!(&o.object, PdfObject::Dictionary(d) if is_forbidden_action(profile, d)),
        )
        .map(|o| o.number)
        .collect();
    for idx in 0..store.objects.len() {
        let number = store.objects[idx].number;
        if forbidden.contains(&number) {
            store.objects[idx].object = PdfObject::Null;
            continue;
        }
        let mut object = std::mem::replace(&mut store.objects[idx].object, PdfObject::Null);
        if !profile.allows_embedded_files()
            && object_dict(&object).and_then(|d| d.get_name("Type")) == Some("EmbeddedFile")
        {
            fixes.embedded_files_removed += 1;
            continue; // left as Null and dropped from the output
        }
        sanitize_value(&mut object, profile, &forbidden, fixes, 0);
        if let PdfObject::Stream { dict, raw } = &mut object {
            sanitize_stream(dict, raw, profile, fixes);
        }
        store.objects[idx].object = object;
    }
}

fn sanitize_value(
    object: &mut PdfObject,
    profile: PdfAProfile,
    forbidden: &HashSet<u32>,
    fixes: &mut PdfAFixSummary,
    depth: usize,
) {
    if depth > 64 {
        return;
    }
    let dict = match object {
        PdfObject::Dictionary(dict) => dict,
        PdfObject::Stream { dict, .. } => dict,
        PdfObject::Array(items) => {
            for item in items {
                sanitize_value(item, profile, forbidden, fixes, depth + 1);
            }
            return;
        }
        _ => return,
    };
    for key in ["JS", "AA", "OpenAction"] {
        if dict.remove(key).is_some() {
            fixes.actions_removed += 1;
        }
    }
    if dict.remove("JavaScript").is_some() {
        fixes.javascript_removed += 1;
    }
    for key in ["A", "PA"] {
        let drop = match dict.get(key) {
            Some(PdfObject::Reference { number, .. }) => forbidden.contains(number),
            Some(PdfObject::Dictionary(action)) => is_forbidden_action(profile, action),
            _ => false,
        };
        if drop {
            dict.remove(key);
            fixes.actions_removed += 1;
        }
    }
    if let Some(next) = dict.get_mut("Next") {
        let keep = |item: &PdfObject| match item {
            PdfObject::Reference { number, .. } => !forbidden.contains(number),
            PdfObject::Dictionary(action) => !is_forbidden_action(profile, action),
            _ => true,
        };
        match next {
            PdfObject::Array(items) => {
                let before = items.len();
                items.retain(keep);
                fixes.actions_removed += before - items.len();
            }
            other => {
                if !keep(other) {
                    *other = PdfObject::Null;
                    fixes.actions_removed += 1;
                }
            }
        }
    }
    // Transfer functions and rendering intents (graphics state).
    if dict.remove("TR").is_some() {
        fixes.transfer_functions_removed += 1;
    }
    if dict
        .get("TR2")
        .is_some_and(|v| v.as_name() != Some("Default"))
    {
        dict.insert("TR2", PdfObject::Name("Default".to_string()));
        fixes.transfer_functions_removed += 1;
    }
    if let Some(intent) = dict.get_name("RI") {
        if !matches!(
            intent,
            "RelativeColorimetric" | "AbsoluteColorimetric" | "Perceptual" | "Saturation"
        ) {
            dict.insert("RI", PdfObject::Name("RelativeColorimetric".to_string()));
        }
    }
    if !profile.allows_embedded_files() && (dict.contains_key("EF") || dict.contains_key("RF")) {
        dict.remove("EF");
        dict.remove("RF");
        fixes.embedded_files_removed += 1;
    }
    let keys: Vec<String> = dict.entries().map(|(k, _)| k.clone()).collect();
    for key in keys {
        if let Some(value) = dict.get_mut(&key) {
            sanitize_value(value, profile, forbidden, fixes, depth + 1);
        }
    }
}

fn sanitize_stream(
    dict: &mut PdfDictionary,
    raw: &mut Vec<u8>,
    profile: PdfAProfile,
    fixes: &mut PdfAFixSummary,
) {
    if dict.get_name("Type") == Some("EmbeddedFile") && !profile.allows_embedded_files() {
        raw.clear();
        dict.remove("Filter");
        dict.remove("DecodeParms");
        fixes.embedded_files_removed += 1;
    }
    let mut external = false;
    for key in ["F", "FFilter", "FDecodeParms"] {
        if dict.get_name("Type") != Some("EmbeddedFile") && dict.remove(key).is_some() {
            external = true;
        }
    }
    if external {
        fixes.external_references_removed += 1;
    }
    match dict.get_name("Subtype") {
        Some("PS") => {
            *dict = dict_of(&[
                ("Type", PdfObject::Name("XObject".to_string())),
                ("Subtype", PdfObject::Name("Form".to_string())),
                ("BBox", PdfObject::Array(vec![PdfObject::Integer(0); 4])),
            ]);
            raw.clear();
            fixes.external_references_removed += 1;
            return;
        }
        Some("Form") => {
            if dict.remove("Ref").is_some() {
                fixes.external_references_removed += 1;
            }
            if dict.get_name("Subtype2") == Some("PS") {
                dict.remove("Subtype2");
                dict.remove("PS");
                fixes.external_references_removed += 1;
            }
        }
        Some("Image") => {
            if dict.get_bool("Interpolate") == Some(true) {
                dict.remove("Interpolate");
                fixes.interpolation_disabled += 1;
            }
            if dict.remove("Alternates").is_some() {
                fixes.external_references_removed += 1;
            }
        }
        _ => {}
    }
    if dict.remove("OPI").is_some() {
        fixes.external_references_removed += 1;
    }
    reencode_lzw(dict, raw, fixes);
}

fn reencode_lzw(dict: &mut PdfDictionary, raw: &mut Vec<u8>, fixes: &mut PdfAFixSummary) {
    let filters = filter_names(dict);
    if !filters.iter().any(|f| f == "LZWDecode" || f == "LZW") {
        return;
    }
    let parms: Vec<PdfObject> = match dict.get("DecodeParms") {
        Some(PdfObject::Array(items)) => items.clone(),
        Some(other) => vec![other.clone()],
        None => Vec::new(),
    };
    let image = [
        "DCTDecode",
        "DCT",
        "JPXDecode",
        "CCITTFaxDecode",
        "CCF",
        "JBIG2Decode",
    ];
    let split = filters
        .iter()
        .position(|f| image.contains(&f.as_str()))
        .unwrap_or(filters.len());
    let mut prefix = PdfDictionary::empty();
    prefix.insert(
        "Filter",
        PdfObject::Array(
            filters[..split]
                .iter()
                .map(|f| PdfObject::Name(f.clone()))
                .collect(),
        ),
    );
    if !parms.is_empty() {
        prefix.insert(
            "DecodeParms",
            PdfObject::Array(
                (0..split)
                    .map(|i| parms.get(i).cloned().unwrap_or(PdfObject::Null))
                    .collect(),
            ),
        );
    }
    let Ok(decoded) = crate::filters::decode_stream_from_dict(&prefix, raw) else {
        return;
    };
    let PdfObject::Stream { raw: encoded, .. } = flate_stream(PdfDictionary::empty(), &decoded)
    else {
        return;
    };
    *raw = encoded;
    let mut new_filters = vec![PdfObject::Name("FlateDecode".to_string())];
    let mut new_parms = vec![PdfObject::Null];
    for (i, f) in filters.iter().enumerate().skip(split) {
        new_filters.push(PdfObject::Name(f.clone()));
        new_parms.push(parms.get(i).cloned().unwrap_or(PdfObject::Null));
    }
    if new_filters.len() == 1 {
        dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
        dict.remove("DecodeParms");
    } else {
        dict.insert("Filter", PdfObject::Array(new_filters));
        if new_parms.iter().all(PdfObject::is_null) {
            dict.remove("DecodeParms");
        } else {
            dict.insert("DecodeParms", PdfObject::Array(new_parms));
        }
    }
    dict.remove("DL");
    fixes.lzw_streams_reencoded += 1;
}

// ---------------------------------------------------------------------------
// Catalog, forms, optional content
// ---------------------------------------------------------------------------

fn fix_catalog(store: &mut Store, root: u32, profile: PdfAProfile, fixes: &mut PdfAFixSummary) {
    let Some(mut catalog) = store.get(root).and_then(object_dict).cloned() else {
        return;
    };
    for key in [
        "Version",
        "NeedsRendering",
        "Requirements",
        "OpenAction",
        "AA",
    ] {
        catalog.remove(key);
    }
    if let Some(mut names) = catalog.get("Names").cloned() {
        with_dict_value(store, &mut names, |names| {
            if names.remove("JavaScript").is_some() {
                fixes.javascript_removed += 1;
            }
            if !profile.allows_embedded_files() && names.remove("EmbeddedFiles").is_some() {
                fixes.embedded_files_removed += 1;
            }
        });
        catalog.insert("Names", names);
    }
    if !profile.allows_embedded_files() {
        catalog.remove("AF");
        catalog.remove("Collection");
    }
    if let Some(mut form) = catalog.get("AcroForm").cloned() {
        with_dict_value(store, &mut form, |form| {
            if form.remove("XFA").is_some() {
                fixes.form_fixes += 1;
            }
            if form.get_bool("NeedAppearances") == Some(true) {
                form.remove("NeedAppearances");
                fixes.form_fixes += 1;
            }
        });
        catalog.insert("AcroForm", form);
    }
    if let Some(oc) = catalog.get("OCProperties").cloned() {
        if profile.is_pdfa1() {
            catalog.remove("OCProperties");
            fixes.optional_content_fixed += 1;
        } else if let Some(mut props) = store.resolve_dict(Some(&oc)) {
            let mut seen: HashSet<Vec<u8>> = HashSet::new();
            let mut counter = 0usize;
            let mut fix = |dict: &mut PdfDictionary, fixes: &mut PdfAFixSummary| {
                counter += 1;
                let mut changed = dict.remove("AS").is_some();
                let name = dict
                    .get("Name")
                    .and_then(PdfObject::as_string)
                    .map(<[u8]>::to_vec);
                match name {
                    Some(n) if seen.insert(n.clone()) => {}
                    _ => {
                        let mut candidate = counter;
                        let mut new = format!("Configuration {candidate}").into_bytes();
                        while !seen.insert(new.clone()) {
                            candidate += 1;
                            new = format!("Configuration {candidate}").into_bytes();
                        }
                        dict.insert("Name", PdfObject::String(new));
                        changed = true;
                    }
                }
                if changed {
                    fixes.optional_content_fixed += 1;
                }
            };
            if let Some(mut d) = props.get("D").cloned() {
                with_dict_value(store, &mut d, |dict| fix(dict, fixes));
                props.insert("D", d);
            }
            if let Some(configs) = props.get("Configs").cloned() {
                if let Some(PdfObject::Array(mut items)) = store.resolve(&configs) {
                    for item in &mut items {
                        with_dict_value(store, item, |dict| fix(dict, fixes));
                    }
                    props.insert("Configs", PdfObject::Array(items));
                }
            }
            match oc {
                PdfObject::Reference { number, .. } => {
                    store.set(number, PdfObject::Dictionary(props))
                }
                _ => {
                    catalog.insert("OCProperties", PdfObject::Dictionary(props));
                }
            }
        }
    }
    store.set(root, PdfObject::Dictionary(catalog));
}

// ---------------------------------------------------------------------------
// Embedded files (PDF/A-3 association)
// ---------------------------------------------------------------------------

fn fix_embedded_files(
    store: &mut Store,
    root: u32,
    profile: PdfAProfile,
    fixes: &mut PdfAFixSummary,
) {
    if !profile.allows_embedded_files() {
        return;
    }
    let mut filespecs = Vec::new();
    let mut embedded_streams = Vec::new();
    for object in &store.objects {
        if let Some(dict) = object_dict(&object.object) {
            if dict.contains_key("EF") {
                filespecs.push(object.number);
                if let Some(ef) = store.resolve_dict(dict.get("EF")) {
                    for key in ["F", "UF"] {
                        if let Some(PdfObject::Reference { number, .. }) = ef.get(key) {
                            embedded_streams.push(*number);
                        }
                    }
                }
            }
        }
    }
    for number in embedded_streams {
        if let Some(dict) = store.get_mut(number).and_then(object_dict_mut) {
            if !dict.contains_key("Subtype") {
                dict.insert(
                    "Subtype",
                    PdfObject::Name("application/octet-stream".to_string()),
                );
            }
            dict.insert("Type", PdfObject::Name("EmbeddedFile".to_string()));
        }
    }
    for number in &filespecs {
        if let Some(dict) = store.get_mut(*number).and_then(object_dict_mut) {
            if !matches!(
                dict.get_name("AFRelationship"),
                Some("Source" | "Data" | "Alternative" | "Supplement" | "Unspecified")
            ) {
                dict.insert("AFRelationship", PdfObject::Name("Unspecified".to_string()));
            }
            dict.insert("Type", PdfObject::Name("Filespec".to_string()));
            if !dict.contains_key("UF") {
                if let Some(f) = dict.get("F").cloned() {
                    dict.insert("UF", f);
                }
            }
            if !dict.contains_key("F") {
                if let Some(uf) = dict.get("UF").cloned() {
                    dict.insert("F", uf);
                }
            }
        }
    }
    if filespecs.is_empty() {
        return;
    }
    let mut existing: Vec<PdfObject> = store
        .get(root)
        .and_then(object_dict)
        .and_then(|c| c.get("AF").cloned())
        .and_then(|af| store.resolve(&af))
        .and_then(|af| af.as_array().map(<[PdfObject]>::to_vec))
        .unwrap_or_default();
    for number in filespecs {
        let present = existing
            .iter()
            .any(|item| item.as_reference().is_some_and(|(n, _)| n == number));
        if !present {
            existing.push(reference(number));
            fixes.embedded_files_associated += 1;
        }
    }
    if let Some(catalog) = store.get_mut(root).and_then(object_dict_mut) {
        catalog.insert("AF", PdfObject::Array(existing));
    }
}

// ---------------------------------------------------------------------------
// Annotations
// ---------------------------------------------------------------------------

fn fix_annotations(
    store: &mut Store,
    _root: u32,
    profile: PdfAProfile,
    usage: &mut ContentUsage,
    report: &mut PdfAConvertReport,
) {
    let pages: Vec<u32> = store
        .objects
        .iter()
        .filter(
            |o| matches!(&o.object, PdfObject::Dictionary(d) if d.get_name("Type") == Some("Page")),
        )
        .map(|o| o.number)
        .collect();
    for page in pages {
        let Some(annots_value) = store
            .get(page)
            .and_then(object_dict)
            .and_then(|d| d.get("Annots").cloned())
        else {
            continue;
        };
        let Some(PdfObject::Array(items)) = store.resolve(&annots_value) else {
            continue;
        };
        let mut kept = Vec::new();
        for item in items {
            let Some(mut annot) = store.resolve_dict(Some(&item)) else {
                continue;
            };
            let subtype = annot.get_name("Subtype").unwrap_or("").to_string();
            if !annotation_allowed(profile, &subtype) {
                report.fixes.annotations_removed += 1;
                continue;
            }
            fix_one_annotation(store, &mut annot, &subtype, profile, usage, report);
            match &item {
                PdfObject::Reference { number, .. } => {
                    let number = *number;
                    store.set(number, PdfObject::Dictionary(annot));
                    kept.push(item.clone());
                }
                _ => kept.push(PdfObject::Dictionary(annot)),
            }
        }
        match annots_value {
            PdfObject::Reference { number, .. } => store.set(number, PdfObject::Array(kept)),
            _ => {
                if let Some(dict) = store.get_mut(page).and_then(object_dict_mut) {
                    dict.insert("Annots", PdfObject::Array(kept));
                }
            }
        }
    }
}

fn fix_one_annotation(
    store: &mut Store,
    annot: &mut PdfDictionary,
    subtype: &str,
    profile: PdfAProfile,
    usage: &mut ContentUsage,
    report: &mut PdfAConvertReport,
) {
    let flags = annot.get_integer("F").unwrap_or(0);
    let hidden_bits = ANNOT_HIDDEN | ANNOT_INVISIBLE | ANNOT_NOVIEW | ANNOT_TOGGLE_NOVIEW;
    let was_hidden = flags & (ANNOT_HIDDEN | ANNOT_INVISIBLE | ANNOT_NOVIEW) != 0;
    if subtype == "Popup" {
        if annot.remove("F").is_some() {
            report.fixes.annotation_flags_fixed += 1;
        }
    } else {
        let fixed = (flags | ANNOT_PRINT) & !hidden_bits;
        if Some(fixed) != annot.get_integer("F") {
            annot.insert("F", PdfObject::Integer(fixed));
            report.fixes.annotation_flags_fixed += 1;
        }
    }
    let rect = annot
        .get("Rect")
        .and_then(|r| store.resolve(r))
        .and_then(|r| {
            let n: Vec<f64> = r
                .as_array()?
                .iter()
                .filter_map(PdfObject::as_number)
                .collect();
            (n.len() == 4).then(|| {
                [
                    n[0].min(n[2]),
                    n[1].min(n[3]),
                    n[0].max(n[2]),
                    n[1].max(n[3]),
                ]
            })
        });
    let is_btn = subtype == "Widget" && field_type_store(store, annot).as_deref() == Some("Btn");

    // Existing appearance: keep only N, and the right N shape for widgets.
    let mut ap = annot.get("AP").and_then(|a| store.resolve_dict(Some(a)));
    if let Some(dict) = &mut ap {
        if dict.entries().any(|(k, _)| k != "N") {
            let n = dict.get("N").cloned();
            *dict = PdfDictionary::empty();
            if let Some(n) = n {
                dict.insert("N", n);
            }
            report.fixes.appearance_dictionaries_trimmed += 1;
        }
        if let Some(normal) = dict.get("N").cloned() {
            let resolved = store.resolve(&normal);
            match (is_btn, &resolved) {
                (true, Some(PdfObject::Stream { .. })) => {
                    let state = annot
                        .get_name("AS")
                        .filter(|s| *s != "Off")
                        .unwrap_or("On")
                        .to_string();
                    let mut states = PdfDictionary::empty();
                    states.insert(state.clone(), normal.clone());
                    dict.insert("N", PdfObject::Dictionary(states));
                    annot.insert("AS", PdfObject::Name(state));
                }
                (false, Some(PdfObject::Dictionary(states)))
                    if subtype == "Widget" || !states.is_empty() =>
                {
                    let chosen = annot
                        .get_name("AS")
                        .and_then(|s| states.get(s).cloned())
                        .or_else(|| states.entries().next().map(|(_, v)| v.clone()));
                    match chosen {
                        Some(stream) => {
                            dict.insert("N", stream);
                        }
                        None => {
                            dict.remove("N");
                        }
                    }
                }
                _ => {}
            }
        }
        if dict.get("N").is_none() {
            ap = None;
        }
    }

    let needs_ap =
        !matches!(subtype, "Popup" | "Link") && rect.is_some_and(|r| r[2] > r[0] && r[3] > r[1]);
    if was_hidden && subtype != "Popup" {
        // Keep a hidden annotation invisible once its flags become printable.
        if let Some(rect) = rect {
            let empty = store.add(appearance_stream(rect, Vec::new(), None));
            let entry = normal_entry(empty, is_btn, annot);
            annot.insert("AP", PdfObject::Dictionary(entry));
            report.fixes.appearances_generated += 1;
            return;
        }
    }
    match ap {
        Some(dict) => {
            annot.insert("AP", PdfObject::Dictionary(dict));
        }
        None if needs_ap && !profile.is_pdfa1() => {
            let rect = rect.unwrap_or([0.0, 0.0, 0.0, 0.0]);
            let (content, gstate) =
                synthesize_appearance(store, annot, subtype, rect, profile, usage);
            if content.is_empty() && subtype != "Widget" {
                report.warnings.push(format!(
                    "{subtype} annotation had no appearance; a blank appearance was generated"
                ));
            }
            let stream = store.add(appearance_stream(rect, content, gstate));
            let entry = normal_entry(stream, is_btn, annot);
            annot.insert("AP", PdfObject::Dictionary(entry));
            report.fixes.appearances_generated += 1;
        }
        None => {
            annot.remove("AP");
        }
    }
}

fn normal_entry(stream: u32, is_btn: bool, annot: &mut PdfDictionary) -> PdfDictionary {
    if is_btn {
        let state = annot.get_name("AS").unwrap_or("Off").to_string();
        annot.insert("AS", PdfObject::Name(state.clone()));
        let mut states = PdfDictionary::empty();
        states.insert(state, reference(stream));
        dict_of(&[("N", PdfObject::Dictionary(states))])
    } else {
        dict_of(&[("N", reference(stream))])
    }
}

fn field_type_store(store: &Store, annot: &PdfDictionary) -> Option<String> {
    let mut current = annot.clone();
    for _ in 0..32 {
        if let Some(ft) = current.get_name("FT") {
            return Some(ft.to_string());
        }
        current = store.resolve_dict(current.get("Parent"))?;
    }
    None
}

fn appearance_stream(rect: [f64; 4], content: Vec<u8>, gstate: Option<PdfDictionary>) -> PdfObject {
    let mut resources = PdfDictionary::empty();
    if let Some(gs) = gstate {
        resources.insert(
            "ExtGState",
            PdfObject::Dictionary(dict_of(&[("GS0", PdfObject::Dictionary(gs))])),
        );
    }
    PdfObject::Stream {
        dict: dict_of(&[
            ("Type", PdfObject::Name("XObject".to_string())),
            ("Subtype", PdfObject::Name("Form".to_string())),
            (
                "BBox",
                PdfObject::Array(rect.iter().map(|v| PdfObject::Real(*v)).collect()),
            ),
            ("Resources", PdfObject::Dictionary(resources)),
        ]),
        raw: content,
    }
}

fn colour_operator(colour: &[f64], stroke: bool, usage: &mut ContentUsage) -> Option<String> {
    let nums = colour
        .iter()
        .map(|v| format!("{v:.3}"))
        .collect::<Vec<_>>()
        .join(" ");
    let op = match colour.len() {
        1 => {
            usage.device_gray = true;
            if stroke {
                "G"
            } else {
                "g"
            }
        }
        3 => {
            usage.device_rgb = true;
            if stroke {
                "RG"
            } else {
                "rg"
            }
        }
        4 => {
            usage.device_cmyk = true;
            if stroke {
                "K"
            } else {
                "k"
            }
        }
        _ => return None,
    };
    Some(format!("{nums} {op}\n"))
}

fn numbers_of(store: &Store, value: Option<&PdfObject>) -> Vec<f64> {
    value
        .and_then(|v| store.resolve(v))
        .and_then(|v| {
            v.as_array()
                .map(|items| items.iter().filter_map(PdfObject::as_number).collect())
        })
        .unwrap_or_default()
}

/// Draw a conservative appearance from the annotation's own geometry.
fn synthesize_appearance(
    store: &Store,
    annot: &PdfDictionary,
    subtype: &str,
    rect: [f64; 4],
    profile: PdfAProfile,
    usage: &mut ContentUsage,
) -> (Vec<u8>, Option<PdfDictionary>) {
    let colour = numbers_of(store, annot.get("C"));
    let interior = numbers_of(store, annot.get("IC"));
    let width = annot
        .get("BS")
        .and_then(|bs| store.resolve_dict(Some(bs)))
        .and_then(|bs| bs.get("W").and_then(PdfObject::as_number))
        .or_else(|| numbers_of(store, annot.get("Border")).get(2).copied())
        .unwrap_or(1.0);
    let mut out = String::new();
    let mut gstate = None;
    let stroke = colour_operator(&colour, true, usage);
    let fill_with_stroke_colour = colour_operator(&colour, false, usage);
    let fill = colour_operator(&interior, false, usage);
    let w = width.max(0.0);
    let h = w / 2.0;
    let [x0, y0, x1, y1] = rect;
    match subtype {
        "Square" | "Circle" if stroke.is_some() || fill.is_some() => {
            out.push_str(&format!("{w:.3} w\n"));
            if let Some(s) = &stroke {
                out.push_str(s);
            }
            if let Some(f) = &fill {
                out.push_str(f);
            }
            let (ax, ay, bx, by) = (x0 + h, y0 + h, x1 - h, y1 - h);
            if subtype == "Square" {
                out.push_str(&format!(
                    "{ax:.3} {ay:.3} {:.3} {:.3} re\n",
                    bx - ax,
                    by - ay
                ));
            } else {
                let (cx, cy, rx, ry) = (
                    (ax + bx) / 2.0,
                    (ay + by) / 2.0,
                    (bx - ax) / 2.0,
                    (by - ay) / 2.0,
                );
                let k = 0.552_284_8;
                out.push_str(&format!("{:.3} {:.3} m\n", cx + rx, cy));
                for (p1, p2, p3) in [
                    (
                        (cx + rx, cy + k * ry),
                        (cx + k * rx, cy + ry),
                        (cx, cy + ry),
                    ),
                    (
                        (cx - k * rx, cy + ry),
                        (cx - rx, cy + k * ry),
                        (cx - rx, cy),
                    ),
                    (
                        (cx - rx, cy - k * ry),
                        (cx - k * rx, cy - ry),
                        (cx, cy - ry),
                    ),
                    (
                        (cx + k * rx, cy - ry),
                        (cx + rx, cy - k * ry),
                        (cx + rx, cy),
                    ),
                ] {
                    out.push_str(&format!(
                        "{:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c\n",
                        p1.0, p1.1, p2.0, p2.1, p3.0, p3.1
                    ));
                }
            }
            out.push_str(match (stroke.is_some(), fill.is_some()) {
                (true, true) => "B\n",
                (true, false) => "S\n",
                _ => "f\n",
            });
        }
        "Line" | "PolyLine" | "Polygon" | "Ink" if stroke.is_some() => {
            out.push_str(&format!("{w:.3} w 1 J 1 j\n"));
            out.push_str(stroke.as_deref().unwrap_or(""));
            let mut paths: Vec<Vec<f64>> = Vec::new();
            match subtype {
                "Line" => paths.push(numbers_of(store, annot.get("L"))),
                "Ink" => {
                    if let Some(PdfObject::Array(lists)) =
                        annot.get("InkList").and_then(|v| store.resolve(v))
                    {
                        for list in lists {
                            paths.push(numbers_of(store, Some(&list)));
                        }
                    }
                }
                _ => paths.push(numbers_of(store, annot.get("Vertices"))),
            }
            for path in paths {
                for (idx, pair) in path.chunks(2).enumerate() {
                    if pair.len() == 2 {
                        let op = if idx == 0 { "m" } else { "l" };
                        out.push_str(&format!("{:.3} {:.3} {op}\n", pair[0], pair[1]));
                    }
                }
                if subtype == "Polygon" {
                    out.push_str("h\n");
                }
            }
            out.push_str("S\n");
        }
        "Highlight" | "Underline" | "StrikeOut" | "Squiggly"
            if fill_with_stroke_colour.is_some() =>
        {
            let quads = numbers_of(store, annot.get("QuadPoints"));
            for quad in quads.chunks(8).filter(|q| q.len() == 8) {
                let xs = [quad[0], quad[2], quad[4], quad[6]];
                let ys = [quad[1], quad[3], quad[5], quad[7]];
                let (qx0, qx1) = (
                    xs.iter().cloned().fold(f64::MAX, f64::min),
                    xs.iter().cloned().fold(f64::MIN, f64::max),
                );
                let (qy0, qy1) = (
                    ys.iter().cloned().fold(f64::MAX, f64::min),
                    ys.iter().cloned().fold(f64::MIN, f64::max),
                );
                let height = qy1 - qy0;
                match subtype {
                    "Highlight" if !profile.is_pdfa1() => {
                        gstate = Some(dict_of(&[
                            ("Type", PdfObject::Name("ExtGState".to_string())),
                            ("BM", PdfObject::Name("Multiply".to_string())),
                        ]));
                        out.push_str("/GS0 gs\n");
                        out.push_str(fill_with_stroke_colour.as_deref().unwrap_or(""));
                        out.push_str(&format!(
                            "{qx0:.3} {qy0:.3} {:.3} {height:.3} re f\n",
                            qx1 - qx0
                        ));
                    }
                    "Underline" | "Squiggly" | "StrikeOut" => {
                        let y = if subtype == "StrikeOut" {
                            qy0 + height / 2.0
                        } else {
                            qy0 + height * 0.07
                        };
                        out.push_str(stroke.as_deref().unwrap_or(""));
                        out.push_str(&format!(
                            "{:.3} w {qx0:.3} {y:.3} m {qx1:.3} {y:.3} l S\n",
                            (height / 14.0).max(0.5)
                        ));
                    }
                    _ => {}
                }
            }
        }
        "Text" => {
            let size = 18.0f64.min(x1 - x0).min(y1 - y0);
            let fill = fill_with_stroke_colour.unwrap_or_else(|| {
                usage.device_gray = true;
                "1 g\n".to_string()
            });
            usage.device_gray = true;
            out.push_str(&fill);
            out.push_str(&format!(
                "0 G 0.5 w {:.3} {:.3} {size:.3} {size:.3} re B\n",
                x0 + 0.5,
                y1 - size + 0.5
            ));
        }
        _ => {}
    }
    (out.into_bytes(), gstate)
}

fn remove_transparency_groups(store: &mut Store, fixes: &mut PdfAFixSummary) {
    for object in &mut store.objects {
        let Some(dict) = object_dict_mut(&mut object.object) else {
            continue;
        };
        let is_holder =
            dict.get_name("Type") == Some("Page") || dict.get_name("Subtype") == Some("Form");
        if is_holder && dict.remove("Group").is_some() {
            fixes.transparency_groups_removed += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// OutputIntent and default colour spaces
// ---------------------------------------------------------------------------

fn icc_profile_ok(raw: &[u8], profile: PdfAProfile) -> Option<i64> {
    if raw.len() < 128 || raw.get(36..40) != Some(b"acsp") {
        return None;
    }
    let major = raw[8];
    if profile.is_pdfa1() && major >= 4 {
        return None;
    }
    if major > 4 {
        return None;
    }
    match &raw[16..20] {
        b"GRAY" => Some(1),
        b"RGB " => Some(3),
        b"CMYK" => Some(4),
        _ => None,
    }
}

fn apply_output_intent(
    doc: &PdfDocument,
    store: &mut Store,
    root: u32,
    profile: PdfAProfile,
    usage: &ContentUsage,
    old_to_new: &HashMap<u32, u32>,
    report: &mut PdfAConvertReport,
) -> i64 {
    // Reuse a valid existing destination profile (PDF/A or PDF/X intent).
    let reader = doc.reader();
    let mut reuse: Option<(u32, i64, PdfDictionary)> = None;
    if let Ok(catalog) = doc.get_catalog() {
        if let Some(PdfObject::Array(intents)) = catalog
            .get("OutputIntents")
            .and_then(|v| reader.resolve(v.clone()).ok())
        {
            for intent in intents {
                let Ok(PdfObject::Dictionary(dict)) = reader.resolve(intent) else {
                    continue;
                };
                let Some((number, _)) = dict
                    .get("DestOutputProfile")
                    .and_then(PdfObject::as_reference)
                else {
                    continue;
                };
                let Ok(stream) = reader.get_and_resolve(number, 0) else {
                    continue;
                };
                let Ok(raw) = crate::filters::decode_stream(&stream, reader) else {
                    continue;
                };
                if let (Some(n), Some(new)) =
                    (icc_profile_ok(&raw, profile), old_to_new.get(&number))
                {
                    let pdfa = dict.get_name("S") == Some("GTS_PDFA1");
                    if reuse.is_none() || pdfa {
                        reuse = Some((*new, n, dict.clone()));
                    }
                    if pdfa {
                        break;
                    }
                }
            }
        }
    }
    let (icc_number, components, condition) = match reuse {
        Some((number, n, dict)) => {
            if let Some(stream) = store.get_mut(number).and_then(object_dict_mut) {
                stream.insert("N", PdfObject::Integer(n));
            }
            report.fixes.output_intent = "existing".to_string();
            let condition = dict
                .get("OutputConditionIdentifier")
                .cloned()
                .unwrap_or_else(|| PdfObject::String(b"Custom".to_vec()));
            (number, n, condition)
        }
        None => {
            let icc = store.add(icc_stream(crate::pdfa_icc::srgb_profile(), 3, "DeviceRGB"));
            report.fixes.output_intent = "sRGB IEC61966-2.1 (generated)".to_string();
            report.fixes.output_intent_added = true;
            (icc, 3, PdfObject::String(b"sRGB IEC61966-2.1".to_vec()))
        }
    };
    let intent = store.add(PdfObject::Dictionary(dict_of(&[
        ("Type", PdfObject::Name("OutputIntent".to_string())),
        ("S", PdfObject::Name("GTS_PDFA1".to_string())),
        ("OutputConditionIdentifier", condition),
        (
            "Info",
            PdfObject::String(b"Wellfriend PDF/A output intent".to_vec()),
        ),
        ("DestOutputProfile", reference(icc_number)),
    ])));
    if let Some(catalog) = store.get_mut(root).and_then(object_dict_mut) {
        catalog.insert("OutputIntents", PdfObject::Array(vec![reference(intent)]));
    }

    let mut defaults: Vec<(&str, PdfObject)> = Vec::new();
    if usage.device_rgb && components != 3 {
        let icc = store.add(icc_stream(crate::pdfa_icc::srgb_profile(), 3, "DeviceRGB"));
        defaults.push((
            "DefaultRGB",
            PdfObject::Array(vec![
                PdfObject::Name("ICCBased".to_string()),
                reference(icc),
            ]),
        ));
    }
    if usage.device_cmyk && components != 4 {
        let icc = store.add(icc_stream(crate::pdfa_icc::cmyk_profile(), 4, "DeviceCMYK"));
        defaults.push((
            "DefaultCMYK",
            PdfObject::Array(vec![
                PdfObject::Name("ICCBased".to_string()),
                reference(icc),
            ]),
        ));
    }
    if !defaults.is_empty() {
        report.fixes.default_color_spaces = defaults.iter().map(|(k, _)| k.to_string()).collect();
        inject_default_colour_spaces(store, &defaults);
    }
    components
}

fn icc_stream(profile: Vec<u8>, n: i64, alternate: &str) -> PdfObject {
    flate_stream(
        dict_of(&[
            ("N", PdfObject::Integer(n)),
            ("Alternate", PdfObject::Name(alternate.to_string())),
        ]),
        &profile,
    )
}

fn inject_default_colour_spaces(store: &mut Store, defaults: &[(&str, PdfObject)]) {
    let holders: Vec<u32> = store
        .objects
        .iter()
        .filter(|o| {
            object_dict(&o.object).is_some_and(|d| {
                d.get_name("Type") == Some("Page")
                    || d.get_name("Subtype") == Some("Form")
                    || d.get_integer("PatternType") == Some(1)
                    || d.get_name("Subtype") == Some("Type3")
            })
        })
        .map(|o| o.number)
        .collect();
    let mut resource_refs = BTreeSet::new();
    for holder in holders {
        let Some(dict) = store.get(holder).and_then(object_dict).cloned() else {
            continue;
        };
        let is_page = dict.get_name("Type") == Some("Page");
        let resources = match dict.get("Resources") {
            Some(PdfObject::Reference { number, .. }) => {
                resource_refs.insert(*number);
                continue;
            }
            Some(PdfObject::Dictionary(res)) => res.clone(),
            None if is_page => {
                inherited_resources(store, &dict).unwrap_or_else(PdfDictionary::empty)
            }
            _ => continue,
        };
        let mut resources = resources;
        add_defaults(store, &mut resources, defaults);
        if let Some(dict) = store.get_mut(holder).and_then(object_dict_mut) {
            dict.insert("Resources", PdfObject::Dictionary(resources));
        }
    }
    for number in resource_refs {
        let Some(mut resources) = store.get(number).and_then(object_dict).cloned() else {
            continue;
        };
        add_defaults(store, &mut resources, defaults);
        store.set(number, PdfObject::Dictionary(resources));
    }
}

fn inherited_resources(store: &Store, page: &PdfDictionary) -> Option<PdfDictionary> {
    let mut current = store.resolve_dict(page.get("Parent"))?;
    for _ in 0..64 {
        if let Some(res) = current.get("Resources") {
            return store.resolve_dict(Some(res));
        }
        current = store.resolve_dict(current.get("Parent"))?;
    }
    None
}

fn add_defaults(store: &mut Store, resources: &mut PdfDictionary, defaults: &[(&str, PdfObject)]) {
    let mut spaces = match resources.get("ColorSpace") {
        Some(PdfObject::Reference { number, .. }) => {
            let number = *number;
            if let Some(dict) = store.get_mut(number).and_then(object_dict_mut) {
                for (key, value) in defaults {
                    if !dict.contains_key(key) {
                        dict.insert(*key, value.clone());
                    }
                }
            }
            return;
        }
        Some(PdfObject::Dictionary(spaces)) => spaces.clone(),
        _ => PdfDictionary::empty(),
    };
    for (key, value) in defaults {
        if !spaces.contains_key(key) {
            spaces.insert(*key, value.clone());
        }
    }
    resources.insert("ColorSpace", PdfObject::Dictionary(spaces));
}

// ---------------------------------------------------------------------------
// Info dictionary and XMP
// ---------------------------------------------------------------------------

struct PdfDate {
    parts: [u32; 6],
    tz: Option<(char, u32, u32)>,
}

fn parse_pdf_date(raw: &str) -> Option<PdfDate> {
    let s = raw.trim().strip_prefix("D:").unwrap_or(raw.trim());
    let chars: Vec<char> = s.chars().collect();
    if chars.len() < 4 || !chars[..4].iter().all(char::is_ascii_digit) {
        return None;
    }
    let read = |start: usize, len: usize, default: u32| -> Option<u32> {
        if start + len <= chars.len() {
            let text: String = chars[start..start + len].iter().collect();
            if text.chars().all(|c| c.is_ascii_digit()) {
                return text.parse().ok();
            }
            return None;
        }
        Some(default)
    };
    let digits = chars.iter().take_while(|c| c.is_ascii_digit()).count();
    let field = |start: usize, len: usize, default: u32| {
        if start >= digits {
            Some(default)
        } else {
            read(start, len, default)
        }
    };
    let parts = [
        read(0, 4, 0)?,
        field(4, 2, 1)?,
        field(6, 2, 1)?,
        field(8, 2, 0)?,
        field(10, 2, 0)?,
        field(12, 2, 0)?,
    ];
    if !(1..=12).contains(&parts[1])
        || !(1..=31).contains(&parts[2])
        || parts[3] > 23
        || parts[4] > 59
        || parts[5] > 59
    {
        return None;
    }
    let tz = match chars.get(digits) {
        Some('Z') => Some(('+', 0, 0)),
        Some(sign @ ('+' | '-')) => {
            let rest: String = chars[digits + 1..]
                .iter()
                .filter(|c| c.is_ascii_digit())
                .collect();
            let hh = rest.get(0..2).and_then(|v| v.parse().ok()).unwrap_or(0);
            let mm = rest.get(2..4).and_then(|v| v.parse().ok()).unwrap_or(0);
            if hh > 23 || mm > 59 {
                return None;
            }
            Some((*sign, hh, mm))
        }
        _ => None,
    };
    Some(PdfDate { parts, tz })
}

impl PdfDate {
    fn pdf(&self) -> String {
        let [y, mo, d, h, mi, s] = self.parts;
        let mut out = format!("D:{y:04}{mo:02}{d:02}{h:02}{mi:02}{s:02}");
        if let Some((sign, hh, mm)) = self.tz {
            out.push_str(&format!("{sign}{hh:02}'{mm:02}'"));
        }
        out
    }

    fn xmp(&self) -> String {
        let [y, mo, d, h, mi, s] = self.parts;
        let mut out = format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}");
        match self.tz {
            Some(('+', 0, 0)) => out.push('Z'),
            Some((sign, hh, mm)) => out.push_str(&format!("{sign}{hh:02}:{mm:02}")),
            None => {}
        }
        out
    }
}

fn pdf_text(value: &str) -> PdfObject {
    if value.chars().all(|c| (' '..='~').contains(&c)) {
        PdfObject::String(value.as_bytes().to_vec())
    } else {
        let mut bytes = vec![0xFE, 0xFF];
        for unit in value.encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        PdfObject::String(bytes)
    }
}

fn rebuild_info(store: &mut Store, info_number: Option<u32>, info: &DocumentInfo) -> u32 {
    let mut dict = info_number
        .and_then(|n| store.get(n))
        .and_then(object_dict)
        .cloned()
        .unwrap_or_else(PdfDictionary::empty);
    for (key, value) in [
        ("Title", &info.title),
        ("Author", &info.author),
        ("Subject", &info.subject),
        ("Keywords", &info.keywords),
        ("Creator", &info.creator),
    ] {
        match value.as_deref().map(|v| v.trim_end_matches('\0')) {
            Some(v) if !v.is_empty() => {
                dict.insert(key, pdf_text(v));
            }
            _ => {
                dict.remove(key);
            }
        }
    }
    dict.insert("Producer", pdf_text(PRODUCER));
    for (key, raw) in [
        ("CreationDate", &info.creation_date_raw),
        ("ModDate", &info.mod_date_raw),
    ] {
        match raw.as_deref().and_then(parse_pdf_date) {
            Some(date) => {
                dict.insert(key, PdfObject::String(date.pdf().into_bytes()));
            }
            None => {
                dict.remove(key);
            }
        }
    }
    dict.remove("Trapped");
    match info_number {
        Some(number) => {
            store.set(number, PdfObject::Dictionary(dict));
            number
        }
        None => store.add(PdfObject::Dictionary(dict)),
    }
}

fn xml_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

fn info_string(dict: &PdfDictionary, key: &str) -> Option<String> {
    dict.get(key)
        .and_then(PdfObject::as_string)
        .map(decode_pdf_text_string)
        .filter(|v| !v.is_empty())
}

fn xmp_metadata_stream(profile: PdfAProfile, store: &Store, info_number: u32) -> PdfObject {
    let info = store
        .get(info_number)
        .and_then(object_dict)
        .cloned()
        .unwrap_or_else(PdfDictionary::empty);
    let mut body = String::new();
    body.push_str(&format!(
        "      <pdfaid:part>{}</pdfaid:part>\n      <pdfaid:conformance>{}</pdfaid:conformance>\n",
        profile.part(),
        profile.conformance()
    ));
    body.push_str("      <dc:format>application/pdf</dc:format>\n");
    if let Some(title) = info_string(&info, "Title") {
        body.push_str(&format!(
            "      <dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:title>\n",
            xml_escape(&title)
        ));
    }
    if let Some(author) = info_string(&info, "Author") {
        body.push_str(&format!(
            "      <dc:creator><rdf:Seq><rdf:li>{}</rdf:li></rdf:Seq></dc:creator>\n",
            xml_escape(&author)
        ));
    }
    if let Some(subject) = info_string(&info, "Subject") {
        body.push_str(&format!(
            "      <dc:description><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:description>\n",
            xml_escape(&subject)
        ));
    }
    if let Some(keywords) = info_string(&info, "Keywords") {
        body.push_str(&format!(
            "      <pdf:Keywords>{}</pdf:Keywords>\n",
            xml_escape(&keywords)
        ));
    }
    body.push_str(&format!(
        "      <pdf:Producer>{}</pdf:Producer>\n",
        xml_escape(PRODUCER)
    ));
    if let Some(creator) = info_string(&info, "Creator") {
        body.push_str(&format!(
            "      <xmp:CreatorTool>{}</xmp:CreatorTool>\n",
            xml_escape(&creator)
        ));
    }
    let created = info
        .get("CreationDate")
        .and_then(PdfObject::as_string)
        .and_then(|raw| parse_pdf_date(&String::from_utf8_lossy(raw)));
    let modified = info
        .get("ModDate")
        .and_then(PdfObject::as_string)
        .and_then(|raw| parse_pdf_date(&String::from_utf8_lossy(raw)));
    if let Some(date) = &created {
        body.push_str(&format!(
            "      <xmp:CreateDate>{}</xmp:CreateDate>\n",
            date.xmp()
        ));
    }
    if let Some(date) = &modified {
        body.push_str(&format!(
            "      <xmp:ModifyDate>{}</xmp:ModifyDate>\n",
            date.xmp()
        ));
        body.push_str(&format!(
            "      <xmp:MetadataDate>{}</xmp:MetadataDate>\n",
            date.xmp()
        ));
    }
    let xml = format!(
        "<?xpacket begin=\"\u{FEFF}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n  <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n    <rdf:Description rdf:about=\"\"\n      xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\"\n      xmlns:dc=\"http://purl.org/dc/elements/1.1/\"\n      xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\"\n      xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\">\n{body}    </rdf:Description>\n  </rdf:RDF>\n</x:xmpmeta>\n<?xpacket end=\"w\"?>"
    );
    PdfObject::Stream {
        dict: dict_of(&[
            ("Type", PdfObject::Name("Metadata".to_string())),
            ("Subtype", PdfObject::Name("XML".to_string())),
        ]),
        raw: xml.into_bytes(),
    }
}

fn add_level_a_structure(store: &mut Store, root: u32) {
    let Some(catalog) = store.get(root).and_then(object_dict).cloned() else {
        return;
    };
    let mut catalog = catalog;
    if !catalog.contains_key("Lang") {
        catalog.insert("Lang", PdfObject::String(b"en-US".to_vec()));
    }
    catalog.insert(
        "MarkInfo",
        PdfObject::Dictionary(dict_of(&[("Marked", PdfObject::Boolean(true))])),
    );
    if !catalog.contains_key("StructTreeRoot") {
        let struct_root = store.reserve();
        let element = store.reserve();
        store.set(
            struct_root,
            PdfObject::Dictionary(dict_of(&[
                ("Type", PdfObject::Name("StructTreeRoot".to_string())),
                ("K", PdfObject::Array(vec![reference(element)])),
            ])),
        );
        store.set(
            element,
            PdfObject::Dictionary(dict_of(&[
                ("Type", PdfObject::Name("StructElem".to_string())),
                ("S", PdfObject::Name("Document".to_string())),
                ("P", reference(struct_root)),
                ("K", PdfObject::Array(Vec::new())),
            ])),
        );
        catalog.insert("StructTreeRoot", reference(struct_root));
    }
    store.set(root, PdfObject::Dictionary(catalog));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_dates_round_trip_to_xmp() {
        let date = parse_pdf_date("D:20240115103000+05'30'").unwrap();
        assert_eq!(date.xmp(), "2024-01-15T10:30:00+05:30");
        assert_eq!(date.pdf(), "D:20240115103000+05'30'");
        let date = parse_pdf_date("D:2024").unwrap();
        assert_eq!(date.xmp(), "2024-01-01T00:00:00");
        assert!(parse_pdf_date("garbage").is_none());
        assert_eq!(
            parse_pdf_date("D:20240115103000Z").unwrap().xmp(),
            "2024-01-15T10:30:00Z"
        );
    }

    #[test]
    fn options_parse_levels() {
        let opts: PdfAConvertOptions = serde_json::from_str(r#"{"level":"3u"}"#).unwrap();
        assert_eq!(opts.profile().unwrap(), PdfAProfile::PdfA3U);
        assert_eq!(
            PdfAConvertOptions::default().profile().unwrap(),
            PdfAProfile::PdfA2B
        );
        for (name, expected) in [
            ("PDF/A-1b", PdfAProfile::PdfA1B),
            ("PDF/A-2b", PdfAProfile::PdfA2B),
            ("PDF/A-3b", PdfAProfile::PdfA3B),
            ("PDF/A-2u", PdfAProfile::PdfA2U),
            ("PDF/A-3u", PdfAProfile::PdfA3U),
        ] {
            let json = format!(r#"{{"profile":"{name}"}}"#);
            let opts: PdfAConvertOptions = serde_json::from_str(&json).unwrap();
            assert_eq!(opts.profile().unwrap(), expected);
            assert_eq!(expected.label(), name);
        }
        assert!(
            serde_json::from_str::<PdfAConvertOptions>(r#"{"level":"9z"}"#)
                .unwrap()
                .profile()
                .is_err()
        );
    }
}
