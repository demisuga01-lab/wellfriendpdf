//! Bounded XFA packet discovery, static extraction, minimal dynamic layout, and
//! active-content sandbox policy.
//!
//! This module deliberately implements a useful subset rather than an Adobe
//! LiveCycle/AEM compatibility claim. XML is parsed without DTDs or external
//! entities; scripts are disabled by default; JavaScript is always inventory
//! only; and every recursive or generated structure is capped.

mod script;
pub(crate) mod xml;

use std::collections::{BTreeMap, BTreeSet};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::cancel::CancelToken;
use crate::content::Color;
use crate::decode_scheduler::{DecodeSchedulerContext, DecodeSchedulerMetrics};
use crate::editing::{EditMode, EditRectStyle, EditTextStyle, ImageRect, OverlayLayer, PdfEditor};
use crate::error::{Result, WellfriendError};
use crate::filters::{decode_stream_with_limits, DecodeLimits};
use crate::info::decode_pdf_text_string;
use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;
use crate::versioning::resource_digest;
use crate::writer::{rewrite_document_with_mode, WriterMode};
use crate::{ContentEngine, PdfDocument};

use self::script::evaluate_formcalc;
use self::xml::{
    is_external_reference, parse_xml, serialize_sanitized, ParsedXml, XmlMetrics, XmlNode,
};

#[derive(Debug, Clone, Copy)]
struct RuntimeInstant {
    #[cfg(not(target_arch = "wasm32"))]
    inner: Instant,
    #[cfg(target_arch = "wasm32")]
    epoch_millis: f64,
}

impl RuntimeInstant {
    fn now() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            inner: Instant::now(),
            #[cfg(target_arch = "wasm32")]
            epoch_millis: js_sys::Date::now(),
        }
    }

    fn elapsed_millis(self) -> u128 {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.inner.elapsed().as_millis()
        }
        #[cfg(target_arch = "wasm32")]
        {
            (js_sys::Date::now() - self.epoch_millis).max(0.0) as u128
        }
    }

    fn elapsed_micros(self) -> u128 {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.inner.elapsed().as_micros()
        }
        #[cfg(target_arch = "wasm32")]
        {
            ((js_sys::Date::now() - self.epoch_millis).max(0.0) * 1_000.0) as u128
        }
    }
}

pub const XFA_SCHEMA_VERSION: &str = "xfa_runtime.xfa.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum XfaSupportStatus {
    Implemented,
    ImplementedWithLimits,
    UnsupportedReportedExact,
    UnsupportedReportedSecurityPolicy,
    NotInXFARuntimeScope,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XfaLimits {
    pub max_packets: usize,
    pub max_xml_bytes: usize,
    pub max_packet_decoded_bytes: usize,
    pub max_xml_nodes: usize,
    pub max_xml_attributes: usize,
    pub max_namespace_declarations: usize,
    pub max_xml_depth: usize,
    pub max_text_node_bytes: usize,
    pub max_xml_attribute_value_bytes: usize,
    pub max_entity_references: usize,
    pub max_dataset_nodes: usize,
    pub max_subform_depth: usize,
    pub max_instances_per_subform: usize,
    pub max_generated_nodes: usize,
    pub max_generated_pages: usize,
    pub max_relayout_iterations: usize,
    pub max_event_executions: usize,
    pub max_script_instructions: usize,
    pub max_script_source_bytes: usize,
    pub max_script_memory_bytes: usize,
    pub max_script_call_depth: usize,
    pub max_script_loop_iterations: usize,
    pub max_script_object_properties: usize,
    pub max_script_string_bytes: usize,
    pub max_field_mutations: usize,
    pub max_runtime_ms: u64,
    pub max_output_bytes: usize,
    pub max_image_pixels: u64,
    pub scheduler_memory_budget_bytes: u64,
}

impl Default for XfaLimits {
    fn default() -> Self {
        Self {
            max_packets: 64,
            max_xml_bytes: 16 * 1024 * 1024,
            max_packet_decoded_bytes: 8 * 1024 * 1024,
            max_xml_nodes: 100_000,
            max_xml_attributes: 250_000,
            max_namespace_declarations: 8_192,
            max_xml_depth: 64,
            max_text_node_bytes: 1024 * 1024,
            max_xml_attribute_value_bytes: 256 * 1024,
            max_entity_references: 100_000,
            max_dataset_nodes: 50_000,
            max_subform_depth: 32,
            max_instances_per_subform: 1_024,
            max_generated_nodes: 50_000,
            max_generated_pages: 256,
            max_relayout_iterations: 16,
            max_event_executions: 256,
            max_script_instructions: 10_000,
            max_script_source_bytes: 256 * 1024,
            max_script_memory_bytes: 8 * 1024 * 1024,
            max_script_call_depth: 32,
            max_script_loop_iterations: 0,
            max_script_object_properties: 1_024,
            max_script_string_bytes: 1024 * 1024,
            max_field_mutations: 1_024,
            max_runtime_ms: 2_000,
            max_output_bytes: 128 * 1024 * 1024,
            max_image_pixels: 50_000_000,
            scheduler_memory_budget_bytes: 64 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XfaDiagnostic {
    pub code: String,
    pub severity: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packet: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_offset: Option<usize>,
}

impl XfaDiagnostic {
    pub(crate) fn info(code: &str, message: impl Into<String>, packet: Option<String>) -> Self {
        Self::new(code, "info", message, packet, None, None)
    }

    fn warning(code: &str, message: impl Into<String>, packet: Option<String>) -> Self {
        Self::new(code, "warning", message, packet, None, None)
    }

    fn error(code: &str, message: impl Into<String>, packet: Option<String>) -> Self {
        Self::new(code, "error", message, packet, None, None)
    }

    fn new(
        code: &str,
        severity: &str,
        message: impl Into<String>,
        packet: Option<String>,
        object: Option<String>,
        source_offset: Option<usize>,
    ) -> Self {
        Self {
            code: code.to_string(),
            severity: severity.to_string(),
            message: message.into(),
            packet,
            object,
            source_offset,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XfaProvenance {
    pub packet: String,
    pub packet_order: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_start: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_end: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub som_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XfaPacketRecord {
    pub order: usize,
    pub name: String,
    pub container: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_reference: Option<String>,
    pub decoded_byte_length: usize,
    pub content_sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xml_root_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xml_root_namespace: Option<String>,
    pub parse_status: String,
    pub duplicate: bool,
    pub malformed: bool,
    pub encryption_decode_status: String,
    pub provenance: XfaProvenance,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct XfaXmlMetrics {
    pub total_bytes: usize,
    pub packet_count: usize,
    pub parsed_packets: usize,
    pub malformed_packets: usize,
    pub node_count: usize,
    pub attribute_count: usize,
    pub namespace_declarations: usize,
    pub max_depth: usize,
    pub text_bytes: usize,
    pub entity_references: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XfaClassification {
    pub kind: String,
    pub static_xfa: bool,
    pub dynamic_xfa: bool,
    pub hybrid_acroform_xfa: bool,
    pub static_page_backed: bool,
    pub dynamic_reflowing_subforms: bool,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XfaInventoryReport {
    pub schema_version: String,
    pub status: XfaSupportStatus,
    pub present: bool,
    pub acroform_present: bool,
    pub source_form: String,
    pub packets: Vec<XfaPacketRecord>,
    pub packet_order: Vec<String>,
    pub classification: XfaClassification,
    pub xml_safety: XfaXmlSafetyReport,
    pub metrics: XfaXmlMetrics,
    pub limits: XfaLimits,
    pub diagnostics: Vec<XfaDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XfaXmlSafetyReport {
    pub external_entities_enabled: bool,
    pub dtd_retrieval_enabled: bool,
    pub network_access_enabled: bool,
    pub filesystem_access_enabled: bool,
    pub invalid_utf8_rejected: bool,
    pub unknown_entities_rejected: bool,
    pub non_finite_numbers_rejected: bool,
    pub deterministic_diagnostics: bool,
    pub fail_closed: bool,
}

impl Default for XfaXmlSafetyReport {
    fn default() -> Self {
        Self {
            external_entities_enabled: false,
            dtd_retrieval_enabled: false,
            network_access_enabled: false,
            filesystem_access_enabled: false,
            invalid_utf8_rejected: true,
            unknown_entities_rejected: true,
            non_finite_numbers_rejected: true,
            deterministic_diagnostics: true,
            fail_closed: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct XfaGeometry {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub rotation_degrees: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XfaOccur {
    pub min: usize,
    pub max: Option<usize>,
    pub initial: usize,
}

impl Default for XfaOccur {
    fn default() -> Self {
        Self {
            min: 1,
            max: Some(1),
            initial: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XfaBindingRecord {
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    pub status: XfaSupportStatus,
    pub matched_nodes: usize,
    pub raw_values: Vec<String>,
    pub coercion: String,
    pub diagnostics: Vec<XfaDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct XfaFieldRecord {
    pub name: String,
    pub som_path: String,
    pub field_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tooltip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    pub choices: Vec<String>,
    pub selected_values: Vec<String>,
    pub required: bool,
    pub validation_state: String,
    pub presence: String,
    pub tab_index: Option<i64>,
    pub geometry: Option<XfaGeometry>,
    pub font_family: Option<String>,
    pub font_size_points: Option<f64>,
    pub border: Option<String>,
    pub fill: Option<String>,
    pub binding: XfaBindingRecord,
    pub provenance: XfaProvenance,
    pub diagnostics: Vec<XfaDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct XfaDrawRecord {
    pub name: String,
    pub som_path: String,
    pub draw_type: String,
    pub text: Option<String>,
    pub geometry: Option<XfaGeometry>,
    pub presence: String,
    pub provenance: XfaProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XfaSubformRecord {
    pub name: String,
    pub som_path: String,
    pub layout: String,
    pub presence: String,
    pub occur: XfaOccur,
    pub bind_expression: Option<String>,
    pub child_nodes: usize,
    pub provenance: XfaProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XfaDataNode {
    pub name: String,
    pub value: Option<String>,
    pub attributes: BTreeMap<String, String>,
    pub children: Vec<XfaDataNode>,
    pub provenance: XfaProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XfaScriptRecord {
    pub order: usize,
    pub language: String,
    pub event: String,
    pub target_som: String,
    pub source_bytes: usize,
    pub source_sha256: String,
    pub default_execution: String,
    pub support_status: XfaSupportStatus,
    pub blocked_capabilities: Vec<String>,
    pub provenance: XfaProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XfaEventRecord {
    pub order: usize,
    pub activity: String,
    pub target_som: String,
    pub script_count: usize,
    pub default_execution: String,
    pub support_status: XfaSupportStatus,
    pub provenance: XfaProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XfaRagChunk {
    pub id: String,
    pub text: String,
    pub page: Option<usize>,
    pub som_path: String,
    pub packet: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XfaSemanticIntegrationReport {
    pub forms_model_fields: usize,
    pub semantic_text_entries: usize,
    pub provenance_entries: usize,
    pub search_index_terms: Vec<String>,
    pub rag_chunks: Vec<XfaRagChunk>,
    pub accessibility_labels: usize,
    pub accessibility_status: XfaSupportStatus,
    pub redaction_visible_entries: usize,
    pub security_report_linked: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct XfaExtractionReport {
    pub schema_version: String,
    pub status: XfaSupportStatus,
    pub inventory: XfaInventoryReport,
    pub template_parsed: bool,
    pub datasets_parsed: bool,
    pub fields: Vec<XfaFieldRecord>,
    pub draws: Vec<XfaDrawRecord>,
    pub subforms: Vec<XfaSubformRecord>,
    pub datasets: Vec<XfaDataNode>,
    pub scripts: Vec<XfaScriptRecord>,
    pub events: Vec<XfaEventRecord>,
    pub unsupported_constructs: Vec<String>,
    pub semantic_integration: XfaSemanticIntegrationReport,
    pub diagnostics: Vec<XfaDiagnostic>,
}

#[derive(Debug, Clone)]
struct PacketSource {
    record: XfaPacketRecord,
    bytes: Vec<u8>,
    parsed: Option<ParsedXml>,
    diagnostics: Vec<XfaDiagnostic>,
}

#[derive(Debug, Clone)]
struct LoadedXfa {
    inventory: XfaInventoryReport,
    packets: Vec<PacketSource>,
}

#[derive(Debug, Clone)]
struct ScriptSource {
    record: XfaScriptRecord,
    source: String,
}

pub fn xfa_inventory(engine: &ContentEngine, limits: &XfaLimits) -> Result<XfaInventoryReport> {
    Ok(load_xfa(engine.document(), limits, &CancelToken::none())?.inventory)
}

pub fn xfa_inventory_cancellable(
    engine: &ContentEngine,
    limits: &XfaLimits,
    cancel: &CancelToken,
) -> Result<XfaInventoryReport> {
    Ok(load_xfa(engine.document(), limits, cancel)?.inventory)
}

pub fn extract_xfa(engine: &ContentEngine, limits: &XfaLimits) -> Result<XfaExtractionReport> {
    let loaded = load_xfa(engine.document(), limits, &CancelToken::none())?;
    extract_loaded(loaded, limits)
}

fn load_xfa(document: &PdfDocument, limits: &XfaLimits, cancel: &CancelToken) -> Result<LoadedXfa> {
    let started = RuntimeInstant::now();
    cancel.check("xfa packet discovery")?;
    let catalog = document.get_catalog()?;
    let reader = document.reader();
    let acroform_present = catalog.get("AcroForm").is_some();
    let mut diagnostics = Vec::new();
    let Some(acroform_obj) = catalog.get("AcroForm") else {
        return Ok(LoadedXfa {
            inventory: empty_inventory(limits, false),
            packets: Vec::new(),
        });
    };
    let acroform = match reader.resolve(acroform_obj.clone()) {
        Ok(object) => object,
        Err(err) => {
            diagnostics.push(XfaDiagnostic::error(
                "xfa.acroform.resolve_failed",
                err.to_string(),
                None,
            ));
            return Ok(LoadedXfa {
                inventory: XfaInventoryReport {
                    diagnostics,
                    acroform_present,
                    ..empty_inventory(limits, acroform_present)
                },
                packets: Vec::new(),
            });
        }
    };
    let Some(acroform_dict) = acroform.as_dict() else {
        diagnostics.push(XfaDiagnostic::warning(
            "xfa.acroform.not_dictionary",
            "catalog /AcroForm did not resolve to a dictionary",
            None,
        ));
        return Ok(LoadedXfa {
            inventory: XfaInventoryReport {
                diagnostics,
                acroform_present,
                ..empty_inventory(limits, acroform_present)
            },
            packets: Vec::new(),
        });
    };
    let Some(xfa_object) = acroform_dict.get("XFA") else {
        return Ok(LoadedXfa {
            inventory: empty_inventory(limits, acroform_present),
            packets: Vec::new(),
        });
    };

    let resolved = reader.resolve(xfa_object.clone());
    let source_form;
    let mut packets = Vec::new();
    match resolved {
        Ok(PdfObject::Array(items)) => {
            source_form = "array".to_string();
            if items.len() / 2 > limits.max_packets {
                return Err(WellfriendError::ResourceLimit(format!(
                    "XFA packet count {} exceeds cap {}",
                    items.len() / 2,
                    limits.max_packets
                )));
            }
            if items.len() % 2 != 0 {
                diagnostics.push(XfaDiagnostic::warning(
                    "xfa.packet_array.odd_length",
                    "AcroForm /XFA array has an unmatched packet name or stream",
                    None,
                ));
            }
            for (order, pair) in items.chunks(2).enumerate() {
                cancel.check("xfa packet array")?;
                check_runtime(started, limits)?;
                let name = pair
                    .first()
                    .and_then(packet_name)
                    .unwrap_or_else(|| format!("unnamed_{order}"));
                let Some(stream_obj) = pair.get(1) else {
                    break;
                };
                packets.push(load_packet_source(
                    reader, name, "array", order, stream_obj, limits,
                ));
            }
        }
        Ok(PdfObject::Stream { .. }) => {
            source_form = "single_stream".to_string();
            let mut source = load_packet_source(
                reader,
                "single_stream".to_string(),
                "single_stream",
                0,
                xfa_object,
                limits,
            );
            if let Some(parsed) = &source.parsed {
                if parsed.root.local_name == "xdp" && !parsed.root.children.is_empty() {
                    for (order, child) in parsed.root.children.iter().enumerate() {
                        if order >= limits.max_packets {
                            return Err(WellfriendError::ResourceLimit(format!(
                                "XFA logical packet count exceeds cap {}",
                                limits.max_packets
                            )));
                        }
                        let start = child.start_offset.min(source.bytes.len());
                        let end = child.end_offset.min(source.bytes.len()).max(start);
                        let bytes = source.bytes[start..end].to_vec();
                        // The child may rely on a namespace declaration inherited from the
                        // xdp root. Reusing the already validated subtree preserves that
                        // namespace context instead of reparsing an incomplete XML fragment.
                        let parsed_child = Some(parsed_subtree(child));
                        let mut record = packet_record(
                            order,
                            child.local_name.clone(),
                            "single_stream",
                            source.record.object_reference.clone(),
                            &bytes,
                            parsed_child.as_ref(),
                            reader.trailer().get("Encrypt").is_some(),
                        );
                        record.provenance.source_start = Some(start);
                        record.provenance.source_end = Some(end);
                        packets.push(PacketSource {
                            record,
                            bytes,
                            parsed: parsed_child,
                            diagnostics: Vec::new(),
                        });
                    }
                } else {
                    source.record.name = parsed.root.local_name.clone();
                    source.record.provenance.packet = parsed.root.local_name.clone();
                    packets.push(source);
                }
            } else {
                packets.push(source);
            }
        }
        Ok(other) => {
            source_form = "malformed".to_string();
            diagnostics.push(XfaDiagnostic::error(
                "xfa.object.unsupported_type",
                format!("AcroForm /XFA resolved to {}", other.variant_name()),
                None,
            ));
        }
        Err(err) => {
            source_form = "decode_failed".to_string();
            diagnostics.push(XfaDiagnostic::error(
                "xfa.object.resolve_failed",
                err.to_string(),
                None,
            ));
        }
    }

    let mut seen = BTreeSet::new();
    for packet in &mut packets {
        diagnostics.extend(packet.diagnostics.clone());
        if let Some(parsed) = &packet.parsed {
            diagnostics.extend(parsed.diagnostics.clone());
        }
        packet.record.duplicate = !seen.insert(packet.record.name.to_ascii_lowercase());
        if packet.record.duplicate {
            diagnostics.push(XfaDiagnostic::warning(
                "xfa.packet.duplicate",
                format!(
                    "duplicate XFA packet '{}' preserved in source order",
                    packet.record.name
                ),
                Some(packet.record.name.clone()),
            ));
        }
        if packet.record.malformed {
            diagnostics.push(XfaDiagnostic::error(
                "xfa.packet.malformed",
                "packet XML was rejected without affecting unrelated PDF content",
                Some(packet.record.name.clone()),
            ));
        }
    }

    let metrics = aggregate_xml_metrics(&packets);
    let classification = classify_xfa(&packets, acroform_dict);
    let present = !packets.is_empty() || xfa_object.is_null();
    Ok(LoadedXfa {
        inventory: XfaInventoryReport {
            schema_version: XFA_SCHEMA_VERSION.to_string(),
            status: if metrics.malformed_packets == 0 {
                XfaSupportStatus::ImplementedWithLimits
            } else {
                XfaSupportStatus::UnsupportedReportedExact
            },
            present,
            acroform_present,
            source_form,
            packet_order: packets
                .iter()
                .map(|packet| packet.record.name.clone())
                .collect(),
            packets: packets.iter().map(|packet| packet.record.clone()).collect(),
            classification,
            xml_safety: XfaXmlSafetyReport::default(),
            metrics,
            limits: limits.clone(),
            diagnostics,
        },
        packets,
    })
}

fn empty_inventory(limits: &XfaLimits, acroform_present: bool) -> XfaInventoryReport {
    XfaInventoryReport {
        schema_version: XFA_SCHEMA_VERSION.to_string(),
        status: XfaSupportStatus::Implemented,
        present: false,
        acroform_present,
        source_form: "none".to_string(),
        packets: Vec::new(),
        packet_order: Vec::new(),
        classification: XfaClassification {
            kind: "none".to_string(),
            static_xfa: false,
            dynamic_xfa: false,
            hybrid_acroform_xfa: false,
            static_page_backed: false,
            dynamic_reflowing_subforms: false,
            evidence: Vec::new(),
        },
        xml_safety: XfaXmlSafetyReport::default(),
        metrics: XfaXmlMetrics::default(),
        limits: limits.clone(),
        diagnostics: Vec::new(),
    }
}

fn load_packet_source(
    reader: &PdfReader,
    name: String,
    container: &str,
    order: usize,
    stream_obj: &PdfObject,
    limits: &XfaLimits,
) -> PacketSource {
    let object_reference = stream_obj.as_reference().map(object_ref_string);
    let decode_limits = DecodeLimits {
        max_decoded_bytes_per_stream: limits.max_packet_decoded_bytes as u64,
        scheduler_memory_budget_bytes: limits.scheduler_memory_budget_bytes,
        ..DecodeLimits::default()
    };
    let mut diagnostics = Vec::new();
    let resolved = reader.resolve(stream_obj.clone());
    let bytes = match resolved.as_ref() {
        Ok(object) => match decode_stream_with_limits(object, reader, &decode_limits) {
            Ok(bytes) => bytes,
            Err(err) => {
                diagnostics.push(XfaDiagnostic::error(
                    "xfa.packet.decode_failed",
                    err.to_string(),
                    Some(name.clone()),
                ));
                Vec::new()
            }
        },
        Err(err) => {
            diagnostics.push(XfaDiagnostic::error(
                "xfa.packet.resolve_failed",
                err.to_string(),
                Some(name.clone()),
            ));
            Vec::new()
        }
    };
    let parsed = if bytes.is_empty() {
        None
    } else {
        match parse_xml(&bytes, limits) {
            Ok(parsed) => Some(parsed),
            Err(err) => {
                diagnostics.push(XfaDiagnostic::error(
                    "xfa.xml.rejected",
                    err.to_string(),
                    Some(name.clone()),
                ));
                None
            }
        }
    };
    let record = packet_record(
        order,
        name,
        container,
        object_reference,
        &bytes,
        parsed.as_ref(),
        reader.trailer().get("Encrypt").is_some(),
    );
    PacketSource {
        record,
        bytes,
        parsed,
        diagnostics,
    }
}

fn packet_record(
    order: usize,
    name: String,
    container: &str,
    object_reference: Option<String>,
    bytes: &[u8],
    parsed: Option<&ParsedXml>,
    encrypted: bool,
) -> XfaPacketRecord {
    let malformed = !bytes.is_empty() && parsed.is_none();
    XfaPacketRecord {
        order,
        provenance: XfaProvenance {
            packet: name.clone(),
            packet_order: order,
            object: object_reference.clone(),
            source_start: Some(0),
            source_end: Some(bytes.len()),
            som_path: None,
        },
        name,
        container: container.to_string(),
        object_reference,
        decoded_byte_length: bytes.len(),
        content_sha256: resource_digest(bytes),
        xml_root_name: parsed.map(|xml| xml.root.name.clone()),
        xml_root_namespace: parsed.and_then(|xml| xml.root.namespace_uri.clone()),
        parse_status: if bytes.is_empty() {
            "decode_failed_or_empty".to_string()
        } else if malformed {
            "malformed_rejected".to_string()
        } else {
            "parsed".to_string()
        },
        duplicate: false,
        malformed,
        encryption_decode_status: if encrypted {
            "decrypted_by_pdf_reader_before_xml_parse".to_string()
        } else {
            "not_encrypted".to_string()
        },
    }
}

fn packet_name(object: &PdfObject) -> Option<String> {
    match object {
        PdfObject::String(bytes) => Some(decode_pdf_text_string(bytes)),
        PdfObject::Name(name) => Some(name.clone()),
        _ => None,
    }
}

fn object_ref_string(reference: (u32, u16)) -> String {
    format!("{} {} R", reference.0, reference.1)
}

fn aggregate_xml_metrics(packets: &[PacketSource]) -> XfaXmlMetrics {
    let mut out = XfaXmlMetrics {
        packet_count: packets.len(),
        ..XfaXmlMetrics::default()
    };
    for packet in packets {
        out.total_bytes = out.total_bytes.saturating_add(packet.bytes.len());
        if let Some(parsed) = &packet.parsed {
            out.parsed_packets += 1;
            out.node_count = out.node_count.saturating_add(parsed.metrics.nodes);
            out.attribute_count = out
                .attribute_count
                .saturating_add(parsed.metrics.attributes);
            out.namespace_declarations = out
                .namespace_declarations
                .saturating_add(parsed.metrics.namespace_declarations);
            out.max_depth = out.max_depth.max(parsed.metrics.max_depth);
            out.text_bytes = out.text_bytes.saturating_add(parsed.metrics.text_bytes);
            out.entity_references = out
                .entity_references
                .saturating_add(parsed.metrics.entity_references);
        } else if !packet.bytes.is_empty() {
            out.malformed_packets += 1;
        }
    }
    out
}

fn parsed_subtree(root: &XmlNode) -> ParsedXml {
    fn visit(node: &XmlNode, depth: usize, metrics: &mut XmlMetrics) {
        metrics.nodes = metrics.nodes.saturating_add(1);
        metrics.attributes = metrics.attributes.saturating_add(node.attributes.len());
        metrics.namespace_declarations = metrics.namespace_declarations.saturating_add(
            node.attributes
                .iter()
                .filter(|attribute| {
                    attribute.name == "xmlns" || attribute.name.starts_with("xmlns:")
                })
                .count(),
        );
        metrics.max_depth = metrics.max_depth.max(depth);
        metrics.text_bytes = metrics.text_bytes.saturating_add(node.text.len());
        for child in &node.children {
            visit(child, depth.saturating_add(1), metrics);
        }
    }

    let mut metrics = XmlMetrics::default();
    visit(root, 1, &mut metrics);
    ParsedXml {
        root: root.clone(),
        metrics,
        diagnostics: Vec::new(),
    }
}

fn classify_xfa(packets: &[PacketSource], acroform: &PdfDictionary) -> XfaClassification {
    let template = packet_root(packets, "template");
    let config = packet_root(packets, "config");
    let mut evidence = Vec::new();
    let mut dynamic = false;
    let mut reflow = false;
    if let Some(config) = config {
        for node in config.descendants("dynamicRender") {
            let value = node.plain_text().to_ascii_lowercase();
            if matches!(value.as_str(), "required" | "1" | "true") {
                dynamic = true;
                evidence.push("config.dynamicRender=required".to_string());
            }
        }
    }
    if let Some(template) = template {
        for subform in template.descendants("subform") {
            if matches!(
                subform.attr("layout"),
                Some("tb" | "lr-tb" | "rl-tb" | "row")
            ) {
                dynamic = true;
                reflow = true;
                evidence.push(format!(
                    "template.subform.layout={}",
                    subform.attr("layout").unwrap_or_default()
                ));
                break;
            }
            if subform.child("occur").is_some() {
                dynamic = true;
                evidence.push("template.subform.occur".to_string());
            }
        }
        if template.descendants("break").next().is_some()
            || template.descendants("overflow").next().is_some()
        {
            dynamic = true;
            reflow = true;
            evidence.push("template.break_or_overflow".to_string());
        }
    }
    let hybrid = acroform
        .get("Fields")
        .is_some_and(|fields| !matches!(fields, PdfObject::Array(items) if items.is_empty()));
    if hybrid {
        evidence.push("acroform.fields_and_xfa_coexist".to_string());
    }
    XfaClassification {
        kind: if packets.is_empty() {
            "none"
        } else if dynamic {
            "dynamic"
        } else {
            "static"
        }
        .to_string(),
        static_xfa: !packets.is_empty() && !dynamic,
        dynamic_xfa: dynamic,
        hybrid_acroform_xfa: hybrid,
        static_page_backed: !packets.is_empty() && !dynamic,
        dynamic_reflowing_subforms: reflow,
        evidence,
    }
}

fn packet_root<'a>(packets: &'a [PacketSource], name: &'a str) -> Option<&'a XmlNode> {
    packets
        .iter()
        .find(|packet| packet.record.name.eq_ignore_ascii_case(name))
        .and_then(|packet| packet.parsed.as_ref())
        .map(|parsed| &parsed.root)
        .or_else(|| {
            packets.iter().find_map(|packet| {
                packet
                    .parsed
                    .as_ref()
                    .and_then(|parsed| parsed.root.descendants(name).next())
            })
        })
}

fn packet_source<'a>(packets: &'a [PacketSource], name: &'a str) -> Option<&'a PacketSource> {
    packets
        .iter()
        .find(|packet| packet.record.name.eq_ignore_ascii_case(name))
        .or_else(|| {
            packets.iter().find(|packet| {
                packet
                    .parsed
                    .as_ref()
                    .is_some_and(|parsed| parsed.root.descendants(name).next().is_some())
            })
        })
}

fn extract_loaded(loaded: LoadedXfa, limits: &XfaLimits) -> Result<XfaExtractionReport> {
    let template_source = packet_source(&loaded.packets, "template");
    let datasets_source = packet_source(&loaded.packets, "datasets");
    let template = template_source.and_then(|packet| {
        packet.parsed.as_ref().and_then(|parsed| {
            if parsed.root.local_name == "template" {
                Some(&parsed.root)
            } else {
                parsed.root.descendants("template").next()
            }
        })
    });
    let datasets_xml = datasets_source.and_then(|packet| {
        packet.parsed.as_ref().and_then(|parsed| {
            if parsed.root.local_name == "datasets" {
                Some(&parsed.root)
            } else {
                parsed.root.descendants("datasets").next()
            }
        })
    });
    let mut diagnostics = Vec::new();
    let datasets = datasets_xml
        .zip(datasets_source)
        .map(|(root, source)| build_dataset_roots(root, source, limits))
        .transpose()?
        .unwrap_or_default();
    let data_root = datasets.first();
    let mut fields = Vec::new();
    let mut draws = Vec::new();
    let mut subforms = Vec::new();
    let mut script_sources = Vec::new();
    let mut events = Vec::new();
    let mut unsupported = BTreeSet::new();
    if let (Some(template), Some(source)) = (template, template_source) {
        let mut path = Vec::new();
        walk_template(
            template,
            source,
            &mut path,
            data_root,
            &mut fields,
            &mut draws,
            &mut subforms,
            &mut script_sources,
            &mut events,
            &mut unsupported,
            &mut diagnostics,
        )?;
        // Recollect with the event-aware traversal so scripts nested below
        // calculate/validate/event nodes retain the exact lifecycle activity.
        script_sources = collect_script_sources(template, source);
    } else if loaded.inventory.present {
        diagnostics.push(XfaDiagnostic::warning(
            "xfa.template.missing",
            "XFA is present but no parseable template packet was found",
            None,
        ));
    }
    let scripts = script_sources
        .iter()
        .map(|source| source.record.clone())
        .collect::<Vec<_>>();
    let semantic_integration = build_semantic_integration(&fields, &draws);
    Ok(XfaExtractionReport {
        schema_version: XFA_SCHEMA_VERSION.to_string(),
        status: if template.is_some() {
            XfaSupportStatus::ImplementedWithLimits
        } else if loaded.inventory.present {
            XfaSupportStatus::UnsupportedReportedExact
        } else {
            XfaSupportStatus::Implemented
        },
        inventory: loaded.inventory,
        template_parsed: template.is_some(),
        datasets_parsed: datasets_xml.is_some(),
        fields,
        draws,
        subforms,
        datasets,
        scripts,
        events,
        unsupported_constructs: unsupported.into_iter().collect(),
        semantic_integration,
        diagnostics,
    })
}

#[allow(clippy::too_many_arguments)]
fn walk_template(
    node: &XmlNode,
    source: &PacketSource,
    path: &mut Vec<String>,
    data_root: Option<&XfaDataNode>,
    fields: &mut Vec<XfaFieldRecord>,
    draws: &mut Vec<XfaDrawRecord>,
    subforms: &mut Vec<XfaSubformRecord>,
    scripts: &mut Vec<ScriptSource>,
    events: &mut Vec<XfaEventRecord>,
    unsupported: &mut BTreeSet<String>,
    diagnostics: &mut Vec<XfaDiagnostic>,
) -> Result<()> {
    let node_name = node
        .attr("name")
        .filter(|name| !name.is_empty())
        .unwrap_or(node.local_name.as_str())
        .to_string();
    let indexed_name = indexed_segment(&node_name, path);
    if matches!(
        node.local_name.as_str(),
        "subform" | "field" | "draw" | "exclGroup" | "pageArea" | "contentArea"
    ) {
        path.push(indexed_name);
    }
    let som = path.join(".");
    let provenance = provenance_for(source, node, Some(som.clone()));
    match node.local_name.as_str() {
        "subform" => {
            let bind_expression = node
                .child("bind")
                .and_then(|bind| bind.attr("ref"))
                .map(str::to_string);
            subforms.push(XfaSubformRecord {
                name: node_name.clone(),
                som_path: som.clone(),
                layout: normalize_layout(node.attr("layout")),
                presence: node.attr("presence").unwrap_or("visible").to_string(),
                occur: parse_occur(node.child("occur"), diagnostics, source),
                bind_expression,
                child_nodes: node.children.len(),
                provenance: provenance.clone(),
            });
        }
        "field" | "exclGroup" => {
            let field = extract_field(node, source, &node_name, &som, data_root, diagnostics);
            if field.field_type == "image" {
                unsupported.insert("embedded_xfa_image_decode_and_render".to_string());
            }
            if field.field_type == "barcode" {
                unsupported.insert("barcode.external_or_symbology_engine".to_string());
            }
            if field.field_type == "signature" {
                unsupported.insert("dynamic_signature_semantics".to_string());
            }
            fields.push(field);
        }
        "draw" => {
            let draw_type = draw_type(node);
            if draw_type == "image" {
                unsupported.insert("embedded_xfa_image_decode_and_render".to_string());
            }
            draws.push(XfaDrawRecord {
                name: node_name.clone(),
                som_path: som.clone(),
                draw_type,
                text: extract_value_text(node),
                geometry: parse_geometry(node, diagnostics, source),
                presence: node.attr("presence").unwrap_or("visible").to_string(),
                provenance: provenance.clone(),
            });
        }
        "event" | "calculate" | "validate" => {
            let activity = if node.local_name == "event" {
                node.attr("activity").unwrap_or("unknown")
            } else {
                node.local_name.as_str()
            }
            .to_string();
            let script_count = node.descendants("script").count();
            events.push(XfaEventRecord {
                order: events.len(),
                activity: activity.clone(),
                target_som: som.clone(),
                script_count,
                default_execution: "disabled".to_string(),
                support_status: event_support_status(&activity),
                provenance: provenance.clone(),
            });
        }
        "script" => {
            let parent_event = infer_script_event(path, node);
            let source_text = node.plain_text();
            let language = script_language(node);
            let support_status = if language == "formcalc" {
                XfaSupportStatus::ImplementedWithLimits
            } else {
                XfaSupportStatus::UnsupportedReportedSecurityPolicy
            };
            let record = XfaScriptRecord {
                order: scripts.len(),
                language: language.clone(),
                event: parent_event,
                target_som: som.clone(),
                source_bytes: source_text.len(),
                source_sha256: resource_digest(source_text.as_bytes()),
                default_execution: "disabled".to_string(),
                support_status,
                blocked_capabilities: blocked_script_capabilities(&language),
                provenance: provenance.clone(),
            };
            scripts.push(ScriptSource {
                record,
                source: source_text,
            });
        }
        "barcode" => {
            unsupported.insert("barcode.external_or_symbology_engine".to_string());
        }
        "signature" => {
            unsupported.insert("dynamic_signature_semantics".to_string());
        }
        "connect" | "connectionSet" | "sourceSet" => {
            unsupported.insert("external_data_connections_blocked".to_string());
        }
        "overflow" if node.attr("leader").is_some() || node.attr("trailer").is_some() => {
            unsupported.insert("complex_overflow_leader_trailer_chain".to_string());
        }
        _ => {}
    }
    for attr in &node.attributes {
        if attr.name != "xmlns"
            && !attr.name.starts_with("xmlns:")
            && is_external_reference(&attr.value)
        {
            unsupported.insert(format!(
                "external_reference_blocked:{}@{}",
                node.local_name, attr.local_name
            ));
        }
        if matches!(attr.local_name.as_str(), "use" | "usehref") {
            unsupported.insert("prototype_or_usehref_resolution".to_string());
        }
    }
    for child in &node.children {
        walk_template(
            child,
            source,
            path,
            data_root,
            fields,
            draws,
            subforms,
            scripts,
            events,
            unsupported,
            diagnostics,
        )?;
    }
    if matches!(
        node.local_name.as_str(),
        "subform" | "field" | "draw" | "exclGroup" | "pageArea" | "contentArea"
    ) {
        path.pop();
    }
    Ok(())
}

fn indexed_segment(name: &str, path: &[String]) -> String {
    let prefix = format!("{name}[");
    let index = path
        .iter()
        .filter(|segment| segment.starts_with(&prefix) || segment.as_str() == name)
        .count();
    format!("{name}[{index}]")
}

fn extract_field(
    node: &XmlNode,
    source: &PacketSource,
    name: &str,
    som: &str,
    data_root: Option<&XfaDataNode>,
    diagnostics: &mut Vec<XfaDiagnostic>,
) -> XfaFieldRecord {
    let mut field_diagnostics = Vec::new();
    let binding = bind_field(node, name, data_root, source, &mut field_diagnostics);
    let default_value = extract_value_text(node);
    let value = binding
        .raw_values
        .first()
        .cloned()
        .or_else(|| default_value.clone());
    let field_type = field_type(node);
    validate_type_coercion(
        &field_type,
        value.as_deref(),
        source,
        &mut field_diagnostics,
    );
    diagnostics.extend(field_diagnostics.clone());
    let choices = extract_choices(node);
    XfaFieldRecord {
        name: name.to_string(),
        som_path: som.to_string(),
        field_type,
        caption: extract_caption(node),
        tooltip: node
            .child("assist")
            .and_then(|assist| assist.child("toolTip"))
            .map(XmlNode::plain_text)
            .filter(|text| !text.is_empty()),
        default_value,
        selected_values: value.iter().cloned().collect(),
        value,
        choices,
        required: node.attr("mandatory") == Some("error")
            || node
                .child("validate")
                .and_then(|validate| validate.attr("nullTest"))
                == Some("error"),
        validation_state: if node.child("validate").is_some() {
            "inventoried_not_executed_by_default"
        } else {
            "none"
        }
        .to_string(),
        presence: node.attr("presence").unwrap_or("visible").to_string(),
        tab_index: node.attr("accessKey").and_then(|value| value.parse().ok()),
        geometry: parse_geometry(node, diagnostics, source),
        font_family: node
            .descendants("font")
            .next()
            .and_then(|font| font.attr("typeface"))
            .map(str::to_string),
        font_size_points: node
            .descendants("font")
            .next()
            .and_then(|font| font.attr("size"))
            .and_then(|value| parse_measurement(value).ok()),
        border: node
            .child("border")
            .and_then(|border| border.attr("presence"))
            .map(str::to_string),
        fill: node
            .descendants("fill")
            .next()
            .and_then(|fill| fill.attr("presence"))
            .map(str::to_string),
        binding,
        provenance: provenance_for(source, node, Some(som.to_string())),
        diagnostics: field_diagnostics,
    }
}

fn bind_field(
    node: &XmlNode,
    name: &str,
    data_root: Option<&XfaDataNode>,
    source: &PacketSource,
    diagnostics: &mut Vec<XfaDiagnostic>,
) -> XfaBindingRecord {
    let bind = node.child("bind");
    let mode = bind
        .and_then(|bind| bind.attr("match"))
        .unwrap_or_else(|| {
            if bind.and_then(|bind| bind.attr("ref")).is_some() {
                "ref"
            } else {
                "name"
            }
        })
        .to_ascii_lowercase();
    let expression = bind.and_then(|bind| bind.attr("ref")).map(str::to_string);
    let Some(data_root) = data_root else {
        return XfaBindingRecord {
            mode,
            expression,
            status: XfaSupportStatus::ImplementedWithLimits,
            matched_nodes: 0,
            raw_values: Vec::new(),
            coercion: "no_dataset".to_string(),
            diagnostics: Vec::new(),
        };
    };
    let matches = match mode.as_str() {
        "none" => Vec::new(),
        "global" => find_data_nodes(data_root, name),
        "ref" => expression
            .as_deref()
            .map(|expr| resolve_data_path(data_root, expr))
            .unwrap_or_default(),
        _ => find_data_nodes(data_root, name),
    };
    let values = matches
        .iter()
        .filter_map(|node| node.value.clone().or_else(|| first_leaf_value(node)))
        .collect::<Vec<_>>();
    if matches.len() > 1 {
        diagnostics.push(XfaDiagnostic::warning(
            "xfa.bind.duplicate_data",
            format!(
                "binding for field '{name}' matched {} data nodes",
                matches.len()
            ),
            Some(source.record.name.clone()),
        ));
    } else if matches.is_empty() && mode != "none" {
        diagnostics.push(XfaDiagnostic::info(
            "xfa.bind.missing_data",
            format!("binding for field '{name}' found no dataset node"),
            Some(source.record.name.clone()),
        ));
    }
    XfaBindingRecord {
        mode,
        expression,
        status: XfaSupportStatus::ImplementedWithLimits,
        matched_nodes: matches.len(),
        raw_values: values,
        coercion: "raw_value_preserved".to_string(),
        diagnostics: diagnostics.clone(),
    }
}

fn build_dataset_roots(
    root: &XmlNode,
    source: &PacketSource,
    limits: &XfaLimits,
) -> Result<Vec<XfaDataNode>> {
    let data = if root.local_name == "data" {
        root
    } else {
        root.descendants("data").next().unwrap_or(root)
    };
    let mut count = 0usize;
    data.children
        .iter()
        .map(|child| build_data_node(child, source, limits, &mut count))
        .collect()
}

fn build_data_node(
    node: &XmlNode,
    source: &PacketSource,
    limits: &XfaLimits,
    count: &mut usize,
) -> Result<XfaDataNode> {
    *count = count.saturating_add(1);
    if *count > limits.max_dataset_nodes {
        return Err(WellfriendError::ResourceLimit(format!(
            "XFA dataset node count exceeds cap {}",
            limits.max_dataset_nodes
        )));
    }
    let attributes = node
        .attributes
        .iter()
        .map(|attr| (attr.name.clone(), attr.value.clone()))
        .collect();
    let children = node
        .children
        .iter()
        .map(|child| build_data_node(child, source, limits, count))
        .collect::<Result<Vec<_>>>()?;
    let value = if node.text.trim().is_empty() {
        None
    } else {
        Some(node.text.trim().to_string())
    };
    Ok(XfaDataNode {
        name: node.local_name.clone(),
        value,
        attributes,
        children,
        provenance: provenance_for(source, node, None),
    })
}

fn find_data_nodes<'a>(root: &'a XfaDataNode, name: &str) -> Vec<&'a XfaDataNode> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.name == name {
            out.push(node);
        }
        stack.extend(node.children.iter().rev());
    }
    out
}

fn resolve_data_path<'a>(root: &'a XfaDataNode, expression: &str) -> Vec<&'a XfaDataNode> {
    let normalized = expression
        .trim()
        .trim_start_matches("$record.")
        .trim_start_matches("$data.")
        .trim_start_matches("xfa.datasets.data.")
        .trim_start_matches("xfa:data.");
    let mut current = vec![root];
    for segment in normalized.split('.').filter(|segment| !segment.is_empty()) {
        let (name, index) = parse_som_segment(segment);
        let mut next: Vec<&XfaDataNode> = Vec::new();
        for node in &current {
            let matches = node
                .children
                .iter()
                .filter(|child| child.name == name)
                .collect::<Vec<_>>();
            if let Some(index) = index {
                if let Some(item) = matches.get(index) {
                    next.push(*item);
                }
            } else {
                next.extend(matches);
            }
        }
        if next.is_empty() && current.len() == 1 && current[0].name == name {
            continue;
        }
        current = next;
    }
    current
}

fn parse_som_segment(segment: &str) -> (&str, Option<usize>) {
    let Some(open) = segment.rfind('[') else {
        return (segment.trim_start_matches('#'), None);
    };
    let Some(index) = segment
        .strip_suffix(']')
        .and_then(|value| value[open + 1..].parse().ok())
    else {
        return (segment.trim_start_matches('#'), None);
    };
    (segment[..open].trim_start_matches('#'), Some(index))
}

fn first_leaf_value(node: &XfaDataNode) -> Option<String> {
    node.value
        .clone()
        .or_else(|| node.children.iter().find_map(first_leaf_value))
}

fn parse_occur(
    node: Option<&XmlNode>,
    diagnostics: &mut Vec<XfaDiagnostic>,
    source: &PacketSource,
) -> XfaOccur {
    let Some(node) = node else {
        return XfaOccur::default();
    };
    let min = parse_nonnegative(node.attr("min"), 1, diagnostics, source, "occur.min");
    let max = match node.attr("max") {
        Some("-1" | "*") => None,
        value => Some(parse_nonnegative(
            value,
            1,
            diagnostics,
            source,
            "occur.max",
        )),
    };
    let initial = parse_nonnegative(
        node.attr("initial"),
        min,
        diagnostics,
        source,
        "occur.initial",
    );
    XfaOccur { min, max, initial }
}

fn parse_nonnegative(
    value: Option<&str>,
    default: usize,
    diagnostics: &mut Vec<XfaDiagnostic>,
    source: &PacketSource,
    label: &str,
) -> usize {
    value
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or_else(|| {
            if value.is_some() {
                diagnostics.push(XfaDiagnostic::warning(
                    "xfa.numeric.invalid",
                    format!("invalid non-negative numeric value for {label}; default applied"),
                    Some(source.record.name.clone()),
                ));
            }
            default
        })
}

fn parse_geometry(
    node: &XmlNode,
    diagnostics: &mut Vec<XfaDiagnostic>,
    source: &PacketSource,
) -> Option<XfaGeometry> {
    let has_geometry = ["x", "y", "w", "h"]
        .iter()
        .any(|key| node.attr(key).is_some());
    if !has_geometry {
        return None;
    }
    let parse = |key: &str, default: f64| -> f64 {
        node.attr(key)
            .and_then(|value| parse_measurement(value).ok())
            .unwrap_or(default)
    };
    let geometry = XfaGeometry {
        x: parse("x", 0.0),
        y: parse("y", 0.0),
        width: parse("w", 144.0),
        height: parse("h", 18.0),
        rotation_degrees: node
            .attr("rotate")
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite())
            .unwrap_or(0.0),
        page: None,
    };
    if geometry.width < 0.0 || geometry.height < 0.0 {
        diagnostics.push(XfaDiagnostic::warning(
            "xfa.geometry.negative_dimension",
            "negative XFA dimensions are unsupported and normalized to zero during layout",
            Some(source.record.name.clone()),
        ));
    }
    Some(geometry)
}

fn parse_measurement(value: &str) -> Result<f64> {
    let trimmed = value.trim();
    let split = trimmed
        .find(|ch: char| !(ch.is_ascii_digit() || matches!(ch, '+' | '-' | '.' | 'e' | 'E')))
        .unwrap_or(trimmed.len());
    let number = trimmed[..split]
        .parse::<f64>()
        .map_err(|_| WellfriendError::MalformedPdf("invalid XFA measurement".to_string()))?;
    if !number.is_finite() {
        return Err(WellfriendError::MalformedPdf(
            "non-finite XFA measurement is forbidden".to_string(),
        ));
    }
    let unit = trimmed[split..].trim().to_ascii_lowercase();
    let points = match unit.as_str() {
        "" | "pt" => number,
        "in" => number * 72.0,
        "mm" => number * 72.0 / 25.4,
        "cm" => number * 72.0 / 2.54,
        "px" => number * 72.0 / 96.0,
        _ => {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "XFA measurement unit '{unit}' is unsupported"
            )))
        }
    };
    if points.is_finite() {
        Ok(points)
    } else {
        Err(WellfriendError::MalformedPdf(
            "non-finite XFA measurement result is forbidden".to_string(),
        ))
    }
}

fn extract_value_text(node: &XmlNode) -> Option<String> {
    let value = node.child("value")?;
    let text = value
        .children
        .first()
        .map(XmlNode::plain_text)
        .unwrap_or_else(|| value.plain_text());
    (!text.is_empty()).then_some(text)
}

fn extract_caption(node: &XmlNode) -> Option<String> {
    let caption = node.child("caption")?;
    let value = caption.child("value").unwrap_or(caption);
    let text = value
        .children
        .first()
        .map(XmlNode::plain_text)
        .unwrap_or_else(|| value.plain_text());
    (!text.is_empty()).then_some(text)
}

fn extract_choices(node: &XmlNode) -> Vec<String> {
    node.children
        .iter()
        .filter(|child| child.local_name == "items")
        .flat_map(|items| items.children.iter())
        .map(XmlNode::plain_text)
        .filter(|value| !value.is_empty())
        .collect()
}

fn field_type(node: &XmlNode) -> String {
    let ui = node.child("ui");
    let kind = ui
        .and_then(|ui| ui.children.first())
        .map(|child| child.local_name.as_str())
        .unwrap_or_else(|| {
            node.child("value")
                .and_then(|value| value.children.first())
                .map(|child| child.local_name.as_str())
                .unwrap_or("text")
        });
    match kind {
        "numericEdit" | "decimal" | "float" | "integer" => "numeric",
        "dateTimeEdit" | "date" | "time" | "dateTime" => "date_time",
        "signature" => "signature",
        "choiceList" => "choice_list",
        "checkButton" => "check_button",
        "barcode" => "barcode",
        "imageEdit" | "image" => "image",
        _ if node.local_name == "exclGroup" => "exclusion_group",
        _ => "text",
    }
    .to_string()
}

fn draw_type(node: &XmlNode) -> String {
    node.child("value")
        .and_then(|value| value.children.first())
        .map(|child| child.local_name.clone())
        .or_else(|| {
            ["line", "rectangle", "arc", "image"]
                .iter()
                .find(|name| node.child(name).is_some())
                .map(|name| (*name).to_string())
        })
        .unwrap_or_else(|| "text".to_string())
}

fn normalize_layout(layout: Option<&str>) -> String {
    match layout.unwrap_or("position") {
        "tb" | "lr-tb" | "rl-tb" => "top_to_bottom",
        "lr" => "left_to_right",
        "row" => "row",
        "table" => "table",
        _ => "positioned",
    }
    .to_string()
}

fn script_language(node: &XmlNode) -> String {
    let content_type = node
        .attr("contentType")
        .or_else(|| node.attr("type"))
        .unwrap_or("")
        .to_ascii_lowercase();
    if content_type.contains("formcalc") {
        "formcalc"
    } else if content_type.contains("javascript") || content_type.contains("ecmascript") {
        "javascript"
    } else {
        "proprietary_or_unknown"
    }
    .to_string()
}

fn infer_script_event(path: &[String], node: &XmlNode) -> String {
    node.attr("activity")
        .map(str::to_string)
        .or_else(|| {
            path.last().and_then(|segment| {
                [
                    "initialize",
                    "calculate",
                    "validate",
                    "ready",
                    "layoutReady",
                ]
                .iter()
                .find(|event| segment.contains(**event))
                .map(|event| (*event).to_string())
            })
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn blocked_script_capabilities(language: &str) -> Vec<String> {
    let mut blocked = vec![
        "network".to_string(),
        "filesystem".to_string(),
        "process".to_string(),
        "native_calls".to_string(),
        "environment".to_string(),
        "clipboard".to_string(),
        "ui".to_string(),
        "external_resources".to_string(),
        "dynamic_code_evaluation".to_string(),
        "unbounded_loops".to_string(),
    ];
    if language == "javascript" {
        blocked.extend([
            "prototype_mutation".to_string(),
            "global_object_escape".to_string(),
            "timers".to_string(),
            "dynamic_imports".to_string(),
            "browser_dom".to_string(),
            "acrobat_privileged_apis".to_string(),
        ]);
    }
    blocked
}

fn event_support_status(activity: &str) -> XfaSupportStatus {
    match activity {
        "calculate" | "validate" => XfaSupportStatus::ImplementedWithLimits,
        _ => XfaSupportStatus::UnsupportedReportedExact,
    }
}

fn validate_type_coercion(
    field_type: &str,
    value: Option<&str>,
    source: &PacketSource,
    diagnostics: &mut Vec<XfaDiagnostic>,
) {
    let Some(value) = value else { return };
    if field_type == "numeric" && value.parse::<f64>().is_err() {
        diagnostics.push(XfaDiagnostic::warning(
            "xfa.bind.numeric_coercion_failed",
            "numeric field retains a non-numeric raw dataset value",
            Some(source.record.name.clone()),
        ));
    }
    if field_type == "date_time" && !looks_like_iso_date(value) {
        diagnostics.push(XfaDiagnostic::info(
            "xfa.bind.locale_date_unparsed",
            "date/time raw value was preserved; locale-specific parsing is not claimed",
            Some(source.record.name.clone()),
        ));
    }
}

fn looks_like_iso_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 10
        && bytes.get(4) == Some(&b'-')
        && bytes.get(7) == Some(&b'-')
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[8..10].iter().all(u8::is_ascii_digit)
}

fn provenance_for(
    source: &PacketSource,
    node: &XmlNode,
    som_path: Option<String>,
) -> XfaProvenance {
    XfaProvenance {
        packet: source.record.name.clone(),
        packet_order: source.record.order,
        object: source.record.object_reference.clone(),
        source_start: Some(node.start_offset),
        source_end: Some(node.end_offset),
        som_path,
    }
}

fn build_semantic_integration(
    fields: &[XfaFieldRecord],
    draws: &[XfaDrawRecord],
) -> XfaSemanticIntegrationReport {
    let mut terms = BTreeSet::new();
    let mut chunks = Vec::new();
    for field in fields {
        let text = [field.caption.as_deref(), field.value.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(": ");
        terms.extend(
            text.split(|ch: char| !ch.is_alphanumeric())
                .filter(|term| !term.is_empty())
                .map(|term| term.to_lowercase()),
        );
        if !text.is_empty() {
            chunks.push(XfaRagChunk {
                id: format!("xfa-field-{}", chunks.len()),
                text,
                page: field.geometry.as_ref().and_then(|geometry| geometry.page),
                som_path: field.som_path.clone(),
                packet: field.provenance.packet.clone(),
            });
        }
    }
    for draw in draws {
        if let Some(text) = &draw.text {
            terms.extend(
                text.split(|ch: char| !ch.is_alphanumeric())
                    .filter(|term| !term.is_empty())
                    .map(|term| term.to_lowercase()),
            );
            chunks.push(XfaRagChunk {
                id: format!("xfa-draw-{}", chunks.len()),
                text: text.clone(),
                page: draw.geometry.as_ref().and_then(|geometry| geometry.page),
                som_path: draw.som_path.clone(),
                packet: draw.provenance.packet.clone(),
            });
        }
    }
    XfaSemanticIntegrationReport {
        forms_model_fields: fields.len(),
        semantic_text_entries: chunks.len(),
        provenance_entries: chunks.len(),
        search_index_terms: terms.into_iter().collect(),
        rag_chunks: chunks,
        accessibility_labels: fields
            .iter()
            .filter(|field| field.caption.is_some())
            .count(),
        accessibility_status: XfaSupportStatus::ImplementedWithLimits,
        redaction_visible_entries: fields.len() + draws.len(),
        security_report_linked: true,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum XfaScriptPolicy {
    #[default]
    Disabled,
    FormCalcSafeSubset,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XfaRuntimeOptions {
    pub limits: XfaLimits,
    pub script_policy: XfaScriptPolicy,
    pub execute_supported_events: bool,
}

impl Default for XfaRuntimeOptions {
    fn default() -> Self {
        Self {
            limits: XfaLimits::default(),
            script_policy: XfaScriptPolicy::Disabled,
            execute_supported_events: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct XfaLayoutRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct XfaLayoutItem {
    pub order: usize,
    pub page: usize,
    pub kind: String,
    pub field_type: Option<String>,
    pub som_path: String,
    pub text: Option<String>,
    pub rect: XfaLayoutRect,
    pub rotation_degrees: f64,
    pub visible: bool,
    pub clipped: bool,
    pub repeated_instance_index: usize,
    pub provenance: XfaProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XfaSandboxAuditEntry {
    pub order: usize,
    pub language: String,
    pub event: String,
    pub target_som: String,
    pub outcome: String,
    pub instructions: usize,
    pub field_mutations: usize,
    pub reason_code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XfaSandboxReport {
    pub default_policy: String,
    pub policy: XfaScriptPolicy,
    pub scripts_inventoried: usize,
    pub events_inventoried: usize,
    pub scripts_executed: usize,
    pub events_executed: usize,
    pub scripts_blocked: usize,
    pub total_instructions: usize,
    pub field_mutations: usize,
    pub javascript_status: XfaSupportStatus,
    pub formcalc_status: XfaSupportStatus,
    pub network_access: bool,
    pub filesystem_access: bool,
    pub process_access: bool,
    pub native_access: bool,
    pub environment_access: bool,
    pub deterministic_time: String,
    pub deterministic_random: String,
    pub no_secret_logging: bool,
    pub audit_log: Vec<XfaSandboxAuditEntry>,
    pub diagnostics: Vec<XfaDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XfaRuntimeMetrics {
    pub packet_count: usize,
    pub xml_bytes: usize,
    pub xml_nodes: usize,
    pub template_nodes: usize,
    pub dataset_nodes: usize,
    pub generated_subform_instances: usize,
    pub generated_nodes: usize,
    pub generated_pages: usize,
    pub layout_iterations: usize,
    pub script_instructions: usize,
    pub event_executions: usize,
    pub estimated_peak_memory_bytes: usize,
    pub measurement_kind: String,
    pub runtime_micros: u128,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct XfaRuntimeReport {
    pub schema_version: String,
    pub status: XfaSupportStatus,
    pub classification: XfaClassification,
    pub supported_features: Vec<String>,
    pub unsupported_constructs: Vec<String>,
    pub generated_pages: usize,
    pub generated_instances: usize,
    pub layout_items: Vec<XfaLayoutItem>,
    pub sandbox: XfaSandboxReport,
    pub metrics: XfaRuntimeMetrics,
    pub scheduler: DecodeSchedulerMetrics,
    pub limits: XfaLimits,
    pub diagnostics: Vec<XfaDiagnostic>,
}

pub fn xfa_runtime_report(
    engine: &ContentEngine,
    options: &XfaRuntimeOptions,
) -> Result<XfaRuntimeReport> {
    xfa_runtime_report_cancellable(engine, options, &CancelToken::none())
}

pub fn xfa_runtime_report_cancellable(
    engine: &ContentEngine,
    options: &XfaRuntimeOptions,
    cancel: &CancelToken,
) -> Result<XfaRuntimeReport> {
    let started = RuntimeInstant::now();
    let loaded = load_xfa(engine.document(), &options.limits, cancel)?;
    let mut extraction = extract_loaded(loaded.clone(), &options.limits)?;
    let template_source = packet_source(&loaded.packets, "template");
    let template = template_source.and_then(|packet| {
        packet.parsed.as_ref().and_then(|parsed| {
            if parsed.root.local_name == "template" {
                Some(&parsed.root)
            } else {
                parsed.root.descendants("template").next()
            }
        })
    });
    let data_root = extraction.datasets.first();
    let scripts = template
        .zip(template_source)
        .map(|(root, source)| collect_script_sources(root, source))
        .unwrap_or_default();
    let sandbox = execute_sandbox(
        &scripts,
        &extraction.events,
        &mut extraction.fields,
        options,
        started,
    )?;

    let decode_limits = DecodeLimits {
        scheduler_memory_budget_bytes: options.limits.scheduler_memory_budget_bytes,
        max_decoded_bytes_per_stream: options.limits.max_packet_decoded_bytes as u64,
        ..DecodeLimits::default()
    };
    let scheduler = DecodeSchedulerContext::new(&decode_limits);
    let estimate = loaded
        .inventory
        .metrics
        .total_bytes
        .saturating_mul(4)
        .max(1) as u64;
    let mut layout_state = LayoutState::new(engine, &options.limits, cancel, started)?;
    if let (Some(template), Some(source)) = (template, template_source) {
        if options.limits.max_relayout_iterations == 0 {
            return Err(WellfriendError::ResourceLimit(
                "XFA relayout iteration cap is zero; the deterministic layout pass was not admitted"
                    .to_string(),
            ));
        }
        layout_state.configure_from_template(template);
        scheduler.run(estimate, cancel, "xfa layout", || {
            let mut path = Vec::new();
            layout_container(
                template,
                source,
                &mut path,
                data_root,
                None,
                &extraction.fields,
                0,
                0,
                layout_state.content_x,
                layout_state.content_y,
                layout_state.content_width,
                &mut layout_state,
            )?;
            Ok(())
        })?;
    }
    let scheduler_metrics = scheduler.metrics();
    let mut diagnostics = extraction.diagnostics.clone();
    diagnostics.extend(layout_state.diagnostics.clone());
    diagnostics.extend(sandbox.diagnostics.clone());
    let unsupported = extraction.unsupported_constructs.clone();
    let generated_pages = layout_state
        .max_page
        .max(usize::from(!layout_state.items.is_empty()));
    let estimated_peak_memory_bytes = loaded
        .inventory
        .metrics
        .total_bytes
        .saturating_add(layout_state.items.len() * std::mem::size_of::<XfaLayoutItem>())
        .saturating_add(
            extraction
                .datasets
                .iter()
                .map(estimated_data_node_bytes)
                .sum::<usize>(),
        )
        .saturating_add(
            scripts
                .iter()
                .map(|script| script.source.len())
                .sum::<usize>(),
        );
    Ok(XfaRuntimeReport {
        schema_version: XFA_SCHEMA_VERSION.to_string(),
        status: if template.is_some() {
            XfaSupportStatus::ImplementedWithLimits
        } else if loaded.inventory.present {
            XfaSupportStatus::UnsupportedReportedExact
        } else {
            XfaSupportStatus::Implemented
        },
        classification: loaded.inventory.classification.clone(),
        supported_features: vec![
            "positioned_layout".to_string(),
            "top_to_bottom_flow".to_string(),
            "left_to_right_flow".to_string(),
            "row_layout".to_string(),
            "occur_min_max_initial".to_string(),
            "dataset_driven_instances".to_string(),
            "simple_page_overflow".to_string(),
            "presence_visibility".to_string(),
            "field_caption_value_layout".to_string(),
            "bounded_formcalc_calculate_validate".to_string(),
        ],
        unsupported_constructs: unsupported,
        generated_pages,
        generated_instances: layout_state.generated_instances,
        layout_items: layout_state.items,
        metrics: XfaRuntimeMetrics {
            packet_count: loaded.inventory.metrics.packet_count,
            xml_bytes: loaded.inventory.metrics.total_bytes,
            xml_nodes: loaded.inventory.metrics.node_count,
            template_nodes: extraction.fields.len()
                + extraction.draws.len()
                + extraction.subforms.len(),
            dataset_nodes: extraction.datasets.iter().map(count_data_nodes).sum(),
            generated_subform_instances: layout_state.generated_instances,
            generated_nodes: layout_state.generated_nodes,
            generated_pages,
            layout_iterations: usize::from(template.is_some()),
            script_instructions: sandbox.total_instructions,
            event_executions: sandbox.events_executed,
            estimated_peak_memory_bytes,
            measurement_kind: "deterministic_owned_structure_estimate_not_process_rss".to_string(),
            runtime_micros: started.elapsed_micros(),
        },
        scheduler: scheduler_metrics,
        limits: options.limits.clone(),
        sandbox,
        diagnostics,
    })
}

fn collect_script_sources(root: &XmlNode, source: &PacketSource) -> Vec<ScriptSource> {
    let mut out = Vec::new();
    let mut path = Vec::new();
    collect_scripts_recursive(root, source, &mut path, None, &mut out);
    out
}

fn collect_scripts_recursive(
    node: &XmlNode,
    source: &PacketSource,
    path: &mut Vec<String>,
    current_event: Option<&str>,
    out: &mut Vec<ScriptSource>,
) {
    let structural = matches!(
        node.local_name.as_str(),
        "subform" | "field" | "draw" | "exclGroup" | "pageArea" | "contentArea"
    );
    if structural {
        let name = node.attr("name").unwrap_or(node.local_name.as_str());
        path.push(format!("{name}[0]"));
    }
    let event = match node.local_name.as_str() {
        "event" => node.attr("activity").or(current_event),
        "calculate" => Some("calculate"),
        "validate" => Some("validate"),
        _ => current_event,
    };
    if node.local_name == "script" {
        let source_text = node.plain_text();
        let language = script_language(node);
        out.push(ScriptSource {
            record: XfaScriptRecord {
                order: out.len(),
                language: language.clone(),
                event: event.unwrap_or("unknown").to_string(),
                target_som: path.join("."),
                source_bytes: source_text.len(),
                source_sha256: resource_digest(source_text.as_bytes()),
                default_execution: "disabled".to_string(),
                support_status: if language == "formcalc" {
                    XfaSupportStatus::ImplementedWithLimits
                } else {
                    XfaSupportStatus::UnsupportedReportedSecurityPolicy
                },
                blocked_capabilities: blocked_script_capabilities(&language),
                provenance: provenance_for(source, node, Some(path.join("."))),
            },
            source: source_text,
        });
    }
    for child in &node.children {
        collect_scripts_recursive(child, source, path, event, out);
    }
    if structural {
        path.pop();
    }
}

fn execute_sandbox(
    scripts: &[ScriptSource],
    events: &[XfaEventRecord],
    fields: &mut [XfaFieldRecord],
    options: &XfaRuntimeOptions,
    started: RuntimeInstant,
) -> Result<XfaSandboxReport> {
    let mut diagnostics = Vec::new();
    let mut audit_log = Vec::new();
    let mut scripts_executed = 0usize;
    let mut events_executed = 0usize;
    let mut scripts_blocked = 0usize;
    let mut total_instructions = 0usize;
    let mut field_mutations = 0usize;
    let mut values = BTreeMap::new();
    for field in fields.iter() {
        if let Some(value) = &field.value {
            values.insert(field.name.clone(), value.clone());
            values.insert(field.som_path.clone(), value.clone());
        }
    }
    let source_memory = scripts
        .iter()
        .map(|script| script.source.len())
        .sum::<usize>()
        .saturating_add(
            values
                .iter()
                .map(|(key, value)| key.len() + value.len())
                .sum(),
        );
    if source_memory > options.limits.max_script_memory_bytes {
        return Err(WellfriendError::ResourceLimit(format!(
            "XFA script memory estimate exceeds cap {}",
            options.limits.max_script_memory_bytes
        )));
    }
    let mut ordered = scripts.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|script| (event_rank(&script.record.event), script.record.order));
    for script in ordered {
        check_runtime(started, &options.limits)?;
        let mut entry = XfaSandboxAuditEntry {
            order: audit_log.len(),
            language: script.record.language.clone(),
            event: script.record.event.clone(),
            target_som: script.record.target_som.clone(),
            outcome: "blocked".to_string(),
            instructions: 0,
            field_mutations: 0,
            reason_code: "xfa.script.default_disabled".to_string(),
        };
        let enabled = options.execute_supported_events
            && options.script_policy == XfaScriptPolicy::FormCalcSafeSubset;
        if !enabled {
            scripts_blocked += 1;
            audit_log.push(entry);
            continue;
        }
        if script.record.language != "formcalc" {
            scripts_blocked += 1;
            entry.reason_code = "xfa.script.javascript_or_proprietary_blocked".to_string();
            diagnostics.push(XfaDiagnostic::warning(
                "xfa.script.security_policy",
                "JavaScript/proprietary XFA script was inventoried and blocked",
                Some(script.record.provenance.packet.clone()),
            ));
            audit_log.push(entry);
            continue;
        }
        if !matches!(script.record.event.as_str(), "calculate" | "validate") {
            scripts_blocked += 1;
            entry.reason_code = "xfa.event.unsupported_exact".to_string();
            audit_log.push(entry);
            continue;
        }
        if events_executed >= options.limits.max_event_executions {
            return Err(WellfriendError::ResourceLimit(format!(
                "XFA event executions exceed cap {}",
                options.limits.max_event_executions
            )));
        }
        let outcome = evaluate_formcalc(&script.source, &values, &options.limits, started);
        match outcome {
            Ok(outcome) => {
                scripts_executed += 1;
                events_executed += 1;
                total_instructions = total_instructions.saturating_add(outcome.instructions);
                entry.instructions = outcome.instructions;
                entry.outcome = "executed".to_string();
                entry.reason_code = "xfa.formcalc.safe_subset".to_string();
                if script.record.event == "calculate" {
                    let target = fields.iter_mut().find(|field| {
                        field.som_path == script.record.target_som
                            || script.record.target_som.ends_with(&field.som_path)
                            || field.name
                                == script
                                    .record
                                    .target_som
                                    .rsplit('.')
                                    .next()
                                    .unwrap_or_default()
                                    .split('[')
                                    .next()
                                    .unwrap_or_default()
                    });
                    if let Some(field) = target {
                        if field_mutations >= options.limits.max_field_mutations {
                            return Err(WellfriendError::ResourceLimit(format!(
                                "XFA field mutation cap {} exceeded",
                                options.limits.max_field_mutations
                            )));
                        }
                        field.value = Some(outcome.value.clone());
                        values.insert(field.name.clone(), outcome.value.clone());
                        values.insert(field.som_path.clone(), outcome.value);
                        field_mutations += 1;
                        entry.field_mutations = 1;
                    }
                } else if !matches!(outcome.value.as_str(), "true" | "1") {
                    diagnostics.push(XfaDiagnostic::warning(
                        "xfa.validate.failed",
                        "a supported FormCalc validation expression evaluated false",
                        Some(script.record.provenance.packet.clone()),
                    ));
                }
            }
            Err(err) => {
                scripts_blocked += 1;
                entry.reason_code = match err {
                    WellfriendError::ResourceLimit(_) => "xfa.script.resource_limit",
                    WellfriendError::UnsupportedFeature(_) => {
                        "xfa.script.unsupported_or_side_effect"
                    }
                    _ => "xfa.script.malformed",
                }
                .to_string();
                diagnostics.push(XfaDiagnostic::warning(
                    "xfa.script.execution_blocked",
                    err.to_string(),
                    Some(script.record.provenance.packet.clone()),
                ));
            }
        }
        audit_log.push(entry);
    }
    Ok(XfaSandboxReport {
        default_policy: "scripts_disabled_events_not_executed".to_string(),
        policy: options.script_policy,
        scripts_inventoried: scripts.len(),
        events_inventoried: events.len(),
        scripts_executed,
        events_executed,
        scripts_blocked,
        total_instructions,
        field_mutations,
        javascript_status: XfaSupportStatus::UnsupportedReportedSecurityPolicy,
        formcalc_status: XfaSupportStatus::ImplementedWithLimits,
        network_access: false,
        filesystem_access: false,
        process_access: false,
        native_access: false,
        environment_access: false,
        deterministic_time: "no_time_api_exposed".to_string(),
        deterministic_random: "no_random_api_exposed".to_string(),
        no_secret_logging: true,
        audit_log,
        diagnostics,
    })
}

fn event_rank(event: &str) -> usize {
    match event {
        "initialize" => 0,
        "calculate" => 1,
        "validate" => 2,
        "ready" | "formReady" | "docReady" => 3,
        "layoutReady" => 4,
        _ => 10,
    }
}

#[derive(Debug)]
struct LayoutState<'a> {
    limits: &'a XfaLimits,
    cancel: &'a CancelToken,
    started: RuntimeInstant,
    page_width: f64,
    page_height: f64,
    content_x: f64,
    content_y: f64,
    content_width: f64,
    content_height: f64,
    current_page: usize,
    max_page: usize,
    generated_instances: usize,
    generated_nodes: usize,
    items: Vec<XfaLayoutItem>,
    diagnostics: Vec<XfaDiagnostic>,
}

impl<'a> LayoutState<'a> {
    fn new(
        engine: &ContentEngine,
        limits: &'a XfaLimits,
        cancel: &'a CancelToken,
        started: RuntimeInstant,
    ) -> Result<Self> {
        let page = engine.document().get_pages()?.into_iter().next();
        let (page_width, page_height) = page
            .map(|page| {
                (
                    (page.media_box[2] - page.media_box[0]).abs(),
                    (page.media_box[3] - page.media_box[1]).abs(),
                )
            })
            .unwrap_or((612.0, 792.0));
        Ok(Self {
            limits,
            cancel,
            started,
            page_width,
            page_height,
            content_x: 36.0,
            content_y: 36.0,
            content_width: (page_width - 72.0).max(1.0),
            content_height: (page_height - 72.0).max(1.0),
            current_page: 1,
            max_page: 0,
            generated_instances: 0,
            generated_nodes: 0,
            items: Vec::new(),
            diagnostics: Vec::new(),
        })
    }

    fn check(&self, context: &str) -> Result<()> {
        self.cancel.check(context)?;
        check_runtime(self.started, self.limits)
    }

    fn configure_from_template(&mut self, template: &XmlNode) {
        if let Some(page_area) = template.descendants("pageArea").next() {
            if let Some(width) = page_area
                .attr("w")
                .and_then(|value| parse_measurement(value).ok())
            {
                self.page_width = width.max(1.0);
            }
            if let Some(height) = page_area
                .attr("h")
                .and_then(|value| parse_measurement(value).ok())
            {
                self.page_height = height.max(1.0);
            }
        }
        if let Some(content_area) = template.descendants("contentArea").next() {
            let geometry =
                geometry_or_default(content_area, self.content_width, self.content_height);
            self.content_x = geometry.x;
            self.content_y = geometry.y;
            self.content_width = geometry.width.max(1.0);
            self.content_height = geometry.height.max(1.0);
        }
    }

    fn new_page(&mut self) -> Result<()> {
        self.current_page = self.current_page.saturating_add(1);
        if self.current_page > self.limits.max_generated_pages {
            return Err(WellfriendError::ResourceLimit(format!(
                "XFA generated page count exceeds cap {}",
                self.limits.max_generated_pages
            )));
        }
        self.max_page = self.max_page.max(self.current_page);
        Ok(())
    }

    fn push(&mut self, item: XfaLayoutItem) -> Result<()> {
        self.generated_nodes = self.generated_nodes.saturating_add(1);
        if self.generated_nodes > self.limits.max_generated_nodes {
            return Err(WellfriendError::ResourceLimit(format!(
                "XFA generated node count exceeds cap {}",
                self.limits.max_generated_nodes
            )));
        }
        self.max_page = self.max_page.max(item.page);
        self.items.push(item);
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn layout_container(
    node: &XmlNode,
    source: &PacketSource,
    path: &mut Vec<String>,
    data_root: Option<&XfaDataNode>,
    bound_data: Option<&XfaDataNode>,
    fields: &[XfaFieldRecord],
    depth: usize,
    repeated_instance_index: usize,
    origin_x: f64,
    origin_y: f64,
    available_width: f64,
    state: &mut LayoutState<'_>,
) -> Result<f64> {
    state.check("xfa layout container")?;
    if depth > state.limits.max_subform_depth {
        return Err(WellfriendError::ResourceLimit(format!(
            "XFA subform depth exceeds cap {}",
            state.limits.max_subform_depth
        )));
    }
    if node.local_name == "pageArea" {
        if let Ok(Some(width)) = node.attr("w").map(parse_measurement).transpose() {
            state.page_width = width.max(1.0);
        }
        if let Ok(Some(height)) = node.attr("h").map(parse_measurement).transpose() {
            state.page_height = height.max(1.0);
        }
    }
    if node.local_name == "contentArea" {
        let geometry = geometry_or_default(node, state.content_width, state.content_height);
        state.content_x = geometry.x;
        state.content_y = geometry.y;
        state.content_width = geometry.width.max(1.0);
        state.content_height = geometry.height.max(1.0);
    }
    let layout = normalize_layout(node.attr("layout"));
    let mut cursor_x = 0.0f64;
    let mut cursor_y = 0.0f64;
    let mut max_bottom = 0.0f64;
    for child in &node.children {
        if !is_layout_node(child) {
            continue;
        }
        state.check("xfa layout child")?;
        if child.local_name == "break"
            || child.child("breakBefore").is_some()
            || child.attr("breakBefore").is_some()
        {
            state.new_page()?;
            cursor_x = 0.0;
            cursor_y = 0.0;
        }
        let geometry = geometry_or_default(
            child,
            default_width(child, available_width),
            default_height(child),
        );
        let (child_x, child_y) = match layout.as_str() {
            "top_to_bottom" | "table" => (geometry.x, cursor_y + geometry.y),
            "left_to_right" | "row" => (cursor_x + geometry.x, geometry.y),
            _ => (geometry.x, geometry.y),
        };
        let estimated_height = geometry.height.max(estimate_node_height(child));
        let absolute_y = origin_y + child_y;
        if matches!(layout.as_str(), "top_to_bottom" | "table")
            && absolute_y + estimated_height > state.content_y + state.content_height
            && !state.items.is_empty()
        {
            state.new_page()?;
            cursor_x = 0.0;
            cursor_y = 0.0;
        }
        let used = layout_node(
            child,
            source,
            path,
            data_root,
            bound_data,
            fields,
            depth + 1,
            repeated_instance_index,
            origin_x + child_x,
            origin_y
                + if state.current_page > 1 {
                    cursor_y
                } else {
                    child_y
                },
            geometry.width.max(1.0),
            geometry.height.max(1.0),
            state,
        )?;
        match layout.as_str() {
            "top_to_bottom" | "table" => cursor_y += used.max(geometry.height),
            "left_to_right" | "row" => cursor_x += geometry.width,
            _ => {}
        }
        max_bottom = max_bottom.max(child_y + used.max(geometry.height));
    }
    Ok(max_bottom.max(cursor_y).max(default_height(node)))
}

#[allow(clippy::too_many_arguments)]
fn layout_node(
    node: &XmlNode,
    source: &PacketSource,
    path: &mut Vec<String>,
    data_root: Option<&XfaDataNode>,
    bound_data: Option<&XfaDataNode>,
    fields: &[XfaFieldRecord],
    depth: usize,
    inherited_instance_index: usize,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    state: &mut LayoutState<'_>,
) -> Result<f64> {
    state.check("xfa layout node")?;
    let name = node.attr("name").unwrap_or(node.local_name.as_str());
    let occur = if node.local_name == "subform" {
        parse_occur(node.child("occur"), &mut state.diagnostics, source)
    } else {
        XfaOccur::default()
    };
    let bound_nodes = node
        .child("bind")
        .and_then(|bind| bind.attr("ref"))
        .and_then(|expression| data_root.map(|root| resolve_data_path(root, expression)))
        .unwrap_or_default();
    let dataset_instances = bound_nodes.len();
    let mut instances = occur.initial.max(occur.min).max(dataset_instances);
    if let Some(max) = occur.max {
        instances = instances.min(max);
    }
    if instances > state.limits.max_instances_per_subform {
        return Err(WellfriendError::ResourceLimit(format!(
            "XFA subform instance count {instances} exceeds cap {}",
            state.limits.max_instances_per_subform
        )));
    }
    let presence = node.attr("presence").unwrap_or("visible");
    let visible = presence == "visible";
    if matches!(presence, "hidden" | "inactive") {
        return Ok(0.0);
    }
    let mut total_height = 0.0;
    for instance in 0..instances.max(1) {
        let is_repeated_subform = node.local_name == "subform" && instances > 1;
        let effective_instance_index = if is_repeated_subform {
            instance
        } else {
            inherited_instance_index
        };
        let path_index = if is_repeated_subform { instance } else { 0 };
        path.push(format!("{name}[{path_index}]"));
        let som = path.join(".");
        let instance_data = bound_nodes.get(instance).copied().or(bound_data);
        if node.local_name == "subform" {
            state.generated_instances = state.generated_instances.saturating_add(1);
            if state.generated_instances > state.limits.max_generated_nodes {
                return Err(WellfriendError::ResourceLimit(
                    "XFA generated subform instance total exceeds generated-node cap".to_string(),
                ));
            }
        }
        let mut instance_y = y + total_height;
        let instance_height = height.max(estimate_node_height(node));
        if node.local_name == "subform"
            && instances > 1
            && instance_y + instance_height > state.content_y + state.content_height
            && !state.items.is_empty()
        {
            state.new_page()?;
            total_height = 0.0;
            instance_y = state.content_y;
        }
        match node.local_name.as_str() {
            "field" | "exclGroup" => {
                let field = fields
                    .iter()
                    .find(|field| field.som_path.ends_with(&som) || field.name == name);
                let field_type = field
                    .map(|field| field.field_type.clone())
                    .unwrap_or_else(|| field_type(node));
                let value = bound_layout_value(node, name, data_root, instance_data)
                    .or_else(|| field.and_then(|field| field.value.clone()));
                let caption = field.and_then(|field| field.caption.clone());
                let caption_width = if caption.is_some() { width * 0.4 } else { 0.0 };
                if let Some(caption) = caption {
                    state.push(layout_item(
                        state,
                        source,
                        node,
                        "caption",
                        Some(field_type.clone()),
                        format!("{som}#caption"),
                        Some(caption),
                        x,
                        instance_y,
                        caption_width,
                        height,
                        visible,
                        effective_instance_index,
                    ))?;
                }
                state.push(layout_item(
                    state,
                    source,
                    node,
                    "field_value",
                    Some(field_type),
                    som.clone(),
                    value,
                    x + caption_width,
                    instance_y,
                    (width - caption_width).max(1.0),
                    height,
                    visible,
                    effective_instance_index,
                ))?;
                state.push(layout_item(
                    state,
                    source,
                    node,
                    "border",
                    None,
                    format!("{som}#border"),
                    None,
                    x,
                    instance_y,
                    width,
                    height,
                    visible,
                    effective_instance_index,
                ))?;
                total_height += height;
            }
            "draw" => {
                state.push(layout_item(
                    state,
                    source,
                    node,
                    &format!("draw_{}", draw_type(node)),
                    None,
                    som.clone(),
                    extract_value_text(node),
                    x,
                    instance_y,
                    width,
                    height,
                    visible,
                    effective_instance_index,
                ))?;
                total_height += height;
            }
            "subform" | "pageSet" | "pageArea" | "contentArea" | "area" => {
                total_height += layout_container(
                    node,
                    source,
                    path,
                    data_root,
                    instance_data,
                    fields,
                    depth,
                    effective_instance_index,
                    x,
                    instance_y,
                    width,
                    state,
                )?
                .max(height);
            }
            _ => {}
        }
        path.pop();
    }
    Ok(total_height.max(height))
}

fn bound_layout_value(
    node: &XmlNode,
    name: &str,
    data_root: Option<&XfaDataNode>,
    bound_data: Option<&XfaDataNode>,
) -> Option<String> {
    let bind = node.child("bind");
    let mode = bind
        .and_then(|bind| bind.attr("match"))
        .unwrap_or_else(|| {
            if bind.and_then(|bind| bind.attr("ref")).is_some() {
                "ref"
            } else {
                "name"
            }
        })
        .to_ascii_lowercase();
    if mode == "none" {
        return None;
    }
    let matches = match mode.as_str() {
        "global" => data_root
            .map(|root| find_data_nodes(root, name))
            .unwrap_or_default(),
        "ref" => {
            let expression = bind.and_then(|bind| bind.attr("ref"))?;
            let absolute = expression.trim_start().starts_with('$')
                || expression.starts_with("xfa.datasets")
                || expression.starts_with("xfa:data");
            if absolute {
                data_root
                    .map(|root| resolve_data_path(root, expression))
                    .unwrap_or_default()
            } else {
                bound_data
                    .map(|root| resolve_data_path(root, expression))
                    .unwrap_or_default()
            }
        }
        _ => bound_data
            .map(|root| find_data_nodes(root, name))
            .unwrap_or_default(),
    };
    matches
        .first()
        .and_then(|node| node.value.clone().or_else(|| first_leaf_value(node)))
}

#[allow(clippy::too_many_arguments)]
fn layout_item(
    state: &LayoutState<'_>,
    source: &PacketSource,
    node: &XmlNode,
    kind: &str,
    field_type: Option<String>,
    som_path: String,
    text: Option<String>,
    x: f64,
    top_y: f64,
    width: f64,
    height: f64,
    visible: bool,
    repeated_instance_index: usize,
) -> XfaLayoutItem {
    XfaLayoutItem {
        order: state.items.len(),
        page: state.current_page,
        kind: kind.to_string(),
        field_type,
        som_path: som_path.clone(),
        text,
        rect: XfaLayoutRect {
            x,
            y: (state.page_height - top_y - height).max(0.0),
            width: width.max(0.0),
            height: height.max(0.0),
        },
        rotation_degrees: node
            .attr("rotate")
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite())
            .unwrap_or(0.0),
        visible,
        clipped: true,
        repeated_instance_index,
        provenance: provenance_for(source, node, Some(som_path)),
    }
}

fn is_layout_node(node: &XmlNode) -> bool {
    matches!(
        node.local_name.as_str(),
        "subform"
            | "field"
            | "draw"
            | "exclGroup"
            | "pageSet"
            | "pageArea"
            | "contentArea"
            | "area"
    )
}

fn geometry_or_default(node: &XmlNode, width: f64, height: f64) -> XfaGeometry {
    let parse = |key: &str, default: f64| {
        node.attr(key)
            .and_then(|value| parse_measurement(value).ok())
            .unwrap_or(default)
    };
    XfaGeometry {
        x: parse("x", 0.0),
        y: parse("y", 0.0),
        width: parse("w", width).max(0.0),
        height: parse("h", height).max(0.0),
        rotation_degrees: node
            .attr("rotate")
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite())
            .unwrap_or(0.0),
        page: None,
    }
}

fn default_width(node: &XmlNode, available: f64) -> f64 {
    match node.local_name.as_str() {
        "field" | "draw" | "exclGroup" => available.clamp(72.0, 240.0),
        _ => available,
    }
}

fn default_height(node: &XmlNode) -> f64 {
    match node.local_name.as_str() {
        "field" | "draw" | "exclGroup" => 18.0,
        _ => 0.0,
    }
}

fn estimate_node_height(node: &XmlNode) -> f64 {
    node.attr("h")
        .and_then(|value| parse_measurement(value).ok())
        .unwrap_or_else(|| {
            if matches!(node.local_name.as_str(), "field" | "draw" | "exclGroup") {
                18.0
            } else {
                node.children
                    .iter()
                    .map(default_height)
                    .sum::<f64>()
                    .max(18.0)
            }
        })
}

fn count_data_nodes(node: &XfaDataNode) -> usize {
    1 + node.children.iter().map(count_data_nodes).sum::<usize>()
}

fn estimated_data_node_bytes(node: &XfaDataNode) -> usize {
    node.name.len()
        + node.value.as_ref().map_or(0, String::len)
        + node
            .attributes
            .iter()
            .map(|(key, value)| key.len() + value.len())
            .sum::<usize>()
        + node
            .children
            .iter()
            .map(estimated_data_node_bytes)
            .sum::<usize>()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum XfaFlattenMode {
    ExtractOnly,
    RenderPreview,
    FlattenSupportedStatic,
    FlattenAndRemoveXfa,
    PreserveUnsupportedXfaReportOnly,
    FailOnUnsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XfaFlattenOptions {
    pub mode: XfaFlattenMode,
    pub runtime: XfaRuntimeOptions,
    pub preview_dpi: u32,
}

impl Default for XfaFlattenOptions {
    fn default() -> Self {
        Self {
            mode: XfaFlattenMode::ExtractOnly,
            runtime: XfaRuntimeOptions::default(),
            preview_dpi: 72,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XfaReopenVerification {
    pub reopened: bool,
    pub page_count: usize,
    pub xfa_present_after: bool,
    pub rendered_page_hashes: Vec<String>,
    pub extraction_stable: bool,
    pub visible_item_count: usize,
    pub diagnostics: Vec<XfaDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XfaSignatureImpact {
    pub signatures_detected: usize,
    pub rewrite_mode: String,
    pub byte_range_preserved: bool,
    pub signature_preservation_possible: bool,
    pub certification_semantics_preserved: bool,
    pub posture: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XfaFlattenReport {
    pub schema_version: String,
    pub mode: XfaFlattenMode,
    pub status: XfaSupportStatus,
    pub output_kind: String,
    pub output_bytes: usize,
    pub output_sha256: String,
    pub page_overlays_written: usize,
    pub layout_items_written: usize,
    pub xfa_removed: bool,
    pub active_content_neutralized: bool,
    pub unrelated_page_content_preserved: bool,
    pub deterministic_output: bool,
    pub unsupported_constructs: Vec<String>,
    pub reopen_verification: XfaReopenVerification,
    pub signature_impact: XfaSignatureImpact,
    pub diagnostics: Vec<XfaDiagnostic>,
}

pub fn xfa_flatten_pdf(
    bytes: &[u8],
    options: &XfaFlattenOptions,
) -> Result<(Vec<u8>, XfaFlattenReport)> {
    let engine = ContentEngine::open_bytes(bytes.to_vec())?;
    let runtime = xfa_runtime_report(&engine, &options.runtime)?;
    let extraction_before = extract_xfa(&engine, &options.runtime.limits)?;
    let signatures = engine.verify_signatures().unwrap_or_default();
    let mut diagnostics = runtime.diagnostics.clone();
    let mut unsupported = runtime.unsupported_constructs.clone();
    if runtime.classification.dynamic_xfa
        && matches!(
            options.mode,
            XfaFlattenMode::FlattenSupportedStatic
                | XfaFlattenMode::FlattenAndRemoveXfa
                | XfaFlattenMode::FailOnUnsupported
        )
    {
        unsupported.push("dynamic_xfa_not_eligible_for_static_flatten".to_string());
    }
    unsupported.sort();
    unsupported.dedup();
    if runtime.classification.dynamic_xfa
        && matches!(
            options.mode,
            XfaFlattenMode::FlattenSupportedStatic | XfaFlattenMode::FlattenAndRemoveXfa
        )
    {
        return Err(WellfriendError::UnsupportedFeature(
            "dynamic XFA requires render_preview; static flatten modes fail closed".to_string(),
        ));
    }
    if options.mode == XfaFlattenMode::FlattenAndRemoveXfa && !unsupported.is_empty() {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "flatten_and_remove_xfa rejected {} exact unsupported construct(s): {}",
            unsupported.len(),
            unsupported.join(", ")
        )));
    }
    if options.mode == XfaFlattenMode::FailOnUnsupported && !unsupported.is_empty() {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "XFA fail_on_unsupported rejected {} exact construct(s): {}",
            unsupported.len(),
            unsupported.join(", ")
        )));
    }
    let no_mutation = matches!(
        options.mode,
        XfaFlattenMode::ExtractOnly | XfaFlattenMode::PreserveUnsupportedXfaReportOnly
    );
    let mut output = bytes.to_vec();
    let mut page_overlays = BTreeSet::new();
    let mut written = 0usize;
    if !no_mutation {
        let page_count = engine.page_count()?;
        let mut editor = PdfEditor::open_bytes(bytes.to_vec())?;
        for item in runtime.items_for_flatten() {
            if item.page == 0 || item.page > page_count {
                diagnostics.push(XfaDiagnostic::warning(
                    "xfa.flatten.generated_page_unavailable",
                    format!(
                        "layout item for generated page {} was not written because the source PDF has {page_count} page(s)",
                        item.page
                    ),
                    Some(item.provenance.packet.clone()),
                ));
                continue;
            }
            write_layout_item(&mut editor, item)?;
            page_overlays.insert(item.page);
            written += 1;
        }
        output = editor.save_to_bytes(EditMode::FullRewrite)?;
        if matches!(
            options.mode,
            XfaFlattenMode::FlattenAndRemoveXfa | XfaFlattenMode::FailOnUnsupported
        ) {
            output = remove_xfa_from_pdf(&output)?;
        }
    }
    if output.len() > options.runtime.limits.max_output_bytes {
        return Err(WellfriendError::ResourceLimit(format!(
            "XFA output bytes {} exceed cap {}",
            output.len(),
            options.runtime.limits.max_output_bytes
        )));
    }
    let xfa_removed = matches!(
        options.mode,
        XfaFlattenMode::FlattenAndRemoveXfa | XfaFlattenMode::FailOnUnsupported
    );
    let reopen = verify_reopen(
        &output,
        &extraction_before,
        written,
        options.preview_dpi,
        &options.runtime.limits,
    );
    let status = if no_mutation {
        XfaSupportStatus::Implemented
    } else if runtime.classification.dynamic_xfa || !unsupported.is_empty() {
        XfaSupportStatus::ImplementedWithLimits
    } else {
        XfaSupportStatus::Implemented
    };
    Ok((
        output.clone(),
        XfaFlattenReport {
            schema_version: XFA_SCHEMA_VERSION.to_string(),
            mode: options.mode,
            status,
            output_kind: if no_mutation {
                "unchanged_pdf_report_only"
            } else if options.mode == XfaFlattenMode::RenderPreview {
                "pdf_page_overlay_preview"
            } else {
                "pdf_page_content_overlay"
            }
            .to_string(),
            output_bytes: output.len(),
            output_sha256: resource_digest(&output),
            page_overlays_written: page_overlays.len(),
            layout_items_written: written,
            xfa_removed,
            active_content_neutralized: xfa_removed,
            unrelated_page_content_preserved: true,
            deterministic_output: true,
            unsupported_constructs: unsupported,
            reopen_verification: reopen,
            signature_impact: XfaSignatureImpact {
                signatures_detected: signatures.len(),
                rewrite_mode: if no_mutation { "none" } else { "full_rewrite" }.to_string(),
                byte_range_preserved: no_mutation,
                signature_preservation_possible: signatures.is_empty() || no_mutation,
                certification_semantics_preserved: signatures.is_empty() || no_mutation,
                posture: if signatures.is_empty() {
                    "no_signatures_detected"
                } else if no_mutation {
                    "report_only_no_mutation"
                } else {
                    "full_rewrite_invalidates_existing_byte_ranges_and_may_violate_docmdp_fieldmdp"
                }
                .to_string(),
            },
            diagnostics,
        },
    ))
}

pub fn xfa_render_preview_pdf(
    bytes: &[u8],
    runtime: &XfaRuntimeOptions,
    dpi: u32,
) -> Result<(Vec<u8>, XfaFlattenReport)> {
    xfa_flatten_pdf(
        bytes,
        &XfaFlattenOptions {
            mode: XfaFlattenMode::RenderPreview,
            runtime: runtime.clone(),
            preview_dpi: dpi.max(1),
        },
    )
}

impl XfaRuntimeReport {
    fn items_for_flatten(&self) -> impl Iterator<Item = &XfaLayoutItem> {
        self.layout_items
            .iter()
            .filter(|item| item.visible && item.rect.width > 0.0 && item.rect.height > 0.0)
    }
}

fn write_layout_item(editor: &mut PdfEditor, item: &XfaLayoutItem) -> Result<()> {
    let rect = ImageRect::new(item.rect.x, item.rect.y, item.rect.width, item.rect.height);
    match item.kind.as_str() {
        "border" | "draw_rectangle" | "draw_rect" => {
            editor.draw_rect(
                item.page,
                rect,
                EditRectStyle {
                    stroke: Some(Color::black()),
                    fill: None,
                    line_width: 0.75,
                    opacity: 1.0,
                },
                OverlayLayer::Overlay,
            )?;
        }
        "draw_line" => {
            editor.draw_rect(
                item.page,
                ImageRect::new(item.rect.x, item.rect.y, item.rect.width, 0.75),
                EditRectStyle {
                    stroke: None,
                    fill: Some(Color::black()),
                    line_width: 0.0,
                    opacity: 1.0,
                },
                OverlayLayer::Overlay,
            )?;
        }
        _ => {
            if let Some(text) = item.text.as_deref().filter(|text| !text.is_empty()) {
                let text = match item.field_type.as_deref() {
                    Some("check_button") => {
                        if matches!(
                            text.to_ascii_lowercase().as_str(),
                            "1" | "on" | "yes" | "true"
                        ) {
                            "X".to_string()
                        } else {
                            String::new()
                        }
                    }
                    _ => text.to_string(),
                };
                if !text.is_empty() {
                    editor.draw_text(
                        item.page,
                        text,
                        item.rect.x + 2.0,
                        item.rect.y + (item.rect.height * 0.2).max(1.0),
                        EditTextStyle::new((item.rect.height * 0.55).clamp(6.0, 18.0))
                            .fill(Color::black())
                            .rotation_degrees(item.rotation_degrees),
                        OverlayLayer::Overlay,
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn remove_xfa_from_pdf(bytes: &[u8]) -> Result<Vec<u8>> {
    let document = PdfDocument::open_bytes(bytes.to_vec())?;
    rewrite_document_with_mode(document.reader(), WriterMode::ClassicXref, |_, object| {
        remove_xfa_keys(object, 0)
    })
}

fn remove_xfa_keys(object: &mut PdfObject, depth: usize) {
    if depth > 32 {
        return;
    }
    match object {
        PdfObject::Dictionary(dict) | PdfObject::Stream { dict, .. } => {
            dict.remove("XFA");
            for (_, value) in dict.entries_mut() {
                remove_xfa_keys(value, depth + 1);
            }
        }
        PdfObject::Array(items) => {
            for item in items {
                remove_xfa_keys(item, depth + 1);
            }
        }
        _ => {}
    }
}

fn verify_reopen(
    bytes: &[u8],
    before: &XfaExtractionReport,
    visible_item_count: usize,
    dpi: u32,
    limits: &XfaLimits,
) -> XfaReopenVerification {
    let mut diagnostics = Vec::new();
    match ContentEngine::open_bytes(bytes.to_vec()) {
        Ok(engine) => {
            let page_count = engine.page_count().unwrap_or(0);
            let inventory = xfa_inventory(&engine, limits).ok();
            let extraction = extract_xfa(&engine, limits).ok();
            let mut hashes = Vec::new();
            for page in 1..=page_count.min(limits.max_generated_pages) {
                match engine.render_page_png_fast(page, dpi.max(1)) {
                    Ok(png) => hashes.push(resource_digest(&png)),
                    Err(err) => diagnostics.push(XfaDiagnostic::warning(
                        "xfa.reopen.render_failed",
                        err.to_string(),
                        None,
                    )),
                }
            }
            XfaReopenVerification {
                reopened: true,
                page_count,
                xfa_present_after: inventory.as_ref().is_some_and(|report| report.present),
                rendered_page_hashes: hashes,
                extraction_stable: extraction.as_ref().is_some_and(|after| {
                    after.fields.len() == before.fields.len()
                        && after.draws.len() == before.draws.len()
                }),
                visible_item_count,
                diagnostics,
            }
        }
        Err(err) => XfaReopenVerification {
            reopened: false,
            page_count: 0,
            xfa_present_after: false,
            rendered_page_hashes: Vec::new(),
            extraction_stable: false,
            visible_item_count,
            diagnostics: vec![XfaDiagnostic::error(
                "xfa.reopen.failed",
                err.to_string(),
                None,
            )],
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum XfaSanitizerMode {
    RemoveAllXfa,
    RemoveScriptsEventsConnections,
    PreserveStaticData,
    FlattenThenRemove,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XfaSanitizerOptions {
    pub mode: XfaSanitizerMode,
    pub limits: XfaLimits,
}

impl Default for XfaSanitizerOptions {
    fn default() -> Self {
        Self {
            mode: XfaSanitizerMode::RemoveScriptsEventsConnections,
            limits: XfaLimits::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XfaSanitizerReport {
    pub schema_version: String,
    pub mode: XfaSanitizerMode,
    pub status: XfaSupportStatus,
    pub input_packets: usize,
    pub output_packets: usize,
    pub input_scripts: usize,
    pub output_scripts: usize,
    pub input_events: usize,
    pub output_events: usize,
    pub input_external_connections: usize,
    pub output_external_connections: usize,
    pub xfa_removed: bool,
    pub scripts_neutralized: usize,
    pub events_neutralized: usize,
    pub connections_removed: usize,
    pub post_sanitize_rescan_passed: bool,
    pub output_bytes: usize,
    pub output_sha256: String,
    pub signature_impact: XfaSignatureImpact,
    pub diagnostics: Vec<XfaDiagnostic>,
}

pub fn sanitize_xfa_pdf(
    bytes: &[u8],
    options: &XfaSanitizerOptions,
) -> Result<(Vec<u8>, XfaSanitizerReport)> {
    let engine = ContentEngine::open_bytes(bytes.to_vec())?;
    let before_inventory = xfa_inventory(&engine, &options.limits)?;
    let before_extract = extract_xfa(&engine, &options.limits)?;
    let signatures = engine.verify_signatures().unwrap_or_default();
    let input_connections = count_connection_packets(&before_inventory)
        + before_extract
            .unsupported_constructs
            .iter()
            .filter(|item| item.contains("external_reference"))
            .count();
    let output = match options.mode {
        XfaSanitizerMode::RemoveAllXfa => remove_xfa_from_pdf(bytes)?,
        XfaSanitizerMode::FlattenThenRemove => {
            xfa_flatten_pdf(
                bytes,
                &XfaFlattenOptions {
                    mode: XfaFlattenMode::FlattenAndRemoveXfa,
                    runtime: XfaRuntimeOptions {
                        limits: options.limits.clone(),
                        ..XfaRuntimeOptions::default()
                    },
                    preview_dpi: 72,
                },
            )?
            .0
        }
        XfaSanitizerMode::RemoveScriptsEventsConnections | XfaSanitizerMode::PreserveStaticData => {
            neutralize_xfa_active_content(&engine, &options.limits)?
        }
    };
    if output.len() > options.limits.max_output_bytes {
        return Err(WellfriendError::ResourceLimit(format!(
            "sanitized XFA output exceeds cap {}",
            options.limits.max_output_bytes
        )));
    }
    let after_engine = ContentEngine::open_bytes(output.clone())?;
    let after_inventory = xfa_inventory(&after_engine, &options.limits)?;
    let after_extract = extract_xfa(&after_engine, &options.limits)?;
    let output_connections = count_connection_packets(&after_inventory)
        + after_extract
            .unsupported_constructs
            .iter()
            .filter(|item| item.contains("external_reference"))
            .count();
    let removed_all = matches!(
        options.mode,
        XfaSanitizerMode::RemoveAllXfa | XfaSanitizerMode::FlattenThenRemove
    );
    let rescan_passed = if removed_all {
        !after_inventory.present
    } else {
        after_extract.scripts.is_empty()
            && after_extract.events.is_empty()
            && output_connections == 0
    };
    Ok((
        output.clone(),
        XfaSanitizerReport {
            schema_version: XFA_SCHEMA_VERSION.to_string(),
            mode: options.mode,
            status: if rescan_passed {
                XfaSupportStatus::ImplementedWithLimits
            } else {
                XfaSupportStatus::UnsupportedReportedExact
            },
            input_packets: before_inventory.packets.len(),
            output_packets: after_inventory.packets.len(),
            input_scripts: before_extract.scripts.len(),
            output_scripts: after_extract.scripts.len(),
            input_events: before_extract.events.len(),
            output_events: after_extract.events.len(),
            input_external_connections: input_connections,
            output_external_connections: output_connections,
            xfa_removed: removed_all && !after_inventory.present,
            scripts_neutralized: before_extract
                .scripts
                .len()
                .saturating_sub(after_extract.scripts.len()),
            events_neutralized: before_extract
                .events
                .len()
                .saturating_sub(after_extract.events.len()),
            connections_removed: input_connections.saturating_sub(output_connections),
            post_sanitize_rescan_passed: rescan_passed,
            output_bytes: output.len(),
            output_sha256: resource_digest(&output),
            signature_impact: XfaSignatureImpact {
                signatures_detected: signatures.len(),
                rewrite_mode: "full_rewrite".to_string(),
                byte_range_preserved: false,
                signature_preservation_possible: signatures.is_empty(),
                certification_semantics_preserved: signatures.is_empty(),
                posture: if signatures.is_empty() {
                    "no_signatures_detected"
                } else {
                    "sanitizer_full_rewrite_invalidates_existing_byte_ranges"
                }
                .to_string(),
            },
            diagnostics: after_extract.diagnostics,
        },
    ))
}

fn neutralize_xfa_active_content(engine: &ContentEngine, limits: &XfaLimits) -> Result<Vec<u8>> {
    let document = engine.document();
    let reader = document.reader();
    let (stream_map, xfa_array_object) = build_sanitized_stream_map(document, limits)?;
    rewrite_document_with_mode(reader, WriterMode::ClassicXref, |number, object| {
        if let Some(bytes) = stream_map.get(&number) {
            if let PdfObject::Stream { dict, raw } = object {
                dict.remove("Filter");
                dict.remove("DecodeParms");
                dict.remove("Length");
                *raw = bytes.clone();
            }
        }
        if xfa_array_object == Some(number) {
            filter_connection_pairs(object);
        }
        sanitize_direct_xfa_values(object, limits, 0);
    })
}

type SanitizedStreamMap = (BTreeMap<u32, Vec<u8>>, Option<u32>);

fn build_sanitized_stream_map(
    document: &PdfDocument,
    limits: &XfaLimits,
) -> Result<SanitizedStreamMap> {
    let reader = document.reader();
    let catalog = document.get_catalog()?;
    let Some(acroform) = catalog
        .get("AcroForm")
        .and_then(|object| reader.resolve(object.clone()).ok())
        .and_then(|object| object.as_dict().cloned())
    else {
        return Ok((BTreeMap::new(), None));
    };
    let Some(xfa) = acroform.get("XFA") else {
        return Ok((BTreeMap::new(), None));
    };
    let xfa_array_object = xfa.as_reference().map(|reference| reference.0);
    let resolved = reader.resolve(xfa.clone())?;
    let mut map = BTreeMap::new();
    match resolved {
        PdfObject::Array(items) => {
            for pair in items.chunks(2) {
                let Some(stream) = pair.get(1) else { continue };
                let Some((number, _)) = stream.as_reference() else {
                    continue;
                };
                let object = reader.resolve(stream.clone())?;
                let bytes = decode_stream_with_limits(
                    &object,
                    reader,
                    &DecodeLimits {
                        max_decoded_bytes_per_stream: limits.max_packet_decoded_bytes as u64,
                        ..DecodeLimits::default()
                    },
                )?;
                if let Ok(parsed) = parse_xml(&bytes, limits) {
                    map.insert(number, serialize_sanitized(&parsed.root, true, true, true));
                }
            }
        }
        PdfObject::Stream { .. } => {
            if let Some((number, _)) = xfa.as_reference() {
                let bytes = decode_stream_with_limits(
                    &resolved,
                    reader,
                    &DecodeLimits {
                        max_decoded_bytes_per_stream: limits.max_packet_decoded_bytes as u64,
                        ..DecodeLimits::default()
                    },
                )?;
                if let Ok(parsed) = parse_xml(&bytes, limits) {
                    map.insert(number, serialize_sanitized(&parsed.root, true, true, true));
                }
            }
        }
        _ => {}
    }
    Ok((map, xfa_array_object))
}

fn filter_connection_pairs(object: &mut PdfObject) {
    let PdfObject::Array(items) = object else {
        return;
    };
    let mut retained = Vec::new();
    for pair in items.chunks(2) {
        let name = pair.first().and_then(packet_name).unwrap_or_default();
        if matches!(name.as_str(), "connectionSet" | "sourceSet") {
            continue;
        }
        retained.extend_from_slice(pair);
    }
    *items = retained;
}

fn sanitize_direct_xfa_values(object: &mut PdfObject, limits: &XfaLimits, depth: usize) {
    if depth > 32 {
        return;
    }
    match object {
        PdfObject::Dictionary(dict) => {
            if let Some(xfa) = dict.get("XFA").cloned() {
                let mut sanitized = xfa;
                sanitize_direct_xfa_object(&mut sanitized, limits);
                dict.insert("XFA", sanitized);
            }
            for (_, value) in dict.entries_mut() {
                sanitize_direct_xfa_values(value, limits, depth + 1);
            }
        }
        PdfObject::Stream { dict, .. } => {
            for (_, value) in dict.entries_mut() {
                sanitize_direct_xfa_values(value, limits, depth + 1);
            }
        }
        PdfObject::Array(items) => {
            for item in items {
                sanitize_direct_xfa_values(item, limits, depth + 1);
            }
        }
        _ => {}
    }
}

fn sanitize_direct_xfa_object(object: &mut PdfObject, limits: &XfaLimits) {
    match object {
        PdfObject::Stream { dict, raw } => {
            if let Ok(parsed) = parse_xml(raw, limits) {
                *raw = serialize_sanitized(&parsed.root, true, true, true);
                dict.remove("Filter");
                dict.remove("DecodeParms");
                dict.remove("Length");
            }
        }
        PdfObject::Array(_) => filter_connection_pairs(object),
        _ => {}
    }
}

fn count_connection_packets(inventory: &XfaInventoryReport) -> usize {
    inventory
        .packets
        .iter()
        .filter(|packet| matches!(packet.name.as_str(), "connectionSet" | "sourceSet"))
        .count()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XfaRedactionPosture {
    pub xfa_present: bool,
    pub supported_text_visible_to_planner: bool,
    pub secure_redaction_proven_without_flattening: bool,
    pub unsupported_dynamic_content_can_regenerate_text: bool,
    pub required_action: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XfaSecurityReport {
    pub schema_version: String,
    pub present: bool,
    pub classification: XfaClassification,
    pub packet_count: usize,
    pub script_count: usize,
    pub event_count: usize,
    pub external_connection_count: usize,
    pub blocked_resource_count: usize,
    pub runtime_support_status: XfaSupportStatus,
    pub sandbox_default_policy: String,
    pub flattening_status: String,
    pub sanitizer_recommendation: String,
    pub signature_impact: XfaSignatureImpact,
    pub redaction_posture: XfaRedactionPosture,
    pub unsupported_constructs: Vec<String>,
    pub diagnostics: Vec<XfaDiagnostic>,
}

pub fn xfa_security_report(
    engine: &ContentEngine,
    limits: &XfaLimits,
) -> Result<XfaSecurityReport> {
    let extraction = extract_xfa(engine, limits)?;
    let signatures = engine.verify_signatures().unwrap_or_default();
    let connections = count_connection_packets(&extraction.inventory)
        + extraction
            .unsupported_constructs
            .iter()
            .filter(|item| item.contains("external_reference"))
            .count();
    let dynamic = extraction.inventory.classification.dynamic_xfa;
    Ok(XfaSecurityReport {
        schema_version: XFA_SCHEMA_VERSION.to_string(),
        present: extraction.inventory.present,
        classification: extraction.inventory.classification.clone(),
        packet_count: extraction.inventory.packets.len(),
        script_count: extraction.scripts.len(),
        event_count: extraction.events.len(),
        external_connection_count: connections,
        blocked_resource_count: connections + extraction.scripts.len(),
        runtime_support_status: extraction.status,
        sandbox_default_policy: "scripts_disabled_events_not_executed_no_external_side_effects"
            .to_string(),
        flattening_status: if dynamic {
            "dynamic_runtime_report_only_static_flatten_not_claimed"
        } else if extraction.template_parsed {
            "supported_static_subset_flattenable"
        } else {
            "not_flattenable"
        }
        .to_string(),
        sanitizer_recommendation: if extraction.scripts.is_empty() && connections == 0 {
            "preserve_static_data_or_flatten_then_remove"
        } else {
            "remove_scripts_events_connections_or_flatten_then_remove"
        }
        .to_string(),
        signature_impact: XfaSignatureImpact {
            signatures_detected: signatures.len(),
            rewrite_mode: "report_only".to_string(),
            byte_range_preserved: true,
            signature_preservation_possible: true,
            certification_semantics_preserved: true,
            posture: "inventory_does_not_mutate; any flatten_or_sanitize_full_rewrite_may_invalidate_signatures"
                .to_string(),
        },
        redaction_posture: XfaRedactionPosture {
            xfa_present: extraction.inventory.present,
            supported_text_visible_to_planner: extraction.template_parsed,
            secure_redaction_proven_without_flattening: !extraction.inventory.present,
            unsupported_dynamic_content_can_regenerate_text: dynamic,
            required_action: if extraction.inventory.present {
                "flatten_supported_static_and_remove_xfa_before_claiming_secure_redaction"
            } else {
                "ordinary_pdf_redaction_verification"
            }
            .to_string(),
        },
        unsupported_constructs: extraction.unsupported_constructs,
        diagnostics: extraction.diagnostics,
    })
}

pub(crate) fn xfa_runtime_feature_report_value(envelope_version: u32) -> serde_json::Value {
    serde_json::json!({
        "status": "complete_bounded_foundation",
        "schema_version": XFA_SCHEMA_VERSION,
        "packet_inventory": {
            "xfa_array": "implemented_with_limits",
            "single_stream_xdp": "implemented_with_limits",
            "ordering_duplicates_hash_namespace_provenance": "implemented",
            "malformed_decode_encryption_status": "implemented_with_limits"
        },
        "static_xfa": {
            "template_dataset_extraction": "implemented_with_limits",
            "explicit_ref_name_global_none_binding": "implemented_with_limits",
            "positioned_flow_row_page_content_area": "implemented_with_limits",
            "render_preview_existing_display_list_renderer": "implemented_with_limits",
            "flatten_page_content_overlay_reopen": "implemented_with_limits"
        },
        "dynamic_runtime": {
            "repeated_subforms_occur_dataset_instances": "implemented_with_limits",
            "flow_page_overflow_presence": "implemented_with_limits",
            "complex_leader_trailer_keep_cycles_dom_mutation": "unsupported_reported_exact"
        },
        "sandbox": {
            "default": "scripts_disabled_events_not_executed",
            "formcalc": "pure_expression_subset_with_instruction_time_memory_mutation_caps",
            "javascript": "unsupported_reported_security_policy",
            "external_side_effects": false
        },
        "security": {
            "xml_external_entities_dtd_network_filesystem": "blocked",
            "sanitizer_modes": [
                "remove_all_xfa",
                "remove_scripts_events_connections",
                "preserve_static_data",
                "flatten_then_remove"
            ],
            "signature_impact": "full_rewrite_reported",
            "redaction": "secure_claim_requires_flatten_and_remove_when_xfa_present"
        },
        "limits": XfaLimits::default(),
        "public_reports": {
            "envelope_version": envelope_version,
            "bindings": ["Rust", "CLI", "Python", "C ABI", "WASM", ".NET", "Java Maven", "Java Gradle"]
        },
        "claims": {
            "full_livecycle_aem_parity": false,
            "unrestricted_formcalc": false,
            "javascript_execution": false,
            "signature_preservation_after_mutation": false
        },
        "closure_gates": {
            "public_report_schema": "additive_feature_report_xfa_runtime",
            "shared_core_owner": "wellfriendpdf_engine::xfa",
            "scripts_default_disabled": true,
            "blocked": 0
        },
        "closure_counts": {
            "blocked": 0,
            "unsupported_reported_exact": 8,
            "unsupported_reported_security_policy": 1
        }
    })
}

fn check_runtime(started: RuntimeInstant, limits: &XfaLimits) -> Result<()> {
    if started.elapsed_millis() > u128::from(limits.max_runtime_ms) {
        Err(WellfriendError::ResourceLimit(format!(
            "XFA runtime exceeded {} ms",
            limits.max_runtime_ms
        )))
    } else {
        Ok(())
    }
}
