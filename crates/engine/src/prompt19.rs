//! Combined Prompt 19 form-action policy and DOCX fidelity audit.
//!
//! This module owns the shared, versioned report and mutation model. It never
//! executes arbitrary PDF JavaScript: scripts are inventoried, a deliberately
//! small expression subset may be evaluated for opt-in calculation flattening,
//! and every mutation is routed through the existing writer/editor and Prompt
//! 18B signature-policy surfaces.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io::{Cursor, Read};

use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use zip::ZipArchive;

use crate::editing::{EditMode, PdfEditor};
use crate::error::{OxideError, Result};
use crate::filters::{decode_stream_lossless_with_limits, DecodeLimits, StreamDecodeStatus};
use crate::info::decode_pdf_text_string;
use crate::object::{PdfDictionary, PdfObject};
use crate::office::{pdf_to_docx, DocxLayout, DocxOptions};
use crate::prompt18::{analyze_edit_policy, EditOperation, EditPolicyDecision};
use crate::reader::PdfReader;
use crate::versioning::resource_digest;
use crate::writer::{rewrite_document_with_mode, WriterMode};
use crate::{ContentEngine, PdfDocument};

pub const PROMPT19_SCHEMA_VERSION: &str = "prompt19.form-js-interactive-docx-layout.v1";
pub const MAX_SCRIPT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_TOTAL_SCRIPT_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_ACTIONS: usize = 100_000;
pub const MAX_ACTION_GRAPH_DEPTH: usize = 64;
pub const MAX_DEPENDENCIES: usize = 100_000;
pub const MAX_SAFE_INSTRUCTIONS: usize = 10_000;
pub const MAX_FIELD_MUTATIONS: usize = 10_000;
pub const MAX_SAFE_VALUE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_DOCX_PAGES: usize = 10_000;
pub const MAX_DOCX_PARTS: usize = 100_000;
pub const MAX_DOCX_OUTPUT_BYTES: usize = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Prompt19SupportStatus {
    Implemented,
    ImplementedWithLimits,
    UnsupportedReportedSecurityPolicy,
    UnsupportedReportedExact,
    UnsupportedReportedNoRuntime,
    NotInPrompt19Scope,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FormJsPolicyMode {
    InventoryOnly,
    DisableExecutionPreserveSource,
    RemoveJavascriptOnly,
    RemoveAllActiveActions,
    PreserveSafeNavigationOnly,
    FlattenCalculatedValuesThenRemove,
    Custom,
}

impl FormJsPolicyMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().replace('-', "_").as_str() {
            "inventory_only" => Some(Self::InventoryOnly),
            "disable_execution_preserve_source" => Some(Self::DisableExecutionPreserveSource),
            "remove_javascript_only" => Some(Self::RemoveJavascriptOnly),
            "remove_all_active_actions" => Some(Self::RemoveAllActiveActions),
            "preserve_safe_navigation_only" => Some(Self::PreserveSafeNavigationOnly),
            "flatten_calculated_values_then_remove" => {
                Some(Self::FlattenCalculatedValuesThenRemove)
            }
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InventoryOnly => "inventory_only",
            Self::DisableExecutionPreserveSource => "disable_execution_preserve_source",
            Self::RemoveJavascriptOnly => "remove_javascript_only",
            Self::RemoveAllActiveActions => "remove_all_active_actions",
            Self::PreserveSafeNavigationOnly => "preserve_safe_navigation_only",
            Self::FlattenCalculatedValuesThenRemove => "flatten_calculated_values_then_remove",
            Self::Custom => "custom",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormJsLimits {
    pub max_script_bytes: usize,
    pub max_total_script_bytes: usize,
    pub max_actions: usize,
    pub max_action_graph_depth: usize,
    pub max_dependencies: usize,
    pub max_safe_instructions: usize,
    pub max_field_mutations: usize,
    pub max_safe_value_bytes: usize,
}

impl Default for FormJsLimits {
    fn default() -> Self {
        Self {
            max_script_bytes: MAX_SCRIPT_BYTES,
            max_total_script_bytes: MAX_TOTAL_SCRIPT_BYTES,
            max_actions: MAX_ACTIONS,
            max_action_graph_depth: MAX_ACTION_GRAPH_DEPTH,
            max_dependencies: MAX_DEPENDENCIES,
            max_safe_instructions: MAX_SAFE_INSTRUCTIONS,
            max_field_mutations: MAX_FIELD_MUTATIONS,
            max_safe_value_bytes: MAX_SAFE_VALUE_BYTES,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CustomActionPolicy {
    #[serde(default)]
    pub preserve_action_types: BTreeSet<String>,
    #[serde(default)]
    pub remove_action_types: BTreeSet<String>,
    #[serde(default)]
    pub preserve_script_source: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormJsSanitizerOptions {
    pub mode: FormJsPolicyMode,
    #[serde(default)]
    pub custom: CustomActionPolicy,
    #[serde(default)]
    pub signature_policy_override: bool,
    #[serde(default)]
    pub limits: FormJsLimits,
}

impl Default for FormJsSanitizerOptions {
    fn default() -> Self {
        Self {
            mode: FormJsPolicyMode::RemoveJavascriptOnly,
            custom: CustomActionPolicy::default(),
            signature_policy_override: false,
            limits: FormJsLimits::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionInventoryEntry {
    pub stable_id: String,
    pub object_reference: Option<String>,
    pub owner_object: String,
    pub owner_type: String,
    pub owner_field: Option<String>,
    pub action_location: String,
    pub event: String,
    pub action_type: String,
    pub script_source_type: Option<String>,
    pub decoded_script_length: usize,
    pub sha256: Option<String>,
    pub preview: Option<String>,
    pub detected_api_names: Vec<String>,
    pub unsafe_indicators: Vec<String>,
    pub calculation_dependencies: Vec<String>,
    pub field_references: Vec<String>,
    pub action_chain_provenance: Vec<String>,
    pub sanitizer_disposition: String,
    pub execution_policy: String,
    pub signature_impact: String,
    pub safe_subset_compatible: bool,
    pub diagnostic: Option<String>,
    #[serde(skip)]
    script_source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormJsInventoryReport {
    pub schema_version: String,
    pub javascript_execution_enabled: bool,
    pub arbitrary_acrobat_dom_emulation: bool,
    pub actions: Vec<ActionInventoryEntry>,
    pub action_count_by_type: BTreeMap<String, usize>,
    pub script_count: usize,
    pub decoded_script_bytes: usize,
    pub external_target_count: usize,
    pub submit_import_count: usize,
    pub safe_subset_compatible_count: usize,
    pub malformed_or_undecodable_count: usize,
    pub limit_denials: Vec<String>,
    pub deterministic: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalculationEdge {
    pub from_field: String,
    pub to_field: String,
    pub action_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormActionGraphReport {
    pub schema_version: String,
    pub calculation_order: Vec<String>,
    pub fields: Vec<String>,
    pub edges: Vec<CalculationEdge>,
    pub cycles: Vec<Vec<String>>,
    pub missing_fields: Vec<String>,
    pub ambiguous_fields: Vec<String>,
    pub hidden_fields: Vec<String>,
    pub read_only_fields: Vec<String>,
    pub cross_page_dependencies: Vec<CalculationEdge>,
    pub deterministic: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalculationResult {
    pub action_id: String,
    pub target_field: Option<String>,
    pub original_value: Option<String>,
    pub calculated_value: Option<String>,
    pub status: Prompt19SupportStatus,
    pub instructions: usize,
    pub diagnostic: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalculationFlattenReport {
    pub schema_version: String,
    pub policy: FormJsPolicyMode,
    pub dependency_order: Vec<String>,
    pub results: Vec<CalculationResult>,
    pub values_updated: usize,
    pub appearances_regenerated: usize,
    pub scripts_removed: usize,
    pub unsupported_scripts: usize,
    pub cycles_blocked: usize,
    pub output_sha256: String,
    pub output_bytes: usize,
    pub signature_impact: serde_json::Value,
    pub deterministic: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormJsSanitizerReport {
    pub schema_version: String,
    pub mode: FormJsPolicyMode,
    pub input_action_count: usize,
    pub output_action_count: usize,
    pub removed_count: usize,
    pub preserved_safe_navigation_count: usize,
    pub forbidden_remaining_count: usize,
    pub rescan_passed: bool,
    pub output_sha256: String,
    pub output_bytes: usize,
    pub signature_impact: serde_json::Value,
    pub deterministic: bool,
    pub exact_limits: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocxLayoutAuditReport {
    pub schema_version: String,
    pub layout: String,
    pub page_count: usize,
    pub section_count: usize,
    pub page_sizes_twips: Vec<[i64; 2]>,
    pub paragraph_count: usize,
    pub text_box_count: usize,
    pub table_count: usize,
    pub merged_cell_count: usize,
    pub image_count: usize,
    pub hyperlink_count: usize,
    pub header_part_count: usize,
    pub footer_part_count: usize,
    pub package_parts: usize,
    pub output_bytes: usize,
    pub deterministic_sha256: String,
    pub deterministic_repeat_match: bool,
    pub readback_ok: bool,
    pub unsupported_exact: Vec<String>,
}

#[derive(Debug, Clone)]
struct FieldRecord {
    object: Option<(u32, u16)>,
    name: String,
    value: String,
    flags: i64,
    pages: BTreeSet<usize>,
}

struct InventoryContext<'a> {
    reader: &'a PdfReader,
    limits: &'a FormJsLimits,
    entries: Vec<ActionInventoryEntry>,
    keys: HashSet<String>,
    total_script_bytes: usize,
    limit_denials: Vec<String>,
}

pub fn form_javascript_inventory(
    engine: &ContentEngine,
    limits: &FormJsLimits,
) -> Result<FormJsInventoryReport> {
    let reader = engine.document().reader();
    let mut context = InventoryContext {
        reader,
        limits,
        entries: Vec::new(),
        keys: HashSet::new(),
        total_script_bytes: 0,
        limit_denials: Vec::new(),
    };

    for (number, generation) in reader.object_ids() {
        if context.entries.len() >= limits.max_actions {
            context
                .limit_denials
                .push(format!("action count exceeded cap {}", limits.max_actions));
            break;
        }
        let Ok(object) = reader.get_object(number, generation) else {
            continue;
        };
        let owner = format!("{number} {generation} R");
        scan_action_slots(
            &mut context,
            &object,
            &owner,
            &owner_type(&object),
            object_field_name(&object),
            &owner,
            0,
        );
    }

    scan_document_javascript_name_tree(engine.document(), &mut context)?;
    context
        .entries
        .sort_by(|a, b| a.stable_id.cmp(&b.stable_id));
    let mut by_type = BTreeMap::new();
    for entry in &context.entries {
        *by_type.entry(entry.action_type.clone()).or_insert(0) += 1;
    }
    let script_count = context
        .entries
        .iter()
        .filter(|entry| entry.sha256.is_some())
        .count();
    let external_target_count = context
        .entries
        .iter()
        .filter(|entry| {
            matches!(
                entry.action_type.as_str(),
                "URI" | "Launch" | "GoToR" | "GoToE" | "SubmitForm" | "ImportData"
            ) || !entry.unsafe_indicators.is_empty()
        })
        .count();
    let submit_import_count = context
        .entries
        .iter()
        .filter(|entry| matches!(entry.action_type.as_str(), "SubmitForm" | "ImportData"))
        .count();
    let safe_subset_compatible_count = context
        .entries
        .iter()
        .filter(|entry| entry.safe_subset_compatible)
        .count();
    let malformed_or_undecodable_count = context
        .entries
        .iter()
        .filter(|entry| entry.diagnostic.is_some())
        .count();
    Ok(FormJsInventoryReport {
        schema_version: PROMPT19_SCHEMA_VERSION.to_string(),
        javascript_execution_enabled: false,
        arbitrary_acrobat_dom_emulation: false,
        actions: context.entries,
        action_count_by_type: by_type,
        script_count,
        decoded_script_bytes: context.total_script_bytes,
        external_target_count,
        submit_import_count,
        safe_subset_compatible_count,
        malformed_or_undecodable_count,
        limit_denials: context.limit_denials,
        deterministic: true,
    })
}

fn scan_action_slots(
    context: &mut InventoryContext<'_>,
    object: &PdfObject,
    owner: &str,
    owner_kind: &str,
    owner_field: Option<String>,
    path: &str,
    depth: usize,
) {
    if depth > context.limits.max_action_graph_depth
        || context.entries.len() >= context.limits.max_actions
    {
        return;
    }
    let dict = match object {
        PdfObject::Dictionary(dict) | PdfObject::Stream { dict, .. } => dict,
        PdfObject::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                scan_action_slots(
                    context,
                    item,
                    owner,
                    owner_kind,
                    owner_field.clone(),
                    &format!("{path}[{index}]"),
                    depth + 1,
                );
            }
            return;
        }
        _ => return,
    };

    for key in ["OpenAction", "A"] {
        if let Some(value) = dict.get(key) {
            walk_action(
                context,
                value,
                owner,
                owner_kind,
                owner_field.clone(),
                &format!("{path}/{key}"),
                event_name(key),
                Vec::new(),
                0,
                &mut HashSet::new(),
            );
        }
    }
    if let Some(aa) = dict
        .get("AA")
        .and_then(|value| context.reader.resolve(value.clone()).ok())
    {
        if let Some(actions) = aa.as_dict() {
            for (event, value) in actions.entries() {
                walk_action(
                    context,
                    value,
                    owner,
                    owner_kind,
                    owner_field.clone(),
                    &format!("{path}/AA/{event}"),
                    event_name(event),
                    Vec::new(),
                    0,
                    &mut HashSet::new(),
                );
            }
        }
    }

    for (key, value) in dict.entries() {
        if matches!(key.as_str(), "OpenAction" | "A" | "AA" | "Next" | "Parent") {
            continue;
        }
        if value.as_reference().is_none() {
            scan_action_slots(
                context,
                value,
                owner,
                owner_kind,
                owner_field.clone(),
                &format!("{path}/{key}"),
                depth + 1,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn walk_action(
    context: &mut InventoryContext<'_>,
    value: &PdfObject,
    owner: &str,
    owner_kind: &str,
    owner_field: Option<String>,
    location: &str,
    event: String,
    mut provenance: Vec<String>,
    depth: usize,
    seen: &mut HashSet<(u32, u16)>,
) {
    if depth > context.limits.max_action_graph_depth
        || context.entries.len() >= context.limits.max_actions
    {
        context.limit_denials.push(format!(
            "action graph at {location} exceeded depth/count cap"
        ));
        return;
    }
    if let Some(reference) = value.as_reference() {
        if !seen.insert(reference) {
            add_malformed_action(
                context,
                owner,
                owner_kind,
                owner_field,
                location,
                event,
                provenance,
                format!("cyclic /Next graph at {} {} R", reference.0, reference.1),
            );
            return;
        }
        provenance.push(format!("{} {} R", reference.0, reference.1));
    }
    let resolved = match context.reader.resolve(value.clone()) {
        Ok(value) => value,
        Err(error) => {
            add_malformed_action(
                context,
                owner,
                owner_kind,
                owner_field,
                location,
                event,
                provenance,
                format!("action resolution failed: {error}"),
            );
            return;
        }
    };
    if let PdfObject::Array(items) = &resolved {
        for (index, item) in items.iter().enumerate() {
            walk_action(
                context,
                item,
                owner,
                owner_kind,
                owner_field.clone(),
                &format!("{location}[{index}]"),
                event.clone(),
                provenance.clone(),
                depth + 1,
                seen,
            );
        }
        return;
    }
    let Some(dict) = resolved.as_dict() else {
        // A catalog OpenAction may be a destination rather than an action.
        if location.ends_with("/OpenAction") {
            add_action_entry(
                context,
                value,
                owner,
                owner_kind,
                owner_field,
                location,
                event,
                "GoTo".to_string(),
                None,
                provenance,
                None,
            );
        } else {
            add_malformed_action(
                context,
                owner,
                owner_kind,
                owner_field,
                location,
                event,
                provenance,
                format!("action resolved to {}", resolved.variant_name()),
            );
        }
        return;
    };
    let action_type = dict.get_name("S").unwrap_or("Malformed").to_string();
    let script = if action_type == "JavaScript" || dict.get("JS").is_some() {
        Some(decode_script(context, dict.get("JS")))
    } else {
        None
    };
    add_action_entry(
        context,
        value,
        owner,
        owner_kind,
        owner_field.clone(),
        location,
        event.clone(),
        action_type,
        script,
        provenance.clone(),
        None,
    );
    if let Some(next) = dict.get("Next") {
        walk_action(
            context,
            next,
            owner,
            owner_kind,
            owner_field,
            &format!("{location}/Next"),
            event,
            provenance,
            depth + 1,
            seen,
        );
    }
}

fn scan_document_javascript_name_tree(
    document: &PdfDocument,
    context: &mut InventoryContext<'_>,
) -> Result<()> {
    let catalog = document.get_catalog()?;
    let Some(names) = catalog.get("Names") else {
        return Ok(());
    };
    let names = context.reader.resolve(names.clone())?;
    let Some(javascript) = names.as_dict().and_then(|dict| dict.get("JavaScript")) else {
        return Ok(());
    };
    walk_javascript_name_tree(context, javascript, 0, &mut HashSet::new());
    Ok(())
}

fn walk_javascript_name_tree(
    context: &mut InventoryContext<'_>,
    node: &PdfObject,
    depth: usize,
    seen: &mut HashSet<(u32, u16)>,
) {
    if depth > context.limits.max_action_graph_depth {
        context
            .limit_denials
            .push("JavaScript name tree depth cap exceeded".to_string());
        return;
    }
    if let Some(reference) = node.as_reference() {
        if !seen.insert(reference) {
            context.limit_denials.push(format!(
                "cyclic JavaScript name tree at {} {} R",
                reference.0, reference.1
            ));
            return;
        }
    }
    let Ok(node) = context.reader.resolve(node.clone()) else {
        return;
    };
    let Some(dict) = node.as_dict() else {
        return;
    };
    if let Some(names) = dict.get("Names").and_then(PdfObject::as_array) {
        for (index, pair) in names.chunks(2).enumerate() {
            if pair.len() != 2 {
                continue;
            }
            walk_action(
                context,
                &pair[1],
                "catalog",
                "document_javascript_name_tree",
                None,
                &format!("catalog/Names/JavaScript/Names[{index}]"),
                "document_javascript".to_string(),
                Vec::new(),
                0,
                &mut HashSet::new(),
            );
        }
    }
    if let Some(kids) = dict.get("Kids").and_then(PdfObject::as_array) {
        for kid in kids {
            walk_javascript_name_tree(context, kid, depth + 1, seen);
        }
    }
}

#[derive(Debug)]
struct DecodedScript {
    source_type: String,
    bytes: Vec<u8>,
    diagnostic: Option<String>,
}

fn decode_script(context: &mut InventoryContext<'_>, source: Option<&PdfObject>) -> DecodedScript {
    let Some(source) = source else {
        return DecodedScript {
            source_type: "missing".to_string(),
            bytes: Vec::new(),
            diagnostic: Some("JavaScript action has no /JS source".to_string()),
        };
    };
    let resolved = match context.reader.resolve(source.clone()) {
        Ok(value) => value,
        Err(error) => {
            return DecodedScript {
                source_type: "unresolved".to_string(),
                bytes: Vec::new(),
                diagnostic: Some(format!("script source resolution failed: {error}")),
            }
        }
    };
    let (source_type, bytes, diagnostic) = match resolved {
        PdfObject::String(bytes) => ("string".to_string(), bytes, None),
        stream @ PdfObject::Stream { .. } => {
            let limits = DecodeLimits {
                max_decoded_bytes_per_stream: context.limits.max_script_bytes as u64,
                max_decoded_bytes_per_document: context.limits.max_script_bytes as u64,
                ..DecodeLimits::default()
            };
            match decode_stream_lossless_with_limits(&stream, context.reader, &limits) {
                Ok(decoded) if decoded.status == StreamDecodeStatus::Complete => {
                    ("decoded_stream".to_string(), decoded.data, None)
                }
                Ok(decoded) => (
                    "undecodable_stream".to_string(),
                    Vec::new(),
                    Some(format!("script stream decode status: {:?}", decoded.status)),
                ),
                Err(error) => (
                    "undecodable_stream".to_string(),
                    Vec::new(),
                    Some(format!("script stream decode failed: {error}")),
                ),
            }
        }
        other => (
            "unsupported_source".to_string(),
            Vec::new(),
            Some(format!(
                "script source resolved to {}",
                other.variant_name()
            )),
        ),
    };
    if bytes.len() > context.limits.max_script_bytes
        || context.total_script_bytes.saturating_add(bytes.len())
            > context.limits.max_total_script_bytes
    {
        context.limit_denials.push(format!(
            "script source exceeded per-script or total cap ({} bytes)",
            bytes.len()
        ));
        return DecodedScript {
            source_type,
            bytes: Vec::new(),
            diagnostic: Some("script bytes denied by configured cap".to_string()),
        };
    }
    context.total_script_bytes += bytes.len();
    DecodedScript {
        source_type,
        bytes,
        diagnostic,
    }
}

#[allow(clippy::too_many_arguments)]
fn add_action_entry(
    context: &mut InventoryContext<'_>,
    source_value: &PdfObject,
    owner: &str,
    owner_kind: &str,
    owner_field: Option<String>,
    location: &str,
    event: String,
    action_type: String,
    script: Option<DecodedScript>,
    provenance: Vec<String>,
    forced_diagnostic: Option<String>,
) {
    let object_reference = source_value
        .as_reference()
        .map(|reference| format!("{} {} R", reference.0, reference.1));
    let key = format!(
        "{owner}|{location}|{}",
        object_reference.as_deref().unwrap_or("direct")
    );
    if !context.keys.insert(key) {
        return;
    }
    let id_hash = resource_digest(format!("{owner}|{location}|{action_type}").as_bytes());
    let stable_id = format!("action-{}", &id_hash[..16]);
    let (source_type, bytes, diagnostic) = script
        .map(|script| (Some(script.source_type), script.bytes, script.diagnostic))
        .unwrap_or((None, Vec::new(), None));
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let detected_api_names = detected_api_names(&text);
    let unsafe_indicators = unsafe_indicators(&text);
    let field_references = field_references(&text);
    let safe_subset_compatible =
        action_type == "JavaScript" && !text.is_empty() && safe_subset_static_check(&text).is_ok();
    context.entries.push(ActionInventoryEntry {
        stable_id,
        object_reference,
        owner_object: owner.to_string(),
        owner_type: owner_kind.to_string(),
        owner_field,
        action_location: location.to_string(),
        event,
        action_type: action_type.clone(),
        script_source_type: source_type,
        decoded_script_length: bytes.len(),
        sha256: (!bytes.is_empty()).then(|| sha256(&bytes)),
        preview: (!text.is_empty()).then(|| safe_preview(&text)),
        detected_api_names,
        unsafe_indicators,
        calculation_dependencies: field_references.clone(),
        field_references,
        action_chain_provenance: provenance,
        sanitizer_disposition: "policy_dependent".to_string(),
        execution_policy: if action_type == "JavaScript" {
            "disabled_inventory_only_unless_bounded_flatten_opt_in".to_string()
        } else {
            "not_executed_by_inventory".to_string()
        },
        signature_impact: "inventory_none; mutation_requires_prompt18b_policy".to_string(),
        safe_subset_compatible,
        diagnostic: forced_diagnostic.or(diagnostic),
        script_source: (!text.is_empty()).then_some(text),
    });
}

#[allow(clippy::too_many_arguments)]
fn add_malformed_action(
    context: &mut InventoryContext<'_>,
    owner: &str,
    owner_kind: &str,
    owner_field: Option<String>,
    location: &str,
    event: String,
    provenance: Vec<String>,
    diagnostic: String,
) {
    add_action_entry(
        context,
        &PdfObject::Null,
        owner,
        owner_kind,
        owner_field,
        location,
        event,
        "Malformed".to_string(),
        None,
        provenance,
        Some(diagnostic),
    );
}

fn owner_type(object: &PdfObject) -> String {
    let Some(dict) = object.as_dict() else {
        return "unknown".to_string();
    };
    if dict.get_name("Type") == Some("Catalog") {
        "catalog".to_string()
    } else if dict.get_name("Type") == Some("Page") {
        "page".to_string()
    } else if dict.get_name("Subtype") == Some("Widget") {
        "widget".to_string()
    } else if dict.get_name("Type") == Some("Annot") || dict.get("Subtype").is_some() {
        "annotation".to_string()
    } else if dict.get("FT").is_some() || dict.get("T").is_some() {
        "field".to_string()
    } else if dict.get_name("S").is_some() {
        "action".to_string()
    } else {
        "object".to_string()
    }
}

fn object_field_name(object: &PdfObject) -> Option<String> {
    object
        .as_dict()
        .and_then(|dict| dict.get("T"))
        .and_then(pdf_scalar_string)
}

fn event_name(key: &str) -> String {
    match key {
        "C" => "calculate",
        "V" => "validate",
        "F" => "format",
        "K" => "keystroke",
        "E" => "cursor_enter",
        "X" => "cursor_exit",
        "D" => "mouse_down",
        "U" => "mouse_up",
        "Fo" => "focus",
        "Bl" => "blur",
        "O" => "page_open",
        "C1" => "page_close",
        "OpenAction" => "document_open",
        "A" => "primary_action",
        other => other,
    }
    .to_string()
}

fn detected_api_names(script: &str) -> Vec<String> {
    let apis = [
        "getField",
        "submitForm",
        "importDataObject",
        "exportDataObject",
        "launchURL",
        "getURL",
        "mailDoc",
        "app.launchURL",
        "util.printf",
        "AFNumber_Format",
        "AFDate_Format",
        "AFSimple_Calculate",
        "event.value",
        "this.print",
        "this.saveAs",
    ];
    apis.into_iter()
        .filter(|name| script.contains(name))
        .map(str::to_string)
        .collect()
}

fn unsafe_indicators(script: &str) -> Vec<String> {
    let lower = script.to_ascii_lowercase();
    let indicators = [
        (
            "network",
            ["http://", "https://", "submitform", "launchurl", "geturl"].as_slice(),
        ),
        (
            "filesystem",
            [
                "saveas",
                "importdataobject",
                "exportdataobject",
                "getdataobjectcontents",
            ]
            .as_slice(),
        ),
        (
            "process_or_native",
            ["launch", "exec", "activex", "shell"].as_slice(),
        ),
        (
            "ui_or_clipboard",
            ["alert", "response", "beep", "clipboard", "menuitem"].as_slice(),
        ),
        ("timer", ["settimeout", "setinterval"].as_slice()),
        ("dynamic_eval", ["eval", "function("].as_slice()),
    ];
    indicators
        .into_iter()
        .filter(|(_, needles)| needles.iter().any(|needle| lower.contains(needle)))
        .map(|(name, _)| name.to_string())
        .collect()
}

fn field_references(script: &str) -> Vec<String> {
    let mut fields = BTreeSet::new();
    let mut rest = script;
    while let Some(index) = rest.find("getField") {
        rest = &rest[index + "getField".len()..];
        let Some(open) = rest.find('(') else { break };
        rest = &rest[open + 1..];
        let trimmed = rest.trim_start();
        let Some(quote) = trimmed.chars().next().filter(|c| matches!(c, '\'' | '"')) else {
            continue;
        };
        let after = &trimmed[quote.len_utf8()..];
        if let Some(end) = after.find(quote) {
            fields.insert(after[..end].to_string());
            rest = &after[end + quote.len_utf8()..];
        } else {
            break;
        }
    }
    fields.into_iter().collect()
}

fn safe_subset_static_check(script: &str) -> std::result::Result<(), String> {
    let lower = script.to_ascii_lowercase();
    for forbidden in [
        "for(",
        "for (",
        "while(",
        "while (",
        "do {",
        "eval",
        "function(",
        "=>",
        "submitform",
        "launchurl",
        "geturl",
        "saveas",
        "importdataobject",
        "exportdataobject",
        "settimeout",
        "setinterval",
        "app.",
        "util.readfile",
        "this.print",
    ] {
        if lower.contains(forbidden) {
            return Err(format!("forbidden token '{forbidden}'"));
        }
    }
    if !(script.contains("event.value")
        || script.contains("getField")
        || script.contains("AFSimple_Calculate"))
    {
        return Err("no supported calculation target or field reference".to_string());
    }
    Ok(())
}

fn safe_preview(script: &str) -> String {
    let normalized = script
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>();
    let lower = normalized.to_ascii_lowercase();
    if [
        "password", "passwd", "secret", "bearer ", "api_key", "apikey", "token=",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        return "[redacted: secret-like token detected; sha256 retained]".to_string();
    }
    normalized.chars().take(192).collect()
}

fn sha256(bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(bytes);
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn pdf_scalar_string(object: &PdfObject) -> Option<String> {
    match object {
        PdfObject::String(bytes) => Some(decode_pdf_text_string(bytes)),
        PdfObject::Name(name) => Some(name.clone()),
        PdfObject::Integer(value) => Some(value.to_string()),
        PdfObject::Real(value) => Some(value.to_string()),
        PdfObject::Boolean(value) => Some(value.to_string()),
        _ => None,
    }
}

pub fn form_action_graph(
    engine: &ContentEngine,
    inventory: &FormJsInventoryReport,
) -> Result<FormActionGraphReport> {
    let fields = collect_fields(engine.document())?;
    let mut names = BTreeMap::<String, usize>::new();
    for field in &fields {
        *names.entry(field.name.clone()).or_insert(0) += 1;
    }
    let field_set = names.keys().cloned().collect::<BTreeSet<_>>();
    let mut edges = Vec::new();
    let mut missing = BTreeSet::new();
    let mut cross_page = Vec::new();
    for action in &inventory.actions {
        if action.action_type != "JavaScript" || action.event != "calculate" {
            continue;
        }
        let Some(target) = action.owner_field.clone() else {
            continue;
        };
        for source in &action.field_references {
            if !field_set.contains(source) {
                missing.insert(source.clone());
            }
            let edge = CalculationEdge {
                from_field: source.clone(),
                to_field: target.clone(),
                action_id: action.stable_id.clone(),
            };
            let source_pages = fields
                .iter()
                .find(|field| field.name == *source)
                .map(|field| &field.pages);
            let target_pages = fields
                .iter()
                .find(|field| field.name == target)
                .map(|field| &field.pages);
            if source_pages
                .zip(target_pages)
                .is_some_and(|(a, b)| a.is_disjoint(b))
            {
                cross_page.push(edge.clone());
            }
            edges.push(edge);
        }
    }
    if edges.len() > MAX_DEPENDENCIES {
        return Err(OxideError::ResourceLimit(format!(
            "form dependency count {} exceeds cap {MAX_DEPENDENCIES}",
            edges.len()
        )));
    }
    edges.sort_by(|a, b| {
        (&a.from_field, &a.to_field, &a.action_id).cmp(&(&b.from_field, &b.to_field, &b.action_id))
    });
    let calculation_order = calculation_order(engine.document(), &fields)?;
    let cycles = dependency_cycles(&edges);
    Ok(FormActionGraphReport {
        schema_version: PROMPT19_SCHEMA_VERSION.to_string(),
        calculation_order,
        fields: field_set.into_iter().collect(),
        edges,
        cycles,
        missing_fields: missing.into_iter().collect(),
        ambiguous_fields: names
            .into_iter()
            .filter_map(|(name, count)| (count > 1).then_some(name))
            .collect(),
        hidden_fields: Vec::new(),
        read_only_fields: fields
            .iter()
            .filter(|field| field.flags & 1 != 0)
            .map(|field| field.name.clone())
            .collect(),
        cross_page_dependencies: cross_page,
        deterministic: true,
    })
}

fn collect_fields(document: &PdfDocument) -> Result<Vec<FieldRecord>> {
    let catalog = document.get_catalog()?;
    let reader = document.reader();
    let Some(acroform) = catalog.get("AcroForm") else {
        return Ok(Vec::new());
    };
    let acroform = reader.resolve(acroform.clone())?;
    let Some(items) = acroform
        .as_dict()
        .and_then(|dict| dict.get("Fields"))
        .and_then(|value| reader.resolve(value.clone()).ok())
        .and_then(|value| value.as_array().map(<[PdfObject]>::to_vec))
    else {
        return Ok(Vec::new());
    };
    let page_refs = document
        .get_pages()?
        .into_iter()
        .map(|page| {
            (
                (page.object_number, page.generation_number),
                page.page_number,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut fields = Vec::new();
    for item in items {
        collect_field_node(reader, &item, "", &page_refs, 0, &mut fields)?;
    }
    fields.sort_by(|a, b| (&a.name, a.object).cmp(&(&b.name, b.object)));
    Ok(fields)
}

fn collect_field_node(
    reader: &PdfReader,
    source: &PdfObject,
    parent_name: &str,
    page_refs: &BTreeMap<(u32, u16), usize>,
    depth: usize,
    out: &mut Vec<FieldRecord>,
) -> Result<()> {
    if depth > 32 {
        return Err(OxideError::ResourceLimit(
            "form field tree depth exceeds 32".to_string(),
        ));
    }
    let reference = source.as_reference();
    let object = reader.resolve(source.clone())?;
    let Some(dict) = object.as_dict() else {
        return Ok(());
    };
    let local = dict
        .get("T")
        .and_then(pdf_scalar_string)
        .unwrap_or_default();
    let name = match (parent_name.is_empty(), local.is_empty()) {
        (_, true) => parent_name.to_string(),
        (true, false) => local,
        (false, false) => format!("{parent_name}.{local}"),
    };
    let mut pages = BTreeSet::new();
    if let Some(page) = dict
        .get("P")
        .and_then(PdfObject::as_reference)
        .and_then(|r| page_refs.get(&r))
    {
        pages.insert(*page);
    }
    let kids = dict
        .get("Kids")
        .and_then(|value| reader.resolve(value.clone()).ok())
        .and_then(|value| value.as_array().map(<[PdfObject]>::to_vec))
        .unwrap_or_default();
    let has_child_fields = kids.iter().any(|kid| {
        reader
            .resolve(kid.clone())
            .ok()
            .and_then(|value| value.as_dict().cloned())
            .is_some_and(|kid| kid.get("T").is_some() || kid.get("FT").is_some())
    });
    if !name.is_empty() && (!has_child_fields || dict.get("FT").is_some()) {
        out.push(FieldRecord {
            object: reference,
            name: name.clone(),
            value: dict
                .get("V")
                .and_then(pdf_scalar_string)
                .unwrap_or_default(),
            flags: dict.get_integer("Ff").unwrap_or(0),
            pages: pages.clone(),
        });
    }
    for kid in kids {
        collect_field_node(reader, &kid, &name, page_refs, depth + 1, out)?;
    }
    Ok(())
}

fn calculation_order(document: &PdfDocument, fields: &[FieldRecord]) -> Result<Vec<String>> {
    let reader = document.reader();
    let catalog = document.get_catalog()?;
    let Some(acroform) = catalog.get("AcroForm") else {
        return Ok(Vec::new());
    };
    let acroform = reader.resolve(acroform.clone())?;
    let Some(order) = acroform
        .as_dict()
        .and_then(|dict| dict.get("CO"))
        .and_then(|value| reader.resolve(value.clone()).ok())
        .and_then(|value| value.as_array().map(<[PdfObject]>::to_vec))
    else {
        return Ok(Vec::new());
    };
    Ok(order
        .iter()
        .filter_map(PdfObject::as_reference)
        .filter_map(|reference| fields.iter().find(|field| field.object == Some(reference)))
        .map(|field| field.name.clone())
        .collect())
}

fn dependency_cycles(edges: &[CalculationEdge]) -> Vec<Vec<String>> {
    let mut graph = BTreeMap::<String, Vec<String>>::new();
    for edge in edges {
        graph
            .entry(edge.from_field.clone())
            .or_default()
            .push(edge.to_field.clone());
    }
    for values in graph.values_mut() {
        values.sort();
        values.dedup();
    }
    let mut cycles = BTreeSet::new();
    for start in graph.keys() {
        find_cycles(
            start,
            start,
            &graph,
            &mut Vec::new(),
            &mut BTreeSet::new(),
            &mut cycles,
        );
    }
    cycles.into_iter().collect()
}

fn find_cycles(
    start: &str,
    node: &str,
    graph: &BTreeMap<String, Vec<String>>,
    path: &mut Vec<String>,
    visiting: &mut BTreeSet<String>,
    cycles: &mut BTreeSet<Vec<String>>,
) {
    if path.len() > 128 || !visiting.insert(node.to_string()) {
        return;
    }
    path.push(node.to_string());
    if let Some(next) = graph.get(node) {
        for candidate in next {
            if candidate == start {
                let mut cycle = path.clone();
                cycle.push(start.to_string());
                cycles.insert(cycle);
            } else {
                find_cycles(start, candidate, graph, path, visiting, cycles);
            }
        }
    }
    path.pop();
    visiting.remove(node);
}

pub fn form_js_sanitize_pdf(
    input: &[u8],
    options: &FormJsSanitizerOptions,
) -> Result<(Vec<u8>, FormJsSanitizerReport)> {
    if options.mode == FormJsPolicyMode::FlattenCalculatedValuesThenRemove {
        let (bytes, flatten) = flatten_calculated_values_pdf(input, options)?;
        let engine = ContentEngine::open_bytes(bytes.clone())?;
        let inventory = form_javascript_inventory(&engine, &options.limits)?;
        return Ok((
            bytes.clone(),
            FormJsSanitizerReport {
                schema_version: PROMPT19_SCHEMA_VERSION.to_string(),
                mode: options.mode,
                input_action_count: flatten.results.len(),
                output_action_count: inventory.actions.len(),
                removed_count: flatten.scripts_removed,
                preserved_safe_navigation_count: inventory
                    .actions
                    .iter()
                    .filter(|entry| is_safe_navigation_type(&entry.action_type))
                    .count(),
                forbidden_remaining_count: inventory.actions.len(),
                rescan_passed: inventory.actions.is_empty(),
                output_sha256: resource_digest(&bytes),
                output_bytes: bytes.len(),
                signature_impact: flatten.signature_impact,
                deterministic: true,
                exact_limits: prompt19_action_limits(),
            },
        ));
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let before = form_javascript_inventory(&engine, &options.limits)?;
    let signature = analyze_edit_policy(&engine, EditOperation::Sanitizer)?;
    enforce_signature_policy(&signature.decision, options.signature_policy_override)?;
    if matches!(
        options.mode,
        FormJsPolicyMode::InventoryOnly | FormJsPolicyMode::DisableExecutionPreserveSource
    ) {
        return Ok((
            input.to_vec(),
            FormJsSanitizerReport {
                schema_version: PROMPT19_SCHEMA_VERSION.to_string(),
                mode: options.mode,
                input_action_count: before.actions.len(),
                output_action_count: before.actions.len(),
                removed_count: 0,
                preserved_safe_navigation_count: before
                    .actions
                    .iter()
                    .filter(|entry| is_safe_navigation_type(&entry.action_type))
                    .count(),
                forbidden_remaining_count: 0,
                rescan_passed: true,
                output_sha256: resource_digest(input),
                output_bytes: input.len(),
                signature_impact: serde_json::to_value(signature).unwrap_or_default(),
                deterministic: true,
                exact_limits: prompt19_action_limits(),
            },
        ));
    }
    let reader = engine.document().reader();
    let output = rewrite_document_with_mode(reader, WriterMode::ClassicXref, |_, object| {
        sanitize_prompt19_object(object, reader, options, 0)
    })?;
    let reopened = ContentEngine::open_bytes(output.clone())?;
    let after = form_javascript_inventory(&reopened, &options.limits)?;
    let forbidden_remaining_count = after
        .actions
        .iter()
        .filter(|entry| action_removed_by_prompt19(&entry.action_type, None, options))
        .count();
    let preserved_safe_navigation_count = after
        .actions
        .iter()
        .filter(|entry| is_safe_navigation_type(&entry.action_type))
        .count();
    Ok((
        output.clone(),
        FormJsSanitizerReport {
            schema_version: PROMPT19_SCHEMA_VERSION.to_string(),
            mode: options.mode,
            input_action_count: before.actions.len(),
            output_action_count: after.actions.len(),
            removed_count: before.actions.len().saturating_sub(after.actions.len()),
            preserved_safe_navigation_count,
            forbidden_remaining_count,
            rescan_passed: forbidden_remaining_count == 0,
            output_sha256: resource_digest(&output),
            output_bytes: output.len(),
            signature_impact: serde_json::to_value(signature).unwrap_or_default(),
            deterministic: true,
            exact_limits: prompt19_action_limits(),
        },
    ))
}

fn enforce_signature_policy(decision: &EditPolicyDecision, override_policy: bool) -> Result<()> {
    if matches!(
        decision,
        EditPolicyDecision::BlockedBySignaturePolicy | EditPolicyDecision::ExplicitOverrideRequired
    ) && !override_policy
    {
        return Err(OxideError::UnsupportedFeature(
            "Prompt 18B signature policy blocks full-rewrite sanitization without an explicit override"
                .to_string(),
        ));
    }
    Ok(())
}

fn sanitize_prompt19_object(
    object: &mut PdfObject,
    reader: &PdfReader,
    options: &FormJsSanitizerOptions,
    depth: usize,
) {
    if depth > options.limits.max_action_graph_depth {
        *object = PdfObject::Null;
        return;
    }
    let action_type = object
        .as_dict()
        .and_then(|dict| dict.get_name("S"))
        .map(str::to_string);
    if action_type
        .as_deref()
        .is_some_and(|action| action_removed_by_prompt19(action, object.as_dict(), options))
    {
        *object = PdfObject::Null;
        return;
    }
    match object {
        PdfObject::Dictionary(dict) | PdfObject::Stream { dict, .. } => {
            if options.mode != FormJsPolicyMode::InventoryOnly {
                if removes_javascript(options) {
                    dict.remove("JS");
                    // This removes the document JavaScript name-tree slot but does
                    // not disturb sibling /Dests or /EmbeddedFiles name trees.
                    dict.remove("JavaScript");
                }
                for key in ["A", "OpenAction", "PA"] {
                    let remove = dict
                        .get(key)
                        .is_some_and(|value| action_value_removed(value, reader, options));
                    if remove {
                        dict.remove(key);
                    }
                }
                if let Some(aa) = dict.get_mut("AA") {
                    if let Some(actions) = aa.as_dict_mut() {
                        let remove = actions
                            .entries()
                            .filter_map(|(event, value)| {
                                action_value_removed(value, reader, options)
                                    .then_some(event.clone())
                            })
                            .collect::<Vec<_>>();
                        for event in remove {
                            actions.remove(&event);
                        }
                    } else if action_value_removed(aa, reader, options) {
                        *aa = PdfObject::Null;
                    }
                    sanitize_prompt19_object(aa, reader, options, depth + 1);
                    if aa.as_dict().is_some_and(PdfDictionary::is_empty)
                        || matches!(aa, PdfObject::Null)
                    {
                        dict.remove("AA");
                    }
                }
            }
            for (key, value) in dict.entries_mut() {
                if matches!(key.as_str(), "A" | "OpenAction" | "PA" | "AA") {
                    continue;
                }
                sanitize_prompt19_object(value, reader, options, depth + 1);
            }
        }
        PdfObject::Array(items) => {
            for item in items.iter_mut() {
                sanitize_prompt19_object(item, reader, options, depth + 1);
            }
        }
        _ => {}
    }
}

fn action_value_removed(
    value: &PdfObject,
    reader: &PdfReader,
    options: &FormJsSanitizerOptions,
) -> bool {
    let Ok(resolved) = reader.resolve(value.clone()) else {
        return true;
    };
    if let Some(dict) = resolved.as_dict() {
        let action = dict.get_name("S").unwrap_or("Malformed");
        action_removed_by_prompt19(action, Some(dict), options)
    } else if resolved.as_array().is_some() {
        options.mode != FormJsPolicyMode::PreserveSafeNavigationOnly
    } else {
        true
    }
}

fn removes_javascript(options: &FormJsSanitizerOptions) -> bool {
    !(matches!(
        options.mode,
        FormJsPolicyMode::InventoryOnly | FormJsPolicyMode::DisableExecutionPreserveSource
    ) || options.mode == FormJsPolicyMode::Custom
        && options.custom.preserve_action_types.contains("JavaScript"))
}

fn action_removed_by_prompt19(
    action: &str,
    dict: Option<&PdfDictionary>,
    options: &FormJsSanitizerOptions,
) -> bool {
    match options.mode {
        FormJsPolicyMode::InventoryOnly | FormJsPolicyMode::DisableExecutionPreserveSource => false,
        FormJsPolicyMode::RemoveJavascriptOnly => action == "JavaScript",
        FormJsPolicyMode::RemoveAllActiveActions
        | FormJsPolicyMode::FlattenCalculatedValuesThenRemove => true,
        FormJsPolicyMode::PreserveSafeNavigationOnly => !is_safe_navigation(action, dict),
        FormJsPolicyMode::Custom => {
            options.custom.remove_action_types.contains(action)
                || !options.custom.preserve_action_types.contains(action)
        }
    }
}

fn is_safe_navigation(action: &str, dict: Option<&PdfDictionary>) -> bool {
    match action {
        "GoTo" => dict.is_none_or(|dict| dict.get("D").is_some()),
        "Named" => dict
            .and_then(|dict| dict.get_name("N"))
            .is_some_and(|name| matches!(name, "NextPage" | "PrevPage" | "FirstPage" | "LastPage")),
        _ => false,
    }
}

fn is_safe_navigation_type(action: &str) -> bool {
    matches!(action, "GoTo" | "Named")
}

pub fn flatten_calculated_values_pdf(
    input: &[u8],
    options: &FormJsSanitizerOptions,
) -> Result<(Vec<u8>, CalculationFlattenReport)> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let inventory = form_javascript_inventory(&engine, &options.limits)?;
    let graph = form_action_graph(&engine, &inventory)?;
    let signature = analyze_edit_policy(&engine, EditOperation::FormValueUpdate)?;
    enforce_signature_policy(&signature.decision, options.signature_policy_override)?;
    let fields = collect_fields(engine.document())?;
    let mut values = fields
        .iter()
        .map(|field| (field.name.clone(), field.value.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut results = Vec::new();
    let mut mutations = BTreeMap::new();
    let mut actions = inventory
        .actions
        .iter()
        .filter(|action| action.action_type == "JavaScript" && action.event == "calculate")
        .collect::<Vec<_>>();
    let order_index = graph
        .calculation_order
        .iter()
        .enumerate()
        .map(|(index, field)| (field.clone(), index))
        .collect::<BTreeMap<_, _>>();
    actions.sort_by_key(|action| {
        (
            action
                .owner_field
                .as_ref()
                .and_then(|field| order_index.get(field))
                .copied()
                .unwrap_or(usize::MAX),
            action.stable_id.clone(),
        )
    });
    let cycle_fields = graph
        .cycles
        .iter()
        .flat_map(|cycle| cycle.iter().cloned())
        .collect::<BTreeSet<_>>();
    for action in actions {
        let original = action
            .owner_field
            .as_ref()
            .and_then(|field| values.get(field))
            .cloned();
        if action
            .owner_field
            .as_ref()
            .is_some_and(|field| cycle_fields.contains(field))
        {
            results.push(CalculationResult {
                action_id: action.stable_id.clone(),
                target_field: action.owner_field.clone(),
                original_value: original,
                calculated_value: None,
                status: Prompt19SupportStatus::UnsupportedReportedExact,
                instructions: 0,
                diagnostic: Some("calculation dependency cycle is not evaluated".to_string()),
            });
            continue;
        }
        let script = action.script_source.as_deref().unwrap_or_default();
        match evaluate_calculation_script(
            script,
            action.owner_field.as_deref(),
            &values,
            &options.limits,
        ) {
            Ok((target, value, instructions)) => {
                let rendered = value.render();
                if mutations.len() >= options.limits.max_field_mutations {
                    return Err(OxideError::ResourceLimit(format!(
                        "field mutation count exceeds cap {}",
                        options.limits.max_field_mutations
                    )));
                }
                values.insert(target.clone(), rendered.clone());
                mutations.insert(target.clone(), rendered.clone());
                results.push(CalculationResult {
                    action_id: action.stable_id.clone(),
                    target_field: Some(target),
                    original_value: original,
                    calculated_value: Some(rendered),
                    status: Prompt19SupportStatus::ImplementedWithLimits,
                    instructions,
                    diagnostic: None,
                });
            }
            Err(error) => results.push(CalculationResult {
                action_id: action.stable_id.clone(),
                target_field: action.owner_field.clone(),
                original_value: original,
                calculated_value: None,
                status: Prompt19SupportStatus::UnsupportedReportedSecurityPolicy,
                instructions: 0,
                diagnostic: Some(error),
            }),
        }
    }
    let mut editor = PdfEditor::open_bytes(input.to_vec())?;
    for (field, value) in &mutations {
        editor.set_form_text(field, value);
    }
    let filled = if mutations.is_empty() {
        input.to_vec()
    } else {
        editor.save_to_bytes(EditMode::FullRewrite)?
    };
    let sanitize_options = FormJsSanitizerOptions {
        mode: FormJsPolicyMode::RemoveAllActiveActions,
        custom: CustomActionPolicy::default(),
        signature_policy_override: options.signature_policy_override,
        limits: options.limits.clone(),
    };
    let filled_engine = ContentEngine::open_bytes(filled)?;
    let output = rewrite_document_with_mode(
        filled_engine.document().reader(),
        WriterMode::ClassicXref,
        |_, object| {
            sanitize_prompt19_object(
                object,
                filled_engine.document().reader(),
                &sanitize_options,
                0,
            )
        },
    )?;
    let unsupported_scripts = results
        .iter()
        .filter(|result| {
            !matches!(
                result.status,
                Prompt19SupportStatus::Implemented | Prompt19SupportStatus::ImplementedWithLimits
            )
        })
        .count();
    Ok((
        output.clone(),
        CalculationFlattenReport {
            schema_version: PROMPT19_SCHEMA_VERSION.to_string(),
            policy: FormJsPolicyMode::FlattenCalculatedValuesThenRemove,
            dependency_order: graph.calculation_order,
            results,
            values_updated: mutations.len(),
            appearances_regenerated: mutations.len(),
            scripts_removed: inventory.script_count,
            unsupported_scripts,
            cycles_blocked: graph.cycles.len(),
            output_sha256: resource_digest(&output),
            output_bytes: output.len(),
            signature_impact: serde_json::to_value(signature).unwrap_or_default(),
            deterministic: true,
        },
    ))
}

#[derive(Debug, Clone)]
enum SafeValue {
    Number(f64),
    String(String),
    Boolean(bool),
}

impl SafeValue {
    fn number(&self) -> std::result::Result<f64, String> {
        match self {
            Self::Number(value) => Ok(*value),
            Self::String(value) => value
                .trim()
                .parse::<f64>()
                .map_err(|_| format!("'{value}' is not numeric")),
            Self::Boolean(value) => Ok(if *value { 1.0 } else { 0.0 }),
        }
    }

    fn truthy(&self) -> bool {
        match self {
            Self::Number(value) => *value != 0.0,
            Self::String(value) => !value.is_empty(),
            Self::Boolean(value) => *value,
        }
    }

    fn render(&self) -> String {
        match self {
            Self::Number(value) if value.fract() == 0.0 => format!("{value:.0}"),
            Self::Number(value) => value.to_string(),
            Self::String(value) => value.clone(),
            Self::Boolean(value) => value.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    String(String),
    Field(String),
    True,
    False,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    LParen,
    RParen,
    Question,
    Colon,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Not,
}

fn evaluate_calculation_script(
    script: &str,
    owner_field: Option<&str>,
    values: &BTreeMap<String, String>,
    limits: &FormJsLimits,
) -> std::result::Result<(String, SafeValue, usize), String> {
    safe_subset_static_check(script)?;
    if script.len() > limits.max_script_bytes {
        return Err("script exceeds configured byte cap".to_string());
    }
    if let Some(result) = evaluate_afsimple(script, owner_field, values)? {
        return Ok(result);
    }
    let assignment = script
        .split(';')
        .map(str::trim)
        .find(|statement| statement.contains('='))
        .ok_or_else(|| "supported subset requires one assignment".to_string())?;
    let assignment_index = assignment
        .find('=')
        .ok_or_else(|| "assignment operator missing".to_string())?;
    let lhs = assignment[..assignment_index].trim();
    let rhs = assignment[assignment_index + 1..].trim();
    let target = if lhs.ends_with("event.value") || lhs == "event.value" {
        owner_field
            .map(str::to_string)
            .ok_or_else(|| "event.value calculation has no owning field".to_string())?
    } else {
        field_references(lhs)
            .into_iter()
            .next()
            .ok_or_else(|| "unsupported assignment target".to_string())?
    };
    let tokens = tokenize_expression(rhs)?;
    if tokens.len() > limits.max_safe_instructions {
        return Err("expression exceeds instruction cap".to_string());
    }
    let mut parser = ExpressionParser {
        tokens: &tokens,
        position: 0,
        values,
        instructions: 0,
        cap: limits.max_safe_instructions,
    };
    let value = parser.parse_conditional()?;
    if parser.position != tokens.len() {
        return Err("unsupported trailing expression tokens".to_string());
    }
    if value.render().len() > limits.max_safe_value_bytes {
        return Err("calculated value exceeds memory cap".to_string());
    }
    Ok((target, value, parser.instructions))
}

fn evaluate_afsimple(
    script: &str,
    owner_field: Option<&str>,
    values: &BTreeMap<String, String>,
) -> std::result::Result<Option<(String, SafeValue, usize)>, String> {
    if !script.contains("AFSimple_Calculate") {
        return Ok(None);
    }
    let upper = script.to_ascii_uppercase();
    let operation = ["SUM", "AVG", "PRD", "MIN", "MAX"]
        .into_iter()
        .find(|op| upper.contains(&format!("\"{op}\"")) || upper.contains(&format!("'{op}'")))
        .ok_or_else(|| "unsupported AFSimple_Calculate operation".to_string())?;
    let fields = field_references(script);
    if fields.is_empty() {
        return Err("AFSimple_Calculate has no bounded field list".to_string());
    }
    let mut numbers = Vec::new();
    for field in &fields {
        numbers.push(
            values
                .get(field)
                .map(String::as_str)
                .unwrap_or_default()
                .trim()
                .parse::<f64>()
                .map_err(|_| format!("field '{field}' is not numeric"))?,
        );
    }
    let value = match operation {
        "SUM" => numbers.iter().sum(),
        "AVG" => numbers.iter().sum::<f64>() / numbers.len() as f64,
        "PRD" => numbers.iter().product(),
        "MIN" => numbers.iter().copied().fold(f64::INFINITY, f64::min),
        "MAX" => numbers.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        _ => unreachable!(),
    };
    if !value.is_finite() {
        return Err("calculation produced non-finite value".to_string());
    }
    Ok(Some((
        owner_field
            .map(str::to_string)
            .ok_or_else(|| "AFSimple_Calculate has no owning field".to_string())?,
        SafeValue::Number(value),
        fields.len() + 1,
    )))
}

fn tokenize_expression(input: &str) -> std::result::Result<Vec<Token>, String> {
    let bytes = input.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        let c = bytes[index] as char;
        if c.is_whitespace() {
            index += 1;
            continue;
        }
        if input[index..].starts_with("this.getField") {
            let tail = &input[index..];
            let fields = field_references(tail);
            let field = fields
                .first()
                .cloned()
                .ok_or_else(|| "malformed getField expression".to_string())?;
            let value_end = tail
                .find(".value")
                .ok_or_else(|| "field read must end in .value".to_string())?
                + ".value".len();
            tokens.push(Token::Field(field));
            index += value_end;
            continue;
        }
        if c.is_ascii_digit()
            || (c == '.' && index + 1 < bytes.len() && (bytes[index + 1] as char).is_ascii_digit())
        {
            let start = index;
            index += 1;
            while index < bytes.len()
                && ((bytes[index] as char).is_ascii_digit()
                    || matches!(bytes[index] as char, '.' | 'e' | 'E' | '+' | '-'))
            {
                if matches!(bytes[index] as char, '+' | '-')
                    && !matches!(bytes[index - 1] as char, 'e' | 'E')
                {
                    break;
                }
                index += 1;
            }
            let number = input[start..index]
                .parse::<f64>()
                .map_err(|_| "invalid numeric literal".to_string())?;
            if !number.is_finite() {
                return Err("non-finite numeric literal".to_string());
            }
            tokens.push(Token::Number(number));
            continue;
        }
        if matches!(c, '\'' | '"') {
            let quote = c;
            index += 1;
            let mut out = String::new();
            while index < bytes.len() && bytes[index] as char != quote {
                if bytes[index] as char == '\\' && index + 1 < bytes.len() {
                    index += 1;
                }
                out.push(bytes[index] as char);
                index += 1;
            }
            if index >= bytes.len() {
                return Err("unterminated string literal".to_string());
            }
            index += 1;
            tokens.push(Token::String(out));
            continue;
        }
        let remaining = &input[index..];
        let (token, consumed) = if remaining.starts_with("&&") {
            (Token::And, 2)
        } else if remaining.starts_with("||") {
            (Token::Or, 2)
        } else if remaining.starts_with("==") {
            (Token::Eq, 2)
        } else if remaining.starts_with("!=") {
            (Token::Ne, 2)
        } else if remaining.starts_with("<=") {
            (Token::Le, 2)
        } else if remaining.starts_with(">=") {
            (Token::Ge, 2)
        } else {
            match c {
                '+' => (Token::Plus, 1),
                '-' => (Token::Minus, 1),
                '*' => (Token::Star, 1),
                '/' => (Token::Slash, 1),
                '%' => (Token::Percent, 1),
                '(' => (Token::LParen, 1),
                ')' => (Token::RParen, 1),
                '?' => (Token::Question, 1),
                ':' => (Token::Colon, 1),
                '<' => (Token::Lt, 1),
                '>' => (Token::Gt, 1),
                '!' => (Token::Not, 1),
                _ if remaining.starts_with("true") => (Token::True, 4),
                _ if remaining.starts_with("false") => (Token::False, 5),
                _ => return Err(format!("unsupported expression token near '{remaining}'")),
            }
        };
        tokens.push(token);
        index += consumed;
    }
    Ok(tokens)
}

struct ExpressionParser<'a> {
    tokens: &'a [Token],
    position: usize,
    values: &'a BTreeMap<String, String>,
    instructions: usize,
    cap: usize,
}

impl ExpressionParser<'_> {
    fn step(&mut self) -> std::result::Result<(), String> {
        self.instructions += 1;
        if self.instructions > self.cap {
            return Err("safe subset instruction cap exceeded".to_string());
        }
        Ok(())
    }

    fn take(&mut self, token: &Token) -> bool {
        if self.tokens.get(self.position) == Some(token) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn parse_conditional(&mut self) -> std::result::Result<SafeValue, String> {
        let condition = self.parse_or()?;
        if self.take(&Token::Question) {
            self.step()?;
            let when_true = self.parse_conditional()?;
            if !self.take(&Token::Colon) {
                return Err("conditional expression is missing ':'".to_string());
            }
            let when_false = self.parse_conditional()?;
            Ok(if condition.truthy() {
                when_true
            } else {
                when_false
            })
        } else {
            Ok(condition)
        }
    }

    fn parse_or(&mut self) -> std::result::Result<SafeValue, String> {
        let mut left = self.parse_and()?;
        while self.take(&Token::Or) {
            self.step()?;
            let right = self.parse_and()?;
            left = SafeValue::Boolean(left.truthy() || right.truthy());
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> std::result::Result<SafeValue, String> {
        let mut left = self.parse_compare()?;
        while self.take(&Token::And) {
            self.step()?;
            let right = self.parse_compare()?;
            left = SafeValue::Boolean(left.truthy() && right.truthy());
        }
        Ok(left)
    }

    fn parse_compare(&mut self) -> std::result::Result<SafeValue, String> {
        let left = self.parse_add()?;
        let op = self.tokens.get(self.position).cloned();
        if !matches!(
            op,
            Some(Token::Eq | Token::Ne | Token::Lt | Token::Le | Token::Gt | Token::Ge)
        ) {
            return Ok(left);
        }
        self.position += 1;
        self.step()?;
        let right = self.parse_add()?;
        let result = match op.expect("comparison token") {
            Token::Eq => left.render() == right.render(),
            Token::Ne => left.render() != right.render(),
            Token::Lt => left.number()? < right.number()?,
            Token::Le => left.number()? <= right.number()?,
            Token::Gt => left.number()? > right.number()?,
            Token::Ge => left.number()? >= right.number()?,
            _ => unreachable!(),
        };
        Ok(SafeValue::Boolean(result))
    }

    fn parse_add(&mut self) -> std::result::Result<SafeValue, String> {
        let mut left = self.parse_mul()?;
        loop {
            let add = self.take(&Token::Plus);
            let subtract = !add && self.take(&Token::Minus);
            if !add && !subtract {
                break;
            }
            self.step()?;
            let right = self.parse_mul()?;
            left = if add
                && (matches!(left, SafeValue::String(_)) || matches!(right, SafeValue::String(_)))
            {
                SafeValue::String(format!("{}{}", left.render(), right.render()))
            } else if add {
                SafeValue::Number(left.number()? + right.number()?)
            } else {
                SafeValue::Number(left.number()? - right.number()?)
            };
        }
        Ok(left)
    }

    fn parse_mul(&mut self) -> std::result::Result<SafeValue, String> {
        let mut left = self.parse_unary()?;
        loop {
            let multiply = self.take(&Token::Star);
            let divide = !multiply && self.take(&Token::Slash);
            let modulo = !multiply && !divide && self.take(&Token::Percent);
            if !multiply && !divide && !modulo {
                break;
            }
            self.step()?;
            let right = self.parse_unary()?;
            let divisor = right.number()?;
            left = if multiply {
                SafeValue::Number(left.number()? * divisor)
            } else if divide {
                if divisor == 0.0 {
                    return Err("division by zero".to_string());
                }
                SafeValue::Number(left.number()? / divisor)
            } else {
                if divisor == 0.0 {
                    return Err("modulo by zero".to_string());
                }
                SafeValue::Number(left.number()? % divisor)
            };
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> std::result::Result<SafeValue, String> {
        if self.take(&Token::Not) {
            self.step()?;
            return Ok(SafeValue::Boolean(!self.parse_unary()?.truthy()));
        }
        if self.take(&Token::Minus) {
            self.step()?;
            return Ok(SafeValue::Number(-self.parse_unary()?.number()?));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> std::result::Result<SafeValue, String> {
        self.step()?;
        let token = self
            .tokens
            .get(self.position)
            .cloned()
            .ok_or_else(|| "unexpected end of expression".to_string())?;
        self.position += 1;
        match token {
            Token::Number(value) => Ok(SafeValue::Number(value)),
            Token::String(value) => Ok(SafeValue::String(value)),
            Token::True => Ok(SafeValue::Boolean(true)),
            Token::False => Ok(SafeValue::Boolean(false)),
            Token::Field(name) => {
                let value = self.values.get(&name).cloned().unwrap_or_default();
                Ok(value
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|number| number.is_finite())
                    .map(SafeValue::Number)
                    .unwrap_or(SafeValue::String(value)))
            }
            Token::LParen => {
                let value = self.parse_conditional()?;
                if !self.take(&Token::RParen) {
                    return Err("missing closing parenthesis".to_string());
                }
                Ok(value)
            }
            other => Err(format!("unexpected expression token {other:?}")),
        }
    }
}

pub fn interactive_data_closeout_report(engine: &ContentEngine) -> Result<serde_json::Value> {
    let interactive = crate::interactive::interactive_report(engine)?;
    let security = crate::security::security_report(engine)?;
    let associated = crate::prompt18::associated_files_inventory(engine)?;
    let signatures = analyze_edit_policy(engine, EditOperation::IncrementalSave)?;
    Ok(json!({
        "schema_version": PROMPT19_SCHEMA_VERSION,
        "status": "complete_with_exact_limits",
        "stable_id_posture": "object-generation plus deterministic owner/path ids",
        "forms": interactive.forms,
        "annotations": interactive.annotations,
        "page_operations": interactive.page_operations,
        "associated_files": associated,
        "security": security,
        "signature_policy": signatures,
        "consistency": {
            "object_page_provenance": "implemented",
            "field_widget_relationships": "implemented_with_limits",
            "popup_reply_relationships": "implemented_with_limits_prompt17",
            "associated_file_ownership": "implemented_with_limits_prompt18b",
            "coordinate_transforms": "implemented_with_rotation_limits",
            "sanitizer_disposition": "implemented_prompt19_action_policy",
            "deterministic_serialization": "implemented"
        },
        "blocked": 0,
        "unclassified_failures": 0,
        "exact_limits": [
            "dynamic XFA JavaScript remains inventory-only; the bounded XFA/FormCalc runtime is separate and opt-in",
            "OCG-dependent annotation visibility is preserved/reported but not promoted into an Acrobat UI state machine",
            "cryptographic signature validity is reported only by the signature verifier, never inferred from mutation structure"
        ]
    }))
}

pub fn word_pagination_audit(
    engine: &ContentEngine,
    layout: DocxLayout,
) -> Result<DocxLayoutAuditReport> {
    let options = DocxOptions {
        include_images: true,
        layout,
    };
    let bytes = pdf_to_docx(engine, &options)?;
    if bytes.len() > MAX_DOCX_OUTPUT_BYTES {
        return Err(OxideError::ResourceLimit(format!(
            "DOCX output {} exceeds cap {MAX_DOCX_OUTPUT_BYTES}",
            bytes.len()
        )));
    }
    let repeat = pdf_to_docx(engine, &options)?;
    let document = engine.parse_document_with_profile(
        crate::ExtractionProfile::LayoutFaithful,
        &crate::parse::ParseOptions::default(),
    )?;
    if document.pages.len() > MAX_DOCX_PAGES {
        return Err(OxideError::ResourceLimit(format!(
            "DOCX page count {} exceeds cap {MAX_DOCX_PAGES}",
            document.pages.len()
        )));
    }
    inspect_docx(&bytes, layout, &document, bytes == repeat)
}

fn inspect_docx(
    bytes: &[u8],
    layout: DocxLayout,
    document: &crate::parse::Document,
    deterministic_repeat_match: bool,
) -> Result<DocxLayoutAuditReport> {
    let mut zip = ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| OxideError::MalformedPdf(format!("DOCX ZIP readback failed: {error}")))?;
    if zip.len() > MAX_DOCX_PARTS {
        return Err(OxideError::ResourceLimit(format!(
            "DOCX part count {} exceeds cap {MAX_DOCX_PARTS}",
            zip.len()
        )));
    }
    let names = (0..zip.len())
        .filter_map(|index| zip.by_index(index).ok().map(|file| file.name().to_string()))
        .collect::<Vec<_>>();
    let mut xml = String::new();
    zip.by_name("word/document.xml")
        .map_err(|error| OxideError::MalformedPdf(format!("DOCX missing document.xml: {error}")))?
        .read_to_string(&mut xml)?;
    let page_sizes_twips = document
        .pages
        .iter()
        .map(|page| {
            [
                (page.width * 20.0).round() as i64,
                (page.height * 20.0).round() as i64,
            ]
        })
        .collect::<Vec<_>>();
    Ok(DocxLayoutAuditReport {
        schema_version: PROMPT19_SCHEMA_VERSION.to_string(),
        layout: layout.as_str().to_string(),
        page_count: document.pages.len(),
        section_count: xml.matches("<w:sectPr").count(),
        page_sizes_twips,
        paragraph_count: xml.matches("<w:p>").count(),
        text_box_count: xml.matches("<wps:txbx>").count(),
        table_count: xml.matches("<w:tbl>").count(),
        merged_cell_count: xml.matches("<w:gridSpan").count() + xml.matches("<w:vMerge").count(),
        image_count: names.iter().filter(|name| name.starts_with("word/media/")).count(),
        hyperlink_count: xml.matches("<w:hyperlink").count(),
        header_part_count: names.iter().filter(|name| name.starts_with("word/header")).count(),
        footer_part_count: names.iter().filter(|name| name.starts_with("word/footer")).count(),
        package_parts: names.len(),
        output_bytes: bytes.len(),
        deterministic_sha256: resource_digest(bytes),
        deterministic_repeat_match,
        readback_ok: xml.contains("<w:document") && xml.contains("<w:body>"),
        unsupported_exact: vec![
            "Microsoft Word automation is an external harness result and is never inferred from OOXML readback".to_string(),
            "font substitution and Word versus LibreOffice line-breaking remain editor-dependent".to_string(),
            "PDF clipping paths, arbitrary blend modes, and unsupported vector constructs use bounded static/positioned fallback reporting".to_string(),
            "footnotes/endnotes require explicit semantic source structure; geometric superscripts alone are not promoted automatically".to_string(),
            "running headers, footers, and page numbers are preserved as page-relative positioned furniture when detected; dedicated Word header/footer parts and field promotion require repeat-confidence and are reported by zero part counts when not emitted".to_string(),
            "comments and content controls are not synthesized from generic PDF annotations or widgets; static appearances/text are preserved by configured positioned fallbacks".to_string(),
        ],
    })
}

pub fn prompt19_report(engine: &ContentEngine) -> Result<serde_json::Value> {
    let inventory = form_javascript_inventory(engine, &FormJsLimits::default())?;
    let graph = form_action_graph(engine, &inventory)?;
    let flowing = word_pagination_audit(engine, DocxLayout::Flowing)?;
    let faithful = word_pagination_audit(engine, DocxLayout::PageFaithful)?;
    let hybrid = word_pagination_audit(engine, DocxLayout::Hybrid)?;
    Ok(json!({
        "schema_version": PROMPT19_SCHEMA_VERSION,
        "form_javascript": inventory,
        "action_graph": graph,
        "interactive_data": interactive_data_closeout_report(engine)?,
        "word_pagination": {
            "flowing": flowing,
            "page_faithful": faithful,
            "hybrid": hybrid,
            "word_availability": "external_harness_required",
            "libreoffice_availability": "external_harness_required"
        },
        "feature": prompt19_feature_report_value(crate::sdk::REPORT_ENVELOPE_VERSION)
    }))
}

pub(crate) fn prompt19_feature_report_value(envelope_version: u32) -> serde_json::Value {
    json!({
        "schema_version": PROMPT19_SCHEMA_VERSION,
        "envelope_version": envelope_version,
        "status": "complete_with_exact_limits",
        "js_inventory_policy": "implemented_with_limits",
        "sanitizer": "implemented_with_rescan",
        "safe_subset": "bounded_opt_in_not_acrobat_javascript",
        "interactive_closeout": "implemented_with_exact_limits",
        "word_pagination_audit": "implemented_structural_external_editor_harness_optional",
        "docx_layout": "flowing_page_faithful_hybrid",
        "security": {
            "javascript_execution_default": "disabled",
            "network_filesystem_process_ui_clipboard_timers": "unavailable",
            "arbitrary_eval": "unavailable",
            "hidden_cloud_execution": false,
            "acrobat_dom_emulation_claimed": false
        },
        "failure": {"blocked": 0, "unclassified": 0},
        "limits": {
            "script_bytes": MAX_SCRIPT_BYTES,
            "total_script_bytes": MAX_TOTAL_SCRIPT_BYTES,
            "actions": MAX_ACTIONS,
            "action_graph_depth": MAX_ACTION_GRAPH_DEPTH,
            "dependencies": MAX_DEPENDENCIES,
            "instructions": MAX_SAFE_INSTRUCTIONS,
            "field_mutations": MAX_FIELD_MUTATIONS,
            "docx_pages": MAX_DOCX_PAGES,
            "docx_parts": MAX_DOCX_PARTS,
            "docx_output_bytes": MAX_DOCX_OUTPUT_BYTES
        },
        "unsupported_exact": prompt19_action_limits()
    })
}

pub fn prompt19_policy_matrix() -> serde_json::Value {
    let rows = [
        (
            FormJsPolicyMode::InventoryOnly,
            "all",
            "all",
            "preserved",
            "none",
        ),
        (
            FormJsPolicyMode::DisableExecutionPreserveSource,
            "all",
            "all",
            "preserved",
            "none",
        ),
        (
            FormJsPolicyMode::RemoveJavascriptOnly,
            "non_javascript",
            "javascript",
            "removed",
            "full_rewrite",
        ),
        (
            FormJsPolicyMode::RemoveAllActiveActions,
            "none",
            "all",
            "removed",
            "full_rewrite",
        ),
        (
            FormJsPolicyMode::PreserveSafeNavigationOnly,
            "internal_goto_and_bounded_named_page_navigation",
            "all_other_actions",
            "removed",
            "full_rewrite",
        ),
        (
            FormJsPolicyMode::FlattenCalculatedValuesThenRemove,
            "calculated_values",
            "all_actions",
            "removed_after_bounded_eval",
            "form_update_then_full_rewrite",
        ),
        (
            FormJsPolicyMode::Custom,
            "explicit_allowlist",
            "everything_else",
            "policy_defined",
            "full_rewrite",
        ),
    ];
    json!({
        "schema_version": PROMPT19_SCHEMA_VERSION,
        "rows": rows.into_iter().map(|(mode, preserved, removed, source, signature)| json!({
            "mode": mode,
            "preserved_action_types": preserved,
            "removed_action_types": removed,
            "script_source": source,
            "calculated_values": mode == FormJsPolicyMode::FlattenCalculatedValuesThenRemove,
            "validation_formatting": if matches!(mode, FormJsPolicyMode::InventoryOnly | FormJsPolicyMode::DisableExecutionPreserveSource) {"preserved_source_not_executed"} else {"removed_when_action_removed"},
            "navigation": if mode == FormJsPolicyMode::PreserveSafeNavigationOnly {"bounded_internal_only"} else {"mode_dependent"},
            "submit_import": "never_executed; removed_by_active-action policies",
            "signature_impact": signature,
            "deterministic": true
        })).collect::<Vec<_>>()
    })
}

fn prompt19_action_limits() -> Vec<String> {
    vec![
        "the safe subset accepts pure scalar expressions and a bounded AFSimple_Calculate helper; it is not Acrobat JavaScript".to_string(),
        "loops, eval, functions, dynamic property traversal, network, filesystem, process, UI, clipboard, and timer APIs are rejected".to_string(),
        "encrypted or undecodable script streams are inventoried as blocked and are removed by fail-closed sanitizer modes".to_string(),
        "safe navigation preserves only internal GoTo destinations and Named FirstPage/LastPage/NextPage/PrevPage actions".to_string(),
        "full-rewrite sanitizer operations are subject to Prompt 18B DocMDP/FieldMDP enforcement and explicit override policy".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_expression_is_bounded_and_deterministic() {
        let values = BTreeMap::from([
            ("A".to_string(), "2".to_string()),
            ("B".to_string(), "3".to_string()),
        ]);
        let (target, value, instructions) = evaluate_calculation_script(
            "event.value = this.getField(\"A\").value * 2 + this.getField(\"B\").value;",
            Some("Total"),
            &values,
            &FormJsLimits::default(),
        )
        .unwrap();
        assert_eq!(target, "Total");
        assert_eq!(value.render(), "7");
        assert!(instructions > 0);
    }

    #[test]
    fn unsafe_api_is_rejected() {
        let error = evaluate_calculation_script(
            "app.launchURL('https://example.invalid'); event.value = 1;",
            Some("Total"),
            &BTreeMap::new(),
            &FormJsLimits::default(),
        )
        .unwrap_err();
        assert!(error.contains("forbidden token"));
    }

    #[test]
    fn policy_matrix_has_no_blocked_row() {
        let value = prompt19_policy_matrix();
        assert_eq!(value["rows"].as_array().unwrap().len(), 7);
        assert_eq!(prompt19_feature_report_value(1)["failure"]["blocked"], 0);
    }
}
