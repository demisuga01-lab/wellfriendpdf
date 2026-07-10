//! Security reporting, sanitization, and deterministic canonicalization helpers.
//!
//! This module is intentionally policy-oriented. It does not execute PDF actions
//! or embedded payloads; it detects, reports, and removes active/risky content
//! according to an explicit sanitizer policy.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::info::{DocumentInfo, EncryptionReport};
use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;
use crate::signature::SignatureReport;
use crate::versioning::{content_defined_chunks, resource_digest};
use crate::writer::{rewrite_document_with_mode, WriterMode};
use crate::{ContentEngine, PdfDocument};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecuritySeverity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SecurityFinding {
    pub code: String,
    pub severity: SecuritySeverity,
    pub location: String,
    pub message: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RiskyContentReport {
    pub javascript_actions: usize,
    pub launch_actions: usize,
    pub submit_form_actions: usize,
    pub uri_actions: usize,
    pub remote_goto_actions: usize,
    pub named_actions: usize,
    pub embedded_files: usize,
    pub file_attachment_annotations: usize,
    pub rich_media_annotations: usize,
    pub open_actions: usize,
    pub additional_actions: usize,
    pub xfa_packets: usize,
    pub metadata_streams: usize,
    pub findings: Vec<SecurityFinding>,
}

impl RiskyContentReport {
    pub fn risky_total(&self) -> usize {
        self.javascript_actions
            + self.launch_actions
            + self.submit_form_actions
            + self.uri_actions
            + self.remote_goto_actions
            + self.named_actions
            + self.embedded_files
            + self.file_attachment_annotations
            + self.rich_media_annotations
            + self.open_actions
            + self.additional_actions
            + self.xfa_packets
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SecurityReport {
    pub schema_version: u32,
    pub encrypted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encryption: Option<EncryptionReport>,
    pub public_key_security_handler_detected: bool,
    pub aes_gcm_detected: bool,
    pub aes_gcm_supported: bool,
    pub permissions_note: String,
    pub signatures: Vec<SignatureReport>,
    pub risky_content: RiskyContentReport,
    pub xfa: crate::xfa::XfaSecurityReport,
    pub findings: Vec<SecurityFinding>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SanitizerPolicy {
    Strict,
    #[default]
    Balanced,
    PreserveVisual,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SanitizerOptions {
    pub policy: SanitizerPolicy,
    pub remove_javascript: bool,
    pub remove_launch_actions: bool,
    pub remove_submit_form_actions: bool,
    pub remove_uri_actions: bool,
    pub remove_remote_goto_actions: bool,
    pub remove_named_actions: bool,
    pub remove_embedded_files: bool,
    pub remove_file_attachment_annotations: bool,
    pub remove_rich_media: bool,
    pub remove_open_action: bool,
    pub remove_additional_actions: bool,
    pub scrub_metadata: bool,
    pub remove_xfa: bool,
}

impl SanitizerOptions {
    pub fn strict() -> Self {
        Self {
            policy: SanitizerPolicy::Strict,
            remove_javascript: true,
            remove_launch_actions: true,
            remove_submit_form_actions: true,
            remove_uri_actions: true,
            remove_remote_goto_actions: true,
            remove_named_actions: true,
            remove_embedded_files: true,
            remove_file_attachment_annotations: true,
            remove_rich_media: true,
            remove_open_action: true,
            remove_additional_actions: true,
            scrub_metadata: true,
            remove_xfa: true,
        }
    }

    pub fn balanced() -> Self {
        Self {
            policy: SanitizerPolicy::Balanced,
            remove_uri_actions: false,
            remove_named_actions: false,
            scrub_metadata: false,
            ..Self::strict()
        }
    }

    pub fn preserve_visual() -> Self {
        Self {
            policy: SanitizerPolicy::PreserveVisual,
            remove_uri_actions: false,
            remove_named_actions: false,
            remove_file_attachment_annotations: false,
            scrub_metadata: false,
            ..Self::strict()
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SanitizerReport {
    pub policy: SanitizerPolicy,
    pub input_risky_total: usize,
    pub output_risky_total: usize,
    pub strict_passed: bool,
    pub removed: BTreeMap<String, usize>,
    pub remaining_risks: Vec<SecurityFinding>,
    pub output_bytes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CanonicalizeOptions {
    pub writer_mode: String,
    pub fixed_source_date_epoch: Option<i64>,
}

impl Default for CanonicalizeOptions {
    fn default() -> Self {
        Self {
            writer_mode: "classic_xref".to_string(),
            fixed_source_date_epoch: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CanonicalizeReport {
    pub writer_mode: String,
    pub input_sha256: String,
    pub output_sha256: String,
    pub output_bytes: usize,
    pub object_count: usize,
    pub deterministic: bool,
    pub signature_impact: String,
    pub nondeterminism_notes: Vec<String>,
    pub content_chunk_count: usize,
}

pub fn security_report(engine: &ContentEngine) -> Result<SecurityReport> {
    let doc = engine.document();
    let info = DocumentInfo::gather(doc)?;
    let reader = doc.reader();
    let encryption = info.encryption.clone();
    let public_key_security_handler_detected = public_key_security_handler_detected(reader);
    let aes_gcm_detected = aes_gcm_detected(reader);
    let signatures = engine.verify_signatures()?;
    let mut risky_content = scan_risky_content(doc)?;
    let xfa = crate::xfa::xfa_security_report(engine, &crate::xfa::XfaLimits::default())?;
    risky_content.xfa_packets = xfa.packet_count;
    let mut findings = risky_content.findings.clone();

    if xfa.script_count > 0 {
        findings.push(SecurityFinding {
            code: "xfa.scripts.disabled_by_default".to_string(),
            severity: SecuritySeverity::Warning,
            location: "/AcroForm/XFA".to_string(),
            message: format!(
                "{} XFA script(s) were inventoried; no XFA script executes under the default policy.",
                xfa.script_count
            ),
        });
    }
    if xfa.external_connection_count > 0 {
        findings.push(SecurityFinding {
            code: "xfa.external_connections.blocked".to_string(),
            severity: SecuritySeverity::Error,
            location: "/AcroForm/XFA".to_string(),
            message: format!(
                "{} XFA external connection/resource reference(s) were inventoried and blocked.",
                xfa.external_connection_count
            ),
        });
    }

    if public_key_security_handler_detected {
        findings.push(SecurityFinding {
            code: "encryption.public_key_handler_unsupported".to_string(),
            severity: SecuritySeverity::Warning,
            location: "/Encrypt".to_string(),
            message: "Public-key security handlers are detected and reported, but certificate-based decryption is not implemented in the default pure-Rust path.".to_string(),
        });
    }
    if aes_gcm_detected {
        findings.push(SecurityFinding {
            code: "encryption.aes_gcm_unsupported".to_string(),
            severity: SecuritySeverity::Warning,
            location: "/Encrypt/CF".to_string(),
            message: "AES-GCM or integrity-extension crypt filters are detected as PDF 2.0 extension work and are not claimed as supported.".to_string(),
        });
    }

    Ok(SecurityReport {
        schema_version: 1,
        encrypted: encryption.is_some(),
        encryption,
        public_key_security_handler_detected,
        aes_gcm_detected,
        aes_gcm_supported: false,
        permissions_note: "Owner-password permissions are viewer-enforced policy after a document is opened; they are not cryptographic secrecy against a processor that has the opening key.".to_string(),
        signatures,
        risky_content,
        xfa,
        findings,
    })
}

pub fn scan_risky_content(doc: &PdfDocument) -> Result<RiskyContentReport> {
    let mut report = RiskyContentReport::default();
    let reader = doc.reader();
    if let Ok(catalog) = doc.get_catalog() {
        scan_dictionary(&catalog, "catalog", &mut report, 0);
    }
    for (number, generation) in reader.object_ids() {
        if let Ok(object) = reader.get_object(number, generation) {
            let location = format!("{number} {generation} obj");
            scan_object(&object, &location, &mut report, 0);
        }
    }
    Ok(report)
}

pub fn sanitize_pdf(
    engine: &ContentEngine,
    options: &SanitizerOptions,
) -> Result<(Vec<u8>, SanitizerReport)> {
    let before = scan_risky_content(engine.document())?;
    let mut removed = BTreeMap::new();
    let output = rewrite_document_with_mode(
        engine.document().reader(),
        WriterMode::ClassicXref,
        |_, object| sanitize_object(object, options, &mut removed, 0),
    )?;
    let sanitized = ContentEngine::open_bytes(output.clone())?;
    let after = scan_risky_content(sanitized.document())?;
    let strict_passed = after.risky_total() == 0;
    Ok((
        output.clone(),
        SanitizerReport {
            policy: options.policy.clone(),
            input_risky_total: before.risky_total(),
            output_risky_total: after.risky_total(),
            strict_passed,
            removed,
            remaining_risks: after.findings,
            output_bytes: output.len(),
        },
    ))
}

pub fn canonicalize_pdf(
    engine: &ContentEngine,
    options: &CanonicalizeOptions,
) -> Result<(Vec<u8>, CanonicalizeReport)> {
    let reader = engine.document().reader();
    let output =
        rewrite_document_with_mode(reader, WriterMode::ClassicXref, |_number, _object| {})?;
    let input_sha256 = resource_digest(reader.file_bytes());
    let output_sha256 = resource_digest(&output);
    let chunks = content_defined_chunks(&output, 256, 1024, 4096);
    let signatures = engine.verify_signatures().unwrap_or_default();
    let signature_impact = if signatures.is_empty() {
        "no_signatures_detected".to_string()
    } else {
        "canonical_full_rewrite_invalidates_existing_byte_ranges".to_string()
    };
    let mut nondeterminism_notes = Vec::new();
    if options.fixed_source_date_epoch.is_none() {
        nondeterminism_notes.push(
            "no fixed source date epoch was supplied; existing metadata timestamps are preserved"
                .to_string(),
        );
    }
    Ok((
        output.clone(),
        CanonicalizeReport {
            writer_mode: options.writer_mode.clone(),
            input_sha256,
            output_sha256,
            output_bytes: output.len(),
            object_count: reader.object_ids().len(),
            deterministic: true,
            signature_impact,
            nondeterminism_notes,
            content_chunk_count: chunks.len(),
        },
    ))
}

fn public_key_security_handler_detected(reader: &PdfReader) -> bool {
    reader.encrypt_dictionary().is_some_and(|dict| {
        dict.get_name("Filter") == Some("Adobe.PubSec") || dict.contains_key("Recipients")
    })
}

fn aes_gcm_detected(reader: &PdfReader) -> bool {
    let Some(dict) = reader.encrypt_dictionary() else {
        return false;
    };
    contains_name_containing(&PdfObject::Dictionary(dict), "GCM")
}

fn contains_name_containing(object: &PdfObject, needle: &str) -> bool {
    match object {
        PdfObject::Name(name) => name.to_ascii_uppercase().contains(needle),
        PdfObject::Array(items) => items
            .iter()
            .any(|item| contains_name_containing(item, needle)),
        PdfObject::Dictionary(dict) => dict
            .entries()
            .any(|(_, value)| contains_name_containing(value, needle)),
        PdfObject::Stream { dict, .. } => dict
            .entries()
            .any(|(_, value)| contains_name_containing(value, needle)),
        _ => false,
    }
}

fn scan_object(object: &PdfObject, location: &str, report: &mut RiskyContentReport, depth: usize) {
    if depth > 24 {
        return;
    }
    match object {
        PdfObject::Dictionary(dict) => scan_dictionary(dict, location, report, depth + 1),
        PdfObject::Stream { dict, .. } => scan_dictionary(dict, location, report, depth + 1),
        PdfObject::Array(items) => {
            for (idx, item) in items.iter().enumerate() {
                scan_object(item, &format!("{location}[{idx}]"), report, depth + 1);
            }
        }
        _ => {}
    }
}

fn scan_dictionary(
    dict: &PdfDictionary,
    location: &str,
    report: &mut RiskyContentReport,
    depth: usize,
) {
    if depth > 24 {
        return;
    }
    if let Some(action) = dict.get_name("S") {
        match action {
            "JavaScript" => add_finding(
                report,
                "active.javascript",
                SecuritySeverity::Error,
                location,
                "JavaScript action detected",
                |r| r.javascript_actions += 1,
            ),
            "Launch" => add_finding(
                report,
                "active.launch",
                SecuritySeverity::Error,
                location,
                "Launch action detected",
                |r| r.launch_actions += 1,
            ),
            "SubmitForm" => add_finding(
                report,
                "active.submit_form",
                SecuritySeverity::Warning,
                location,
                "SubmitForm action detected",
                |r| r.submit_form_actions += 1,
            ),
            "URI" => add_finding(
                report,
                "active.uri",
                SecuritySeverity::Warning,
                location,
                "External URI action detected",
                |r| r.uri_actions += 1,
            ),
            "GoToR" => add_finding(
                report,
                "active.remote_goto",
                SecuritySeverity::Warning,
                location,
                "Remote GoTo action detected",
                |r| r.remote_goto_actions += 1,
            ),
            "Named" => add_finding(
                report,
                "active.named_action",
                SecuritySeverity::Warning,
                location,
                "Named action detected",
                |r| r.named_actions += 1,
            ),
            "Rendition" => add_finding(
                report,
                "active.rendition",
                SecuritySeverity::Warning,
                location,
                "Rendition action detected",
                |r| r.rich_media_annotations += 1,
            ),
            _ => {}
        }
    }
    if dict.contains_key("JS") || dict.contains_key("JavaScript") {
        add_finding(
            report,
            "active.javascript_entry",
            SecuritySeverity::Error,
            location,
            "JavaScript entry detected",
            |r| r.javascript_actions += 1,
        );
    }
    if dict.contains_key("OpenAction") {
        add_finding(
            report,
            "active.open_action",
            SecuritySeverity::Error,
            location,
            "Document OpenAction detected",
            |r| r.open_actions += 1,
        );
    }
    if dict.contains_key("AA") {
        add_finding(
            report,
            "active.additional_actions",
            SecuritySeverity::Error,
            location,
            "Additional actions dictionary detected",
            |r| r.additional_actions += 1,
        );
    }
    if dict.contains_key("XFA") {
        add_finding(
            report,
            "active.xfa",
            SecuritySeverity::Warning,
            location,
            "XFA packet detected",
            |r| r.xfa_packets += 1,
        );
    }
    if dict.get_name("Type") == Some("EmbeddedFile") || dict.contains_key("EF") {
        add_finding(
            report,
            "payload.embedded_file",
            SecuritySeverity::Warning,
            location,
            "Embedded file or file specification detected",
            |r| r.embedded_files += 1,
        );
    }
    if dict.get_name("Subtype") == Some("FileAttachment") {
        add_finding(
            report,
            "payload.file_attachment_annotation",
            SecuritySeverity::Warning,
            location,
            "FileAttachment annotation detected",
            |r| r.file_attachment_annotations += 1,
        );
    }
    if matches!(
        dict.get_name("Subtype"),
        Some("RichMedia" | "Movie" | "Sound" | "Screen" | "3D")
    ) {
        add_finding(
            report,
            "active.rich_media",
            SecuritySeverity::Warning,
            location,
            "Rich media annotation detected",
            |r| r.rich_media_annotations += 1,
        );
    }
    if dict.get_name("Type") == Some("Metadata") || dict.contains_key("Metadata") {
        report.metadata_streams += 1;
    }
    for (key, value) in dict.entries() {
        scan_object(value, &format!("{location}/{key}"), report, depth + 1);
    }
}

fn add_finding(
    report: &mut RiskyContentReport,
    code: &str,
    severity: SecuritySeverity,
    location: &str,
    message: &str,
    count: impl FnOnce(&mut RiskyContentReport),
) {
    count(report);
    report.findings.push(SecurityFinding {
        code: code.to_string(),
        severity,
        location: location.to_string(),
        message: message.to_string(),
    });
}

fn sanitize_object(
    object: &mut PdfObject,
    options: &SanitizerOptions,
    removed: &mut BTreeMap<String, usize>,
    depth: usize,
) {
    if depth > 24 {
        return;
    }
    let null_reason = match object {
        PdfObject::Dictionary(dict) => null_reason_for_dictionary(dict, options),
        PdfObject::Stream { dict, .. } => null_reason_for_dictionary(dict, options),
        _ => None,
    };
    if let Some(reason) = null_reason {
        increment_removed(removed, reason);
        *object = PdfObject::Null;
        return;
    }
    match object {
        PdfObject::Dictionary(dict) | PdfObject::Stream { dict, .. } => {
            sanitize_dictionary(dict, options, removed, depth + 1);
        }
        PdfObject::Array(items) => {
            for item in items {
                sanitize_object(item, options, removed, depth + 1);
            }
        }
        _ => {}
    }
}

fn sanitize_dictionary(
    dict: &mut PdfDictionary,
    options: &SanitizerOptions,
    removed: &mut BTreeMap<String, usize>,
    depth: usize,
) {
    let removals = [
        ("OpenAction", options.remove_open_action, "open_action"),
        (
            "AA",
            options.remove_additional_actions,
            "additional_actions",
        ),
        (
            "RichMediaActivation",
            options.remove_additional_actions,
            "rich_media_activation",
        ),
        (
            "RichMediaDeactivation",
            options.remove_additional_actions,
            "rich_media_deactivation",
        ),
        (
            "Activation",
            options.remove_additional_actions,
            "media_activation",
        ),
        (
            "Deactivation",
            options.remove_additional_actions,
            "media_deactivation",
        ),
        ("JS", options.remove_javascript, "javascript"),
        ("JavaScript", options.remove_javascript, "javascript"),
        ("XFA", options.remove_xfa, "xfa"),
        ("Metadata", options.scrub_metadata, "metadata"),
        (
            "EmbeddedFiles",
            options.remove_embedded_files,
            "embedded_files",
        ),
        ("URI", options.remove_uri_actions, "uri"),
    ];
    for (key, enabled, reason) in removals {
        if enabled && dict.remove(key).is_some() {
            increment_removed(removed, reason);
        }
    }

    if should_remove_direct_action(dict.get("A"), options) && dict.remove("A").is_some() {
        increment_removed(removed, "action_reference");
    }
    if should_remove_direct_action(dict.get("PA"), options) && dict.remove("PA").is_some() {
        increment_removed(removed, "previous_action");
    }

    for (_, value) in dict.entries_mut() {
        sanitize_object(value, options, removed, depth + 1);
    }
}

fn null_reason_for_dictionary(
    dict: &PdfDictionary,
    options: &SanitizerOptions,
) -> Option<&'static str> {
    if options.remove_additional_actions && matches!(dict.get_name("S"), Some("MCD" | "MCS")) {
        return Some("media_clip_action");
    }
    if let Some(action) = dict.get_name("S") {
        if action_removed_by_policy(action, options) {
            return Some("action_object");
        }
    }
    if options.remove_embedded_files
        && (dict.get_name("Type") == Some("EmbeddedFile")
            || dict.get_name("Type") == Some("Filespec")
            || dict.contains_key("EF"))
    {
        return Some("embedded_file");
    }
    if options.remove_file_attachment_annotations
        && dict.get_name("Subtype") == Some("FileAttachment")
    {
        return Some("file_attachment_annotation");
    }
    if options.remove_rich_media
        && matches!(
            dict.get_name("Subtype"),
            Some("RichMedia" | "Movie" | "Sound" | "Screen" | "3D")
        )
    {
        return Some("rich_media");
    }
    if options.remove_rich_media
        && (matches!(
            dict.get_name("Type"),
            Some("RichMediaContent" | "RichMediaSettings" | "3D")
        ) || dict.contains_key("RichMediaContent")
            || dict.contains_key("RichMediaSettings")
            || dict.contains_key("MediaClip")
            || dict.contains_key("3DD"))
    {
        return Some("rich_media_payload");
    }
    if options.scrub_metadata && dict.get_name("Type") == Some("Metadata") {
        return Some("metadata");
    }
    None
}

fn should_remove_direct_action(value: Option<&PdfObject>, options: &SanitizerOptions) -> bool {
    let Some(PdfObject::Dictionary(dict)) = value else {
        return false;
    };
    dict.get_name("S")
        .is_some_and(|action| action_removed_by_policy(action, options))
}

fn action_removed_by_policy(action: &str, options: &SanitizerOptions) -> bool {
    match action {
        "JavaScript" => options.remove_javascript,
        "Launch" => options.remove_launch_actions,
        "SubmitForm" => options.remove_submit_form_actions,
        "URI" => options.remove_uri_actions,
        "GoToR" => options.remove_remote_goto_actions,
        "Named" => options.remove_named_actions,
        "Rendition" => options.remove_rich_media,
        _ => false,
    }
}

fn increment_removed(removed: &mut BTreeMap<String, usize>, key: &'static str) {
    *removed.entry(key.to_string()).or_insert(0) += 1;
}
