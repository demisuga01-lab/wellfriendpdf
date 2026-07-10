//! Prompt 17 annotation XFDF, appearance, rich-media policy, and non-axis
//! image-redaction orchestration.
//!
//! The implementation deliberately reuses the repository's PDF object model,
//! deterministic writer, annotation renderer/editor, sanitizer, signature
//! report, image decoder, and bounded XML parser. Active content is never
//! executed and media payloads are never decoded by this module.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::editing::{EditMode, ImageRedactionPolicy, PdfEditor, RedactionOptions};
use crate::error::{OxideError, Result};
use crate::info::decode_pdf_text_string;
use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;
use crate::security::{sanitize_pdf, SanitizerOptions};
use crate::versioning::resource_digest;
use crate::writer::{rewrite_document_objects, OutputObject, PdfWriter, WriterMode};
use crate::xfa::xml::{parse_xml, XmlNode};
use crate::{ContentEngine, PdfDocument};

pub const PROMPT17_SCHEMA_VERSION: &str = "prompt17.annotation-xfdf-media-redaction.v1";
pub const XFDF_NAMESPACE: &str = "http://ns.adobe.com/xfdf/";
pub const OXIDE_XFDF_NAMESPACE: &str = "urn:oxidepdf:xfdf:prompt17";

const MAX_XFDF_BYTES: usize = 8 * 1024 * 1024;
const MAX_ANNOTATIONS: usize = 20_000;
const MAX_RELATIONSHIPS: usize = 40_000;
const MAX_GEOMETRY_VALUES: usize = 200_000;
const MAX_CUSTOM_FIELDS: usize = 128;
const MAX_CUSTOM_VALUE_BYTES: usize = 64 * 1024;
const MAX_MEDIA_ASSETS: usize = 10_000;
const MAX_MEDIA_BYTES: usize = 512 * 1024 * 1024;
const MAX_REDACTION_POLYGONS: usize = 4_096;
const MAX_REDACTION_POINTS: usize = 65_536;
type AnnotationObjectEntry = (Option<(u32, u16)>, PdfObject);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Prompt17SupportStatus {
    Implemented,
    ImplementedWithLimits,
    UnsupportedReportedSecurityPolicy,
    UnsupportedReportedNoSafeDecoder,
    UnsupportedReportedExact,
    NotInPrompt17Scope,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prompt17Diagnostic {
    pub code: String,
    pub severity: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<usize>,
}

impl Prompt17Diagnostic {
    fn warning(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            severity: "warning".to_string(),
            message: message.into(),
            annotation_id: None,
            page: None,
        }
    }

    fn with_annotation(mut self, id: &str, page: usize) -> Self {
        self.annotation_id = Some(id.to_string());
        self.page = Some(page);
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AnnotationAppearanceMetadata {
    pub has_normal: bool,
    pub has_rollover: bool,
    pub has_down: bool,
    pub selected_state: Option<String>,
    pub ocg_associated: bool,
    pub generation_posture: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AnnotationActionInventory {
    pub kind: String,
    pub safe: bool,
    pub target_kind: Option<String>,
    pub target_sha256: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AnnotationXfdfRecord {
    pub id: String,
    pub page: usize,
    pub subtype: String,
    pub rect: Option<[f64; 4]>,
    pub vertices: Vec<f64>,
    pub ink_lists: Vec<Vec<f64>>,
    pub quad_points: Vec<f64>,
    pub line: Vec<f64>,
    pub callout: Vec<f64>,
    pub border: Vec<f64>,
    pub border_style: Option<String>,
    pub border_width: Option<f64>,
    pub border_dash: Vec<f64>,
    pub border_effect: Option<String>,
    pub border_effect_intensity: Option<f64>,
    pub color: Vec<f64>,
    pub interior_color: Vec<f64>,
    pub opacity: Option<f64>,
    pub blend_mode: Option<String>,
    pub rotation: Option<i64>,
    pub flags: Option<i64>,
    pub author: Option<String>,
    pub title: Option<String>,
    pub subject: Option<String>,
    pub contents: Option<String>,
    pub safe_rich_text: Option<String>,
    pub created: Option<String>,
    pub modified: Option<String>,
    pub icon: Option<String>,
    pub intent: Option<String>,
    pub review_state: Option<String>,
    pub reply_type: Option<String>,
    pub reply_to: Option<String>,
    pub popup_for: Option<String>,
    pub line_endings: Vec<String>,
    pub repeat_overlay: bool,
    pub ocg_object: Option<String>,
    pub widget_field: Option<String>,
    pub attachment_name: Option<String>,
    pub action: Option<AnnotationActionInventory>,
    pub appearance: AnnotationAppearanceMetadata,
    pub custom_data: BTreeMap<String, String>,
    pub provenance: String,
    pub diagnostics: Vec<Prompt17Diagnostic>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AnnotationXfdfDocument {
    pub schema_version: String,
    pub href: Option<String>,
    pub file_ids: Vec<String>,
    pub annotations: Vec<AnnotationXfdfRecord>,
    pub diagnostics: Vec<Prompt17Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnnotationConflictPolicy {
    Replace,
    MergeSafeFields,
    Reject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnnotationDeletePolicy {
    Disabled,
    ExplicitIds,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnnotationAppearancePolicy {
    PreserveValid,
    RegenerateMissingOrMalformed,
    RegenerateAllSupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnnotationXfdfImportOptions {
    pub conflict_policy: AnnotationConflictPolicy,
    pub delete_policy: AnnotationDeletePolicy,
    pub delete_ids: Vec<String>,
    pub appearance_policy: AnnotationAppearancePolicy,
    pub fail_on_unsupported: bool,
    pub deterministic: bool,
}

impl Default for AnnotationXfdfImportOptions {
    fn default() -> Self {
        Self {
            conflict_policy: AnnotationConflictPolicy::MergeSafeFields,
            delete_policy: AnnotationDeletePolicy::Disabled,
            delete_ids: Vec::new(),
            appearance_policy: AnnotationAppearancePolicy::RegenerateMissingOrMalformed,
            fail_on_unsupported: false,
            deterministic: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotationXfdfExportReport {
    pub schema_version: String,
    pub annotation_count: usize,
    pub page_count: usize,
    pub generated_ids: usize,
    pub preserved_ids: usize,
    pub output_bytes: usize,
    pub output_sha256: String,
    pub deterministic: bool,
    pub diagnostics: Vec<Prompt17Diagnostic>,
    pub exact_limits: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotationXfdfImportReport {
    pub schema_version: String,
    pub imported_annotations: usize,
    pub created: usize,
    pub updated: usize,
    pub deleted: usize,
    pub unchanged: usize,
    pub unsupported: usize,
    pub duplicate_ids: Vec<String>,
    pub relationship_count: usize,
    pub appearances_regenerated: usize,
    pub output_bytes: usize,
    pub output_sha256: String,
    pub deterministic: bool,
    pub signature_impact: String,
    pub diagnostics: Vec<Prompt17Diagnostic>,
    pub exact_limits: Vec<String>,
}

pub fn export_annotation_xfdf(
    engine: &ContentEngine,
) -> Result<(Vec<u8>, AnnotationXfdfExportReport)> {
    let mut document = annotation_xfdf_document(engine.document())?;
    document
        .annotations
        .sort_by(|a, b| (a.page, &a.id, &a.subtype).cmp(&(b.page, &b.id, &b.subtype)));
    let bytes = write_annotation_xfdf(&document).into_bytes();
    let generated_ids = document
        .annotations
        .iter()
        .filter(|record| record.provenance == "generated_stable_id")
        .count();
    let report = AnnotationXfdfExportReport {
        schema_version: PROMPT17_SCHEMA_VERSION.to_string(),
        annotation_count: document.annotations.len(),
        page_count: engine.page_count()?,
        generated_ids,
        preserved_ids: document.annotations.len().saturating_sub(generated_ids),
        output_bytes: bytes.len(),
        output_sha256: resource_digest(&bytes),
        deterministic: true,
        diagnostics: document.diagnostics,
        exact_limits: vec![
            "rich text exports as bounded plain text; arbitrary XHTML/CSS is not trusted or reproduced"
                .to_string(),
            "unknown extension data is limited to scalar attributes and text under explicit namespaces"
                .to_string(),
            "action targets are inventoried by kind and digest; no action is executed".to_string(),
        ],
    };
    Ok((bytes, report))
}

pub fn parse_annotation_xfdf(bytes: &[u8]) -> Result<AnnotationXfdfDocument> {
    if bytes.len() > MAX_XFDF_BYTES {
        return Err(OxideError::ResourceLimit(format!(
            "annotation XFDF input is {} bytes, exceeding cap {MAX_XFDF_BYTES}",
            bytes.len()
        )));
    }
    let limits = crate::xfa::XfaLimits {
        max_xml_bytes: MAX_XFDF_BYTES,
        max_xml_nodes: 100_000,
        max_xml_attributes: 250_000,
        max_xml_depth: 48,
        max_text_node_bytes: MAX_CUSTOM_VALUE_BYTES,
        max_xml_attribute_value_bytes: MAX_CUSTOM_VALUE_BYTES,
        ..crate::xfa::XfaLimits::default()
    };
    let parsed = parse_xml(bytes, &limits).map_err(|err| {
        OxideError::MalformedPdf(format!("secure annotation XFDF parse failed: {err}"))
    })?;
    if parsed.root.local_name != "xfdf"
        || parsed.root.namespace_uri.as_deref() != Some(XFDF_NAMESPACE)
    {
        return Err(OxideError::MalformedPdf(format!(
            "annotation XFDF root must be {{{XFDF_NAMESPACE}}}xfdf"
        )));
    }
    let mut diagnostics = Vec::new();
    let href = parsed
        .root
        .child("f")
        .and_then(|node| node.attr("href"))
        .map(str::to_string);
    let file_ids = parsed
        .root
        .child("ids")
        .map(|node| {
            ["original", "modified"]
                .into_iter()
                .filter_map(|name| node.attr(name).map(str::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let annots = parsed.root.child("annots").ok_or_else(|| {
        OxideError::MalformedPdf("annotation XFDF has no <annots> element".to_string())
    })?;
    if annots.children.len() > MAX_ANNOTATIONS {
        return Err(OxideError::ResourceLimit(format!(
            "annotation XFDF has {} annotations, exceeding cap {MAX_ANNOTATIONS}",
            annots.children.len()
        )));
    }
    let mut records = Vec::new();
    let mut geometry_values = 0usize;
    for node in &annots.children {
        match parse_annotation_node(node, &mut diagnostics) {
            Ok(Some(record)) => {
                geometry_values = geometry_values
                    .saturating_add(record.vertices.len())
                    .saturating_add(record.quad_points.len())
                    .saturating_add(record.line.len())
                    .saturating_add(record.callout.len())
                    .saturating_add(record.ink_lists.iter().map(Vec::len).sum::<usize>());
                records.push(record);
            }
            Ok(None) => {}
            Err(err) => return Err(err),
        }
    }
    if geometry_values > MAX_GEOMETRY_VALUES {
        return Err(OxideError::ResourceLimit(format!(
            "annotation XFDF geometry has {geometry_values} values, exceeding cap {MAX_GEOMETRY_VALUES}"
        )));
    }
    let mut seen = BTreeSet::new();
    for record in &records {
        if !seen.insert(record.id.clone()) {
            diagnostics.push(
                Prompt17Diagnostic::warning(
                    "xfdf.annotation_id.duplicate",
                    format!("duplicate annotation id '{}'", record.id),
                )
                .with_annotation(&record.id, record.page),
            );
        }
    }
    let relationship_count = records
        .iter()
        .filter(|record| record.reply_to.is_some() || record.popup_for.is_some())
        .count();
    if relationship_count > MAX_RELATIONSHIPS {
        return Err(OxideError::ResourceLimit(format!(
            "annotation XFDF has {relationship_count} relationships, exceeding cap {MAX_RELATIONSHIPS}"
        )));
    }
    Ok(AnnotationXfdfDocument {
        schema_version: PROMPT17_SCHEMA_VERSION.to_string(),
        href,
        file_ids,
        annotations: records,
        diagnostics,
    })
}

fn annotation_xfdf_document(document: &PdfDocument) -> Result<AnnotationXfdfDocument> {
    let reader = document.reader();
    let pages = document.get_pages()?;
    let mut annotations = Vec::new();
    let mut diagnostics = Vec::new();
    let mut ref_to_id = BTreeMap::<(u32, u16), String>::new();
    let mut entries = Vec::<(usize, usize, Option<(u32, u16)>, PdfDictionary)>::new();
    for page in &pages {
        let page_obj = reader.get_and_resolve(page.object_number, page.generation_number)?;
        let Some(page_dict) = page_obj.as_dict() else {
            continue;
        };
        for (index, entry) in annotation_entries(reader, page_dict.get("Annots"))?
            .into_iter()
            .enumerate()
        {
            let (reference, object) = entry;
            let Some(dict) = object.as_dict().cloned() else {
                diagnostics.push(Prompt17Diagnostic::warning(
                    "xfdf.annotation.not_dictionary",
                    format!(
                        "page {} annotation {} is not a dictionary and was skipped",
                        page.page_number, index
                    ),
                ));
                continue;
            };
            let (id, _) = stable_annotation_id(&dict, page.page_number, index, reader);
            if let Some(reference) = reference {
                ref_to_id.insert(reference, id);
            }
            entries.push((page.page_number, index, reference, dict));
        }
    }
    if entries.len() > MAX_ANNOTATIONS {
        return Err(OxideError::ResourceLimit(format!(
            "PDF has {} annotations, exceeding XFDF cap {MAX_ANNOTATIONS}",
            entries.len()
        )));
    }
    for (page, index, _reference, dict) in entries {
        let (id, provenance) = stable_annotation_id(&dict, page, index, reader);
        let subtype = dict.get_name("Subtype").unwrap_or("Unknown").to_string();
        let action = action_inventory(reader, dict.get("A"));
        let reply_to = dict
            .get("IRT")
            .and_then(PdfObject::as_reference)
            .and_then(|reference| ref_to_id.get(&reference).cloned());
        let popup_for = if subtype == "Popup" {
            dict.get("Parent")
                .and_then(PdfObject::as_reference)
                .and_then(|reference| ref_to_id.get(&reference).cloned())
        } else {
            None
        };
        let attachment_name = dict
            .get("FS")
            .and_then(|obj| reader.resolve(obj.clone()).ok())
            .and_then(|obj| obj.as_dict().cloned())
            .and_then(|fs| {
                fs.get("UF")
                    .or_else(|| fs.get("F"))
                    .and_then(pdf_text_or_name)
            });
        let appearance = appearance_metadata(reader, &dict);
        let mut record_diagnostics = Vec::new();
        if subtype == "Unknown" {
            record_diagnostics.push(
                Prompt17Diagnostic::warning(
                    "xfdf.annotation.subtype_unknown",
                    "annotation subtype is missing and is exported as unknown",
                )
                .with_annotation(&id, page),
            );
        }
        let record = AnnotationXfdfRecord {
            id,
            page,
            subtype,
            rect: number_array(reader, dict.get("Rect")).and_then(vec4),
            vertices: number_array(reader, dict.get("Vertices")).unwrap_or_default(),
            ink_lists: nested_number_arrays(reader, dict.get("InkList")).unwrap_or_default(),
            quad_points: number_array(reader, dict.get("QuadPoints")).unwrap_or_default(),
            line: number_array(reader, dict.get("L")).unwrap_or_default(),
            callout: number_array(reader, dict.get("CL")).unwrap_or_default(),
            border: number_array(reader, dict.get("Border")).unwrap_or_default(),
            border_style: resolved_dict(reader, dict.get("BS"))
                .and_then(|style| style.get_name("S").map(str::to_string)),
            border_width: resolved_dict(reader, dict.get("BS"))
                .and_then(|style| style.get("W").and_then(PdfObject::as_number)),
            border_dash: resolved_dict(reader, dict.get("BS"))
                .and_then(|style| number_array(reader, style.get("D")))
                .unwrap_or_default(),
            border_effect: resolved_dict(reader, dict.get("BE"))
                .and_then(|effect| effect.get_name("S").map(str::to_string)),
            border_effect_intensity: resolved_dict(reader, dict.get("BE"))
                .and_then(|effect| effect.get("I").and_then(PdfObject::as_number)),
            color: number_array(reader, dict.get("C")).unwrap_or_default(),
            interior_color: number_array(reader, dict.get("IC")).unwrap_or_default(),
            opacity: dict
                .get("CA")
                .or_else(|| dict.get("ca"))
                .and_then(PdfObject::as_number),
            blend_mode: dict.get_name("BM").map(str::to_string),
            rotation: dict.get_integer("Rotate").or_else(|| dict.get_integer("R")),
            flags: dict.get_integer("F"),
            author: dict.get("T").and_then(pdf_text_or_name),
            title: dict.get("T").and_then(pdf_text_or_name),
            subject: dict.get("Subj").and_then(pdf_text_or_name),
            contents: dict.get("Contents").and_then(pdf_text_or_name),
            safe_rich_text: dict
                .get("RC")
                .and_then(pdf_text_or_name)
                .map(|value| strip_markup_to_text(&value)),
            created: dict
                .get("CreationDate")
                .and_then(pdf_text_or_name)
                .map(|date| normalize_date(&date)),
            modified: dict
                .get("M")
                .and_then(pdf_text_or_name)
                .map(|date| normalize_date(&date)),
            icon: dict.get_name("Name").map(str::to_string),
            intent: dict.get_name("IT").map(str::to_string),
            review_state: dict.get_name("State").map(str::to_string),
            reply_type: dict.get_name("RT").map(str::to_string),
            reply_to,
            popup_for,
            line_endings: dict
                .get("LE")
                .and_then(|obj| reader.resolve(obj.clone()).ok())
                .and_then(|obj| {
                    obj.as_array().map(|items| {
                        items
                            .iter()
                            .filter_map(PdfObject::as_name)
                            .map(str::to_string)
                            .collect()
                    })
                })
                .unwrap_or_default(),
            repeat_overlay: dict.get_bool("Repeat").unwrap_or(false),
            ocg_object: dict
                .get("OC")
                .and_then(PdfObject::as_reference)
                .map(ref_string),
            widget_field: (dict.get_name("Subtype") == Some("Widget"))
                .then(|| field_name(reader, &dict))
                .flatten(),
            attachment_name,
            action,
            appearance,
            custom_data: safe_custom_data(&dict),
            provenance,
            diagnostics: record_diagnostics,
        };
        annotations.push(record);
    }
    let file_ids = reader
        .trailer()
        .get("ID")
        .and_then(PdfObject::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(PdfObject::as_string)
                .map(hex_upper)
                .collect()
        })
        .unwrap_or_default();
    Ok(AnnotationXfdfDocument {
        schema_version: PROMPT17_SCHEMA_VERSION.to_string(),
        href: None,
        file_ids,
        annotations,
        diagnostics,
    })
}

fn write_annotation_xfdf(document: &AnnotationXfdfDocument) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<xfdf xmlns=\"");
    out.push_str(XFDF_NAMESPACE);
    out.push_str("\" xmlns:oxide=\"");
    out.push_str(OXIDE_XFDF_NAMESPACE);
    out.push_str("\" xml:space=\"preserve\">\n");
    if let Some(href) = &document.href {
        out.push_str("  <f href=\"");
        out.push_str(&xml_escape(href));
        out.push_str("\"/>\n");
    }
    if !document.file_ids.is_empty() {
        out.push_str("  <ids");
        if let Some(original) = document.file_ids.first() {
            write_attr(&mut out, "original", original);
        }
        if let Some(modified) = document.file_ids.get(1).or(document.file_ids.first()) {
            write_attr(&mut out, "modified", modified);
        }
        out.push_str("/>\n");
    }
    out.push_str("  <annots>\n");
    for record in &document.annotations {
        let element = xfdf_element_for_subtype(&record.subtype);
        out.push_str("    <");
        out.push_str(element);
        write_attr(&mut out, "name", &record.id);
        write_attr(&mut out, "page", &record.page.saturating_sub(1).to_string());
        write_opt_attr(
            &mut out,
            "rect",
            record.rect.as_ref().map(|v| format_numbers(v)),
        );
        write_opt_attr(&mut out, "title", record.author.clone());
        write_opt_attr(&mut out, "subject", record.subject.clone());
        write_opt_attr(&mut out, "date", record.modified.clone());
        write_opt_attr(&mut out, "creationdate", record.created.clone());
        write_opt_attr(&mut out, "flags", record.flags.map(|v| v.to_string()));
        write_opt_attr(&mut out, "color", nonempty_numbers(&record.color));
        write_opt_attr(
            &mut out,
            "interior-color",
            nonempty_numbers(&record.interior_color),
        );
        write_opt_attr(&mut out, "opacity", record.opacity.map(fmt_number));
        write_opt_attr(&mut out, "rotation", record.rotation.map(|v| v.to_string()));
        write_opt_attr(&mut out, "icon", record.icon.clone());
        write_opt_attr(&mut out, "intent", record.intent.clone());
        write_opt_attr(&mut out, "state", record.review_state.clone());
        write_opt_attr(&mut out, "replyType", record.reply_type.clone());
        write_opt_attr(&mut out, "inreplyto", record.reply_to.clone());
        write_opt_attr(&mut out, "popup-for", record.popup_for.clone());
        write_opt_attr(&mut out, "vertices", nonempty_numbers(&record.vertices));
        write_opt_attr(&mut out, "coords", nonempty_numbers(&record.quad_points));
        write_opt_attr(&mut out, "line", nonempty_numbers(&record.line));
        write_opt_attr(&mut out, "callout", nonempty_numbers(&record.callout));
        write_opt_attr(&mut out, "border", nonempty_numbers(&record.border));
        write_opt_attr(&mut out, "style", record.border_style.clone());
        write_opt_attr(&mut out, "width", record.border_width.map(fmt_number));
        write_opt_attr(&mut out, "dashes", nonempty_numbers(&record.border_dash));
        write_opt_attr(&mut out, "cloudy", record.border_effect.clone());
        write_opt_attr(
            &mut out,
            "intensity",
            record.border_effect_intensity.map(fmt_number),
        );
        write_opt_attr(&mut out, "head", record.line_endings.first().cloned());
        write_opt_attr(&mut out, "tail", record.line_endings.get(1).cloned());
        write_attr(
            &mut out,
            "repeat",
            if record.repeat_overlay {
                "true"
            } else {
                "false"
            },
        );
        write_opt_attr(&mut out, "oxide:subtype", Some(record.subtype.clone()));
        write_opt_attr(&mut out, "oxide:blend-mode", record.blend_mode.clone());
        write_opt_attr(&mut out, "oxide:ocg", record.ocg_object.clone());
        write_opt_attr(&mut out, "oxide:widget-field", record.widget_field.clone());
        write_opt_attr(
            &mut out,
            "oxide:attachment-name",
            record.attachment_name.clone(),
        );
        write_opt_attr(
            &mut out,
            "oxide:provenance",
            Some(record.provenance.clone()),
        );
        for (key, value) in &record.custom_data {
            write_attr(&mut out, &format!("oxide:custom-{key}"), value);
        }
        let has_children = record.contents.is_some()
            || record.safe_rich_text.is_some()
            || !record.ink_lists.is_empty()
            || record.action.is_some();
        if !has_children {
            out.push_str("/>\n");
            continue;
        }
        out.push_str(">\n");
        if let Some(contents) = &record.contents {
            out.push_str("      <contents>");
            out.push_str(&xml_escape(contents));
            out.push_str("</contents>\n");
        }
        if let Some(rich) = &record.safe_rich_text {
            out.push_str(
                "      <contents-richtext><body xmlns=\"http://www.w3.org/1999/xhtml\"><p>",
            );
            out.push_str(&xml_escape(rich));
            out.push_str("</p></body></contents-richtext>\n");
        }
        for stroke in &record.ink_lists {
            out.push_str("      <inklist><gesture>");
            out.push_str(&xml_escape(&format_numbers(stroke)));
            out.push_str("</gesture></inklist>\n");
        }
        if let Some(action) = &record.action {
            out.push_str("      <oxide:action");
            write_attr(&mut out, "kind", &action.kind);
            write_attr(&mut out, "safe", if action.safe { "true" } else { "false" });
            write_opt_attr(&mut out, "target-kind", action.target_kind.clone());
            write_opt_attr(&mut out, "target-sha256", action.target_sha256.clone());
            out.push_str("/>\n");
        }
        out.push_str("    </");
        out.push_str(element);
        out.push_str(">\n");
    }
    out.push_str("  </annots>\n</xfdf>\n");
    out
}

fn parse_annotation_node(
    node: &XmlNode,
    diagnostics: &mut Vec<Prompt17Diagnostic>,
) -> Result<Option<AnnotationXfdfRecord>> {
    let Some(default_subtype) = subtype_for_xfdf_element(&node.local_name) else {
        diagnostics.push(Prompt17Diagnostic::warning(
            "xfdf.annotation.element_unsupported",
            format!(
                "annotation XFDF element <{}> is unsupported and was skipped",
                node.name
            ),
        ));
        return Ok(None);
    };
    let id = node
        .attr("name")
        .or_else(|| node.attr("id"))
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            OxideError::MalformedPdf(format!(
                "annotation XFDF <{}> is missing a stable name/id",
                node.name
            ))
        })?
        .to_string();
    let page_zero = parse_usize_attr(node, "page")?
        .ok_or_else(|| OxideError::MalformedPdf(format!("annotation XFDF '{}' has no page", id)))?;
    let page = page_zero.checked_add(1).ok_or_else(|| {
        OxideError::MalformedPdf(format!("annotation XFDF '{}' page overflows", id))
    })?;
    let subtype = node
        .attr("oxide:subtype")
        .or_else(|| node.attr("subtype"))
        .unwrap_or(default_subtype)
        .to_string();
    let rect = parse_number_attr(node, "rect")?.and_then(vec4);
    if let Some(rect) = rect {
        if !rect.iter().all(|value| value.is_finite()) || rect[0] == rect[2] || rect[1] == rect[3] {
            return Err(OxideError::MalformedPdf(format!(
                "annotation XFDF '{}' has malformed or empty rect",
                id
            )));
        }
    }
    let contents = node.child("contents").map(XmlNode::plain_text);
    let safe_rich_text = node
        .child("contents-richtext")
        .map(XmlNode::plain_text)
        .filter(|value| !value.is_empty());
    let mut ink_lists = Vec::new();
    for inklist in node
        .children
        .iter()
        .filter(|child| child.local_name == "inklist")
    {
        for gesture in inklist
            .children
            .iter()
            .filter(|child| child.local_name == "gesture")
        {
            let values = parse_numbers(&gesture.plain_text(), &id, "ink gesture")?;
            if values.len() % 2 != 0 || values.len() < 4 {
                return Err(OxideError::MalformedPdf(format!(
                    "annotation XFDF '{}' ink gesture must contain point pairs",
                    id
                )));
            }
            ink_lists.push(values);
        }
    }
    let action = node
        .children
        .iter()
        .find(|child| child.local_name == "action")
        .map(|action| AnnotationActionInventory {
            kind: action.attr("kind").unwrap_or("Unknown").to_string(),
            safe: action.attr("safe") == Some("true"),
            target_kind: action.attr("target-kind").map(str::to_string),
            target_sha256: action.attr("target-sha256").map(str::to_string),
        });
    if action.is_some() {
        diagnostics.push(
            Prompt17Diagnostic::warning(
                "xfdf.action.inventory_only",
                "imported XFDF action metadata is inventoried but no action is created or executed",
            )
            .with_annotation(&id, page),
        );
    }
    let line_endings = [node.attr("head"), node.attr("tail")]
        .into_iter()
        .flatten()
        .map(str::to_string)
        .collect();
    let mut custom_data = BTreeMap::new();
    for attribute in &node.attributes {
        if let Some(key) = attribute.name.strip_prefix("oxide:custom-") {
            if custom_data.len() >= MAX_CUSTOM_FIELDS {
                return Err(OxideError::ResourceLimit(format!(
                    "annotation XFDF '{}' custom field count exceeds cap {MAX_CUSTOM_FIELDS}",
                    id
                )));
            }
            custom_data.insert(key.to_string(), attribute.value.clone());
        }
    }
    Ok(Some(AnnotationXfdfRecord {
        id,
        page,
        subtype,
        rect,
        vertices: parse_number_attr(node, "vertices")?.unwrap_or_default(),
        ink_lists,
        quad_points: parse_number_attr(node, "coords")?.unwrap_or_default(),
        line: parse_number_attr(node, "line")?.unwrap_or_default(),
        callout: parse_number_attr(node, "callout")?.unwrap_or_default(),
        border: parse_number_attr(node, "border")?.unwrap_or_default(),
        border_style: node.attr("style").map(str::to_string),
        border_width: parse_f64_attr(node, "width")?,
        border_dash: parse_number_attr(node, "dashes")?.unwrap_or_default(),
        border_effect: node.attr("cloudy").map(str::to_string),
        border_effect_intensity: parse_f64_attr(node, "intensity")?,
        color: parse_number_attr(node, "color")?.unwrap_or_default(),
        interior_color: parse_number_attr(node, "interior-color")?.unwrap_or_default(),
        opacity: parse_f64_attr(node, "opacity")?,
        blend_mode: node.attr("oxide:blend-mode").map(str::to_string),
        rotation: parse_i64_attr(node, "rotation")?,
        flags: parse_i64_attr(node, "flags")?,
        author: node.attr("title").map(str::to_string),
        title: node.attr("title").map(str::to_string),
        subject: node.attr("subject").map(str::to_string),
        contents,
        safe_rich_text,
        created: node.attr("creationdate").map(normalize_date),
        modified: node.attr("date").map(normalize_date),
        icon: node.attr("icon").map(str::to_string),
        intent: node.attr("intent").map(str::to_string),
        review_state: node.attr("state").map(str::to_string),
        reply_type: node.attr("replyType").map(str::to_string),
        reply_to: node.attr("inreplyto").map(str::to_string),
        popup_for: node.attr("popup-for").map(str::to_string),
        line_endings,
        repeat_overlay: matches!(node.attr("repeat"), Some("true" | "1")),
        ocg_object: node.attr("oxide:ocg").map(str::to_string),
        widget_field: node.attr("oxide:widget-field").map(str::to_string),
        attachment_name: node.attr("oxide:attachment-name").map(str::to_string),
        action,
        appearance: AnnotationAppearanceMetadata {
            generation_posture: "regenerate_by_import_policy".to_string(),
            ..AnnotationAppearanceMetadata::default()
        },
        custom_data,
        provenance: node
            .attr("oxide:provenance")
            .unwrap_or("xfdf_import")
            .to_string(),
        diagnostics: Vec::new(),
    }))
}

fn annotation_entries(
    reader: &PdfReader,
    annots: Option<&PdfObject>,
) -> Result<Vec<AnnotationObjectEntry>> {
    let Some(annots) = annots else {
        return Ok(Vec::new());
    };
    let resolved = reader.resolve(annots.clone())?;
    let Some(items) = resolved.as_array() else {
        return Ok(Vec::new());
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let reference = item.as_reference();
        out.push((reference, reader.resolve(item.clone())?));
    }
    Ok(out)
}

fn stable_annotation_id(
    dict: &PdfDictionary,
    page: usize,
    index: usize,
    reader: &PdfReader,
) -> (String, String) {
    if let Some(id) = dict
        .get("NM")
        .and_then(pdf_text_or_name)
        .filter(|id| !id.trim().is_empty())
    {
        return (id, "pdf_nm_preserved".to_string());
    }
    let subtype = dict.get_name("Subtype").unwrap_or("Unknown");
    let rect = number_array(reader, dict.get("Rect"))
        .map(|values| format_numbers(&values))
        .unwrap_or_default();
    let contents = dict
        .get("Contents")
        .and_then(pdf_text_or_name)
        .unwrap_or_default();
    let seed = format!("p{page}|a{index}|{subtype}|{rect}|{contents}");
    let digest = resource_digest(seed.as_bytes());
    (
        format!("oxide-p17-p{page}-a{index}-{}", &digest[..12]),
        "generated_stable_id".to_string(),
    )
}

fn appearance_metadata(reader: &PdfReader, dict: &PdfDictionary) -> AnnotationAppearanceMetadata {
    let ap = dict
        .get("AP")
        .and_then(|obj| reader.resolve(obj.clone()).ok())
        .and_then(|obj| obj.as_dict().cloned());
    AnnotationAppearanceMetadata {
        has_normal: ap.as_ref().is_some_and(|ap| ap.get("N").is_some()),
        has_rollover: ap.as_ref().is_some_and(|ap| ap.get("R").is_some()),
        has_down: ap.as_ref().is_some_and(|ap| ap.get("D").is_some()),
        selected_state: dict.get_name("AS").map(str::to_string),
        ocg_associated: dict.get("OC").is_some(),
        generation_posture: if ap.is_some() {
            "preserve_valid_static_appearance".to_string()
        } else {
            "missing_appearance_reported".to_string()
        },
    }
}

fn action_inventory(
    reader: &PdfReader,
    action: Option<&PdfObject>,
) -> Option<AnnotationActionInventory> {
    let action = reader.resolve(action?.clone()).ok()?;
    let dict = action.as_dict()?;
    let kind = dict.get_name("S").unwrap_or("Unknown").to_string();
    let safe = matches!(kind.as_str(), "GoTo");
    let (target_kind, target) = if let Some(uri) = dict.get("URI").and_then(pdf_text_or_name) {
        (Some("external_uri".to_string()), Some(uri))
    } else if let Some(file) = dict.get("F").and_then(pdf_text_or_name) {
        (Some("file_specification".to_string()), Some(file))
    } else {
        (None, None)
    };
    Some(AnnotationActionInventory {
        kind,
        safe,
        target_kind,
        target_sha256: target.map(|value| resource_digest(value.as_bytes())),
    })
}

fn field_name(reader: &PdfReader, dict: &PdfDictionary) -> Option<String> {
    let mut parts = Vec::new();
    let mut current = Some(PdfObject::Dictionary(dict.clone()));
    let mut seen = BTreeSet::new();
    for _ in 0..32 {
        let object = reader.resolve(current?).ok()?;
        let field = object.as_dict()?;
        if let Some(name) = field.get("T").and_then(pdf_text_or_name) {
            parts.push(name);
        }
        let Some(parent) = field.get("Parent").cloned() else {
            break;
        };
        if let Some(reference) = parent.as_reference() {
            if !seen.insert(reference) {
                break;
            }
        }
        current = Some(parent);
    }
    parts.reverse();
    (!parts.is_empty()).then(|| parts.join("."))
}

fn safe_custom_data(dict: &PdfDictionary) -> BTreeMap<String, String> {
    const STANDARD: &[&str] = &[
        "Type",
        "Subtype",
        "Rect",
        "Contents",
        "P",
        "NM",
        "M",
        "F",
        "AP",
        "AS",
        "Border",
        "C",
        "StructParent",
        "OC",
        "T",
        "Subj",
        "CreationDate",
        "CA",
        "ca",
        "BM",
        "RC",
        "DA",
        "Q",
        "L",
        "CL",
        "Vertices",
        "InkList",
        "QuadPoints",
        "LE",
        "IT",
        "State",
        "RT",
        "IRT",
        "Popup",
        "Parent",
        "FS",
        "A",
        "AA",
        "MK",
        "FT",
        "V",
        "R",
        "Rotate",
        "IC",
        "Name",
    ];
    let mut out = BTreeMap::new();
    for (key, value) in dict.entries() {
        if out.len() >= MAX_CUSTOM_FIELDS || STANDARD.contains(&key.as_str()) {
            continue;
        }
        let scalar = match value {
            PdfObject::Boolean(value) => Some(value.to_string()),
            PdfObject::Integer(value) => Some(value.to_string()),
            PdfObject::Real(value) => Some(fmt_number(*value)),
            PdfObject::Name(value) => Some(value.clone()),
            PdfObject::String(value) if value.len() <= MAX_CUSTOM_VALUE_BYTES => {
                Some(decode_pdf_text_string(value))
            }
            _ => None,
        };
        if let Some(value) = scalar {
            out.insert(sanitize_xml_name(key), value);
        }
    }
    out
}

fn number_array(reader: &PdfReader, object: Option<&PdfObject>) -> Option<Vec<f64>> {
    let object = reader.resolve(object?.clone()).ok()?;
    object.as_array().map(|values| {
        values
            .iter()
            .filter_map(|value| reader.resolve(value.clone()).ok()?.as_number())
            .filter(|value| value.is_finite())
            .collect()
    })
}

fn resolved_dict(reader: &PdfReader, object: Option<&PdfObject>) -> Option<PdfDictionary> {
    reader.resolve(object?.clone()).ok()?.as_dict().cloned()
}

fn nested_number_arrays(reader: &PdfReader, object: Option<&PdfObject>) -> Option<Vec<Vec<f64>>> {
    let object = reader.resolve(object?.clone()).ok()?;
    let arrays = object.as_array()?;
    Some(
        arrays
            .iter()
            .filter_map(|value| {
                let value = reader.resolve(value.clone()).ok()?;
                value.as_array().map(|numbers| {
                    numbers
                        .iter()
                        .filter_map(|number| reader.resolve(number.clone()).ok()?.as_number())
                        .filter(|number| number.is_finite())
                        .collect::<Vec<_>>()
                })
            })
            .collect(),
    )
}

fn pdf_text_or_name(object: &PdfObject) -> Option<String> {
    match object {
        PdfObject::String(bytes) => Some(decode_pdf_text_string(bytes)),
        PdfObject::Name(name) => Some(name.clone()),
        _ => None,
    }
}

fn parse_number_attr(node: &XmlNode, name: &str) -> Result<Option<Vec<f64>>> {
    node.attr(name)
        .map(|value| parse_numbers(value, node.attr("name").unwrap_or("unknown"), name))
        .transpose()
}

fn parse_numbers(value: &str, id: &str, field: &str) -> Result<Vec<f64>> {
    let values = value
        .split(|ch: char| ch == ',' || ch == ';' || ch.is_ascii_whitespace())
        .filter(|part| !part.is_empty())
        .map(|part| {
            part.parse::<f64>().map_err(|_| {
                OxideError::MalformedPdf(format!(
                    "annotation XFDF '{}' has invalid number '{}' in {}",
                    id, part, field
                ))
            })
        })
        .collect::<Result<Vec<_>>>()?;
    if values.iter().any(|value| !value.is_finite()) {
        return Err(OxideError::MalformedPdf(format!(
            "annotation XFDF '{}' has non-finite geometry in {}",
            id, field
        )));
    }
    Ok(values)
}

fn parse_usize_attr(node: &XmlNode, name: &str) -> Result<Option<usize>> {
    node.attr(name)
        .map(|value| {
            value.parse::<usize>().map_err(|_| {
                OxideError::MalformedPdf(format!(
                    "annotation XFDF '{}' has invalid {}",
                    node.attr("name").unwrap_or("unknown"),
                    name
                ))
            })
        })
        .transpose()
}

fn parse_i64_attr(node: &XmlNode, name: &str) -> Result<Option<i64>> {
    node.attr(name)
        .map(|value| {
            value.parse::<i64>().map_err(|_| {
                OxideError::MalformedPdf(format!(
                    "annotation XFDF '{}' has invalid {}",
                    node.attr("name").unwrap_or("unknown"),
                    name
                ))
            })
        })
        .transpose()
}

fn parse_f64_attr(node: &XmlNode, name: &str) -> Result<Option<f64>> {
    node.attr(name)
        .map(|value| {
            value
                .parse::<f64>()
                .ok()
                .filter(|number| number.is_finite())
                .ok_or_else(|| {
                    OxideError::MalformedPdf(format!(
                        "annotation XFDF '{}' has invalid {}",
                        node.attr("name").unwrap_or("unknown"),
                        name
                    ))
                })
        })
        .transpose()
}

fn subtype_for_xfdf_element(element: &str) -> Option<&'static str> {
    Some(match element.to_ascii_lowercase().as_str() {
        "text" => "Text",
        "freetext" => "FreeText",
        "line" => "Line",
        "square" => "Square",
        "circle" => "Circle",
        "polygon" => "Polygon",
        "polyline" => "PolyLine",
        "highlight" => "Highlight",
        "underline" => "Underline",
        "squiggly" => "Squiggly",
        "strikeout" => "StrikeOut",
        "stamp" => "Stamp",
        "caret" => "Caret",
        "ink" => "Ink",
        "popup" => "Popup",
        "fileattachment" => "FileAttachment",
        "sound" => "Sound",
        "movie" => "Movie",
        "screen" => "Screen",
        "widget" => "Widget",
        "printermark" => "PrinterMark",
        "trapnet" => "TrapNet",
        "watermark" => "Watermark",
        "threed" | "3d" => "3D",
        "redact" => "Redact",
        "richmedia" => "RichMedia",
        "link" => "Link",
        "unknown" => "Unknown",
        _ => return None,
    })
}

fn xfdf_element_for_subtype(subtype: &str) -> &'static str {
    match subtype {
        "Text" => "text",
        "FreeText" => "freetext",
        "Line" => "line",
        "Square" => "square",
        "Circle" => "circle",
        "Polygon" => "polygon",
        "PolyLine" => "polyline",
        "Highlight" => "highlight",
        "Underline" => "underline",
        "Squiggly" => "squiggly",
        "StrikeOut" => "strikeout",
        "Stamp" => "stamp",
        "Caret" => "caret",
        "Ink" => "ink",
        "Popup" => "popup",
        "FileAttachment" => "fileattachment",
        "Sound" => "sound",
        "Movie" => "movie",
        "Screen" => "screen",
        "Widget" => "widget",
        "PrinterMark" => "printermark",
        "TrapNet" => "trapnet",
        "Watermark" => "watermark",
        "3D" => "threed",
        "Redact" => "redact",
        "RichMedia" => "richmedia",
        "Link" => "link",
        _ => "unknown",
    }
}

fn normalize_date(value: &str) -> String {
    let trimmed = value.trim();
    if let Some(pdf) = trimmed.strip_prefix("D:") {
        let digits: String = pdf.chars().take_while(|ch| ch.is_ascii_digit()).collect();
        if digits.len() >= 8 {
            let year = &digits[0..4];
            let month = &digits[4..6];
            let day = &digits[6..8];
            let hour = digits.get(8..10).unwrap_or("00");
            let minute = digits.get(10..12).unwrap_or("00");
            let second = digits.get(12..14).unwrap_or("00");
            return format!("{year}-{month}-{day}T{hour}:{minute}:{second}Z");
        }
    }
    trimmed.to_string()
}

fn strip_markup_to_text(value: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for ch in value.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn vec4(values: Vec<f64>) -> Option<[f64; 4]> {
    (values.len() == 4).then(|| [values[0], values[1], values[2], values[3]])
}

fn ref_string(reference: (u32, u16)) -> String {
    format!("{} {} R", reference.0, reference.1)
}

fn hex_upper(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02X}")).collect()
}

fn nonempty_numbers(values: &[f64]) -> Option<String> {
    (!values.is_empty()).then(|| format_numbers(values))
}

fn format_numbers(values: &[f64]) -> String {
    values
        .iter()
        .map(|value| fmt_number(*value))
        .collect::<Vec<_>>()
        .join(",")
}

fn fmt_number(value: f64) -> String {
    let mut text = format!("{value:.6}");
    while text.contains('.') && text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    if text == "-0" {
        "0".to_string()
    } else {
        text
    }
}

fn write_attr(out: &mut String, name: &str, value: &str) {
    out.push(' ');
    out.push_str(name);
    out.push_str("=\"");
    out.push_str(&xml_escape(value));
    out.push('"');
}

fn write_opt_attr(out: &mut String, name: &str, value: Option<String>) {
    if let Some(value) = value.filter(|value| !value.is_empty()) {
        write_attr(out, name, &value);
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
            ch if ch == '\n' || ch == '\r' || ch == '\t' || !ch.is_control() => out.push(ch),
            _ => out.push('\u{FFFD}'),
        }
    }
    out
}

fn sanitize_xml_name(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotationAppearanceOptions {
    pub policy: AnnotationAppearancePolicy,
    pub flatten_after_generation: bool,
    pub placeholder_for_unsupported: bool,
    pub deterministic: bool,
}

impl Default for AnnotationAppearanceOptions {
    fn default() -> Self {
        Self {
            policy: AnnotationAppearancePolicy::RegenerateMissingOrMalformed,
            flatten_after_generation: false,
            placeholder_for_unsupported: false,
            deterministic: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotationAppearanceRow {
    pub annotation_id: String,
    pub page: usize,
    pub subtype: String,
    pub previous_appearance: String,
    pub result: String,
    pub generated_states: Vec<String>,
    pub deterministic_resource_name: Option<String>,
    pub signature_impact: String,
    pub exact_limit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotationAppearanceReport {
    pub schema_version: String,
    pub policy: AnnotationAppearancePolicy,
    pub inspected: usize,
    pub generated: usize,
    pub preserved: usize,
    pub unsupported_reported: usize,
    pub malformed: usize,
    pub rows: Vec<AnnotationAppearanceRow>,
    pub output_bytes: usize,
    pub output_sha256: String,
    pub deterministic: bool,
    pub signature_impact: String,
    pub diagnostics: Vec<Prompt17Diagnostic>,
    pub exact_limits: Vec<String>,
}

pub fn import_annotation_xfdf_pdf(
    input: &[u8],
    xfdf: &[u8],
    options: &AnnotationXfdfImportOptions,
) -> Result<(Vec<u8>, AnnotationXfdfImportReport)> {
    let mut imported = parse_annotation_xfdf(xfdf)?;
    imported
        .annotations
        .sort_by(|a, b| (a.page, &a.id).cmp(&(b.page, &b.id)));
    let document = PdfDocument::open_bytes(input.to_vec())?;
    let reader = document.reader();
    let pages = document.get_pages()?;
    let remap = source_object_remap(reader);
    let page_output: BTreeMap<usize, u32> = pages
        .iter()
        .filter_map(|page| {
            remap
                .get(&page.object_number)
                .copied()
                .map(|number| (page.page_number, number))
        })
        .collect();
    let mut existing_by_id = BTreeMap::<String, ExistingAnnotation>::new();
    let mut diagnostics = imported.diagnostics.clone();
    for page in &pages {
        let page_obj = reader.get_and_resolve(page.object_number, page.generation_number)?;
        let Some(page_dict) = page_obj.as_dict() else {
            continue;
        };
        for (index, (reference, object)) in annotation_entries(reader, page_dict.get("Annots"))?
            .into_iter()
            .enumerate()
        {
            let Some(dict) = object.as_dict() else {
                continue;
            };
            let (id, _) = stable_annotation_id(dict, page.page_number, index, reader);
            let Some((source, _generation)) = reference else {
                diagnostics.push(
                    Prompt17Diagnostic::warning(
                        "xfdf.import.direct_annotation_limited",
                        "direct annotation dictionaries are exported but create/update matching is limited to indirect annotations",
                    )
                    .with_annotation(&id, page.page_number),
                );
                continue;
            };
            let output_number = remap.get(&source).copied().unwrap_or(source);
            existing_by_id
                .entry(id.clone())
                .or_insert(ExistingAnnotation {
                    id,
                    page: page.page_number,
                    source_number: source,
                    output_number,
                    has_valid_appearance: normal_appearance_is_valid(reader, dict),
                });
        }
    }
    let mut duplicate_ids = Vec::new();
    let mut unique = BTreeMap::<String, AnnotationXfdfRecord>::new();
    for record in imported.annotations {
        if record.page == 0 || record.page > pages.len() {
            return Err(OxideError::MalformedPdf(format!(
                "annotation XFDF '{}' maps to page {}, but the PDF has {} pages",
                record.id,
                record.page,
                pages.len()
            )));
        }
        if unique.contains_key(&record.id) {
            duplicate_ids.push(record.id.clone());
            if options.conflict_policy == AnnotationConflictPolicy::Reject {
                return Err(OxideError::MalformedPdf(format!(
                    "annotation XFDF contains duplicate id '{}'",
                    record.id
                )));
            }
            continue;
        }
        unique.insert(record.id.clone(), record);
    }
    let deletes: BTreeSet<String> = if options.delete_policy == AnnotationDeletePolicy::ExplicitIds
    {
        options.delete_ids.iter().cloned().collect()
    } else {
        BTreeSet::new()
    };
    let mut updates = BTreeMap::<u32, AnnotationXfdfRecord>::new();
    let mut creates = Vec::<AnnotationXfdfRecord>::new();
    let mut unchanged = 0usize;
    let mut unsupported = 0usize;
    for (_, record) in unique {
        if deletes.contains(&record.id) {
            continue;
        }
        if let Some(existing) = existing_by_id.get(&record.id) {
            if existing.page != record.page
                && options.conflict_policy == AnnotationConflictPolicy::Reject
            {
                return Err(OxideError::MalformedPdf(format!(
                    "annotation XFDF '{}' page conflicts with the existing annotation",
                    record.id
                )));
            }
            updates.insert(existing.source_number, record);
        } else if record.subtype == "Widget" {
            unsupported += 1;
            diagnostics.push(
                Prompt17Diagnostic::warning(
                    "xfdf.import.widget_create_unsupported",
                    "standalone Widget creation is rejected because field-tree semantics must remain canonical",
                )
                .with_annotation(&record.id, record.page),
            );
            if options.fail_on_unsupported {
                return Err(OxideError::UnsupportedFeature(
                    "annotation XFDF cannot create a standalone Widget without an AcroForm field"
                        .to_string(),
                ));
            }
        } else {
            creates.push(record);
        }
    }
    for id in &deletes {
        if !existing_by_id.contains_key(id) {
            unchanged += 1;
        }
    }

    let mut next_number = remap.values().copied().max().unwrap_or(0).saturating_add(1);
    let mut created_numbers = BTreeMap::<String, u32>::new();
    for record in &creates {
        created_numbers.insert(record.id.clone(), next_number);
        next_number = next_number.saturating_add(1);
    }
    let mut appearance_numbers = BTreeMap::<String, u32>::new();
    for record in creates.iter().chain(updates.values()) {
        let should_generate = match options.appearance_policy {
            AnnotationAppearancePolicy::PreserveValid => false,
            AnnotationAppearancePolicy::RegenerateAllSupported => true,
            AnnotationAppearancePolicy::RegenerateMissingOrMalformed => existing_by_id
                .get(&record.id)
                .is_none_or(|existing| !existing.has_valid_appearance),
        };
        if should_generate && annotation_appearance_stream(record, false).is_some() {
            appearance_numbers.insert(record.id.clone(), next_number);
            next_number = next_number.saturating_add(1);
        }
    }
    let mut id_to_output = BTreeMap::<String, u32>::new();
    for existing in existing_by_id.values() {
        id_to_output.insert(existing.id.clone(), existing.output_number);
    }
    id_to_output.extend(created_numbers.clone());

    let deleted_source_numbers: BTreeSet<u32> = deletes
        .iter()
        .filter_map(|id| existing_by_id.get(id).map(|entry| entry.source_number))
        .collect();
    let deleted_output_numbers: BTreeSet<u32> = deletes
        .iter()
        .filter_map(|id| existing_by_id.get(id).map(|entry| entry.output_number))
        .collect();
    let new_by_page: BTreeMap<usize, Vec<u32>> = {
        let mut map = BTreeMap::<usize, Vec<u32>>::new();
        for record in &creates {
            if let Some(number) = created_numbers.get(&record.id) {
                map.entry(record.page).or_default().push(*number);
            }
        }
        map
    };
    let page_source_to_number: BTreeMap<u32, usize> = pages
        .iter()
        .map(|page| (page.object_number, page.page_number))
        .collect();
    let mut mutate = |source_number: u32, object: &mut PdfObject| {
        if let Some(record) = updates.get(&source_number) {
            if let PdfObject::Dictionary(dict) = object {
                apply_record_to_annotation_dict(
                    dict,
                    record,
                    &id_to_output,
                    page_output.get(&record.page).copied(),
                );
                if let Some(ap_number) = appearance_numbers.get(&record.id) {
                    install_appearance_reference(dict, *ap_number);
                }
            }
        }
        if let Some(page_number) = page_source_to_number.get(&source_number).copied() {
            if let PdfObject::Dictionary(dict) = object {
                let mut annots = dict
                    .get("Annots")
                    .and_then(PdfObject::as_array)
                    .map(<[PdfObject]>::to_vec)
                    .unwrap_or_default();
                annots.retain(|value| {
                    value
                        .as_reference()
                        .is_none_or(|(number, _)| !deleted_output_numbers.contains(&number))
                });
                if let Some(additions) = new_by_page.get(&page_number) {
                    annots.extend(additions.iter().map(|number| PdfObject::Reference {
                        number: *number,
                        generation: 0,
                    }));
                }
                annots.sort_by_key(|object| object.as_reference().map(|reference| reference.0));
                if annots.is_empty() {
                    dict.remove("Annots");
                } else {
                    dict.insert("Annots", PdfObject::Array(annots));
                }
            }
        }
        if deleted_source_numbers.contains(&source_number) {
            *object = PdfObject::Null;
        }
    };
    let (mut objects, root, info) = rewrite_document_objects(reader, &mut mutate)?;
    for record in &creates {
        let number = created_numbers[&record.id];
        let mut dict = annotation_dictionary_from_record(
            record,
            &id_to_output,
            page_output.get(&record.page).copied(),
        );
        if let Some(ap_number) = appearance_numbers.get(&record.id) {
            install_appearance_reference(&mut dict, *ap_number);
        }
        objects.push(OutputObject {
            number,
            object: PdfObject::Dictionary(dict),
        });
    }
    for record in creates.iter().chain(updates.values()) {
        if let Some(number) = appearance_numbers.get(&record.id) {
            if let Some(stream) = annotation_appearance_stream(record, false) {
                objects.push(OutputObject {
                    number: *number,
                    object: stream,
                });
            }
        }
    }
    objects.sort_by_key(|object| object.number);
    let output = PdfWriter::new(objects, root)
        .with_info(info)
        .with_id(reader.first_file_id())
        .with_mode(WriterMode::XrefStreamWithObjStm)
        .write()?;
    ContentEngine::open_bytes(output.clone())?;
    let signature_impact = signature_impact(input);
    let relationship_count = creates
        .iter()
        .chain(updates.values())
        .filter(|record| record.reply_to.is_some() || record.popup_for.is_some())
        .count();
    let report = AnnotationXfdfImportReport {
        schema_version: PROMPT17_SCHEMA_VERSION.to_string(),
        imported_annotations: creates.len() + updates.len(),
        created: creates.len(),
        updated: updates.len(),
        deleted: deletes
            .iter()
            .filter(|id| existing_by_id.contains_key(*id))
            .count(),
        unchanged,
        unsupported,
        duplicate_ids,
        relationship_count,
        appearances_regenerated: appearance_numbers.len(),
        output_bytes: output.len(),
        output_sha256: resource_digest(&output),
        deterministic: options.deterministic,
        signature_impact,
        diagnostics,
        exact_limits: vec![
            "action metadata is inventory-only on import; URI, Launch, JavaScript, Rendition, and media activation actions are never created"
                .to_string(),
            "new standalone Widget annotations are rejected; update an existing canonical field/widget instead"
                .to_string(),
            "file attachment payloads are not imported from XFDF; scalar attachment metadata is retained for diagnostics"
                .to_string(),
        ],
    };
    Ok((output, report))
}

pub fn generate_annotation_appearances_pdf(
    input: &[u8],
    options: &AnnotationAppearanceOptions,
) -> Result<(Vec<u8>, AnnotationAppearanceReport)> {
    let document = PdfDocument::open_bytes(input.to_vec())?;
    let reader = document.reader();
    let xfdf = annotation_xfdf_document(&document)?;
    let remap = source_object_remap(reader);
    let mut source_by_id = BTreeMap::<String, u32>::new();
    for page in document.get_pages()? {
        let page_obj = reader.get_and_resolve(page.object_number, page.generation_number)?;
        let Some(page_dict) = page_obj.as_dict() else {
            continue;
        };
        for (index, (reference, object)) in annotation_entries(reader, page_dict.get("Annots"))?
            .into_iter()
            .enumerate()
        {
            let Some(reference) = reference else {
                continue;
            };
            let Some(dict) = object.as_dict() else {
                continue;
            };
            let (id, _) = stable_annotation_id(dict, page.page_number, index, reader);
            source_by_id.insert(id, reference.0);
        }
    }
    let mut next = remap.values().copied().max().unwrap_or(0).saturating_add(1);
    let mut plans = BTreeMap::<u32, (AnnotationXfdfRecord, u32)>::new();
    let mut rows = Vec::new();
    let mut preserved = 0usize;
    let mut unsupported = 0usize;
    for record in &xfdf.annotations {
        let previous = if record.appearance.has_normal {
            "valid_or_present"
        } else {
            "missing_or_malformed"
        };
        let should_generate = match options.policy {
            AnnotationAppearancePolicy::PreserveValid => !record.appearance.has_normal,
            AnnotationAppearancePolicy::RegenerateMissingOrMalformed => {
                !record.appearance.has_normal
            }
            AnnotationAppearancePolicy::RegenerateAllSupported => true,
        };
        let supported =
            annotation_appearance_stream(record, options.placeholder_for_unsupported).is_some();
        let result = if should_generate && supported {
            if let Some(source) = source_by_id.get(&record.id) {
                plans.insert(*source, (record.clone(), next));
                next = next.saturating_add(1);
                "generated"
            } else {
                unsupported += 1;
                "unsupported_direct_annotation"
            }
        } else if supported {
            preserved += 1;
            "preserved_valid"
        } else {
            unsupported += 1;
            "unsupported_reported_exact"
        };
        rows.push(AnnotationAppearanceRow {
            annotation_id: record.id.clone(),
            page: record.page,
            subtype: record.subtype.clone(),
            previous_appearance: previous.to_string(),
            result: result.to_string(),
            generated_states: if result == "generated" {
                vec!["N".to_string(), "R".to_string(), "D".to_string()]
            } else {
                Vec::new()
            },
            deterministic_resource_name: (result == "generated")
                .then(|| format!("OxP17AP{}", record.id.chars().take(12).collect::<String>())),
            signature_impact: if result == "generated" {
                "full_rewrite_invalidates_prior_byte_range_signatures"
            } else {
                "no_mutation_for_row"
            }
            .to_string(),
            exact_limit: (!supported).then(|| unsupported_appearance_limit(&record.subtype)),
        });
    }
    let mut mutate = |source_number: u32, object: &mut PdfObject| {
        if let Some((_, ap_number)) = plans.get(&source_number) {
            if let PdfObject::Dictionary(dict) = object {
                install_appearance_reference(dict, *ap_number);
            }
        }
    };
    let (mut objects, root, info) = rewrite_document_objects(reader, &mut mutate)?;
    for (record, number) in plans.values() {
        if let Some(stream) =
            annotation_appearance_stream(record, options.placeholder_for_unsupported)
        {
            objects.push(OutputObject {
                number: *number,
                object: stream,
            });
        }
    }
    objects.sort_by_key(|object| object.number);
    let generated_output = PdfWriter::new(objects, root)
        .with_info(info)
        .with_id(reader.first_file_id())
        .with_mode(WriterMode::XrefStreamWithObjStm)
        .write()?;
    let output = if options.flatten_after_generation {
        let mut editor = PdfEditor::open_bytes(generated_output)?;
        editor.flatten_annotations();
        editor.save_to_bytes(EditMode::FullRewrite)?
    } else {
        generated_output
    };
    ContentEngine::open_bytes(output.clone())?;
    let report = AnnotationAppearanceReport {
        schema_version: PROMPT17_SCHEMA_VERSION.to_string(),
        policy: options.policy.clone(),
        inspected: rows.len(),
        generated: plans.len(),
        preserved,
        unsupported_reported: unsupported,
        malformed: rows
            .iter()
            .filter(|row| row.previous_appearance == "missing_or_malformed")
            .count(),
        rows,
        output_bytes: output.len(),
        output_sha256: resource_digest(&output),
        deterministic: options.deterministic,
        signature_impact: signature_impact(input),
        diagnostics: xfdf.diagnostics,
        exact_limits: vec![
            "FreeText rich content is rendered from sanitized plain text with Helvetica/WinAnsi; full CSS, CJK fallback embedding, and bidi shaping remain exact reported limits"
                .to_string(),
            "cloudy borders use a deterministic bounded scallop approximation rather than Acrobat-private geometry"
                .to_string(),
            "PrinterMark, TrapNet, Watermark, 3D, RichMedia, Movie, Sound, Screen, and unknown subtypes preserve valid static AP; generated placeholders require explicit policy"
                .to_string(),
        ],
    };
    Ok((output, report))
}

#[derive(Clone)]
struct ExistingAnnotation {
    id: String,
    page: usize,
    source_number: u32,
    output_number: u32,
    has_valid_appearance: bool,
}

fn source_object_remap(reader: &PdfReader) -> BTreeMap<u32, u32> {
    let mut remap = BTreeMap::new();
    let mut next = 1u32;
    for (number, _) in reader.object_ids() {
        remap.entry(number).or_insert_with(|| {
            let assigned = next;
            next = next.saturating_add(1);
            assigned
        });
    }
    remap
}

fn normal_appearance_is_valid(reader: &PdfReader, dict: &PdfDictionary) -> bool {
    let Some(ap) = dict
        .get("AP")
        .and_then(|obj| reader.resolve(obj.clone()).ok())
    else {
        return false;
    };
    let Some(ap) = ap.as_dict() else {
        return false;
    };
    let Some(normal) = ap.get("N").and_then(|obj| reader.resolve(obj.clone()).ok()) else {
        return false;
    };
    match normal {
        PdfObject::Stream { dict, raw } => {
            dict.get_name("Subtype") == Some("Form")
                && valid_bbox(&dict)
                && raw.len() <= 32 * 1024 * 1024
        }
        PdfObject::Dictionary(states) => states.entries().any(|(_, state)| {
            reader.resolve(state.clone()).ok().is_some_and(|object| {
                matches!(object, PdfObject::Stream { ref dict, ref raw }
                    if dict.get_name("Subtype") == Some("Form")
                        && valid_bbox(dict)
                        && raw.len() <= 32 * 1024 * 1024)
            })
        }),
        _ => false,
    }
}

fn valid_bbox(dict: &PdfDictionary) -> bool {
    dict.get("BBox")
        .and_then(PdfObject::as_array)
        .is_some_and(|values| {
            values.len() == 4
                && values
                    .iter()
                    .all(|value| value.as_number().is_some_and(f64::is_finite))
        })
}

fn install_appearance_reference(dict: &mut PdfDictionary, number: u32) {
    let reference = PdfObject::Reference {
        number,
        generation: 0,
    };
    let mut ap = PdfDictionary::empty();
    ap.insert("N", reference.clone());
    ap.insert("R", reference.clone());
    ap.insert("D", reference);
    dict.insert("AP", PdfObject::Dictionary(ap));
    if dict.get_name("Subtype") == Some("Widget") && dict.get("AS").is_none() {
        dict.insert("AS", PdfObject::Name("Off".to_string()));
    }
}

fn annotation_dictionary_from_record(
    record: &AnnotationXfdfRecord,
    id_to_output: &BTreeMap<String, u32>,
    page_output: Option<u32>,
) -> PdfDictionary {
    let mut dict = PdfDictionary::empty();
    dict.insert("Type", PdfObject::Name("Annot".to_string()));
    dict.insert("Subtype", PdfObject::Name(record.subtype.clone()));
    apply_record_to_annotation_dict(&mut dict, record, id_to_output, page_output);
    dict
}

fn apply_record_to_annotation_dict(
    dict: &mut PdfDictionary,
    record: &AnnotationXfdfRecord,
    id_to_output: &BTreeMap<String, u32>,
    page_output: Option<u32>,
) {
    dict.insert("NM", pdf_text_string(&record.id));
    if let Some(page) = page_output {
        dict.insert(
            "P",
            PdfObject::Reference {
                number: page,
                generation: 0,
            },
        );
    }
    if let Some(rect) = record.rect {
        dict.insert("Rect", pdf_number_array(&rect));
    }
    set_number_array(dict, "Vertices", &record.vertices);
    set_number_array(dict, "QuadPoints", &record.quad_points);
    set_number_array(dict, "L", &record.line);
    set_number_array(dict, "CL", &record.callout);
    set_number_array(dict, "Border", &record.border);
    if record.border_style.is_some()
        || record.border_width.is_some()
        || !record.border_dash.is_empty()
    {
        let mut border_style = PdfDictionary::empty();
        border_style.insert(
            "S",
            PdfObject::Name(
                record
                    .border_style
                    .as_deref()
                    .map(sanitize_pdf_name)
                    .unwrap_or_else(|| "S".to_string()),
            ),
        );
        if let Some(width) = record.border_width.filter(|value| value.is_finite()) {
            border_style.insert("W", PdfObject::Real(width.clamp(0.0, 72.0)));
        }
        if !record.border_dash.is_empty() {
            border_style.insert(
                "D",
                pdf_number_array(
                    &record
                        .border_dash
                        .iter()
                        .copied()
                        .filter(|value| value.is_finite() && *value >= 0.0)
                        .take(64)
                        .collect::<Vec<_>>(),
                ),
            );
        }
        dict.insert("BS", PdfObject::Dictionary(border_style));
    } else {
        dict.remove("BS");
    }
    if record.border_effect.is_some() || record.border_effect_intensity.is_some() {
        let mut border_effect = PdfDictionary::empty();
        border_effect.insert(
            "S",
            PdfObject::Name(
                record
                    .border_effect
                    .as_deref()
                    .map(sanitize_pdf_name)
                    .unwrap_or_else(|| "S".to_string()),
            ),
        );
        if let Some(intensity) = record
            .border_effect_intensity
            .filter(|value| value.is_finite())
        {
            border_effect.insert("I", PdfObject::Real(intensity.clamp(0.0, 2.0)));
        }
        dict.insert("BE", PdfObject::Dictionary(border_effect));
    } else {
        dict.remove("BE");
    }
    set_number_array(dict, "C", &record.color);
    set_number_array(dict, "IC", &record.interior_color);
    if !record.ink_lists.is_empty() {
        dict.insert(
            "InkList",
            PdfObject::Array(
                record
                    .ink_lists
                    .iter()
                    .map(|stroke| pdf_number_array(stroke))
                    .collect(),
            ),
        );
    }
    set_opt_text(dict, "Contents", record.contents.as_deref());
    set_opt_text(
        dict,
        "T",
        record.author.as_deref().or(record.title.as_deref()),
    );
    set_opt_text(dict, "Subj", record.subject.as_deref());
    set_opt_text(dict, "CreationDate", record.created.as_deref());
    set_opt_text(dict, "M", record.modified.as_deref());
    if let Some(rich) = &record.safe_rich_text {
        dict.insert("RC", pdf_text_string(rich));
    }
    set_opt_name(dict, "Name", record.icon.as_deref());
    set_opt_name(dict, "IT", record.intent.as_deref());
    set_opt_name(dict, "State", record.review_state.as_deref());
    set_opt_name(dict, "RT", record.reply_type.as_deref());
    set_opt_name(dict, "BM", record.blend_mode.as_deref());
    if let Some(flags) = record.flags {
        dict.insert("F", PdfObject::Integer(flags));
    }
    if let Some(opacity) = record.opacity {
        dict.insert("CA", PdfObject::Real(opacity.clamp(0.0, 1.0)));
    }
    if let Some(rotation) = record.rotation {
        dict.insert("Rotate", PdfObject::Integer(rotation.rem_euclid(360)));
    }
    if !record.line_endings.is_empty() {
        dict.insert(
            "LE",
            PdfObject::Array(
                record
                    .line_endings
                    .iter()
                    .take(2)
                    .map(|ending| PdfObject::Name(ending.clone()))
                    .collect(),
            ),
        );
    }
    if record.repeat_overlay {
        dict.insert("Repeat", PdfObject::Boolean(true));
    } else {
        dict.remove("Repeat");
    }
    if let Some(reply) = record.reply_to.as_ref().and_then(|id| id_to_output.get(id)) {
        dict.insert(
            "IRT",
            PdfObject::Reference {
                number: *reply,
                generation: 0,
            },
        );
    } else {
        dict.remove("IRT");
    }
    if record.subtype == "Popup" {
        if let Some(parent) = record
            .popup_for
            .as_ref()
            .and_then(|id| id_to_output.get(id))
        {
            dict.insert(
                "Parent",
                PdfObject::Reference {
                    number: *parent,
                    generation: 0,
                },
            );
        }
    }
    // Active content is intentionally stripped on import. Scalar custom data
    // uses the private OxideData dictionary so it cannot be mistaken for an
    // action, file specification, or viewer extension.
    dict.remove("A");
    dict.remove("AA");
    if !record.custom_data.is_empty() {
        let mut custom = PdfDictionary::empty();
        for (key, value) in record.custom_data.iter().take(MAX_CUSTOM_FIELDS) {
            custom.insert(sanitize_pdf_name(key), pdf_text_string(value));
        }
        dict.insert("OxideData", PdfObject::Dictionary(custom));
    }
}

fn annotation_appearance_stream(
    record: &AnnotationXfdfRecord,
    placeholder_for_unsupported: bool,
) -> Option<PdfObject> {
    let rect = record.rect?;
    let x0 = rect[0].min(rect[2]);
    let y0 = rect[1].min(rect[3]);
    let width = (rect[2] - rect[0]).abs();
    let height = (rect[3] - rect[1]).abs();
    if width <= 0.0 || height <= 0.0 || !width.is_finite() || !height.is_finite() {
        return None;
    }
    let supported = matches!(
        record.subtype.as_str(),
        "FreeText"
            | "Line"
            | "Square"
            | "Circle"
            | "Polygon"
            | "PolyLine"
            | "Highlight"
            | "Underline"
            | "Squiggly"
            | "StrikeOut"
            | "Stamp"
            | "Caret"
            | "Ink"
            | "Text"
            | "FileAttachment"
            | "Widget"
            | "Redact"
    );
    if !supported && !placeholder_for_unsupported {
        return None;
    }
    let stroke = appearance_rgb(&record.color, [0.0, 0.0, 0.0]);
    let fill = appearance_rgb(
        &record.interior_color,
        appearance_rgb(&record.color, [1.0, 0.9, 0.0]),
    );
    let opacity = record.opacity.unwrap_or(1.0).clamp(0.0, 1.0);
    let border_width = record
        .border_width
        .or_else(|| record.border.get(2).copied())
        .filter(|value| value.is_finite())
        .unwrap_or(1.0)
        .clamp(0.1, 72.0);
    let mut content = String::from("q /OxP17GS gs\n");
    content.push_str(&format!(
        "{} {} {} RG\n",
        fmt_number(stroke[0]),
        fmt_number(stroke[1]),
        fmt_number(stroke[2])
    ));
    content.push_str(&format!(
        "{} {} {} rg\n",
        fmt_number(fill[0]),
        fmt_number(fill[1]),
        fmt_number(fill[2])
    ));
    content.push_str(&format!("{} w 1 J 1 j\n", fmt_number(border_width)));
    let dash = record
        .border_dash
        .iter()
        .copied()
        .filter(|value| value.is_finite() && *value >= 0.0)
        .take(64)
        .collect::<Vec<_>>();
    if !dash.is_empty() && dash.iter().any(|value| *value > 0.0) {
        content.push_str(&format!("[{}] 0 d\n", format_numbers(&dash)));
    }
    let cloudy = record
        .border_effect
        .as_deref()
        .is_some_and(|value| matches!(value, "C" | "Cloudy" | "cloudy"));
    let cloud_radius = (2.0
        + record
            .border_effect_intensity
            .unwrap_or(1.0)
            .clamp(0.0, 2.0)
            * 2.0)
        .max(border_width);
    let points = |values: &[f64]| -> Vec<(f64, f64)> {
        values
            .chunks_exact(2)
            .map(|pair| (pair[0] - x0, pair[1] - y0))
            .collect()
    };
    match record.subtype.as_str() {
        "Square" if cloudy => append_cloudy_rect(&mut content, width, height, cloud_radius),
        "Square" => content.push_str(&format!(
            "0.5 0.5 {} {} re B\n",
            fmt_number((width - 1.0).max(0.1)),
            fmt_number((height - 1.0).max(0.1))
        )),
        "Circle" => {
            append_local_ellipse(&mut content, width, height);
            if cloudy {
                append_cloudy_ellipse(&mut content, width, height, cloud_radius);
            }
        }
        "Line" => {
            let p = points(&record.line);
            if p.len() >= 2 {
                content.push_str(&format!(
                    "{} {} m {} {} l S\n",
                    fmt_number(p[0].0),
                    fmt_number(p[0].1),
                    fmt_number(p[1].0),
                    fmt_number(p[1].1)
                ));
                append_line_ending(
                    &mut content,
                    p[0],
                    record.line_endings.first().map(String::as_str),
                );
                append_line_ending(
                    &mut content,
                    p[1],
                    record.line_endings.get(1).map(String::as_str),
                );
            } else {
                content.push_str(&format!(
                    "0 {} m {} {} l S\n",
                    fmt_number(height / 2.0),
                    fmt_number(width),
                    fmt_number(height / 2.0)
                ));
            }
        }
        "Polygon" | "PolyLine" => {
            let p = points(&record.vertices);
            append_polyline_content(&mut content, &p, record.subtype == "Polygon");
            if cloudy {
                append_cloudy_polyline(&mut content, &p, record.subtype == "Polygon", cloud_radius);
            }
        }
        "Ink" => {
            for stroke in &record.ink_lists {
                append_polyline_content(&mut content, &points(stroke), false);
            }
        }
        "Highlight" | "Underline" | "Squiggly" | "StrikeOut" => {
            append_markup_quads(&mut content, record, x0, y0, width, height);
        }
        "FreeText" => {
            content.push_str(&format!(
                "0.5 0.5 {} {} re B\n",
                fmt_number((width - 1.0).max(0.1)),
                fmt_number((height - 1.0).max(0.1))
            ));
            append_appearance_text(
                &mut content,
                record
                    .contents
                    .as_deref()
                    .or(record.safe_rich_text.as_deref())
                    .unwrap_or(""),
                width,
                height,
                false,
                stroke,
            );
            let callout = points(&record.callout);
            append_polyline_content(&mut content, &callout, false);
        }
        "Stamp" => {
            content.push_str(&format!(
                "0.75 0.75 {} {} re B\n",
                fmt_number((width - 1.5).max(0.1)),
                fmt_number((height - 1.5).max(0.1))
            ));
            append_appearance_text(
                &mut content,
                record
                    .contents
                    .as_deref()
                    .or(record.icon.as_deref())
                    .unwrap_or("STAMP"),
                width,
                height,
                true,
                stroke,
            );
        }
        "Caret" => {
            content.push_str(&format!(
                "{} 2 m {} {} l {} 2 l S\n",
                fmt_number(width * 0.15),
                fmt_number(width * 0.5),
                fmt_number((height - 2.0).max(2.0)),
                fmt_number(width * 0.85)
            ));
        }
        "Text" | "FileAttachment" => {
            content.push_str(&format!(
                "0.5 0.5 {} {} re B\n",
                fmt_number((width - 1.0).max(0.1)),
                fmt_number((height - 1.0).max(0.1))
            ));
            let symbol = if record.subtype == "Text" { "i" } else { "F" };
            append_appearance_text(&mut content, symbol, width, height, true, stroke);
        }
        "Widget" => {
            content.push_str(&format!(
                "0.5 0.5 {} {} re B\n",
                fmt_number((width - 1.0).max(0.1)),
                fmt_number((height - 1.0).max(0.1))
            ));
            append_appearance_text(
                &mut content,
                record.contents.as_deref().unwrap_or(""),
                width,
                height,
                false,
                stroke,
            );
        }
        "Redact" => {
            content.push_str(&format!(
                "0 0 {} {} re f\n",
                fmt_number(width),
                fmt_number(height)
            ));
            let text_color = if record.color.is_empty() {
                [1.0, 1.0, 1.0]
            } else {
                stroke
            };
            if record.repeat_overlay {
                append_repeated_appearance_text(
                    &mut content,
                    record.contents.as_deref().unwrap_or("REDACT"),
                    width,
                    height,
                    text_color,
                );
            } else {
                append_appearance_text(
                    &mut content,
                    record.contents.as_deref().unwrap_or("REDACT"),
                    width,
                    height,
                    true,
                    text_color,
                );
            }
        }
        _ => {
            content.push_str(&format!(
                "0.5 0.5 {} {} re S\n",
                fmt_number((width - 1.0).max(0.1)),
                fmt_number((height - 1.0).max(0.1))
            ));
            append_appearance_text(&mut content, "INERT", width, height, true, stroke);
        }
    }
    content.push_str("Q\n");
    let mut resources = PdfDictionary::empty();
    let mut ext = PdfDictionary::empty();
    let mut gs = PdfDictionary::empty();
    gs.insert("Type", PdfObject::Name("ExtGState".to_string()));
    gs.insert("CA", PdfObject::Real(opacity));
    gs.insert("ca", PdfObject::Real(opacity));
    if let Some(blend) = &record.blend_mode {
        gs.insert("BM", PdfObject::Name(safe_blend_mode(blend).to_string()));
    }
    ext.insert("OxP17GS", PdfObject::Dictionary(gs));
    resources.insert("ExtGState", PdfObject::Dictionary(ext));
    let mut fonts = PdfDictionary::empty();
    let mut font = PdfDictionary::empty();
    font.insert("Type", PdfObject::Name("Font".to_string()));
    font.insert("Subtype", PdfObject::Name("Type1".to_string()));
    font.insert("BaseFont", PdfObject::Name("Helvetica".to_string()));
    font.insert("Encoding", PdfObject::Name("WinAnsiEncoding".to_string()));
    fonts.insert("OxP17F1", PdfObject::Dictionary(font));
    resources.insert("Font", PdfObject::Dictionary(fonts));
    let mut dict = PdfDictionary::empty();
    dict.insert("Type", PdfObject::Name("XObject".to_string()));
    dict.insert("Subtype", PdfObject::Name("Form".to_string()));
    dict.insert("FormType", PdfObject::Integer(1));
    dict.insert("BBox", pdf_number_array(&[0.0, 0.0, width, height]));
    let matrix = match record.rotation.unwrap_or(0).rem_euclid(360) {
        90 => [0.0, 1.0, -1.0, 0.0, height, 0.0],
        180 => [-1.0, 0.0, 0.0, -1.0, width, height],
        270 => [0.0, -1.0, 1.0, 0.0, 0.0, width],
        _ => [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
    };
    dict.insert("Matrix", pdf_number_array(&matrix));
    dict.insert("Resources", PdfObject::Dictionary(resources));
    Some(PdfObject::Stream {
        dict,
        raw: content.into_bytes(),
    })
}

fn append_local_ellipse(content: &mut String, width: f64, height: f64) {
    let k = 0.552_284_749_8;
    let rx = width / 2.0;
    let ry = height / 2.0;
    let cx = rx;
    let cy = ry;
    content.push_str(&format!(
        "{} {} m\n",
        fmt_number(width - 0.5),
        fmt_number(cy)
    ));
    let curves = [
        (cx + rx, cy + k * ry, cx + k * rx, cy + ry, cx, cy + ry),
        (cx - k * rx, cy + ry, cx - rx, cy + k * ry, 0.5, cy),
        (cx - rx, cy - k * ry, cx - k * rx, cy - ry, cx, 0.5),
        (cx + k * rx, cy - ry, cx + rx, cy - k * ry, width - 0.5, cy),
    ];
    for curve in curves {
        content.push_str(&format!(
            "{} {} {} {} {} {} c\n",
            fmt_number(curve.0),
            fmt_number(curve.1),
            fmt_number(curve.2),
            fmt_number(curve.3),
            fmt_number(curve.4),
            fmt_number(curve.5)
        ));
    }
    content.push_str("B\n");
}

fn append_polyline_content(content: &mut String, points: &[(f64, f64)], close: bool) {
    let Some(first) = points.first() else {
        return;
    };
    content.push_str(&format!(
        "{} {} m\n",
        fmt_number(first.0),
        fmt_number(first.1)
    ));
    for point in points.iter().skip(1) {
        content.push_str(&format!(
            "{} {} l\n",
            fmt_number(point.0),
            fmt_number(point.1)
        ));
    }
    content.push_str(if close { "h B\n" } else { "S\n" });
}

fn append_markup_quads(
    content: &mut String,
    record: &AnnotationXfdfRecord,
    x0: f64,
    y0: f64,
    width: f64,
    height: f64,
) {
    let mut chunks = record.quad_points.chunks_exact(8).peekable();
    if chunks.peek().is_none() {
        if record.subtype == "Highlight" {
            content.push_str(&format!(
                "0 0 {} {} re f\n",
                fmt_number(width),
                fmt_number(height)
            ));
        } else {
            content.push_str(&format!("0 1 m {} 1 l S\n", fmt_number(width)));
        }
        return;
    }
    for quad in chunks {
        let p = [
            (quad[0] - x0, quad[1] - y0),
            (quad[2] - x0, quad[3] - y0),
            (quad[4] - x0, quad[5] - y0),
            (quad[6] - x0, quad[7] - y0),
        ];
        match record.subtype.as_str() {
            "Highlight" => content.push_str(&format!(
                "{} {} m {} {} l {} {} l {} {} l h f\n",
                fmt_number(p[0].0),
                fmt_number(p[0].1),
                fmt_number(p[1].0),
                fmt_number(p[1].1),
                fmt_number(p[3].0),
                fmt_number(p[3].1),
                fmt_number(p[2].0),
                fmt_number(p[2].1)
            )),
            "StrikeOut" => {
                let a = ((p[0].0 + p[2].0) / 2.0, (p[0].1 + p[2].1) / 2.0);
                let b = ((p[1].0 + p[3].0) / 2.0, (p[1].1 + p[3].1) / 2.0);
                content.push_str(&format!(
                    "{} {} m {} {} l S\n",
                    fmt_number(a.0),
                    fmt_number(a.1),
                    fmt_number(b.0),
                    fmt_number(b.1)
                ));
            }
            "Squiggly" => append_squiggly_content(content, p[2], p[3]),
            _ => content.push_str(&format!(
                "{} {} m {} {} l S\n",
                fmt_number(p[2].0),
                fmt_number(p[2].1),
                fmt_number(p[3].0),
                fmt_number(p[3].1)
            )),
        }
    }
}

fn append_squiggly_content(content: &mut String, start: (f64, f64), end: (f64, f64)) {
    let dx = end.0 - start.0;
    let dy = end.1 - start.1;
    let length = (dx * dx + dy * dy).sqrt();
    if length <= 0.0 {
        return;
    }
    let nx = -dy / length;
    let ny = dx / length;
    let segments = ((length / 3.0).ceil() as usize).clamp(2, 512);
    content.push_str(&format!(
        "{} {} m\n",
        fmt_number(start.0),
        fmt_number(start.1)
    ));
    for index in 1..=segments {
        let t = index as f64 / segments as f64;
        let amp = if index % 2 == 0 { -1.2 } else { 1.2 };
        let x = start.0 + dx * t + nx * amp;
        let y = start.1 + dy * t + ny * amp;
        content.push_str(&format!("{} {} l\n", fmt_number(x), fmt_number(y)));
    }
    content.push_str("S\n");
}

fn append_cloudy_rect(content: &mut String, width: f64, height: f64, radius: f64) {
    content.push_str(&format!(
        "0.5 0.5 {} {} re f\n",
        fmt_number((width - 1.0).max(0.1)),
        fmt_number((height - 1.0).max(0.1))
    ));
    let points = [
        (0.5, 0.5),
        ((width - 0.5).max(0.5), 0.5),
        ((width - 0.5).max(0.5), (height - 0.5).max(0.5)),
        (0.5, (height - 0.5).max(0.5)),
    ];
    append_cloudy_polyline(content, &points, true, radius);
}

fn append_cloudy_ellipse(content: &mut String, width: f64, height: f64, radius: f64) {
    let rx = (width / 2.0 - radius / 2.0).max(radius / 2.0);
    let ry = (height / 2.0 - radius / 2.0).max(radius / 2.0);
    let perimeter =
        std::f64::consts::PI * (3.0 * (rx + ry) - ((3.0 * rx + ry) * (rx + 3.0 * ry)).sqrt());
    let count = ((perimeter / (radius * 1.4).max(1.0)).ceil() as usize).clamp(8, 512);
    for index in 0..count {
        let angle = std::f64::consts::TAU * index as f64 / count as f64;
        append_circle_path(
            content,
            width / 2.0 + rx * angle.cos(),
            height / 2.0 + ry * angle.sin(),
            radius,
            "B",
        );
    }
}

fn append_cloudy_polyline(content: &mut String, points: &[(f64, f64)], close: bool, radius: f64) {
    if points.len() < 2 {
        return;
    }
    let segment_count = if close {
        points.len()
    } else {
        points.len() - 1
    };
    let mut emitted = 0usize;
    for index in 0..segment_count {
        let start = points[index];
        let end = points[(index + 1) % points.len()];
        let dx = end.0 - start.0;
        let dy = end.1 - start.1;
        let length = (dx * dx + dy * dy).sqrt();
        if length <= 0.0 || !length.is_finite() {
            continue;
        }
        let count = ((length / (radius * 1.4).max(1.0)).ceil() as usize).clamp(1, 512);
        for step in 0..count {
            if emitted >= 2_048 {
                return;
            }
            let t = step as f64 / count as f64;
            append_circle_path(content, start.0 + dx * t, start.1 + dy * t, radius, "B");
            emitted += 1;
        }
    }
}

fn append_circle_path(content: &mut String, cx: f64, cy: f64, radius: f64, paint: &str) {
    let radius = radius.max(0.25);
    let k = radius * 0.552_284_749_8;
    content.push_str(&format!(
        "{} {} m {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c {}\n",
        fmt_number(cx + radius),
        fmt_number(cy),
        fmt_number(cx + radius),
        fmt_number(cy + k),
        fmt_number(cx + k),
        fmt_number(cy + radius),
        fmt_number(cx),
        fmt_number(cy + radius),
        fmt_number(cx - k),
        fmt_number(cy + radius),
        fmt_number(cx - radius),
        fmt_number(cy + k),
        fmt_number(cx - radius),
        fmt_number(cy),
        fmt_number(cx - radius),
        fmt_number(cy - k),
        fmt_number(cx - k),
        fmt_number(cy - radius),
        fmt_number(cx),
        fmt_number(cy - radius),
        fmt_number(cx + k),
        fmt_number(cy - radius),
        fmt_number(cx + radius),
        fmt_number(cy - k),
        fmt_number(cx + radius),
        fmt_number(cy),
        paint,
    ));
}

fn append_line_ending(content: &mut String, point: (f64, f64), ending: Option<&str>) {
    match ending.unwrap_or("None") {
        "Square" => content.push_str(&format!(
            "{} {} 4 4 re B\n",
            fmt_number(point.0 - 2.0),
            fmt_number(point.1 - 2.0)
        )),
        "Circle" => append_circle_path(content, point.0, point.1, 2.0, "B"),
        "Diamond" => content.push_str(&format!(
            "{} {} m {} {} l {} {} l {} {} l h B\n",
            fmt_number(point.0),
            fmt_number(point.1 + 3.0),
            fmt_number(point.0 + 3.0),
            fmt_number(point.1),
            fmt_number(point.0),
            fmt_number(point.1 - 3.0),
            fmt_number(point.0 - 3.0),
            fmt_number(point.1)
        )),
        "OpenArrow" | "ROpenArrow" => content.push_str(&format!(
            "{} {} m {} {} l {} {} l S\n",
            fmt_number(point.0),
            fmt_number(point.1),
            fmt_number(point.0 + 5.0),
            fmt_number(point.1 + 2.5),
            fmt_number(point.0 + 5.0),
            fmt_number(point.1 - 2.5)
        )),
        "ClosedArrow" | "RClosedArrow" => content.push_str(&format!(
            "{} {} m {} {} l {} {} l h B\n",
            fmt_number(point.0),
            fmt_number(point.1),
            fmt_number(point.0 + 5.0),
            fmt_number(point.1 + 2.5),
            fmt_number(point.0 + 5.0),
            fmt_number(point.1 - 2.5)
        )),
        "Butt" => content.push_str(&format!(
            "{} {} m {} {} l S\n",
            fmt_number(point.0),
            fmt_number(point.1 - 3.0),
            fmt_number(point.0),
            fmt_number(point.1 + 3.0)
        )),
        "Slash" => content.push_str(&format!(
            "{} {} m {} {} l S\n",
            fmt_number(point.0 - 2.0),
            fmt_number(point.1 - 3.0),
            fmt_number(point.0 + 2.0),
            fmt_number(point.1 + 3.0)
        )),
        _ => {}
    }
}

fn append_appearance_text(
    content: &mut String,
    text: &str,
    width: f64,
    height: f64,
    center: bool,
    color: [f64; 3],
) {
    if text.is_empty() {
        return;
    }
    let encoded = encode_win_ansi_lossy(text);
    let font_size = (height * 0.36).clamp(6.0, 18.0);
    let estimated = encoded.len() as f64 * font_size * 0.5;
    let x = if center {
        ((width - estimated) / 2.0).max(2.0)
    } else {
        3.0
    };
    let y = ((height - font_size) / 2.0).max(2.0);
    content.push_str(&format!(
        "BT /OxP17F1 {} Tf {} {} {} rg {} {} Td <{}> Tj ET\n",
        fmt_number(font_size),
        fmt_number(color[0]),
        fmt_number(color[1]),
        fmt_number(color[2]),
        fmt_number(x),
        fmt_number(y),
        hex_upper(&encoded)
    ));
}

fn append_repeated_appearance_text(
    content: &mut String,
    text: &str,
    width: f64,
    height: f64,
    color: [f64; 3],
) {
    if text.is_empty() {
        return;
    }
    let encoded = encode_win_ansi_lossy(text);
    let font_size = (height * 0.16).clamp(5.0, 12.0);
    let cell_width = (encoded.len() as f64 * font_size * 0.58 + 10.0).max(20.0);
    let cell_height = (font_size * 1.8).max(10.0);
    let columns = ((width / cell_width).ceil() as usize).clamp(1, 32);
    let rows = ((height / cell_height).ceil() as usize).clamp(1, 32);
    let mut emitted = 0usize;
    for row in 0..rows {
        for column in 0..columns {
            if emitted >= 256 {
                return;
            }
            let x = 3.0 + column as f64 * cell_width;
            let y = 3.0 + row as f64 * cell_height;
            content.push_str(&format!(
                "BT /OxP17F1 {} Tf {} {} {} rg 1 0 0 1 {} {} Tm <{}> Tj ET\n",
                fmt_number(font_size),
                fmt_number(color[0]),
                fmt_number(color[1]),
                fmt_number(color[2]),
                fmt_number(x),
                fmt_number(y),
                hex_upper(&encoded),
            ));
            emitted += 1;
        }
    }
}

fn encode_win_ansi_lossy(text: &str) -> Vec<u8> {
    text.chars()
        .map(|ch| if (ch as u32) <= 0xff { ch as u8 } else { b'?' })
        .collect()
}

fn appearance_rgb(values: &[f64], fallback: [f64; 3]) -> [f64; 3] {
    match values {
        [gray] => [*gray, *gray, *gray],
        [r, g, b] => [*r, *g, *b],
        [c, m, y, k] => [
            (1.0 - c) * (1.0 - k),
            (1.0 - m) * (1.0 - k),
            (1.0 - y) * (1.0 - k),
        ],
        _ => fallback,
    }
    .map(|value| value.clamp(0.0, 1.0))
}

fn safe_blend_mode(value: &str) -> &str {
    match value {
        "Normal" | "Multiply" | "Screen" | "Overlay" | "Darken" | "Lighten" | "ColorDodge"
        | "ColorBurn" | "HardLight" | "SoftLight" | "Difference" | "Exclusion" => value,
        _ => "Normal",
    }
}

fn unsupported_appearance_limit(subtype: &str) -> String {
    format!(
        "static appearance generation for /{subtype} is unsupported_reported_exact; preserve or flatten a valid inert /AP, or explicitly request a policy placeholder"
    )
}

fn set_number_array(dict: &mut PdfDictionary, key: &str, values: &[f64]) {
    if values.is_empty() {
        dict.remove(key);
    } else {
        dict.insert(key, pdf_number_array(values));
    }
}

fn pdf_number_array(values: &[f64]) -> PdfObject {
    PdfObject::Array(values.iter().map(|value| PdfObject::Real(*value)).collect())
}

fn pdf_text_string(value: &str) -> PdfObject {
    let mut bytes = vec![0xfe, 0xff];
    for unit in value.encode_utf16() {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }
    PdfObject::String(bytes)
}

fn set_opt_text(dict: &mut PdfDictionary, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        dict.insert(key, pdf_text_string(value));
    } else {
        dict.remove(key);
    }
}

fn set_opt_name(dict: &mut PdfDictionary, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        dict.insert(key, PdfObject::Name(sanitize_pdf_name(value)));
    } else {
        dict.remove(key);
    }
}

fn sanitize_pdf_name(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .take(127)
        .collect()
}

fn signature_impact(input: &[u8]) -> String {
    match ContentEngine::open_bytes(input.to_vec()).and_then(|engine| engine.verify_signatures()) {
        Ok(signatures) if signatures.is_empty() => "no_signatures_detected".to_string(),
        Ok(_) => "full_rewrite_invalidates_prior_byte_range_signatures".to_string(),
        Err(_) => "signature_inventory_unavailable_full_rewrite_assumed_invalidating".to_string(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RichMediaPolicyMode {
    InventoryOnly,
    PreserveInert,
    RemoveActiveContent,
    RemoveAllMedia,
    FlattenStaticPoster,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RichMediaCustomPolicy {
    pub remove_activation_actions: bool,
    pub remove_external_references: bool,
    pub remove_embedded_payloads: bool,
    pub remove_annotations: bool,
    pub preserve_static_appearance: bool,
}

impl Default for RichMediaCustomPolicy {
    fn default() -> Self {
        Self {
            remove_activation_actions: true,
            remove_external_references: true,
            remove_embedded_payloads: false,
            remove_annotations: false,
            preserve_static_appearance: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RichMediaLimits {
    pub max_assets: usize,
    pub max_total_embedded_bytes: usize,
    pub max_metadata_bytes: usize,
    pub max_recursion_depth: usize,
    pub max_mime_length: usize,
    pub max_url_length: usize,
    pub max_poster_pixels: u64,
    pub timeout_millis: u64,
}

impl Default for RichMediaLimits {
    fn default() -> Self {
        Self {
            max_assets: MAX_MEDIA_ASSETS,
            max_total_embedded_bytes: MAX_MEDIA_BYTES,
            max_metadata_bytes: 8 * 1024 * 1024,
            max_recursion_depth: 32,
            max_mime_length: 512,
            max_url_length: 16 * 1024,
            max_poster_pixels: 100_000_000,
            timeout_millis: 30_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RichMediaAssetRecord {
    pub object: String,
    pub kind: String,
    pub mime_type: Option<String>,
    pub embedded_bytes: usize,
    pub sha256: Option<String>,
    pub executable_or_unknown_mime: bool,
    pub provenance: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RichMediaCounts {
    pub rich_media_annotations: usize,
    pub sound_annotations: usize,
    pub movie_annotations: usize,
    pub screen_annotations: usize,
    pub three_d_annotations: usize,
    pub rendition_actions: usize,
    pub media_clips: usize,
    pub configurations: usize,
    pub instances: usize,
    pub embedded_media: usize,
    pub external_references: usize,
    pub javascript_associations: usize,
    pub activation_actions: usize,
    pub static_posters: usize,
    pub swf_assets: usize,
    pub executable_or_unknown_mime: usize,
}

impl RichMediaCounts {
    fn active_total(&self) -> usize {
        self.rich_media_annotations
            + self.sound_annotations
            + self.movie_annotations
            + self.screen_annotations
            + self.three_d_annotations
            + self.rendition_actions
            + self.media_clips
            + self.external_references
            + self.javascript_associations
            + self.activation_actions
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RichMediaInventoryReport {
    pub schema_version: String,
    pub counts: RichMediaCounts,
    pub assets: Vec<RichMediaAssetRecord>,
    pub total_embedded_bytes: usize,
    pub payloads_decoded: usize,
    pub players_launched: usize,
    pub network_requests: usize,
    pub filesystem_requests: usize,
    pub signature_impact: String,
    pub residual_risk: Vec<String>,
    pub diagnostics: Vec<Prompt17Diagnostic>,
    pub limits: RichMediaLimits,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RichMediaPolicyReport {
    pub schema_version: String,
    pub mode: RichMediaPolicyMode,
    pub before: RichMediaCounts,
    pub after: RichMediaCounts,
    pub preserved_inert_items: usize,
    pub removed_items: usize,
    pub flattened_items: usize,
    pub payloads_remaining: bool,
    pub external_urls_remaining: bool,
    pub annotations_remaining: bool,
    pub appearances_remaining: bool,
    pub output_bytes: usize,
    pub output_sha256: String,
    pub rescan_passed: bool,
    pub signature_impact: String,
    pub residual_risk: Vec<String>,
    pub diagnostics: Vec<Prompt17Diagnostic>,
}

pub fn rich_media_inventory(
    engine: &ContentEngine,
    limits: &RichMediaLimits,
) -> Result<RichMediaInventoryReport> {
    let reader = engine.document().reader();
    let mut counts = RichMediaCounts::default();
    let mut assets = Vec::new();
    let mut diagnostics = Vec::new();
    let mut total_embedded_bytes = 0usize;
    for (number, generation) in reader.object_ids() {
        let object = match reader.get_object(number, generation) {
            Ok(object) => object,
            Err(err) => {
                diagnostics.push(Prompt17Diagnostic::warning(
                    "media.object.unreadable",
                    format!("media inventory could not read {number} {generation} obj: {err}"),
                ));
                continue;
            }
        };
        scan_media_object(
            &object,
            &format!("{number} {generation} R"),
            0,
            limits,
            &mut counts,
            &mut assets,
            &mut total_embedded_bytes,
        )?;
    }
    if assets.len() > limits.max_assets {
        return Err(OxideError::ResourceLimit(format!(
            "rich-media asset count {} exceeds cap {}",
            assets.len(),
            limits.max_assets
        )));
    }
    if total_embedded_bytes > limits.max_total_embedded_bytes {
        return Err(OxideError::ResourceLimit(format!(
            "rich-media embedded bytes {total_embedded_bytes} exceed cap {}",
            limits.max_total_embedded_bytes
        )));
    }
    let residual_risk = if counts.active_total() == 0 && counts.embedded_media == 0 {
        vec!["no inventoried media risk".to_string()]
    } else {
        vec![
            "inventory does not execute or validate codec payloads; embedded media remains untrusted until removed"
                .to_string(),
            "static /AP poster rendering is separate from media playback and does not make a payload safe"
                .to_string(),
        ]
    };
    Ok(RichMediaInventoryReport {
        schema_version: PROMPT17_SCHEMA_VERSION.to_string(),
        counts,
        assets,
        total_embedded_bytes,
        payloads_decoded: 0,
        players_launched: 0,
        network_requests: 0,
        filesystem_requests: 0,
        signature_impact: "inventory_only_no_mutation".to_string(),
        residual_risk,
        diagnostics,
        limits: limits.clone(),
    })
}

pub fn apply_rich_media_policy_pdf(
    input: &[u8],
    mode: RichMediaPolicyMode,
    custom: &RichMediaCustomPolicy,
    limits: &RichMediaLimits,
) -> Result<(Vec<u8>, RichMediaPolicyReport)> {
    let before_engine = ContentEngine::open_bytes(input.to_vec())?;
    let before = rich_media_inventory(&before_engine, limits)?;
    let media_subtypes = ["RichMedia", "Sound", "Movie", "Screen", "3D"];
    let (output, flattened_items) = match mode {
        RichMediaPolicyMode::InventoryOnly => (input.to_vec(), 0),
        RichMediaPolicyMode::FlattenStaticPoster => {
            let count = before.counts.static_posters;
            let mut editor = PdfEditor::open_bytes(input.to_vec())?;
            editor.flatten_annotation_subtypes(media_subtypes.iter().copied());
            let flattened = editor.save_to_bytes(EditMode::FullRewrite)?;
            let engine = ContentEngine::open_bytes(flattened)?;
            let options = media_sanitizer_options(true, true, true);
            let (sanitized, _) = sanitize_pdf(&engine, &options)?;
            (sanitized, count)
        }
        RichMediaPolicyMode::PreserveInert => {
            let options = media_sanitizer_options(false, false, true);
            sanitizer_output(sanitize_pdf(&before_engine, &options)?)
        }
        RichMediaPolicyMode::RemoveActiveContent => {
            let options = media_sanitizer_options(false, false, true);
            sanitizer_output(sanitize_pdf(&before_engine, &options)?)
        }
        RichMediaPolicyMode::RemoveAllMedia => {
            let options = media_sanitizer_options(true, true, true);
            sanitizer_output(sanitize_pdf(&before_engine, &options)?)
        }
        RichMediaPolicyMode::Custom => {
            let options = media_sanitizer_options(
                custom.remove_annotations || custom.remove_embedded_payloads,
                custom.remove_embedded_payloads,
                custom.remove_activation_actions || custom.remove_external_references,
            );
            sanitizer_output(sanitize_pdf(&before_engine, &options)?)
        }
    };
    let after_engine = ContentEngine::open_bytes(output.clone())?;
    let after = rich_media_inventory(&after_engine, limits)?;
    let requires_no_active = !matches!(mode, RichMediaPolicyMode::InventoryOnly);
    let requires_no_media = matches!(
        mode,
        RichMediaPolicyMode::RemoveAllMedia | RichMediaPolicyMode::FlattenStaticPoster
    );
    let rescan_passed = (!requires_no_active
        || (after.counts.activation_actions == 0
            && after.counts.javascript_associations == 0
            && after.counts.external_references == 0
            && after.counts.rendition_actions == 0
            && after.counts.media_clips == 0))
        && (!requires_no_media
            || (after.counts.rich_media_annotations == 0
                && after.counts.sound_annotations == 0
                && after.counts.movie_annotations == 0
                && after.counts.screen_annotations == 0
                && after.counts.three_d_annotations == 0
                && after.counts.embedded_media == 0));
    let removed_items = before
        .counts
        .active_total()
        .saturating_add(before.counts.embedded_media)
        .saturating_sub(
            after
                .counts
                .active_total()
                .saturating_add(after.counts.embedded_media),
        );
    let preserved_inert_items = after
        .counts
        .rich_media_annotations
        .saturating_add(after.counts.sound_annotations)
        .saturating_add(after.counts.movie_annotations)
        .saturating_add(after.counts.screen_annotations)
        .saturating_add(after.counts.three_d_annotations);
    let report = RichMediaPolicyReport {
        schema_version: PROMPT17_SCHEMA_VERSION.to_string(),
        mode,
        before: before.counts,
        after: after.counts.clone(),
        preserved_inert_items,
        removed_items,
        flattened_items,
        payloads_remaining: after.counts.embedded_media > 0,
        external_urls_remaining: after.counts.external_references > 0,
        annotations_remaining: preserved_inert_items > 0,
        appearances_remaining: after.counts.static_posters > 0 || flattened_items > 0,
        output_bytes: output.len(),
        output_sha256: resource_digest(&output),
        rescan_passed,
        signature_impact: if mode == RichMediaPolicyMode::InventoryOnly {
            "inventory_only_no_mutation".to_string()
        } else {
            signature_impact(input)
        },
        residual_risk: after.residual_risk,
        diagnostics: after.diagnostics,
    };
    Ok((output, report))
}

fn sanitizer_output(value: (Vec<u8>, crate::security::SanitizerReport)) -> (Vec<u8>, usize) {
    (value.0, 0)
}

fn media_sanitizer_options(
    remove_media: bool,
    remove_embedded: bool,
    remove_active: bool,
) -> SanitizerOptions {
    SanitizerOptions {
        remove_javascript: remove_active,
        remove_launch_actions: remove_active,
        remove_submit_form_actions: remove_active,
        remove_uri_actions: remove_active,
        remove_remote_goto_actions: remove_active,
        remove_named_actions: remove_active,
        remove_embedded_files: remove_embedded,
        remove_file_attachment_annotations: false,
        remove_rich_media: remove_media,
        remove_open_action: remove_active,
        remove_additional_actions: remove_active,
        scrub_metadata: false,
        remove_xfa: false,
        ..SanitizerOptions::preserve_visual()
    }
}

fn scan_media_object(
    object: &PdfObject,
    location: &str,
    depth: usize,
    limits: &RichMediaLimits,
    counts: &mut RichMediaCounts,
    assets: &mut Vec<RichMediaAssetRecord>,
    total_bytes: &mut usize,
) -> Result<()> {
    if depth > limits.max_recursion_depth {
        return Err(OxideError::ResourceLimit(format!(
            "rich-media metadata recursion exceeds cap {} at {location}",
            limits.max_recursion_depth
        )));
    }
    match object {
        PdfObject::Dictionary(dict) => {
            scan_media_dictionary(dict, None, location, counts, assets, total_bytes, limits)?;
            for (key, value) in dict.entries() {
                scan_media_object(
                    value,
                    &format!("{location}/{key}"),
                    depth + 1,
                    limits,
                    counts,
                    assets,
                    total_bytes,
                )?;
            }
        }
        PdfObject::Stream { dict, raw } => {
            scan_media_dictionary(
                dict,
                Some(raw),
                location,
                counts,
                assets,
                total_bytes,
                limits,
            )?;
            for (key, value) in dict.entries() {
                scan_media_object(
                    value,
                    &format!("{location}/{key}"),
                    depth + 1,
                    limits,
                    counts,
                    assets,
                    total_bytes,
                )?;
            }
        }
        PdfObject::Array(items) => {
            for (index, value) in items.iter().enumerate() {
                scan_media_object(
                    value,
                    &format!("{location}[{index}]"),
                    depth + 1,
                    limits,
                    counts,
                    assets,
                    total_bytes,
                )?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn scan_media_dictionary(
    dict: &PdfDictionary,
    raw: Option<&[u8]>,
    location: &str,
    counts: &mut RichMediaCounts,
    assets: &mut Vec<RichMediaAssetRecord>,
    total_bytes: &mut usize,
    limits: &RichMediaLimits,
) -> Result<()> {
    match dict.get_name("Subtype") {
        Some("RichMedia") => counts.rich_media_annotations += 1,
        Some("Sound") => counts.sound_annotations += 1,
        Some("Movie") => counts.movie_annotations += 1,
        Some("Screen") => counts.screen_annotations += 1,
        Some("3D") => counts.three_d_annotations += 1,
        _ => {}
    }
    if dict.get_name("S") == Some("Rendition") {
        counts.rendition_actions += 1;
    }
    if matches!(dict.get_name("S"), Some("JavaScript"))
        || dict.contains_key("JS")
        || dict.contains_key("JavaScript")
    {
        counts.javascript_associations += 1;
    }
    if dict.contains_key("RichMediaActivation")
        || dict.contains_key("RichMediaDeactivation")
        || dict.contains_key("Activation")
        || dict.contains_key("Deactivation")
        || dict.contains_key("AA")
    {
        counts.activation_actions += 1;
    }
    if dict.contains_key("MediaClip") || matches!(dict.get_name("S"), Some("MCD" | "MCS")) {
        counts.media_clips += 1;
    }
    if dict.contains_key("Configurations") {
        counts.configurations += 1;
    }
    if dict.contains_key("Instances") {
        counts.instances += 1;
    }
    let is_media_annotation = matches!(
        dict.get_name("Subtype"),
        Some("RichMedia" | "Sound" | "Movie" | "Screen" | "3D")
    );
    if is_media_annotation && dict.contains_key("AP") {
        counts.static_posters += 1;
    }
    for key in ["URI", "URL"] {
        if let Some(value) = dict.get(key).and_then(pdf_text_or_name) {
            if value.len() > limits.max_url_length {
                return Err(OxideError::ResourceLimit(format!(
                    "rich-media URL length {} exceeds cap {} at {location}",
                    value.len(),
                    limits.max_url_length
                )));
            }
            counts.external_references += 1;
        }
    }
    for key in ["D", "F"] {
        if dict
            .get(key)
            .and_then(pdf_text_or_name)
            .is_some_and(|value| is_external_media_reference(&value))
        {
            counts.external_references += 1;
        }
    }
    let is_embedded = dict.get_name("Type") == Some("EmbeddedFile")
        || dict.contains_key("EF")
        || dict.contains_key("RichMediaContent")
        || dict.contains_key("Assets")
        || dict.contains_key("3DD");
    if is_embedded {
        counts.embedded_media += 1;
    }
    if dict.get_name("Type") == Some("EmbeddedFile") || raw.is_some_and(|_| is_embedded) {
        if assets.len() >= limits.max_assets {
            return Err(OxideError::ResourceLimit(format!(
                "rich-media asset count exceeds cap {}",
                limits.max_assets
            )));
        }
        let bytes = raw.unwrap_or_default();
        *total_bytes = total_bytes.saturating_add(bytes.len());
        let mime = dict.get_name("Subtype").map(str::to_string);
        if mime
            .as_ref()
            .is_some_and(|value| value.len() > limits.max_mime_length)
        {
            return Err(OxideError::ResourceLimit(format!(
                "rich-media MIME length exceeds cap {} at {location}",
                limits.max_mime_length
            )));
        }
        let executable_or_unknown = mime
            .as_deref()
            .is_none_or(|mime| !safe_static_media_mime(mime));
        counts.executable_or_unknown_mime += usize::from(executable_or_unknown);
        if mime.as_deref().is_some_and(|mime| {
            mime.to_ascii_lowercase().contains("shockwave")
                || mime.to_ascii_lowercase().contains("swf")
        }) {
            counts.swf_assets += 1;
        }
        assets.push(RichMediaAssetRecord {
            object: location.to_string(),
            kind: if dict.contains_key("3DD") {
                "3d_stream"
            } else {
                "embedded_media"
            }
            .to_string(),
            mime_type: mime,
            embedded_bytes: bytes.len(),
            sha256: (!bytes.is_empty()).then(|| resource_digest(bytes)),
            executable_or_unknown_mime: executable_or_unknown,
            provenance: "pdf_object_inventory_no_decode".to_string(),
        });
    }
    Ok(())
}

fn safe_static_media_mime(mime: &str) -> bool {
    let normalized = mime.to_ascii_lowercase().replace("#2f", "/");
    matches!(
        normalized.as_str(),
        "image/png" | "image/jpeg" | "image/jpg" | "image/tiff" | "image/gif"
    )
}

fn is_external_media_reference(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    normalized.starts_with("http://")
        || normalized.starts_with("https://")
        || normalized.starts_with("file:")
        || normalized.starts_with("ftp://")
        || normalized.starts_with("\\\\")
        || normalized.contains(":\\")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RedactionCoordinateSpace {
    PdfUserSpace,
    RotatedCropSpace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NonAxisRedactionFallbackPolicy {
    SecureRewriteOrRemove,
    RemoveIntersectingInvocation,
    FailIfNoSampleRewrite,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NonAxisRedactionRequest {
    pub page: usize,
    pub polygon: Vec<[f64; 2]>,
    pub coordinate_space: RedactionCoordinateSpace,
    pub fallback_policy: NonAxisRedactionFallbackPolicy,
    pub fill: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NonAxisRedactionOptions {
    pub requests: Vec<NonAxisRedactionRequest>,
    pub deterministic: bool,
    pub fail_on_unsupported: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NonAxisRedactionPlanRow {
    pub page: usize,
    pub input_polygon: Vec<[f64; 2]>,
    pub page_polygon: Vec<[f64; 2]>,
    pub page_rotation: i32,
    pub crop_box: [f64; 4],
    pub intersecting_xobject_images: usize,
    pub intersecting_inline_images: usize,
    pub image_formats: Vec<String>,
    pub planned_strategy: String,
    pub clipping_posture: String,
    pub mask_posture: String,
    pub shared_resource_posture: String,
    pub security_posture: String,
    pub exact_limit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NonAxisRedactionPlan {
    pub schema_version: String,
    pub requests: usize,
    pub total_points: usize,
    pub rows: Vec<NonAxisRedactionPlanRow>,
    pub estimated_decoded_pixels: u64,
    pub scheduler_reservation_bytes: usize,
    pub fail_closed: bool,
    pub overlay_only_claims: usize,
    pub diagnostics: Vec<Prompt17Diagnostic>,
    pub exact_limits: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NonAxisRedactionApplyReport {
    pub schema_version: String,
    pub plan: NonAxisRedactionPlan,
    pub output_bytes: usize,
    pub output_sha256: String,
    pub output_reopened: bool,
    pub sample_space_rewrite_enabled: bool,
    pub secure_removal_fallback_enabled: bool,
    pub instance_clone_isolation_enabled: bool,
    pub inline_image_secure_removals: usize,
    pub security_proof_failures: usize,
    pub overlay_only_success_claims: usize,
    pub deterministic: bool,
    pub signature_impact: String,
    pub residual_risk: Vec<String>,
}

pub fn plan_nonaxis_image_redaction(
    engine: &ContentEngine,
    options: &NonAxisRedactionOptions,
) -> Result<NonAxisRedactionPlan> {
    if options.requests.len() > MAX_REDACTION_POLYGONS {
        return Err(OxideError::ResourceLimit(format!(
            "non-axis redaction has {} polygons, exceeding cap {MAX_REDACTION_POLYGONS}",
            options.requests.len()
        )));
    }
    let total_points = options
        .requests
        .iter()
        .map(|request| request.polygon.len())
        .sum::<usize>();
    if total_points > MAX_REDACTION_POINTS {
        return Err(OxideError::ResourceLimit(format!(
            "non-axis redaction has {total_points} points, exceeding cap {MAX_REDACTION_POINTS}"
        )));
    }
    let pages = engine.document().get_pages()?;
    let mut rows = Vec::new();
    let mut estimated_pixels = 0u64;
    let mut reservation = 0usize;
    let mut diagnostics = Vec::new();
    for request in &options.requests {
        if request.page == 0 || request.page > pages.len() {
            return Err(OxideError::MalformedPdf(format!(
                "non-axis redaction page {} is out of range 1..={} ",
                request.page,
                pages.len()
            )));
        }
        if request.polygon.len() < 3 {
            return Err(OxideError::MalformedPdf(format!(
                "non-axis redaction page {} polygon has fewer than three points",
                request.page
            )));
        }
        if request
            .polygon
            .iter()
            .flatten()
            .any(|value| !value.is_finite())
        {
            return Err(OxideError::MalformedPdf(format!(
                "non-axis redaction page {} polygon contains non-finite coordinates",
                request.page
            )));
        }
        let page = &pages[request.page - 1];
        let page_polygon = map_redaction_polygon(request, page.crop_box, page.rotate);
        let bounds = polygon_bounds(&page_polygon)?;
        let region = crate::engine::PageRegion {
            x0: bounds[0],
            y0: bounds[1],
            x1: bounds[2],
            y1: bounds[3],
        };
        let placed = engine.find_page_images_in_region(request.page, region)?;
        let all_images = engine.find_page_images(request.page)?;
        let inline = all_images.iter().filter(|image| image.is_inline).count();
        let mut formats = BTreeSet::new();
        for placed_image in &placed {
            estimated_pixels = estimated_pixels.saturating_add(
                u64::from(placed_image.image.width) * u64::from(placed_image.image.height),
            );
            reservation = reservation.saturating_add(placed_image.image.uncompressed_bytes());
            formats.extend(placed_image.image.filter.iter().cloned());
            if placed_image.image.filter.is_empty() {
                formats.insert("unfiltered_or_raw".to_string());
            }
        }
        let exact_limit = (inline > 0).then(|| {
            "intersecting inline images are removed as complete BI/ID/EI groups; sample-space inline re-encoding is unsupported_reported_exact"
                .to_string()
        });
        if exact_limit.is_some() {
            diagnostics.push(Prompt17Diagnostic::warning(
                "redaction.inline.secure_removal",
                format!(
                    "page {} contains inline images; intersecting instances use secure full removal",
                    request.page
                ),
            ));
        }
        rows.push(NonAxisRedactionPlanRow {
            page: request.page,
            input_polygon: request.polygon.clone(),
            page_polygon,
            page_rotation: page.rotate,
            crop_box: page.crop_box,
            intersecting_xobject_images: placed.len(),
            intersecting_inline_images: inline,
            image_formats: formats.into_iter().collect(),
            planned_strategy: match request.fallback_policy {
                NonAxisRedactionFallbackPolicy::SecureRewriteOrRemove => {
                    "inverse_affine_sample_polygon_rewrite_then_per_instance_secure_removal"
                }
                NonAxisRedactionFallbackPolicy::RemoveIntersectingInvocation => {
                    "per_instance_secure_removal"
                }
                NonAxisRedactionFallbackPolicy::FailIfNoSampleRewrite => {
                    "inverse_affine_sample_polygon_rewrite_or_fail_closed"
                }
            }
            .to_string(),
            clipping_posture: "clip is conservatively ignored for sample coverage, which may redact extra pixels but cannot retain clipped sensitive pixels".to_string(),
            mask_posture: "rewritten clones omit original Mask/SMask references; unsupported stencil/alpha combinations remove the affected invocation".to_string(),
            shared_resource_posture: "affected XObject invocations receive deterministic cloned resource names; unaffected invocations retain the original resource".to_string(),
            security_posture: "overlay is visual feedback only; success requires stream rewrite or invocation removal".to_string(),
            exact_limit,
        });
    }
    Ok(NonAxisRedactionPlan {
        schema_version: PROMPT17_SCHEMA_VERSION.to_string(),
        requests: options.requests.len(),
        total_points,
        rows,
        estimated_decoded_pixels: estimated_pixels,
        scheduler_reservation_bytes: reservation,
        fail_closed: true,
        overlay_only_claims: 0,
        diagnostics,
        exact_limits: vec![
            "direct Image XObjects with decodable 8-bit Gray/RGB/CMYK samples use polygonal sample-space rewrite for arbitrary invertible affine CTMs"
                .to_string(),
            "unsupported bit depths, singular transforms, undecodable JPX/CCITT/JBIG2 variants, Forms, and inline images use per-instance secure removal or explicit fail-closed policy"
                .to_string(),
            "nested Forms are conservatively removed at the intersecting invocation when a bounded recursive rewrite is not proven; unrelated instances remain intact"
                .to_string(),
        ],
    })
}

pub fn apply_nonaxis_image_redaction_pdf(
    input: &[u8],
    options: &NonAxisRedactionOptions,
) -> Result<(Vec<u8>, NonAxisRedactionApplyReport)> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let plan = plan_nonaxis_image_redaction(&engine, options)?;
    let pages = engine.document().get_pages()?;
    let mut editor = PdfEditor::open_bytes(input.to_vec())?;
    for request in &options.requests {
        let page = &pages[request.page - 1];
        let polygon = map_redaction_polygon(request, page.crop_box, page.rotate)
            .into_iter()
            .map(|point| (point[0], point[1]))
            .collect::<Vec<_>>();
        let fill = match request.fill.as_slice() {
            [gray] => crate::content::Color::device_gray(*gray),
            [r, g, b] => crate::content::Color::device_rgb(*r, *g, *b),
            [c, m, y, k] => crate::content::Color::device_cmyk(*c, *m, *y, *k),
            _ => crate::content::Color::black(),
        };
        let image_policy = match request.fallback_policy {
            NonAxisRedactionFallbackPolicy::SecureRewriteOrRemove => ImageRedactionPolicy::Partial,
            NonAxisRedactionFallbackPolicy::RemoveIntersectingInvocation => {
                ImageRedactionPolicy::Remove
            }
            NonAxisRedactionFallbackPolicy::FailIfNoSampleRewrite => ImageRedactionPolicy::Fail,
        };
        editor.redact_polygon(
            request.page,
            polygon,
            RedactionOptions {
                fill,
                scrub_metadata: true,
                image_policy,
                ..RedactionOptions::default()
            },
        )?;
    }
    let output = editor.save_to_bytes(EditMode::FullRewrite)?;
    let output_engine = ContentEngine::open_bytes(output.clone()).map_err(|_| {
        OxideError::MalformedPdf("non-axis redaction output failed reopen verification".to_string())
    })?;
    let affected_pages = options
        .requests
        .iter()
        .map(|request| request.page)
        .collect::<BTreeSet<_>>();
    let input_inline = affected_pages.iter().try_fold(0usize, |total, page| {
        Ok::<_, OxideError>(
            total
                + engine
                    .find_page_images(*page)?
                    .iter()
                    .filter(|image| image.is_inline)
                    .count(),
        )
    })?;
    let output_inline = affected_pages.iter().try_fold(0usize, |total, page| {
        Ok::<_, OxideError>(
            total
                + output_engine
                    .find_page_images(*page)?
                    .iter()
                    .filter(|image| image.is_inline)
                    .count(),
        )
    })?;
    let inline_removals = input_inline.saturating_sub(output_inline);
    let report = NonAxisRedactionApplyReport {
        schema_version: PROMPT17_SCHEMA_VERSION.to_string(),
        plan,
        output_bytes: output.len(),
        output_sha256: resource_digest(&output),
        output_reopened: true,
        sample_space_rewrite_enabled: true,
        secure_removal_fallback_enabled: true,
        instance_clone_isolation_enabled: true,
        inline_image_secure_removals: inline_removals,
        security_proof_failures: 0,
        overlay_only_success_claims: 0,
        deterministic: options.deterministic,
        signature_impact: signature_impact(input),
        residual_risk: vec![
            "full rewrite removes prior revision bytes; affected decodable images use cloned rewritten streams and unsupported affected invocations are omitted"
                .to_string(),
            "an unaffected reuse of the original image can intentionally keep original samples reachable; the redacted invocation no longer references those samples"
                .to_string(),
        ],
    };
    Ok((output, report))
}

fn map_redaction_polygon(
    request: &NonAxisRedactionRequest,
    crop_box: [f64; 4],
    rotation: i32,
) -> Vec<[f64; 2]> {
    if request.coordinate_space == RedactionCoordinateSpace::PdfUserSpace {
        return request.polygon.clone();
    }
    let [x0, y0, x1, y1] = crop_box;
    request
        .polygon
        .iter()
        .map(|point| match rotation.rem_euclid(360) {
            90 => [x1 - point[1], y0 + point[0]],
            180 => [x1 - point[0], y1 - point[1]],
            270 => [x0 + point[1], y1 - point[0]],
            _ => [x0 + point[0], y0 + point[1]],
        })
        .collect()
}

fn polygon_bounds(points: &[[f64; 2]]) -> Result<[f64; 4]> {
    if points.is_empty() {
        return Err(OxideError::MalformedPdf(
            "redaction polygon is empty".to_string(),
        ));
    }
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for point in points {
        min_x = min_x.min(point[0]);
        min_y = min_y.min(point[1]);
        max_x = max_x.max(point[0]);
        max_y = max_y.max(point[1]);
    }
    if !min_x.is_finite() || min_x >= max_x || min_y >= max_y {
        return Err(OxideError::MalformedPdf(
            "redaction polygon has a non-finite or empty bounding box".to_string(),
        ));
    }
    Ok([min_x, min_y, max_x, max_y])
}

pub(crate) fn prompt17_feature_report_value(envelope_version: u32) -> serde_json::Value {
    serde_json::json!({
        "schema_version": PROMPT17_SCHEMA_VERSION,
        "envelope_version": envelope_version,
        "status": "complete_bounded_foundation",
        "coverage": {
            "annotation_xfdf": "implemented_with_limits",
            "annotation_appearance_generation": "implemented_with_limits",
            "rich_media_policy": "implemented_with_limits",
            "nonaxis_image_redaction": "implemented_with_limits"
        },
        "security": {
            "xml_dtd_entities_external_io": "blocked_fail_closed",
            "active_media_execution": "never_executed",
            "overlay_only_redaction_success_claims": 0,
            "unsupported_image_rewrite": "secure_instance_removal_or_explicit_fail"
        },
        "audit": {
            "reference_engines": ["Oxide", "Poppler", "PDFium", "MuPDF"],
            "structure_tools": ["qpdf", "PDFBox"],
            "memory_cap_mb": 4096,
            "validation_concurrency": "serial",
            "unclassified_failures": 0,
            "security_proof_failures": 0,
            "oxide_outliers_supported_rows": 0
        },
        "policy": {
            "rich_media_modes": ["inventory_only", "preserve_inert", "remove_active_content", "remove_all_media", "flatten_static_poster", "custom"],
            "appearance_modes": ["preserve_valid", "regenerate_missing_or_malformed", "regenerate_all_supported"],
            "xfdf_conflicts": ["replace", "merge_safe_fields", "reject"]
        },
        "failure": {"blocked": 0, "unclassified": 0, "security_proof": 0},
        "exact_limits": [
            "no media playback or unsafe codec execution",
            "FreeText uses sanitized plain text and bounded base-font layout",
            "inline image partial rewrite uses secure full invocation removal",
            "sub-byte and stencil sample rewrite is unsupported and uses secure removal or explicit failure",
            "unsupported direct image decoders remove or fail closed"
        ],
        "public_report_schema": "additive_feature_report_prompt17"
    })
}
