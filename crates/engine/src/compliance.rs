//! PDF compliance validation and bounded conversion helpers.
//!
//! Conversion lives in [`crate::pdfa_convert`]: it embeds substitute fonts,
//! adds the OutputIntent and XMP metadata, removes forbidden features and only
//! returns output that passes the validator in this module.

use std::collections::BTreeSet;

use serde::Serialize;

use crate::document::PdfDocument;
use crate::error::{Result, WellfriendError};
use crate::fonts_report::{list_fonts, FontInfo};
use crate::info::{decode_pdf_text_string, DocumentInfo};
use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;
use crate::writer::{rewrite_document_objects, OutputObject, PdfWriter, WriterMode};

/// Supported PDF/A validation/conversion profiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PdfAProfile {
    /// PDF/A-1b: basic visual archival preservation, no transparency.
    PdfA1B,
    /// PDF/A-2b: basic visual archival preservation, transparency allowed.
    PdfA2B,
    /// PDF/A-2a: visual archival preservation plus tagged structure.
    PdfA2A,
    /// PDF/A-2u: PDF/A-2b plus a Unicode mapping for every glyph.
    PdfA2U,
    /// PDF/A-3b: PDF/A-2b plus associated embedded files.
    PdfA3B,
    /// PDF/A-3a: PDF/A-3b plus tagged structure.
    PdfA3A,
    /// PDF/A-3u: PDF/A-3b plus a Unicode mapping for every glyph.
    PdfA3U,
}

impl PdfAProfile {
    pub fn part(self) -> i32 {
        match self {
            Self::PdfA1B => 1,
            Self::PdfA2B | Self::PdfA2A | Self::PdfA2U => 2,
            Self::PdfA3B | Self::PdfA3A | Self::PdfA3U => 3,
        }
    }

    pub fn conformance(self) -> &'static str {
        match self {
            Self::PdfA1B | Self::PdfA2B | Self::PdfA3B => "B",
            Self::PdfA2A | Self::PdfA3A => "A",
            Self::PdfA2U | Self::PdfA3U => "U",
        }
    }

    /// Short level code used by the CLI/HTTP contract (`1b`, `2b`, `2u`, ...).
    pub fn code(self) -> &'static str {
        match self {
            Self::PdfA1B => "1b",
            Self::PdfA2B => "2b",
            Self::PdfA2A => "2a",
            Self::PdfA2U => "2u",
            Self::PdfA3B => "3b",
            Self::PdfA3A => "3a",
            Self::PdfA3U => "3u",
        }
    }

    /// Parse `2b`, `pdfa2b`, `PDF/A-2b`, `pdf-a-2u`, ... (case-insensitive).
    pub fn parse(value: &str) -> Option<Self> {
        let normalized: String = value
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect();
        let short = normalized.trim_start_matches("pdfa");
        Some(match short {
            "1b" => Self::PdfA1B,
            "2b" => Self::PdfA2B,
            "2a" => Self::PdfA2A,
            "2u" => Self::PdfA2U,
            "3b" => Self::PdfA3B,
            "3a" => Self::PdfA3A,
            "3u" => Self::PdfA3U,
            _ => return None,
        })
    }

    pub(crate) fn requires_unicode(self) -> bool {
        matches!(
            self,
            Self::PdfA2U | Self::PdfA3U | Self::PdfA2A | Self::PdfA3A
        )
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::PdfA1B => "PDF/A-1b",
            Self::PdfA2B => "PDF/A-2b",
            Self::PdfA2A => "PDF/A-2a",
            Self::PdfA3B => "PDF/A-3b",
            Self::PdfA3A => "PDF/A-3a",
            Self::PdfA2U => "PDF/A-2u",
            Self::PdfA3U => "PDF/A-3u",
        }
    }

    pub(crate) fn is_pdfa1(self) -> bool {
        matches!(self, Self::PdfA1B)
    }

    pub(crate) fn is_level_a(self) -> bool {
        matches!(self, Self::PdfA2A | Self::PdfA3A)
    }

    pub(crate) fn allows_embedded_files(self) -> bool {
        matches!(self, Self::PdfA3B | Self::PdfA3A | Self::PdfA3U)
    }
}

/// Severity for compliance findings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComplianceSeverity {
    Error,
    Warning,
}

/// A structured compliance finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ComplianceViolation {
    pub rule: String,
    pub location: String,
    pub severity: ComplianceSeverity,
    pub message: String,
}

impl ComplianceViolation {
    pub(crate) fn error(
        rule: &str,
        location: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            rule: rule.to_string(),
            location: location.into(),
            severity: ComplianceSeverity::Error,
            message: message.into(),
        }
    }

    pub(crate) fn warning(
        rule: &str,
        location: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            rule: rule.to_string(),
            location: location.into(),
            severity: ComplianceSeverity::Warning,
            message: message.into(),
        }
    }
}

/// Result of a PDF/A validation pass.
#[derive(Debug, Clone, Serialize)]
pub struct PdfAValidationReport {
    pub profile: PdfAProfile,
    pub compliant: bool,
    pub violations: Vec<ComplianceViolation>,
}

/// Result of a PDF/UA validation pass.
#[derive(Debug, Clone, Serialize)]
pub struct PdfUaValidationReport {
    pub compliant: bool,
    pub violations: Vec<ComplianceViolation>,
}

/// Summary of a PDF/A conversion.
#[derive(Debug, Clone, Serialize)]
pub struct PdfAConversionReport {
    pub profile: PdfAProfile,
    pub validation: PdfAValidationReport,
    pub blocked_fonts: Vec<FontInfo>,
}

/// Validate a document against the implemented PDF/A profile rules.
pub fn validate_pdfa(doc: &PdfDocument, profile: PdfAProfile) -> Result<PdfAValidationReport> {
    let reader = doc.reader();
    let mut violations = Vec::new();
    let catalog = doc.get_catalog()?;

    if reader.is_encrypted() {
        violations.push(ComplianceViolation::error(
            "pdfa.encryption",
            "trailer",
            "PDF/A documents must not be encrypted",
        ));
    }
    if profile.is_pdfa1() && pdf_version_gt(reader.version(), "1.4") {
        violations.push(ComplianceViolation::error(
            "pdfa1.version",
            "header",
            "PDF/A-1 is based on PDF 1.4 and must not use later PDF features",
        ));
    }
    if reader
        .first_file_id()
        .map(|id| id.is_empty())
        .unwrap_or(true)
    {
        violations.push(ComplianceViolation::error(
            "pdfa.file_id",
            "trailer/ID",
            "PDF/A requires a non-empty trailer ID array",
        ));
    }

    validate_output_intent(&catalog, reader, &mut violations);
    validate_xmp(&catalog, reader, profile, &mut violations);
    validate_fonts(doc, &mut violations)?;
    validate_disallowed_objects(reader, profile, &mut violations)?;
    validate_embedded_file_rules(reader, profile, &mut violations)?;
    crate::pdfa_rules::validate_extended_rules(doc, &catalog, profile, &mut violations)?;
    if profile.is_level_a() {
        validate_level_a_structure(&catalog, reader, &mut violations);
    }

    Ok(PdfAValidationReport {
        profile,
        compliant: !violations
            .iter()
            .any(|v| v.severity == ComplianceSeverity::Error),
        violations,
    })
}

/// Convert a document to the requested PDF/A profile.
///
/// Delegates to [`crate::pdfa_convert::convert_pdf_to_pdfa`] with bundled
/// substitute fonts only (no system fonts) so output is reproducible. The
/// output always passes Wellfriend's PDF/A validator; otherwise an
/// `UnsupportedFeature` error starting with `PDFA_CONVERSION_FAILED` (or
/// `PDFA1_TRANSPARENCY`) lists the failing rules.
pub fn convert_to_pdfa(doc: &PdfDocument, profile: PdfAProfile) -> Result<Vec<u8>> {
    let options = crate::pdfa_convert::PdfAConvertOptions::for_profile(profile);
    let outcome = crate::pdfa_convert::convert_pdf_to_pdfa(doc, &options)?;
    match outcome.bytes {
        Some(bytes) => Ok(bytes),
        None => {
            let report = outcome.report;
            let rules = report
                .failed_rules
                .iter()
                .map(|rule| format!("{} ({})", rule.rule, rule.message))
                .collect::<Vec<_>>()
                .join("; ");
            Err(WellfriendError::UnsupportedFeature(format!(
                "{}: {} [{rules}]",
                report
                    .code
                    .as_deref()
                    .unwrap_or(crate::pdfa_convert::PDFA_CONVERSION_FAILED),
                report.message.unwrap_or_default()
            )))
        }
    }
}

/// Convert and immediately validate with Wellfriend's validator.
pub fn convert_to_pdfa_checked(
    doc: &PdfDocument,
    profile: PdfAProfile,
) -> Result<(Vec<u8>, PdfAConversionReport)> {
    let bytes = convert_to_pdfa(doc, profile)?;
    let converted = PdfDocument::open_bytes(bytes.clone())?;
    let validation = validate_pdfa(&converted, profile)?;
    Ok((
        bytes,
        PdfAConversionReport {
            profile,
            blocked_fonts: Vec::new(),
            validation,
        },
    ))
}

/// Validate basic PDF/UA accessibility requirements.
pub fn validate_pdfua(doc: &PdfDocument) -> Result<PdfUaValidationReport> {
    let catalog = doc.get_catalog()?;
    let reader = doc.reader();
    let mut violations = Vec::new();

    validate_tagged_structure_basics(&catalog, reader, "pdfua", "PDF/UA", &mut violations);

    Ok(PdfUaValidationReport {
        compliant: !violations
            .iter()
            .any(|v| v.severity == ComplianceSeverity::Error),
        violations,
    })
}

/// Add language, MarkInfo, and a minimal structure root when missing.
///
/// This is assistive tagging only; it is not a full PDF/UA conformance
/// guarantee because human-reviewed reading order and alt text are still needed.
pub fn improve_pdfua_best_effort(doc: &PdfDocument, lang: &str) -> Result<Vec<u8>> {
    let (mut objects, root, info) = rewrite_document_objects(doc.reader(), &mut |_, _| {})?;
    let struct_root_number = objects.iter().map(|obj| obj.number).max().unwrap_or(0) + 1;
    let document_element_number = struct_root_number + 1;
    let mut append_structure = false;
    let root_obj = objects
        .iter_mut()
        .find(|obj| obj.number == root)
        .ok_or_else(|| {
            WellfriendError::MalformedPdf("PDF/UA improve: missing catalog".to_string())
        })?;
    let PdfObject::Dictionary(catalog) = &mut root_obj.object else {
        return Err(WellfriendError::MalformedPdf(
            "PDF/UA improve: catalog is not a dictionary".to_string(),
        ));
    };
    catalog.insert("Lang", PdfObject::String(lang.as_bytes().to_vec()));
    catalog.insert(
        "MarkInfo",
        PdfObject::Dictionary(dict(&[("Marked", PdfObject::Boolean(true))])),
    );
    if !catalog.contains_key("StructTreeRoot") {
        catalog.insert("StructTreeRoot", reference(struct_root_number));
        append_structure = true;
    }
    if append_structure {
        objects.push(level_a_struct_tree_root(
            struct_root_number,
            document_element_number,
        ));
        objects.push(level_a_document_struct_elem(
            struct_root_number,
            document_element_number,
        ));
    }
    objects.sort_by_key(|obj| obj.number);
    PdfWriter::new(objects, root)
        .with_info(info)
        .with_id(doc.reader().first_file_id())
        .with_mode(WriterMode::XrefStreamWithObjStm)
        .write()
}

fn validate_level_a_structure(
    catalog: &PdfDictionary,
    reader: &PdfReader,
    violations: &mut Vec<ComplianceViolation>,
) {
    validate_tagged_structure_basics(catalog, reader, "pdfa.level_a", "PDF/A Level A", violations);
}

fn validate_tagged_structure_basics(
    catalog: &PdfDictionary,
    reader: &PdfReader,
    rule_prefix: &str,
    label: &str,
    violations: &mut Vec<ComplianceViolation>,
) {
    if !catalog.contains_key("Lang") {
        violations.push(ComplianceViolation::error(
            &format!("{rule_prefix}.lang"),
            "Catalog",
            format!("{label} documents must declare a document language"),
        ));
    }
    let marked = catalog
        .get("MarkInfo")
        .and_then(|obj| reader.resolve(obj.clone()).ok())
        .and_then(|obj| obj.as_dict().cloned())
        .and_then(|dict| dict.get_bool("Marked"))
        .unwrap_or(false);
    if !marked {
        violations.push(ComplianceViolation::error(
            &format!("{rule_prefix}.marked"),
            "Catalog/MarkInfo",
            format!("{label} documents must be marked/tagged"),
        ));
    }
    let Some(root_obj) = catalog.get("StructTreeRoot") else {
        violations.push(ComplianceViolation::error(
            &format!("{rule_prefix}.structure"),
            "Catalog",
            format!("{label} documents require a StructTreeRoot"),
        ));
        return;
    };
    let Ok(root) = reader.resolve(root_obj.clone()) else {
        violations.push(ComplianceViolation::error(
            &format!("{rule_prefix}.structure"),
            "StructTreeRoot",
            "StructTreeRoot could not be resolved",
        ));
        return;
    };
    let Some(root_dict) = root.as_dict() else {
        violations.push(ComplianceViolation::error(
            &format!("{rule_prefix}.structure"),
            "StructTreeRoot",
            "StructTreeRoot must resolve to a dictionary",
        ));
        return;
    };
    validate_structure_root_k(
        reader,
        root_dict,
        rule_prefix,
        violations,
        &mut BTreeSet::new(),
    );
    validate_structure_alt_text(
        reader,
        root_dict,
        rule_prefix,
        violations,
        &mut BTreeSet::new(),
        0,
    );
}

fn validate_structure_root_k(
    reader: &PdfReader,
    root_dict: &PdfDictionary,
    rule_prefix: &str,
    violations: &mut Vec<ComplianceViolation>,
    visited: &mut BTreeSet<(u32, u16)>,
) {
    let Some(kids) = root_dict.get("K") else {
        violations.push(ComplianceViolation::error(
            &format!("{rule_prefix}.structure.k"),
            "StructTreeRoot/K",
            "Tagged documents require an explicit structure tree root /K entry",
        ));
        return;
    };
    if structure_k_is_empty(reader, kids, visited, 0) {
        violations.push(ComplianceViolation::error(
            &format!("{rule_prefix}.structure.k"),
            "StructTreeRoot/K",
            "Tagged documents require a non-empty structure tree",
        ));
    }
}

fn structure_k_is_empty(
    reader: &PdfReader,
    object: &PdfObject,
    visited: &mut BTreeSet<(u32, u16)>,
    depth: usize,
) -> bool {
    if depth > 64 {
        return false;
    }
    match object {
        PdfObject::Array(items) => items.is_empty(),
        PdfObject::Null => true,
        PdfObject::Reference { number, generation } => {
            if !visited.insert((*number, *generation)) {
                return false;
            }
            let empty = reader
                .get_and_resolve(*number, *generation)
                .map(|resolved| structure_k_is_empty(reader, &resolved, visited, depth + 1))
                .unwrap_or(false);
            visited.remove(&(*number, *generation));
            empty
        }
        _ => false,
    }
}

fn validate_output_intent(
    catalog: &PdfDictionary,
    reader: &PdfReader,
    violations: &mut Vec<ComplianceViolation>,
) {
    let Some(output_intents) = catalog
        .get("OutputIntents")
        .and_then(|obj| reader.resolve(obj.clone()).ok())
    else {
        violations.push(ComplianceViolation::error(
            "pdfa.output_intent",
            "Catalog",
            "PDF/A requires an OutputIntent with an ICC profile",
        ));
        return;
    };
    let Some(items) = output_intents.as_array() else {
        violations.push(ComplianceViolation::error(
            "pdfa.output_intent",
            "Catalog/OutputIntents",
            "OutputIntents must be an array",
        ));
        return;
    };
    if items.is_empty() {
        violations.push(ComplianceViolation::error(
            "pdfa.output_intent",
            "Catalog/OutputIntents",
            "OutputIntents must not be empty",
        ));
    }
    for (idx, item) in items.iter().enumerate() {
        let Ok(intent) = reader.resolve(item.clone()) else {
            violations.push(ComplianceViolation::error(
                "pdfa.output_intent",
                format!("Catalog/OutputIntents[{idx}]"),
                "OutputIntent could not be resolved",
            ));
            continue;
        };
        let Some(dict) = intent.as_dict() else {
            violations.push(ComplianceViolation::error(
                "pdfa.output_intent",
                format!("Catalog/OutputIntents[{idx}]"),
                "OutputIntent must be a dictionary",
            ));
            continue;
        };
        if dict.get_name("S") != Some("GTS_PDFA1") {
            violations.push(ComplianceViolation::error(
                "pdfa.output_intent.s",
                format!("Catalog/OutputIntents[{idx}]/S"),
                "OutputIntent /S must be /GTS_PDFA1",
            ));
        }
        let Some(dest) = dict.get("DestOutputProfile") else {
            violations.push(ComplianceViolation::error(
                "pdfa.output_intent.icc",
                format!("Catalog/OutputIntents[{idx}]"),
                "OutputIntent must reference DestOutputProfile",
            ));
            continue;
        };
        let Ok(profile) = reader.resolve(dest.clone()) else {
            continue;
        };
        let decoded = crate::filters::decode_stream(&profile, reader).ok();
        let Some((profile_dict, raw)) = profile
            .as_stream()
            .map(|(dict, raw)| (dict, decoded.as_deref().unwrap_or(raw)))
        else {
            violations.push(ComplianceViolation::error(
                "pdfa.output_intent.icc",
                format!("Catalog/OutputIntents[{idx}]/DestOutputProfile"),
                "ICC profile must be a stream",
            ));
            continue;
        };
        if profile_dict.get_integer("N") != Some(3) {
            violations.push(ComplianceViolation::warning(
                "pdfa.output_intent.icc",
                format!("Catalog/OutputIntents[{idx}]/DestOutputProfile/N"),
                "Only RGB OutputIntent profiles are validated by Wellfriend",
            ));
        }
        if raw.len() < 128 || raw.get(36..40) != Some(b"acsp") {
            violations.push(ComplianceViolation::error(
                "pdfa.output_intent.icc",
                format!("Catalog/OutputIntents[{idx}]/DestOutputProfile"),
                "ICC profile stream must contain an ICC header signature",
            ));
        }
    }
}

fn validate_xmp(
    catalog: &PdfDictionary,
    reader: &PdfReader,
    profile: PdfAProfile,
    violations: &mut Vec<ComplianceViolation>,
) {
    let Some(metadata) = catalog.get("Metadata") else {
        violations.push(ComplianceViolation::error(
            "pdfa.xmp",
            "Catalog",
            "PDF/A requires a catalog Metadata XMP stream",
        ));
        return;
    };
    let Ok(metadata) = reader.resolve(metadata.clone()) else {
        violations.push(ComplianceViolation::error(
            "pdfa.xmp",
            "Catalog/Metadata",
            "Metadata could not be resolved",
        ));
        return;
    };
    let Some((dict, raw)) = metadata.as_stream() else {
        violations.push(ComplianceViolation::error(
            "pdfa.xmp",
            "Catalog/Metadata",
            "Metadata must be an XMP stream",
        ));
        return;
    };
    if dict.get_name("Subtype") != Some("XML") {
        violations.push(ComplianceViolation::error(
            "pdfa.xmp.subtype",
            "Catalog/Metadata/Subtype",
            "Metadata stream subtype must be /XML",
        ));
    }
    let xmp = String::from_utf8_lossy(raw);
    if !xmp.contains(&format!("<pdfaid:part>{}</pdfaid:part>", profile.part())) {
        violations.push(ComplianceViolation::error(
            "pdfa.xmp.pdfaid_part",
            "Catalog/Metadata",
            format!("XMP must declare pdfaid:part {}", profile.part()),
        ));
    }
    if !xmp.contains(&format!(
        "<pdfaid:conformance>{}</pdfaid:conformance>",
        profile.conformance()
    )) {
        violations.push(ComplianceViolation::error(
            "pdfa.xmp.pdfaid_conformance",
            "Catalog/Metadata",
            "XMP must declare pdfaid:conformance",
        ));
    }
    validate_xmp_info_sync(reader, &xmp, violations);
}

fn validate_xmp_info_sync(
    reader: &PdfReader,
    xmp: &str,
    violations: &mut Vec<ComplianceViolation>,
) {
    let Some((number, generation)) = reader.info_reference() else {
        return;
    };
    let Ok(PdfObject::Dictionary(info)) = reader.get_and_resolve(number, generation) else {
        return;
    };
    for key in [
        "Title", "Author", "Subject", "Keywords", "Creator", "Producer",
    ] {
        let Some(value) = info
            .get(key)
            .and_then(|obj| obj.as_string())
            .map(decode_pdf_text_string)
        else {
            continue;
        };
        if value.is_empty() {
            continue;
        }
        let escaped = xml_escape(&value);
        if !xmp.contains(&escaped) {
            violations.push(ComplianceViolation::error(
                "pdfa.xmp.info_sync",
                format!("Info/{key}"),
                "PDF/A requires document information fields to be synchronized with XMP metadata",
            ));
        }
    }
}

fn validate_fonts(doc: &PdfDocument, violations: &mut Vec<ComplianceViolation>) -> Result<()> {
    for font in list_fonts(doc)? {
        if !font.embedded {
            violations.push(ComplianceViolation::error(
                "pdfa.font.embedded",
                format!("object {} {}", font.object_number, font.generation),
                format!("font '{}' is not embedded", font.name),
            ));
        }
    }
    Ok(())
}

fn validate_disallowed_objects(
    reader: &PdfReader,
    profile: PdfAProfile,
    violations: &mut Vec<ComplianceViolation>,
) -> Result<()> {
    for (number, generation) in reader.object_ids() {
        let object = reader.get_object(number, generation)?;
        scan_disallowed(
            &object,
            &format!("object {number} {generation}"),
            profile,
            violations,
            0,
        );
    }
    Ok(())
}

#[derive(Default)]
struct EmbeddedFileRuleScan {
    embedded_file_streams: usize,
    associated_file_specs: usize,
}

fn validate_embedded_file_rules(
    reader: &PdfReader,
    profile: PdfAProfile,
    violations: &mut Vec<ComplianceViolation>,
) -> Result<()> {
    let mut scan = EmbeddedFileRuleScan::default();
    for (number, generation) in reader.object_ids() {
        let object = reader.get_object(number, generation)?;
        scan_embedded_file_rules(
            &object,
            &format!("object {number} {generation}"),
            profile,
            violations,
            &mut scan,
            0,
        );
    }
    if profile.allows_embedded_files()
        && scan.embedded_file_streams > 0
        && scan.associated_file_specs == 0
    {
        violations.push(ComplianceViolation::error(
            "pdfa3.embedded_file.afrelationship",
            "EmbeddedFiles",
            "PDF/A-3 embedded files must be associated through a FileSpec with /AFRelationship",
        ));
    }
    Ok(())
}

fn scan_embedded_file_rules(
    object: &PdfObject,
    location: &str,
    profile: PdfAProfile,
    violations: &mut Vec<ComplianceViolation>,
    scan: &mut EmbeddedFileRuleScan,
    depth: usize,
) {
    if depth > 64 {
        return;
    }
    match object {
        PdfObject::Dictionary(dict) => {
            if dict.get_name("Type") == Some("EmbeddedFile") {
                scan.embedded_file_streams += 1;
            }
            if is_file_spec_dict(dict) {
                match dict.get_name("AFRelationship") {
                    Some(name) if valid_af_relationship(name) => {
                        scan.associated_file_specs += 1;
                    }
                    Some(name) if profile.allows_embedded_files() => {
                        violations.push(ComplianceViolation::error(
                            "pdfa3.embedded_file.afrelationship",
                            format!("{location}/AFRelationship"),
                            format!("'{name}' is not a valid PDF/A-3 AFRelationship"),
                        ));
                    }
                    None if profile.allows_embedded_files() => {
                        violations.push(ComplianceViolation::error(
                            "pdfa3.embedded_file.afrelationship",
                            location,
                            "PDF/A-3 FileSpec dictionaries for embedded files require /AFRelationship",
                        ));
                    }
                    _ => {}
                }
            }
            for (key, value) in dict.entries() {
                scan_embedded_file_rules(
                    value,
                    &format!("{location}/{key}"),
                    profile,
                    violations,
                    scan,
                    depth + 1,
                );
            }
        }
        PdfObject::Array(items) => {
            for (idx, item) in items.iter().enumerate() {
                scan_embedded_file_rules(
                    item,
                    &format!("{location}[{idx}]"),
                    profile,
                    violations,
                    scan,
                    depth + 1,
                );
            }
        }
        PdfObject::Stream { dict, .. } => {
            scan_embedded_file_rules(
                &PdfObject::Dictionary(dict.clone()),
                location,
                profile,
                violations,
                scan,
                depth + 1,
            );
        }
        _ => {}
    }
}

fn is_file_spec_dict(dict: &PdfDictionary) -> bool {
    matches!(dict.get_name("Type"), Some("Filespec" | "FileSpec")) || dict.contains_key("EF")
}

fn valid_af_relationship(name: &str) -> bool {
    matches!(
        name,
        "Source" | "Data" | "Alternative" | "Supplement" | "Unspecified"
    )
}

fn scan_disallowed(
    object: &PdfObject,
    location: &str,
    profile: PdfAProfile,
    violations: &mut Vec<ComplianceViolation>,
    depth: usize,
) {
    if depth > 64 {
        return;
    }
    match object {
        PdfObject::Dictionary(dict) => {
            for key in ["JavaScript", "JS", "AA", "OpenAction", "Launch"] {
                if dict.contains_key(key) {
                    violations.push(ComplianceViolation::error(
                        "pdfa.action.disallowed",
                        format!("{location}/{key}"),
                        "PDF/A forbids JavaScript, launch, and automatic actions",
                    ));
                }
            }
            if dict.get_name("Type") == Some("EmbeddedFile") && !profile.allows_embedded_files() {
                violations.push(ComplianceViolation::error(
                    "pdfa.embedded_file",
                    location,
                    "This PDF/A profile does not allow EmbeddedFile streams",
                ));
            }
            if profile.is_pdfa1() {
                if matches!(dict.get_name("Type"), Some("ObjStm" | "XRef")) {
                    violations.push(ComplianceViolation::error(
                        "pdfa1.xref_structure",
                        location,
                        "PDF/A-1 forbids PDF 1.5 cross-reference and object stream structures",
                    ));
                }
                if dict.get_name("S") == Some("Transparency") || dict.contains_key("Group") {
                    violations.push(ComplianceViolation::error(
                        "pdfa1.transparency",
                        location,
                        "PDF/A-1 forbids transparency groups",
                    ));
                }
                for alpha_key in ["ca", "CA"] {
                    if dict
                        .get(alpha_key)
                        .and_then(PdfObject::as_number)
                        .map(|v| v < 1.0)
                        .unwrap_or(false)
                    {
                        violations.push(ComplianceViolation::error(
                            "pdfa1.transparency",
                            format!("{location}/{alpha_key}"),
                            "PDF/A-1 forbids transparency",
                        ));
                    }
                }
            }
            for (key, value) in dict.entries() {
                scan_disallowed(
                    value,
                    &format!("{location}/{key}"),
                    profile,
                    violations,
                    depth + 1,
                );
            }
        }
        PdfObject::Array(items) => {
            for (idx, item) in items.iter().enumerate() {
                scan_disallowed(
                    item,
                    &format!("{location}[{idx}]"),
                    profile,
                    violations,
                    depth + 1,
                );
            }
        }
        PdfObject::Stream { dict, .. } => {
            scan_disallowed(
                &PdfObject::Dictionary(dict.clone()),
                location,
                profile,
                violations,
                depth + 1,
            );
        }
        _ => {}
    }
}

pub(crate) fn writer_for_profile(profile: PdfAProfile) -> (WriterMode, &'static str) {
    match profile {
        PdfAProfile::PdfA1B => (WriterMode::ClassicXref, "1.4"),
        PdfAProfile::PdfA2B
        | PdfAProfile::PdfA2A
        | PdfAProfile::PdfA2U
        | PdfAProfile::PdfA3B
        | PdfAProfile::PdfA3A
        | PdfAProfile::PdfA3U => (WriterMode::XrefStreamWithObjStm, "1.7"),
    }
}

fn pdf_version_gt(actual: &str, max: &str) -> bool {
    parse_pdf_version(actual) > parse_pdf_version(max)
}

fn parse_pdf_version(version: &str) -> (u16, u16) {
    let mut parts = version.split('.');
    let major = parts
        .next()
        .and_then(|part| part.parse::<u16>().ok())
        .unwrap_or(0);
    let minor = parts
        .next()
        .and_then(|part| part.parse::<u16>().ok())
        .unwrap_or(0);
    (major, minor)
}

fn level_a_struct_tree_root(struct_root_number: u32, document_element_number: u32) -> OutputObject {
    OutputObject {
        number: struct_root_number,
        object: PdfObject::Dictionary(dict(&[
            ("Type", PdfObject::Name("StructTreeRoot".to_string())),
            (
                "K",
                PdfObject::Array(vec![reference(document_element_number)]),
            ),
        ])),
    }
}

fn level_a_document_struct_elem(
    struct_root_number: u32,
    document_element_number: u32,
) -> OutputObject {
    OutputObject {
        number: document_element_number,
        object: PdfObject::Dictionary(dict(&[
            ("Type", PdfObject::Name("StructElem".to_string())),
            ("S", PdfObject::Name("Document".to_string())),
            ("P", reference(struct_root_number)),
            ("K", PdfObject::Array(Vec::new())),
        ])),
    }
}

pub(crate) fn pdfa_file_id(
    doc: &PdfDocument,
    profile: PdfAProfile,
    info: &DocumentInfo,
) -> Vec<u8> {
    if let Some(id) = doc.reader().first_file_id() {
        if !id.is_empty() {
            return id;
        }
    }

    let mut seed = Vec::new();
    seed.extend_from_slice(b"wellfriendpdfa-file-id\0");
    seed.extend_from_slice(profile.label().as_bytes());
    seed.push(0);
    seed.extend_from_slice(doc.reader().version().as_bytes());
    seed.push(0);
    if let Some(size) = doc.reader().size() {
        seed.extend_from_slice(size.to_string().as_bytes());
    }
    seed.push(0);
    if let Some((root, generation)) = doc.reader().root_reference() {
        seed.extend_from_slice(root.to_string().as_bytes());
        seed.push(b' ');
        seed.extend_from_slice(generation.to_string().as_bytes());
    }
    for value in [
        &info.title,
        &info.author,
        &info.subject,
        &info.keywords,
        &info.creator,
        &info.producer,
    ]
    .into_iter()
    .flatten()
    {
        seed.push(0);
        seed.extend_from_slice(value.as_bytes());
    }
    crate::crypto::md5(&seed).to_vec()
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
            _ => out.push(ch),
        }
    }
    out
}

fn validate_structure_alt_text(
    reader: &PdfReader,
    dict: &PdfDictionary,
    rule_prefix: &str,
    violations: &mut Vec<ComplianceViolation>,
    visited: &mut BTreeSet<(u32, u16)>,
    depth: usize,
) {
    if depth > 128 {
        return;
    }
    if dict.get_name("Type") == Some("StructElem") {
        if let Some(tag) = dict.get_name("S") {
            if !standard_structure_tag(tag) {
                violations.push(ComplianceViolation::error(
                    &format!("{rule_prefix}.structure.tag"),
                    "StructElem/S",
                    format!("'{tag}' is not a standard logical structure tag"),
                ));
            }
            if tag == "Figure" && !dict.contains_key("Alt") {
                violations.push(ComplianceViolation::error(
                    &format!("{rule_prefix}.figure.alt"),
                    "StructElem/Figure",
                    "Figure structure elements require alternate text",
                ));
            }
        } else {
            violations.push(ComplianceViolation::error(
                &format!("{rule_prefix}.structure.tag"),
                "StructElem/S",
                "Structure elements require an /S role",
            ));
        }
    }
    if let Some(kids) = dict.get("K") {
        validate_structure_kids(reader, kids, rule_prefix, violations, visited, depth + 1);
    }
}

fn validate_structure_kids(
    reader: &PdfReader,
    object: &PdfObject,
    rule_prefix: &str,
    violations: &mut Vec<ComplianceViolation>,
    visited: &mut BTreeSet<(u32, u16)>,
    depth: usize,
) {
    match object {
        PdfObject::Reference { number, generation } => {
            if !visited.insert((*number, *generation)) {
                return;
            }
            if let Ok(resolved) = reader.get_and_resolve(*number, *generation) {
                validate_structure_kids(
                    reader,
                    &resolved,
                    rule_prefix,
                    violations,
                    visited,
                    depth + 1,
                );
            }
            visited.remove(&(*number, *generation));
        }
        PdfObject::Dictionary(dict) => {
            validate_structure_alt_text(reader, dict, rule_prefix, violations, visited, depth + 1)
        }
        PdfObject::Array(items) => {
            for item in items {
                validate_structure_kids(reader, item, rule_prefix, violations, visited, depth + 1);
            }
        }
        _ => {}
    }
}

fn standard_structure_tag(tag: &str) -> bool {
    matches!(
        tag,
        "Document"
            | "Part"
            | "Art"
            | "Sect"
            | "Div"
            | "BlockQuote"
            | "Caption"
            | "TOC"
            | "TOCI"
            | "Index"
            | "NonStruct"
            | "Private"
            | "P"
            | "H"
            | "H1"
            | "H2"
            | "H3"
            | "H4"
            | "H5"
            | "H6"
            | "L"
            | "LI"
            | "Lbl"
            | "LBody"
            | "Table"
            | "TR"
            | "TH"
            | "TD"
            | "THead"
            | "TBody"
            | "TFoot"
            | "Span"
            | "Quote"
            | "Note"
            | "Reference"
            | "BibEntry"
            | "Code"
            | "Link"
            | "Annot"
            | "Ruby"
            | "RB"
            | "RT"
            | "RP"
            | "Warichu"
            | "WT"
            | "WP"
            | "Figure"
            | "Formula"
            | "Form"
    )
}

fn reference(number: u32) -> PdfObject {
    PdfObject::Reference {
        number,
        generation: 0,
    }
}

fn dict(entries: &[(&str, PdfObject)]) -> PdfDictionary {
    let mut out = PdfDictionary::empty();
    for (key, value) in entries {
        out.insert(*key, value.clone());
    }
    out
}
