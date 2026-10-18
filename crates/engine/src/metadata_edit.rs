//! Document metadata editing (`edit-metadata`).
//!
//! Sets the standard document-information entries (Title, Author, Subject,
//! Keywords, Creator, Producer, CreationDate, ModDate) and custom keys, and
//! keeps the trailer `/Info` dictionary and the catalog XMP packet
//! (`/Root/Metadata`) consistent with each other.
//!
//! # Write strategy and signatures
//!
//! The edit is always written as an **incremental update**: the original bytes
//! remain an exact prefix of the output, and only the Info dictionary, the XMP
//! metadata stream, and (when the document had no XMP stream) the catalog are
//! appended. Existing signature dictionaries and `/ByteRange`-covered bytes are
//! never touched, so the cryptographic value of every existing signature stays
//! valid; verifiers will report that the signature no longer covers the whole
//! file (a later revision exists).
//!
//! Signature *policy* is taken from [`crate::secure_mutation::analyze_edit_policy`]
//! for [`EditOperation::MetadataUpdate`]:
//!
//! - unsigned document: `safe_incremental`;
//! - approval signatures or DocMDP P=2/P=3: `incremental_with_warning` (the
//!   update is appended; viewers may flag "modified after signing");
//! - certification signature with DocMDP P=1 (no changes permitted):
//!   `blocked_by_signature_policy`, refused unless
//!   [`MetadataEdit::signature_policy_override`] is set.
//!
//! Encrypted inputs are refused by the incremental writer; callers that hold
//! the password can decrypt first (which rewrites the file and therefore
//! cannot preserve signatures).

use std::collections::BTreeMap;
use std::io::Cursor;

use quick_xml::escape::escape;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::ResolveResult;
use quick_xml::{NsReader, Writer};
use serde::{Deserialize, Serialize};

use crate::engine::ContentEngine;
use crate::error::{Result, WellfriendError};
use crate::info::decode_pdf_text_string;
use crate::object::{PdfDictionary, PdfObject};
use crate::secure_mutation::{analyze_edit_policy, EditOperation, EditPolicyDecision};
use crate::writer::{write_incremental_update_with_info, IncrementalObject};

const NS_RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const NS_DC: &str = "http://purl.org/dc/elements/1.1/";
const NS_XMP: &str = "http://ns.adobe.com/xap/1.0/";
const NS_PDF: &str = "http://ns.adobe.com/pdf/1.3/";
/// Adobe's namespace for custom document-information keys in XMP.
const NS_PDFX: &str = "http://ns.adobe.com/pdfx/1.3/";

/// Info keys managed by the typed fields; custom keys may not use them.
const RESERVED_INFO_KEYS: &[&str] = &[
    "Title",
    "Author",
    "Subject",
    "Keywords",
    "Creator",
    "Producer",
    "CreationDate",
    "ModDate",
    "Trapped",
];

/// A metadata edit. `None` leaves a field unchanged; `Some("")` removes it
/// from both Info and XMP. Dates accept ISO 8601 (`2024-05-01`,
/// `2024-05-01T10:20:30Z`, `2024-05-01T10:20:30+05:30`) or PDF date strings
/// (`D:20240501102030+05'30'`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MetadataEdit {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub keywords: Option<String>,
    pub creator: Option<String>,
    pub producer: Option<String>,
    #[serde(alias = "creationDate")]
    pub creation_date: Option<String>,
    #[serde(alias = "modificationDate", alias = "modDate")]
    pub modification_date: Option<String>,
    /// Custom Info keys (written to XMP under the `pdfx` namespace). An empty
    /// value removes the key. Keys must match `[A-Za-z_][A-Za-z0-9_.-]*`.
    #[serde(alias = "fields")]
    pub custom: BTreeMap<String, String>,
    /// Append the update even when signature policy (DocMDP P=1) forbids it.
    #[serde(alias = "signaturePolicyOverride")]
    pub signature_policy_override: bool,
}

impl MetadataEdit {
    fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.author.is_none()
            && self.subject.is_none()
            && self.keywords.is_none()
            && self.creator.is_none()
            && self.producer.is_none()
            && self.creation_date.is_none()
            && self.modification_date.is_none()
            && self.custom.is_empty()
    }
}

/// Result of [`edit_metadata_pdf`].
#[derive(Debug, Clone, Serialize)]
pub struct MetadataEditReport {
    pub schema_version: String,
    /// Final Info values after the edit (decoded text; dates in PDF form).
    pub info: BTreeMap<String, String>,
    /// Final values written to XMP, keyed `prefix:Local`.
    pub xmp: BTreeMap<String, String>,
    pub info_created: bool,
    pub xmp_created: bool,
    pub incremental: bool,
    pub original_prefix_preserved: bool,
    pub signatures_present: bool,
    pub signature_policy_decision: EditPolicyDecision,
    pub signature_note: String,
    pub original_bytes: usize,
    pub output_bytes: usize,
    pub pages: usize,
}

/// A parsed calendar date with optional UTC offset (minutes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MetaDate {
    year: u16,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
    /// `Some(0)` = `Z`; `None` = unspecified.
    offset_minutes: Option<i16>,
}

impl MetaDate {
    fn to_pdf(self) -> String {
        let mut out = format!(
            "D:{:04}{:02}{:02}{:02}{:02}{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        );
        match self.offset_minutes {
            Some(0) => out.push('Z'),
            Some(offset) => {
                let sign = if offset < 0 { '-' } else { '+' };
                let abs = offset.unsigned_abs();
                out.push_str(&format!("{sign}{:02}'{:02}'", abs / 60, abs % 60));
            }
            None => {}
        }
        out
    }

    fn to_iso(self) -> String {
        let mut out = format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        );
        match self.offset_minutes {
            Some(0) => out.push('Z'),
            Some(offset) => {
                let sign = if offset < 0 { '-' } else { '+' };
                let abs = offset.unsigned_abs();
                out.push_str(&format!("{sign}{:02}:{:02}", abs / 60, abs % 60));
            }
            None => {}
        }
        out
    }

    fn validate(self) -> Option<Self> {
        let valid = (1..=12).contains(&self.month)
            && (1..=31).contains(&self.day)
            && self.hour <= 23
            && self.minute <= 59
            && self.second <= 59
            && self
                .offset_minutes
                .is_none_or(|offset| offset.unsigned_abs() <= 14 * 60);
        valid.then_some(self)
    }
}

fn digits(text: &str, start: usize, len: usize) -> Option<u16> {
    let slice = text.get(start..start + len)?;
    if !slice.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    slice.parse().ok()
}

/// Parse a PDF date string (`D:YYYYMMDDHHmmSSOHH'mm'`, every part after the
/// year optional).
fn parse_pdf_date(raw: &str) -> Option<MetaDate> {
    let text = raw.trim();
    let text = text.strip_prefix("D:").unwrap_or(text);
    let year = digits(text, 0, 4)?;
    let mut date = MetaDate {
        year,
        month: 1,
        day: 1,
        hour: 0,
        minute: 0,
        second: 0,
        offset_minutes: None,
    };
    let mut pos = 4;
    for slot in 0..5 {
        let Some(value) = digits(text, pos, 2) else {
            break;
        };
        let value = value as u8;
        match slot {
            0 => date.month = value,
            1 => date.day = value,
            2 => date.hour = value,
            3 => date.minute = value,
            _ => date.second = value,
        }
        pos += 2;
    }
    let rest = &text[pos..];
    if let Some(first) = rest.chars().next() {
        match first {
            'Z' | 'z' => date.offset_minutes = Some(0),
            '+' | '-' => {
                let tz = rest[1..].replace('\'', "");
                let hours = digits(&tz, 0, 2)? as i16;
                let minutes = digits(&tz, 2, 2).unwrap_or(0) as i16;
                let total = hours * 60 + minutes;
                date.offset_minutes = Some(if first == '-' { -total } else { total });
            }
            _ => return None,
        }
    }
    date.validate()
}

/// Parse an ISO 8601 / XMP date (`YYYY[-MM[-DD[Thh:mm[:ss[.fff]][TZD]]]]`).
fn parse_iso_date(raw: &str) -> Option<MetaDate> {
    let text = raw.trim();
    let year = digits(text, 0, 4)?;
    let mut date = MetaDate {
        year,
        month: 1,
        day: 1,
        hour: 0,
        minute: 0,
        second: 0,
        offset_minutes: None,
    };
    let bytes = text.as_bytes();
    let mut pos = 4;
    if bytes.get(pos) == Some(&b'-') {
        date.month = digits(text, pos + 1, 2)? as u8;
        pos += 3;
        if bytes.get(pos) == Some(&b'-') {
            date.day = digits(text, pos + 1, 2)? as u8;
            pos += 3;
        }
    }
    if matches!(bytes.get(pos), Some(b'T') | Some(b't') | Some(b' ')) {
        date.hour = digits(text, pos + 1, 2)? as u8;
        if bytes.get(pos + 3) != Some(&b':') {
            return None;
        }
        date.minute = digits(text, pos + 4, 2)? as u8;
        pos += 6;
        if bytes.get(pos) == Some(&b':') {
            date.second = digits(text, pos + 1, 2)? as u8;
            pos += 3;
            if bytes.get(pos) == Some(&b'.') {
                pos += 1;
                while bytes.get(pos).is_some_and(u8::is_ascii_digit) {
                    pos += 1;
                }
            }
        }
        match bytes.get(pos) {
            Some(b'Z') | Some(b'z') => {
                date.offset_minutes = Some(0);
                pos += 1;
            }
            Some(sign @ (b'+' | b'-')) => {
                let hours = digits(text, pos + 1, 2)? as i16;
                let mut minutes = 0;
                let mut next = pos + 3;
                if bytes.get(next) == Some(&b':') {
                    next += 1;
                }
                if let Some(value) = digits(text, next, 2) {
                    minutes = value as i16;
                    next += 2;
                }
                let total = hours * 60 + minutes;
                date.offset_minutes = Some(if *sign == b'-' { -total } else { total });
                pos = next;
            }
            _ => {}
        }
    }
    if pos != text.len() {
        return None;
    }
    date.validate()
}

fn parse_any_date(raw: &str) -> Option<MetaDate> {
    let trimmed = raw.trim();
    if trimmed.starts_with("D:") {
        parse_pdf_date(trimmed)
    } else {
        match trimmed.as_bytes().get(4) {
            // `YYYY-...` is ISO 8601 only; never reinterpret it as a PDF date.
            Some(b'-') => parse_iso_date(trimmed),
            _ => parse_iso_date(trimmed).or_else(|| parse_pdf_date(trimmed)),
        }
    }
}

fn pdf_text_string(text: &str) -> PdfObject {
    if text.is_ascii() {
        PdfObject::String(text.as_bytes().to_vec())
    } else {
        let mut bytes = vec![0xFE, 0xFF];
        for code in text.encode_utf16() {
            bytes.extend_from_slice(&code.to_be_bytes());
        }
        PdfObject::String(bytes)
    }
}

fn valid_custom_key(key: &str) -> bool {
    let mut chars = key.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        && !RESERVED_INFO_KEYS.contains(&key)
}

fn info_text(info: &PdfDictionary, key: &str) -> Option<String> {
    match info.get(key)? {
        PdfObject::String(bytes) => Some(decode_pdf_text_string(bytes)),
        PdfObject::Name(name) => Some(name.clone()),
        _ => None,
    }
}

/// One XMP property we write: rendered element plus its display pair.
struct XmpProperty {
    xml: String,
    display: (String, String),
}

fn xmp_simple(prefix: &str, local: &str, value: &str) -> XmpProperty {
    XmpProperty {
        xml: format!("<{prefix}:{local}>{}</{prefix}:{local}>", escape(value)),
        display: (format!("{prefix}:{local}"), value.to_string()),
    }
}

fn xmp_lang_alt(local: &str, value: &str) -> XmpProperty {
    XmpProperty {
        xml: format!(
            "<dc:{local}><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:{local}>",
            escape(value)
        ),
        display: (format!("dc:{local}"), value.to_string()),
    }
}

fn xmp_container(local: &str, container: &str, items: &[String], display: &str) -> XmpProperty {
    let mut xml = format!("<dc:{local}><rdf:{container}>");
    for item in items {
        xml.push_str(&format!("<rdf:li>{}</rdf:li>", escape(item.as_str())));
    }
    xml.push_str(&format!("</rdf:{container}></dc:{local}>"));
    XmpProperty {
        xml,
        display: (format!("dc:{local}"), display.to_string()),
    }
}

fn split_list(value: &str, separators: &[char]) -> Vec<String> {
    value
        .split(|c| separators.contains(&c))
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

/// The managed XMP view: properties to (re)write and the (namespace, local)
/// pairs to remove from the existing packet before writing.
struct XmpPlan {
    write: Vec<XmpProperty>,
    remove: Vec<(&'static str, String)>,
}

impl XmpPlan {
    fn removes(&self, namespace: &[u8], local: &[u8]) -> bool {
        self.remove
            .iter()
            .any(|(ns, name)| ns.as_bytes() == namespace && name.as_bytes() == local)
    }
}

fn build_xmp_plan(
    final_info: &BTreeMap<String, String>,
    touched: &[String],
    touched_custom: &[String],
) -> XmpPlan {
    let mut write = Vec::new();
    let mut remove: Vec<(&'static str, String)> = Vec::new();
    let mut manage = |info_key: &str, targets: &[(&'static str, &str)]| -> Option<String> {
        let value = final_info.get(info_key).cloned();
        if value.is_some() || touched.iter().any(|key| key == info_key) {
            for (ns, local) in targets {
                remove.push((ns, (*local).to_string()));
            }
        }
        value
    };
    if let Some(value) = manage("Title", &[(NS_DC, "title")]) {
        write.push(xmp_lang_alt("title", &value));
    }
    if let Some(value) = manage("Author", &[(NS_DC, "creator")]) {
        let authors = split_list(&value, &[';']);
        write.push(xmp_container("creator", "Seq", &authors, &value));
    }
    if let Some(value) = manage("Subject", &[(NS_DC, "description")]) {
        write.push(xmp_lang_alt("description", &value));
    }
    if let Some(value) = manage("Keywords", &[(NS_PDF, "Keywords"), (NS_DC, "subject")]) {
        write.push(xmp_simple("pdf", "Keywords", &value));
        let items = split_list(&value, &[',', ';']);
        write.push(xmp_container("subject", "Bag", &items, &value));
    }
    if let Some(value) = manage("Creator", &[(NS_XMP, "CreatorTool")]) {
        write.push(xmp_simple("xmp", "CreatorTool", &value));
    }
    if let Some(value) = manage("Producer", &[(NS_PDF, "Producer")]) {
        write.push(xmp_simple("pdf", "Producer", &value));
    }
    if let Some(value) = manage("CreationDate", &[(NS_XMP, "CreateDate")]) {
        if let Some(date) = parse_pdf_date(&value) {
            write.push(xmp_simple("xmp", "CreateDate", &date.to_iso()));
        }
    }
    if let Some(value) = manage(
        "ModDate",
        &[(NS_XMP, "ModifyDate"), (NS_XMP, "MetadataDate")],
    ) {
        if let Some(date) = parse_pdf_date(&value) {
            let iso = date.to_iso();
            write.push(xmp_simple("xmp", "ModifyDate", &iso));
            write.push(xmp_simple("xmp", "MetadataDate", &iso));
        }
    }
    for key in touched_custom {
        remove.push((NS_PDFX, key.clone()));
        if let Some(value) = final_info.get(key) {
            write.push(xmp_simple("pdfx", key, value));
        }
    }
    XmpPlan { write, remove }
}

fn new_description(plan: &XmpPlan) -> String {
    let mut out = format!(
        "<rdf:Description rdf:about=\"\" xmlns:dc=\"{NS_DC}\" xmlns:xmp=\"{NS_XMP}\" xmlns:pdf=\"{NS_PDF}\" xmlns:pdfx=\"{NS_PDFX}\">\n"
    );
    for property in &plan.write {
        out.push_str("      ");
        out.push_str(&property.xml);
        out.push('\n');
    }
    out.push_str("    </rdf:Description>\n  ");
    out
}

fn fresh_packet(plan: &XmpPlan) -> String {
    format!(
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n  <rdf:RDF xmlns:rdf=\"{NS_RDF}\">\n    {}</rdf:RDF>\n</x:xmpmeta>\n<?xpacket end=\"w\"?>",
        new_description(plan)
    )
}

fn xml_err(err: impl std::fmt::Display) -> WellfriendError {
    WellfriendError::MalformedPdf(format!("XMP metadata is not well-formed XML: {err}"))
}

/// Rewrite an existing XMP packet: drop managed properties (element and
/// attribute forms) and append one `rdf:Description` holding their new values
/// just before `</rdf:RDF>`. Every other property is preserved verbatim.
fn merge_xmp(existing: &str, plan: &XmpPlan) -> Result<String> {
    let mut reader = NsReader::from_str(existing);
    reader.config_mut().trim_text(false);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut depth = 0usize;
    let mut description_depth: Option<usize> = None;
    let mut skip_depth: Option<usize> = None;
    let mut inserted = false;
    loop {
        let (resolved, event) = reader.read_resolved_event().map_err(xml_err)?;
        let element_ns = match &resolved {
            ResolveResult::Bound(ns) => Some(ns.0.as_bytes().to_vec()),
            _ => None,
        };
        match event {
            Event::Eof => break,
            Event::Start(start) => {
                depth += 1;
                if skip_depth.is_some() {
                    continue;
                }
                let local = start.local_name().as_ref().as_bytes().to_vec();
                if description_depth.is_some_and(|d| depth == d + 1)
                    && element_ns
                        .as_deref()
                        .is_some_and(|ns| plan.removes(ns, &local))
                {
                    skip_depth = Some(depth);
                    continue;
                }
                let is_description =
                    element_ns.as_deref() == Some(NS_RDF.as_bytes()) && local == b"Description";
                let start = if is_description {
                    description_depth = Some(depth);
                    strip_managed_attributes(&reader, &start, plan)?
                } else {
                    start.into_owned()
                };
                writer.write_event(Event::Start(start)).map_err(xml_err)?;
            }
            Event::Empty(start) => {
                if skip_depth.is_some() {
                    continue;
                }
                let local = start.local_name().as_ref().as_bytes().to_vec();
                if description_depth.is_some_and(|d| depth == d)
                    && element_ns
                        .as_deref()
                        .is_some_and(|ns| plan.removes(ns, &local))
                {
                    continue;
                }
                let is_description =
                    element_ns.as_deref() == Some(NS_RDF.as_bytes()) && local == b"Description";
                let start = if is_description {
                    strip_managed_attributes(&reader, &start, plan)?
                } else {
                    start.into_owned()
                };
                writer.write_event(Event::Empty(start)).map_err(xml_err)?;
            }
            Event::End(end) => {
                let current = depth;
                depth = depth.saturating_sub(1);
                if let Some(skip) = skip_depth {
                    if current == skip {
                        skip_depth = None;
                    }
                    continue;
                }
                if description_depth == Some(current) {
                    description_depth = None;
                }
                let is_rdf = element_ns.as_deref() == Some(NS_RDF.as_bytes())
                    && end.local_name().as_ref() == "RDF";
                if is_rdf && !inserted {
                    std::io::Write::write_all(writer.get_mut(), new_description(plan).as_bytes())
                        .map_err(xml_err)?;
                    inserted = true;
                }
                writer.write_event(Event::End(end)).map_err(xml_err)?;
            }
            other => {
                if skip_depth.is_some() {
                    continue;
                }
                writer.write_event(other).map_err(xml_err)?;
            }
        }
    }
    if !inserted {
        return Err(xml_err("packet has no rdf:RDF element"));
    }
    String::from_utf8(writer.into_inner().into_inner()).map_err(xml_err)
}

fn strip_managed_attributes(
    reader: &NsReader<&[u8]>,
    start: &BytesStart<'_>,
    plan: &XmpPlan,
) -> Result<BytesStart<'static>> {
    let name = start.name().as_ref().to_owned();
    let mut out = BytesStart::new(name);
    for attribute in start.attributes() {
        let attribute = attribute.map_err(xml_err)?;
        let (resolved, local) = reader.resolver().resolve_attribute(attribute.key);
        if let ResolveResult::Bound(ns) = resolved {
            if plan.removes(ns.0.as_bytes(), local.as_ref().as_bytes()) {
                continue;
            }
        }
        out.push_attribute(attribute);
    }
    Ok(out)
}

fn decode_xmp_packet(engine: &ContentEngine) -> Option<String> {
    let bytes = crate::standards_engine::read_xmp_metadata(engine)?;
    let text = String::from_utf8(bytes).ok()?;
    Some(text.trim_start_matches('\u{feff}').to_string())
}

/// Apply `edit` to `input` as an incremental update. Returns the new PDF and
/// a report of the final Info/XMP values and the signature posture.
pub fn edit_metadata_pdf(
    input: &[u8],
    edit: &MetadataEdit,
) -> Result<(Vec<u8>, MetadataEditReport)> {
    if edit.is_empty() {
        return Err(WellfriendError::ParseError(
            "invalid_options: edit-metadata: no fields to change".to_string(),
        ));
    }
    for key in edit.custom.keys() {
        if !valid_custom_key(key) {
            return Err(WellfriendError::ParseError(format!(
                "invalid_options: edit-metadata: invalid custom key '{key}' (use [A-Za-z_][A-Za-z0-9_.-]* and not a standard Info key)"
            )));
        }
    }

    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let reader = engine.document().reader();
    if reader.is_encrypted() {
        return Err(WellfriendError::EncryptedPdf(
            "edit-metadata: encrypted input; decrypt first (this rewrites the file and cannot preserve signatures)".to_string(),
        ));
    }
    let policy = analyze_edit_policy(&engine, EditOperation::MetadataUpdate)?;
    if matches!(
        policy.decision,
        EditPolicyDecision::BlockedBySignaturePolicy | EditPolicyDecision::ExplicitOverrideRequired
    ) && !edit.signature_policy_override
    {
        return Err(WellfriendError::UnsupportedFeature(
            "edit-metadata: blocked by signature policy (certification signature forbids changes); set signature_policy_override to append anyway".to_string(),
        ));
    }
    let signatures_present = !policy.cryptographic_reports.is_empty()
        || !policy.structural_policies.is_empty()
        || policy.impact.signature_count > 0;

    let mut next_number = reader.size().unwrap_or(0).max(1) as u32;
    if let Some(max) = reader.object_ids().iter().map(|(n, _)| *n).max() {
        next_number = next_number.max(max + 1);
    }
    let mut allocate = || {
        let number = next_number;
        next_number += 1;
        number
    };

    // --- Info dictionary --------------------------------------------------
    let (info_ref, mut info, info_created) = match reader.info_reference() {
        Some((number, generation)) => {
            let dict = reader
                .get_and_resolve(number, generation)?
                .as_dict()
                .cloned()
                .unwrap_or_else(PdfDictionary::empty);
            ((number, generation), dict, false)
        }
        None => ((allocate(), 0u16), PdfDictionary::empty(), true),
    };

    let mut touched = Vec::new();
    let text_fields = [
        ("Title", &edit.title),
        ("Author", &edit.author),
        ("Subject", &edit.subject),
        ("Keywords", &edit.keywords),
        ("Creator", &edit.creator),
        ("Producer", &edit.producer),
    ];
    for (key, value) in text_fields {
        if let Some(value) = value {
            touched.push(key.to_string());
            if value.is_empty() {
                info.remove(key);
            } else {
                info.insert(key, pdf_text_string(value));
            }
        }
    }
    for (key, value) in [
        ("CreationDate", &edit.creation_date),
        ("ModDate", &edit.modification_date),
    ] {
        if let Some(value) = value {
            touched.push(key.to_string());
            if value.trim().is_empty() {
                info.remove(key);
            } else {
                let date = parse_any_date(value).ok_or_else(|| {
                    WellfriendError::ParseError(format!(
                        "invalid_options: edit-metadata: unrecognised date '{value}' (use ISO 8601 or D:YYYYMMDDHHmmSS)"
                    ))
                })?;
                info.insert(key, PdfObject::String(date.to_pdf().into_bytes()));
            }
        }
    }
    let mut touched_custom = Vec::new();
    for (key, value) in &edit.custom {
        touched_custom.push(key.clone());
        if value.is_empty() {
            info.remove(key);
        } else {
            info.insert(key.as_str(), pdf_text_string(value));
        }
    }

    let mut final_info = BTreeMap::new();
    for key in RESERVED_INFO_KEYS.iter().copied() {
        if let Some(value) = info_text(&info, key).filter(|v| !v.is_empty()) {
            final_info.insert(key.to_string(), value);
        }
    }
    for key in &touched_custom {
        if let Some(value) = info_text(&info, key).filter(|v| !v.is_empty()) {
            final_info.insert(key.clone(), value);
        }
    }

    // --- XMP -------------------------------------------------------------
    let plan = build_xmp_plan(&final_info, &touched, &touched_custom);
    let existing_packet = decode_xmp_packet(&engine);
    let packet = match existing_packet.as_deref() {
        Some(existing) => merge_xmp(existing, &plan).unwrap_or_else(|_| fresh_packet(&plan)),
        None => fresh_packet(&plan),
    };
    let mut xmp_dict = PdfDictionary::empty();
    xmp_dict.insert("Type", PdfObject::Name("Metadata".to_string()));
    xmp_dict.insert("Subtype", PdfObject::Name("XML".to_string()));
    let xmp_stream = PdfObject::Stream {
        dict: xmp_dict,
        raw: packet.into_bytes(),
    };

    let mut changed = vec![IncrementalObject {
        number: info_ref.0,
        generation: info_ref.1,
        object: PdfObject::Dictionary(info),
    }];
    let (root_number, root_generation) = reader.root_reference().ok_or_else(|| {
        WellfriendError::MalformedPdf("edit-metadata: trailer is missing /Root".to_string())
    })?;
    let catalog = reader
        .get_and_resolve(root_number, root_generation)?
        .as_dict()
        .cloned()
        .ok_or_else(|| WellfriendError::MalformedPdf("catalog is not a dictionary".to_string()))?;
    let xmp_created = match catalog.get("Metadata") {
        Some(PdfObject::Reference { number, generation }) => {
            changed.push(IncrementalObject {
                number: *number,
                generation: *generation,
                object: xmp_stream,
            });
            false
        }
        _ => {
            let number = allocate();
            changed.push(IncrementalObject {
                number,
                generation: 0,
                object: xmp_stream,
            });
            let mut catalog = catalog;
            catalog.insert(
                "Metadata",
                PdfObject::Reference {
                    number,
                    generation: 0,
                },
            );
            changed.push(IncrementalObject {
                number: root_number,
                generation: root_generation,
                object: PdfObject::Dictionary(catalog),
            });
            true
        }
    };

    let output =
        write_incremental_update_with_info(reader, changed, info_created.then_some(info_ref))?;
    let original_prefix_preserved = output.starts_with(input);
    if !original_prefix_preserved {
        return Err(WellfriendError::MalformedPdf(
            "edit-metadata: incremental writer did not preserve the original prefix".to_string(),
        ));
    }
    let pages = ContentEngine::open_bytes(output.clone())
        .and_then(|reopened| reopened.page_count())
        .map_err(|error| {
            WellfriendError::MalformedPdf(format!("edit-metadata output failed reopen: {error}"))
        })?;

    let signature_note = if signatures_present {
        "existing signatures keep their signed byte ranges and values; verifiers will report the signature covers an earlier revision and viewers may flag changes after signing".to_string()
    } else {
        "unsigned document".to_string()
    };
    let report = MetadataEditReport {
        schema_version: "1.0".to_string(),
        xmp: plan.write.iter().map(|p| p.display.clone()).collect(),
        info: final_info,
        info_created,
        xmp_created,
        incremental: true,
        original_prefix_preserved,
        signatures_present,
        signature_policy_decision: policy.decision,
        signature_note,
        original_bytes: input.len(),
        output_bytes: output.len(),
        pages,
    };
    Ok((output, report))
}

/// Parse a [`MetadataEdit`] from the JSON options object shared by the CLI
/// (`--options`), the HTTP route and the C API.
pub fn parse_metadata_edit_options(options_json: &str) -> Result<MetadataEdit> {
    let value: serde_json::Value = serde_json::from_str(options_json).map_err(|err| {
        WellfriendError::ParseError(format!("invalid_json: edit-metadata options: {err}"))
    })?;
    serde_json::from_value(value).map_err(|err| {
        WellfriendError::ParseError(format!("invalid_options: edit-metadata options: {err}"))
    })
}

/// JSON wrapper: `options_json` is a [`MetadataEdit`] object.
pub fn edit_metadata_json(
    input: &[u8],
    options_json: &str,
) -> Result<(Vec<u8>, MetadataEditReport)> {
    edit_metadata_pdf(input, &parse_metadata_edit_options(options_json)?)
}

/// Read back the document metadata as seen in Info and XMP (for tests and
/// `edit-metadata --json` verification).
pub fn read_metadata_views(engine: &ContentEngine) -> (BTreeMap<String, String>, Option<String>) {
    let reader = engine.document().reader();
    let mut info = BTreeMap::new();
    if let Some((number, generation)) = reader.info_reference() {
        if let Ok(object) = reader.get_and_resolve(number, generation) {
            if let Some(dict) = object.as_dict() {
                for (key, _) in dict.iter() {
                    if let Some(value) = info_text(dict, key) {
                        info.insert(key.clone(), value);
                    }
                }
            }
        }
    }
    (info, decode_xmp_packet(engine))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_round_trip_between_iso_and_pdf() {
        let date = parse_any_date("2024-05-01T10:20:30+05:30").unwrap();
        assert_eq!(date.to_pdf(), "D:20240501102030+05'30'");
        assert_eq!(date.to_iso(), "2024-05-01T10:20:30+05:30");
        let pdf = parse_any_date("D:20240501102030-08'00'").unwrap();
        assert_eq!(pdf.to_iso(), "2024-05-01T10:20:30-08:00");
        assert_eq!(
            parse_any_date("2024-05-01").unwrap().to_pdf(),
            "D:20240501000000"
        );
        assert_eq!(parse_any_date("D:2024").unwrap().month, 1);
        assert!(parse_any_date("2024-13-01").is_none());
        assert!(parse_any_date("yesterday").is_none());
    }

    #[test]
    fn custom_keys_are_validated() {
        assert!(valid_custom_key("Department"));
        assert!(valid_custom_key("_x.y-z"));
        assert!(!valid_custom_key("Title"));
        assert!(!valid_custom_key("9lives"));
        assert!(!valid_custom_key("has space"));
    }

    #[test]
    fn merge_replaces_managed_properties_and_keeps_others() {
        let existing = format!(
            "<?xpacket begin=\"\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?><x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"{NS_RDF}\"><rdf:Description rdf:about=\"\" xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\" xmlns:dc=\"{NS_DC}\" xmlns:p=\"{NS_PDF}\" p:Producer=\"Old\"><pdfaid:part>2</pdfaid:part><dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">Old title</rdf:li></rdf:Alt></dc:title></rdf:Description></rdf:RDF></x:xmpmeta><?xpacket end=\"w\"?>"
        );
        let mut info = BTreeMap::new();
        info.insert("Title".to_string(), "New & improved".to_string());
        info.insert("Producer".to_string(), "Wellfriend".to_string());
        let plan = build_xmp_plan(&info, &["Title".to_string()], &[]);
        let merged = merge_xmp(&existing, &plan).unwrap();
        assert!(merged.contains("<pdfaid:part>2</pdfaid:part>"));
        assert!(!merged.contains("Old title"));
        assert!(!merged.contains("p:Producer=\"Old\""));
        assert!(merged.contains("New &amp; improved"));
        assert!(merged.contains("<pdf:Producer>Wellfriend</pdf:Producer>"));
        assert!(merged.contains("<?xpacket end=\"w\"?>"));
        let mut reader = NsReader::from_str(&merged);
        while reader.read_event().unwrap() != Event::Eof {}
    }
}
