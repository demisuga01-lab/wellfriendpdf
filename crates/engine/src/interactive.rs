//! Interactive/data-layer reports for tables, forms, annotations, page
//! navigation state, and redaction verification.
//!
//! Prompt 07 keeps mutation in the existing writer/editor paths. This module is
//! the shared audit surface: consumers can see which field/widget/annotation
//! structures exist, what page operations must preserve, and whether a produced
//! redaction still exposes target terms.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::document::{PdfDocument, PdfPage};
use crate::engine::ContentEngine;
use crate::error::Result;
use crate::info::decode_pdf_text_string;
use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;
use crate::text::TextSearchOptions;

const MAX_FIELD_DEPTH: usize = 32;
const MAX_OUTLINE_NODES: usize = 10_000;
type AnnotationEntry = (Option<(u32, u16)>, PdfObject);

#[derive(Debug, Clone, Serialize)]
pub struct InteractiveDiagnostic {
    pub code: String,
    pub severity: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
}

impl InteractiveDiagnostic {
    fn info(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            severity: "info".to_string(),
            message: message.into(),
            page: None,
            object: None,
        }
    }

    fn warning(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            severity: "warning".to_string(),
            message: message.into(),
            page: None,
            object: None,
        }
    }

    fn warning_on_page(code: &str, page: usize, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            severity: "warning".to_string(),
            message: message.into(),
            page: Some(page),
            object: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct InteractiveReport {
    pub schema_version: u32,
    pub forms: FormReport,
    pub annotations: AnnotationReport,
    pub page_operations: PageOperationsReport,
    pub diagnostics: Vec<InteractiveDiagnostic>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FormReport {
    pub has_acroform: bool,
    pub need_appearances: bool,
    pub sig_flags: Option<i64>,
    pub calculation_order_len: usize,
    pub fields: Vec<FormFieldReport>,
    pub xfa: XfaReport,
    pub diagnostics: Vec<InteractiveDiagnostic>,
}

#[derive(Debug, Clone, Serialize)]
pub struct XfaReport {
    pub present: bool,
    pub packet_count: usize,
    pub dynamic: Option<bool>,
    pub supported: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FormFieldReport {
    pub full_name: String,
    pub partial_name: Option<String>,
    pub field_type: String,
    pub flags: Option<i64>,
    pub value: Option<String>,
    pub default_value: Option<String>,
    pub attributes: Vec<FieldAttributeSource>,
    pub widgets: Vec<FormWidgetReport>,
    pub is_signature: bool,
    pub has_javascript: bool,
    pub diagnostics: Vec<InteractiveDiagnostic>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FieldAttributeSource {
    pub name: String,
    pub inherited: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FormWidgetReport {
    pub page: Option<usize>,
    pub rect: Option<[f64; 4]>,
    pub has_appearance: bool,
    pub annotation_flags: Option<i64>,
    pub object: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnnotationReport {
    pub annotations: Vec<AnnotationInfo>,
    pub by_subtype: BTreeMap<String, usize>,
    pub unsafe_actions: usize,
    pub diagnostics: Vec<InteractiveDiagnostic>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnnotationInfo {
    pub page: usize,
    pub index: usize,
    pub subtype: String,
    pub rect: Option<[f64; 4]>,
    pub contents: Option<String>,
    pub flags: Option<i64>,
    pub color: Option<Vec<f64>>,
    pub quad_points: Vec<[f64; 8]>,
    pub has_appearance: bool,
    pub action: Option<AnnotationActionInfo>,
    pub object: Option<String>,
    pub diagnostics: Vec<InteractiveDiagnostic>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnnotationActionInfo {
    pub kind: String,
    pub safe: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PageOperationsReport {
    pub page_count: usize,
    pub pages: Vec<PageBoxReport>,
    pub outlines_present: bool,
    pub outline_count: usize,
    pub page_labels_present: bool,
    pub named_destinations_present: bool,
    pub embedded_files_present: bool,
    pub acroform_present: bool,
    pub signatures_may_be_invalidated_by_rewrite: bool,
    pub diagnostics: Vec<InteractiveDiagnostic>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PageBoxReport {
    pub page: usize,
    pub object: String,
    pub media_box: [f64; 4],
    pub crop_box: [f64; 4],
    pub rotate: i32,
    pub annotations: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct RedactionVerificationReport {
    pub terms: Vec<String>,
    pub extractable_hits: Vec<RedactionHit>,
    pub raw_byte_hits: Vec<String>,
    pub verified_absent: bool,
    pub diagnostics: Vec<InteractiveDiagnostic>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RedactionHit {
    pub term: String,
    pub page: usize,
    pub match_count: usize,
}

#[derive(Clone, Default)]
struct InheritedFieldAttrs {
    ft: Option<FieldAttr>,
    ff: Option<FieldAttr>,
    da: Option<FieldAttr>,
    dr: Option<FieldAttr>,
    q: Option<FieldAttr>,
    opt: Option<FieldAttr>,
    max_len: Option<FieldAttr>,
    v: Option<FieldAttr>,
    dv: Option<FieldAttr>,
}

#[derive(Clone)]
struct FieldAttr {
    object: PdfObject,
    inherited: bool,
}

#[derive(Clone)]
struct FieldNodeContext {
    object_ref: Option<(u32, u16)>,
    dict: PdfDictionary,
    name: String,
}

pub fn interactive_report(engine: &ContentEngine) -> Result<InteractiveReport> {
    let forms = forms_report(engine)?;
    let annotations = annotation_report(engine)?;
    let page_operations = page_operations_report(engine)?;
    let mut diagnostics = Vec::new();
    diagnostics.extend(forms.diagnostics.clone());
    diagnostics.extend(annotations.diagnostics.clone());
    diagnostics.extend(page_operations.diagnostics.clone());
    Ok(InteractiveReport {
        schema_version: 1,
        forms,
        annotations,
        page_operations,
        diagnostics,
    })
}

pub fn forms_report(engine: &ContentEngine) -> Result<FormReport> {
    forms_report_document(engine.document())
}

pub fn annotation_report(engine: &ContentEngine) -> Result<AnnotationReport> {
    annotation_report_document(engine.document())
}

pub fn page_operations_report(engine: &ContentEngine) -> Result<PageOperationsReport> {
    page_operations_report_document(engine.document())
}

pub fn redaction_verification_report(
    bytes: &[u8],
    terms: &[String],
) -> Result<RedactionVerificationReport> {
    let engine = ContentEngine::open_bytes(bytes.to_vec())?;
    let pages: Vec<usize> = (1..=engine.page_count()?).collect();
    let mut extractable_hits = Vec::new();
    for term in terms {
        let matches = engine.search_text(
            &pages,
            term,
            TextSearchOptions {
                case_sensitive: false,
                include_hidden: true,
                ..TextSearchOptions::default()
            },
        )?;
        let mut by_page = BTreeMap::<usize, usize>::new();
        for hit in matches {
            *by_page.entry(hit.page).or_default() += 1;
        }
        for (page, match_count) in by_page {
            extractable_hits.push(RedactionHit {
                term: term.clone(),
                page,
                match_count,
            });
        }
    }

    let haystack = String::from_utf8_lossy(bytes).to_lowercase();
    let raw_byte_hits: Vec<String> = terms
        .iter()
        .filter(|term| haystack.contains(&term.to_lowercase()))
        .cloned()
        .collect();
    let verified_absent = extractable_hits.is_empty() && raw_byte_hits.is_empty();
    let mut diagnostics = Vec::new();
    if !verified_absent {
        diagnostics.push(InteractiveDiagnostic::warning(
            "redaction.verify.leak",
            "one or more terms remain extractable or visible in raw bytes",
        ));
    }
    Ok(RedactionVerificationReport {
        terms: terms.to_vec(),
        extractable_hits,
        raw_byte_hits,
        verified_absent,
        diagnostics,
    })
}

fn forms_report_document(document: &PdfDocument) -> Result<FormReport> {
    let catalog = document.get_catalog()?;
    let reader = document.reader();
    let pages = document.get_pages()?;
    let mut diagnostics = Vec::new();
    let Some(acroform_obj) = catalog.get("AcroForm") else {
        return Ok(FormReport {
            has_acroform: false,
            need_appearances: false,
            sig_flags: None,
            calculation_order_len: 0,
            fields: Vec::new(),
            xfa: XfaReport {
                present: false,
                packet_count: 0,
                dynamic: None,
                supported: false,
            },
            diagnostics,
        });
    };
    let acroform = reader.resolve(acroform_obj.clone())?;
    let Some(acroform_dict) = acroform.as_dict() else {
        diagnostics.push(InteractiveDiagnostic::warning(
            "form.acroform.not_dictionary",
            "catalog /AcroForm did not resolve to a dictionary",
        ));
        return Ok(FormReport {
            has_acroform: true,
            need_appearances: false,
            sig_flags: None,
            calculation_order_len: 0,
            fields: Vec::new(),
            xfa: XfaReport {
                present: false,
                packet_count: 0,
                dynamic: None,
                supported: false,
            },
            diagnostics,
        });
    };

    let page_annots = page_annotation_refs(reader, &pages)?;
    let mut fields = Vec::new();
    let mut visited = BTreeSet::new();
    let field_items = acroform_dict
        .get("Fields")
        .and_then(|obj| reader.resolve(obj.clone()).ok())
        .and_then(|obj| obj.as_array().map(|items| items.to_vec()))
        .unwrap_or_default();
    for field in &field_items {
        walk_form_field(
            reader,
            field,
            "",
            InheritedFieldAttrs::default(),
            &page_annots,
            0,
            &mut visited,
            &mut fields,
            &mut diagnostics,
        )?;
    }

    let xfa = xfa_report(acroform_dict, reader);
    if xfa.present {
        diagnostics.push(InteractiveDiagnostic::warning(
            "form.xfa.detected",
            "XFA packets are detected and reported; dynamic XFA execution/flattening is unsupported",
        ));
    }
    if acroform_dict.get("CO").is_some() {
        diagnostics.push(InteractiveDiagnostic::info(
            "form.calculation_order.detected",
            "AcroForm calculation order is reported; JavaScript calculations are not executed",
        ));
    }

    Ok(FormReport {
        has_acroform: true,
        need_appearances: acroform_dict.get_bool("NeedAppearances").unwrap_or(false),
        sig_flags: acroform_dict.get_integer("SigFlags"),
        calculation_order_len: acroform_dict
            .get("CO")
            .and_then(|obj| reader.resolve(obj.clone()).ok())
            .and_then(|obj| obj.as_array().map(|items| items.len()))
            .unwrap_or(0),
        fields,
        xfa,
        diagnostics,
    })
}

#[allow(clippy::too_many_arguments)]
fn walk_form_field(
    reader: &PdfReader,
    object: &PdfObject,
    parent_name: &str,
    inherited: InheritedFieldAttrs,
    page_annots: &BTreeMap<(u32, u16), usize>,
    depth: usize,
    visited: &mut BTreeSet<(u32, u16)>,
    fields: &mut Vec<FormFieldReport>,
    diagnostics: &mut Vec<InteractiveDiagnostic>,
) -> Result<()> {
    if depth > MAX_FIELD_DEPTH {
        diagnostics.push(InteractiveDiagnostic::warning(
            "form.field.depth_cap",
            "field tree traversal hit depth cap",
        ));
        return Ok(());
    }
    let object_ref = object.as_reference();
    if let Some(reference) = object_ref {
        if !visited.insert(reference) {
            diagnostics.push(InteractiveDiagnostic::warning(
                "form.field.cycle",
                format!(
                    "field tree cycle or duplicate reference at {}",
                    object_ref_string(reference)
                ),
            ));
            return Ok(());
        }
    }
    let resolved = reader.resolve(object.clone())?;
    let Some(dict) = resolved.as_dict().cloned() else {
        return Ok(());
    };
    let local_name = dict.get("T").and_then(pdf_string_or_name);
    let full_name = join_field_name(parent_name, local_name.as_deref());
    let inherited = inherit_field_attrs(&dict, reader, inherited);
    let kids = resolve_array(reader, dict.get("Kids"));
    let child_fields: Vec<PdfObject> = kids
        .iter()
        .filter(|kid| kid_is_field(reader, kid))
        .cloned()
        .collect();
    if !child_fields.is_empty() {
        for kid in &child_fields {
            walk_form_field(
                reader,
                kid,
                &full_name,
                inherited.clone(),
                page_annots,
                depth + 1,
                visited,
                fields,
                diagnostics,
            )?;
        }
        return Ok(());
    }

    let Some(ft) = inherited
        .ft
        .as_ref()
        .and_then(|attr| attr.object.as_name().map(str::to_string))
    else {
        return Ok(());
    };
    let mut field_diags = Vec::new();
    let widgets = collect_field_widgets(
        reader,
        &FieldNodeContext {
            object_ref,
            dict: dict.clone(),
            name: full_name.clone(),
        },
        &kids,
        page_annots,
        &mut field_diags,
    )?;
    let has_javascript = has_javascript_action(&dict, reader)
        || kids.iter().any(|kid| {
            reader
                .resolve(kid.clone())
                .ok()
                .and_then(|obj| obj.as_dict().cloned())
                .map(|kid_dict| has_javascript_action(&kid_dict, reader))
                .unwrap_or(false)
        });
    if has_javascript {
        field_diags.push(InteractiveDiagnostic::warning(
            "form.javascript.detected",
            format!(
                "field '{}' contains JavaScript action(s); not executed",
                full_name
            ),
        ));
    }
    let attributes = vec![
        attr_source("FT", &inherited.ft),
        attr_source("Ff", &inherited.ff),
        attr_source("DA", &inherited.da),
        attr_source("DR", &inherited.dr),
        attr_source("Q", &inherited.q),
        attr_source("Opt", &inherited.opt),
        attr_source("MaxLen", &inherited.max_len),
        attr_source("V", &inherited.v),
        attr_source("DV", &inherited.dv),
    ]
    .into_iter()
    .flatten()
    .collect();
    fields.push(FormFieldReport {
        full_name: if full_name.is_empty() {
            "(unnamed field)".to_string()
        } else {
            full_name
        },
        partial_name: local_name,
        field_type: field_type_label(
            &ft,
            inherited.ff.as_ref().and_then(|f| f.object.as_integer()),
        ),
        flags: inherited
            .ff
            .as_ref()
            .and_then(|attr| attr.object.as_integer()),
        value: inherited
            .v
            .as_ref()
            .and_then(|attr| form_value_text(&attr.object)),
        default_value: inherited
            .dv
            .as_ref()
            .and_then(|attr| form_value_text(&attr.object)),
        attributes,
        widgets,
        is_signature: ft == "Sig",
        has_javascript,
        diagnostics: field_diags,
    });
    Ok(())
}

fn inherit_field_attrs(
    dict: &PdfDictionary,
    reader: &PdfReader,
    mut inherited: InheritedFieldAttrs,
) -> InheritedFieldAttrs {
    inherit_one(dict, reader, "FT", &mut inherited.ft);
    inherit_one(dict, reader, "Ff", &mut inherited.ff);
    inherit_one(dict, reader, "DA", &mut inherited.da);
    inherit_one(dict, reader, "DR", &mut inherited.dr);
    inherit_one(dict, reader, "Q", &mut inherited.q);
    inherit_one(dict, reader, "Opt", &mut inherited.opt);
    inherit_one(dict, reader, "MaxLen", &mut inherited.max_len);
    inherit_one(dict, reader, "V", &mut inherited.v);
    inherit_one(dict, reader, "DV", &mut inherited.dv);
    inherited
}

fn inherit_one(
    dict: &PdfDictionary,
    reader: &PdfReader,
    key: &str,
    target: &mut Option<FieldAttr>,
) {
    if let Some(value) = dict
        .get(key)
        .and_then(|obj| reader.resolve(obj.clone()).ok())
    {
        *target = Some(FieldAttr {
            object: value,
            inherited: false,
        });
    } else if let Some(existing) = target.as_mut() {
        existing.inherited = true;
    }
}

fn collect_field_widgets(
    reader: &PdfReader,
    node: &FieldNodeContext,
    kids: &[PdfObject],
    page_annots: &BTreeMap<(u32, u16), usize>,
    diagnostics: &mut Vec<InteractiveDiagnostic>,
) -> Result<Vec<FormWidgetReport>> {
    let mut widgets = Vec::new();
    if node.dict.get_name("Subtype") == Some("Widget") || node.dict.get("Rect").is_some() {
        widgets.push(widget_report(
            reader,
            node.object_ref,
            &node.dict,
            page_annots,
            diagnostics,
        ));
    }
    for kid in kids {
        let kid_ref = kid.as_reference();
        let resolved = reader.resolve(kid.clone())?;
        let Some(kid_dict) = resolved.as_dict() else {
            continue;
        };
        if kid_dict.get_name("Subtype") == Some("Widget") || kid_dict.get("Rect").is_some() {
            widgets.push(widget_report(
                reader,
                kid_ref,
                kid_dict,
                page_annots,
                diagnostics,
            ));
        }
    }
    if widgets.is_empty() {
        diagnostics.push(InteractiveDiagnostic::warning(
            "form.widget.missing",
            format!("field '{}' has no widget annotation", node.name),
        ));
    }
    Ok(widgets)
}

fn widget_report(
    reader: &PdfReader,
    object_ref: Option<(u32, u16)>,
    dict: &PdfDictionary,
    page_annots: &BTreeMap<(u32, u16), usize>,
    diagnostics: &mut Vec<InteractiveDiagnostic>,
) -> FormWidgetReport {
    let rect = rect_of(dict, reader);
    let page = object_ref.and_then(|reference| page_annots.get(&reference).copied());
    if page.is_none() {
        diagnostics.push(InteractiveDiagnostic::warning(
            "form.widget.orphan",
            "widget is not reachable from any page /Annots array",
        ));
    }
    FormWidgetReport {
        page,
        rect,
        has_appearance: dict.get("AP").is_some(),
        annotation_flags: dict.get_integer("F"),
        object: object_ref.map(object_ref_string),
    }
}

fn xfa_report(acroform: &PdfDictionary, reader: &PdfReader) -> XfaReport {
    let Some(xfa_obj) = acroform.get("XFA") else {
        return XfaReport {
            present: false,
            packet_count: 0,
            dynamic: None,
            supported: false,
        };
    };
    let resolved = reader.resolve(xfa_obj.clone()).unwrap_or(PdfObject::Null);
    let packet_count = match &resolved {
        PdfObject::Array(items) => items.len() / 2,
        PdfObject::Stream { .. } => 1,
        _ => 0,
    };
    XfaReport {
        present: true,
        packet_count,
        dynamic: None,
        supported: false,
    }
}

fn annotation_report_document(document: &PdfDocument) -> Result<AnnotationReport> {
    let reader = document.reader();
    let pages = document.get_pages()?;
    let mut annotations = Vec::new();
    let mut by_subtype = BTreeMap::new();
    let mut unsafe_actions = 0usize;
    let mut diagnostics = Vec::new();
    for page in &pages {
        let page_obj = reader.get_and_resolve(page.object_number, page.generation_number)?;
        let Some(page_dict) = page_obj.as_dict() else {
            continue;
        };
        let annot_refs = resolve_annotation_entries(reader, page_dict.get("Annots"))?;
        for (index, (annot_ref, annot_obj)) in annot_refs.into_iter().enumerate() {
            let Some(dict) = annot_obj.as_dict() else {
                continue;
            };
            let subtype = dict.get_name("Subtype").unwrap_or("Unknown").to_string();
            *by_subtype.entry(subtype.clone()).or_default() += 1;
            let action = annotation_action(reader, dict.get("A"));
            if action.as_ref().is_some_and(|a| !a.safe) {
                unsafe_actions += 1;
                diagnostics.push(InteractiveDiagnostic::warning_on_page(
                    "annotation.action.unsafe",
                    page.page_number,
                    format!("annotation /{subtype} contains an unsafe action"),
                ));
            }
            let mut annotation_diags = Vec::new();
            if matches!(
                subtype.as_str(),
                "Highlight" | "Underline" | "StrikeOut" | "Squiggly"
            ) && dict.get("QuadPoints").is_none()
            {
                annotation_diags.push(InteractiveDiagnostic::warning_on_page(
                    "annotation.quadpoints.missing",
                    page.page_number,
                    format!("text markup annotation /{subtype} has no QuadPoints"),
                ));
            }
            annotations.push(AnnotationInfo {
                page: page.page_number,
                index,
                subtype,
                rect: rect_of(dict, reader),
                contents: dict.get("Contents").and_then(pdf_string_or_name),
                flags: dict.get_integer("F"),
                color: number_array(reader, dict.get("C")),
                quad_points: quad_points(reader, dict.get("QuadPoints")),
                has_appearance: dict.get("AP").is_some(),
                action,
                object: annot_ref.map(object_ref_string),
                diagnostics: annotation_diags,
            });
        }
    }
    Ok(AnnotationReport {
        annotations,
        by_subtype,
        unsafe_actions,
        diagnostics,
    })
}

fn page_operations_report_document(document: &PdfDocument) -> Result<PageOperationsReport> {
    let catalog = document.get_catalog()?;
    let reader = document.reader();
    let pages = document.get_pages()?;
    let mut diagnostics = Vec::new();
    let page_reports = pages
        .iter()
        .map(|page| page_box_report(reader, page))
        .collect::<Result<Vec<_>>>()?;
    let outline_count = match catalog.get("Outlines") {
        Some(obj) => count_outline_nodes(reader, obj, &mut diagnostics),
        None => 0,
    };
    let signatures_may_be_invalidated_by_rewrite =
        catalog.get("AcroForm").is_some() || reader.trailer().get("Encrypt").is_some();
    if signatures_may_be_invalidated_by_rewrite {
        diagnostics.push(InteractiveDiagnostic::warning(
            "pageops.signature_invalidation",
            "full-save page operations may invalidate existing signatures; Prompt 09 owns cryptographic validation",
        ));
    }
    Ok(PageOperationsReport {
        page_count: pages.len(),
        pages: page_reports,
        outlines_present: catalog.get("Outlines").is_some(),
        outline_count,
        page_labels_present: catalog.get("PageLabels").is_some(),
        named_destinations_present: catalog.get("Dests").is_some()
            || catalog
                .get("Names")
                .and_then(|obj| reader.resolve(obj.clone()).ok())
                .and_then(|obj| obj.as_dict().cloned())
                .is_some_and(|names| names.get("Dests").is_some()),
        embedded_files_present: catalog
            .get("Names")
            .and_then(|obj| reader.resolve(obj.clone()).ok())
            .and_then(|obj| obj.as_dict().cloned())
            .is_some_and(|names| names.get("EmbeddedFiles").is_some()),
        acroform_present: catalog.get("AcroForm").is_some(),
        signatures_may_be_invalidated_by_rewrite,
        diagnostics,
    })
}

fn page_box_report(reader: &PdfReader, page: &PdfPage) -> Result<PageBoxReport> {
    let page_obj = reader.get_and_resolve(page.object_number, page.generation_number)?;
    let annotations = page_obj
        .as_dict()
        .and_then(|dict| dict.get("Annots"))
        .and_then(|obj| reader.resolve(obj.clone()).ok())
        .and_then(|obj| obj.as_array().map(|items| items.len()))
        .unwrap_or(0);
    Ok(PageBoxReport {
        page: page.page_number,
        object: object_ref_string((page.object_number, page.generation_number)),
        media_box: page.media_box,
        crop_box: page.crop_box,
        rotate: page.rotate,
        annotations,
    })
}

fn count_outline_nodes(
    reader: &PdfReader,
    outlines_obj: &PdfObject,
    diagnostics: &mut Vec<InteractiveDiagnostic>,
) -> usize {
    let Ok(outlines) = reader.resolve(outlines_obj.clone()) else {
        return 0;
    };
    let Some(dict) = outlines.as_dict() else {
        return 0;
    };
    let mut count = 0usize;
    let mut stack = Vec::new();
    let mut seen = BTreeSet::new();
    if let Some(first) = dict.get("First") {
        stack.push(first.clone());
    }
    while let Some(node_obj) = stack.pop() {
        if count >= MAX_OUTLINE_NODES {
            diagnostics.push(InteractiveDiagnostic::warning(
                "pageops.outlines.cap",
                "outline traversal hit node cap",
            ));
            break;
        }
        if let Some(reference) = node_obj.as_reference() {
            if !seen.insert(reference) {
                continue;
            }
        }
        let Ok(node) = reader.resolve(node_obj) else {
            continue;
        };
        let Some(node_dict) = node.as_dict() else {
            continue;
        };
        count += 1;
        if let Some(next) = node_dict.get("Next") {
            stack.push(next.clone());
        }
        if let Some(first) = node_dict.get("First") {
            stack.push(first.clone());
        }
    }
    count
}

fn page_annotation_refs(
    reader: &PdfReader,
    pages: &[PdfPage],
) -> Result<BTreeMap<(u32, u16), usize>> {
    let mut map = BTreeMap::new();
    for page in pages {
        let page_obj = reader.get_and_resolve(page.object_number, page.generation_number)?;
        let Some(page_dict) = page_obj.as_dict() else {
            continue;
        };
        for (annot_ref, _) in resolve_annotation_entries(reader, page_dict.get("Annots"))? {
            if let Some(reference) = annot_ref {
                map.insert(reference, page.page_number);
            }
        }
    }
    Ok(map)
}

fn resolve_annotation_entries(
    reader: &PdfReader,
    annots: Option<&PdfObject>,
) -> Result<Vec<AnnotationEntry>> {
    let Some(annots) = annots else {
        return Ok(Vec::new());
    };
    let resolved = reader.resolve(annots.clone())?;
    let Some(items) = resolved.as_array() else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for item in items {
        let reference = item.as_reference();
        let obj = reader.resolve(item.clone())?;
        out.push((reference, obj));
    }
    Ok(out)
}

fn kid_is_field(reader: &PdfReader, object: &PdfObject) -> bool {
    reader
        .resolve(object.clone())
        .ok()
        .and_then(|obj| obj.as_dict().cloned())
        .is_some_and(|dict| dict.contains_key("T") || dict.contains_key("FT"))
}

fn resolve_array(reader: &PdfReader, object: Option<&PdfObject>) -> Vec<PdfObject> {
    object
        .and_then(|obj| reader.resolve(obj.clone()).ok())
        .and_then(|obj| obj.as_array().map(|items| items.to_vec()))
        .unwrap_or_default()
}

fn rect_of(dict: &PdfDictionary, reader: &PdfReader) -> Option<[f64; 4]> {
    let values = number_array(reader, dict.get("Rect"))?;
    if values.len() != 4 {
        return None;
    }
    Some([
        values[0].min(values[2]),
        values[1].min(values[3]),
        values[0].max(values[2]),
        values[1].max(values[3]),
    ])
}

fn number_array(reader: &PdfReader, object: Option<&PdfObject>) -> Option<Vec<f64>> {
    let resolved = reader.resolve(object?.clone()).ok()?;
    let array = resolved.as_array()?;
    let mut out = Vec::with_capacity(array.len());
    for item in array {
        out.push(reader.resolve(item.clone()).ok()?.as_number()?);
    }
    Some(out)
}

fn quad_points(reader: &PdfReader, object: Option<&PdfObject>) -> Vec<[f64; 8]> {
    let Some(values) = number_array(reader, object) else {
        return Vec::new();
    };
    values
        .chunks_exact(8)
        .map(|chunk| {
            [
                chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6], chunk[7],
            ]
        })
        .collect()
}

fn annotation_action(
    reader: &PdfReader,
    action: Option<&PdfObject>,
) -> Option<AnnotationActionInfo> {
    let action = reader.resolve(action?.clone()).ok()?;
    let dict = action.as_dict()?;
    let kind = dict.get_name("S").unwrap_or("Unknown").to_string();
    let target = dict
        .get("URI")
        .or_else(|| dict.get("D"))
        .or_else(|| dict.get("F"))
        .or_else(|| dict.get("JS"))
        .and_then(action_target_string);
    let safe = !matches!(kind.as_str(), "Launch" | "JavaScript" | "SubmitForm");
    Some(AnnotationActionInfo { kind, safe, target })
}

fn has_javascript_action(dict: &PdfDictionary, reader: &PdfReader) -> bool {
    dict.get("A")
        .and_then(|action| annotation_action(reader, Some(action)))
        .is_some_and(|action| action.kind == "JavaScript")
        || dict.get("AA").is_some_and(|aa| {
            reader
                .resolve(aa.clone())
                .ok()
                .and_then(|obj| obj.as_dict().cloned())
                .is_some_and(|aa_dict| {
                    aa_dict.entries().any(|(_, action)| {
                        annotation_action(reader, Some(action))
                            .is_some_and(|info| info.kind == "JavaScript")
                    })
                })
        })
}

fn action_target_string(object: &PdfObject) -> Option<String> {
    match object {
        PdfObject::String(bytes) => Some(decode_pdf_text_string(bytes)),
        PdfObject::Name(name) => Some(name.clone()),
        PdfObject::Array(_) => Some("[array destination]".to_string()),
        PdfObject::Dictionary(_) => Some("[dictionary]".to_string()),
        PdfObject::Reference { number, generation } => {
            Some(object_ref_string((*number, *generation)))
        }
        _ => None,
    }
}

fn attr_source(name: &str, attr: &Option<FieldAttr>) -> Option<FieldAttributeSource> {
    attr.as_ref().map(|attr| FieldAttributeSource {
        name: name.to_string(),
        inherited: attr.inherited,
    })
}

fn field_type_label(ft: &str, flags: Option<i64>) -> String {
    match ft {
        "Tx" => "text".to_string(),
        "Ch" => "choice".to_string(),
        "Btn" => {
            let flags = flags.unwrap_or(0);
            if flags & (1 << 16) != 0 {
                "push_button".to_string()
            } else if flags & (1 << 15) != 0 {
                "radio".to_string()
            } else {
                "checkbox".to_string()
            }
        }
        "Sig" => "signature".to_string(),
        other => format!("unknown:{other}"),
    }
}

fn form_value_text(object: &PdfObject) -> Option<String> {
    match object {
        PdfObject::String(bytes) => Some(decode_pdf_text_string(bytes)),
        PdfObject::Name(name) => Some(name.clone()),
        PdfObject::Array(items) => Some(
            items
                .iter()
                .filter_map(pdf_string_or_name)
                .collect::<Vec<_>>()
                .join(", "),
        ),
        _ => None,
    }
}

fn pdf_string_or_name(object: &PdfObject) -> Option<String> {
    match object {
        PdfObject::String(bytes) => Some(decode_pdf_text_string(bytes)),
        PdfObject::Name(name) => Some(name.clone()),
        _ => None,
    }
}

fn join_field_name(parent: &str, local: Option<&str>) -> String {
    match (parent.is_empty(), local.unwrap_or("").is_empty()) {
        (true, true) => String::new(),
        (true, false) => local.unwrap_or("").to_string(),
        (false, true) => parent.to_string(),
        (false, false) => format!("{}.{}", parent, local.unwrap_or("")),
    }
}

fn object_ref_string(reference: (u32, u16)) -> String {
    format!("{} {} R", reference.0, reference.1)
}
