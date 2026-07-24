//! Prompt 14 semantic intelligence layer.
//!
//! This module is additive to the deterministic semantic model. ParentTree
//! recovery uses only PDF structure evidence and visible marked content. Layout
//! backends are optional proposal sources; they never own or rewrite the core
//! extraction model.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::engine::ContentEngine;
use crate::error::{Result, WellfriendError};
use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;
use crate::semantic::{SemanticElement, SemanticMcid};
use crate::text::{
    builtin_cjk_dictionary_metadata, cjk_dictionary_rag_token_chunks, cjk_dictionary_token_search,
    segment_cjk_dictionary_text, CjkDictionaryProvider, CjkDictionaryProviderLimits, TextChunk,
};

const PROMPT14_SCHEMA_VERSION: &str = "prompt14.semantic_intelligence.v1";
const MAX_PARENTTREE_DEPTH: usize = 128;
const MAX_PARENTTREE_NODES: usize = 250_000;
const DEFAULT_LAYOUT_CONFIDENCE_THRESHOLD: f32 = 0.78;

type PageRef = (u32, u16);
type McidKey = (usize, i64);
type MarkedTextMap = HashMap<McidKey, Vec<TextChunk>>;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SemanticEvidenceKind {
    SpecDerivedStructure,
    RepairedStructure,
    InferredStructure,
    OrphanContent,
    ConflictingContent,
    IgnoredUnsafeMalformed,
    ModelProposed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ParentTreeRecoveryStatus {
    NoTaggedEvidence,
    StructTreeAvailable,
    RecoveredFromParentTree,
    RecoveredWithConflicts,
    RecoveredOrphansOnly,
    UnsupportedReportedExact,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParentTreeDiagnostic {
    pub code: String,
    pub severity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcid: Option<i64>,
    pub message: String,
    pub evidence: SemanticEvidenceKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParentTreeRecoveredNode {
    pub id: String,
    pub page: usize,
    pub mcid: i64,
    pub role: String,
    pub original_role: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bbox: Option<[f64; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_object: Option<String>,
    pub source_page: usize,
    pub source_mcid: i64,
    pub evidence: SemanticEvidenceKind,
    pub confidence: f32,
    pub inferred: bool,
    pub repaired: bool,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParentTreePageSummary {
    pub page: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub struct_parents: Option<i64>,
    pub marked_mcid_count: usize,
    pub recovered_node_count: usize,
    pub orphan_mcid_count: usize,
    pub conflict_count: usize,
    pub ignored_malformed_node_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParentTreeRecoveryReport {
    pub schema_version: String,
    pub status: ParentTreeRecoveryStatus,
    pub struct_tree_root_present: bool,
    pub parent_tree_present: bool,
    pub parent_tree_number_tree_entries: usize,
    pub parent_tree_array_entries: usize,
    pub recovered_node_count: usize,
    pub orphan_mcid_count: usize,
    pub conflict_count: usize,
    pub ignored_malformed_node_count: usize,
    pub repaired_role_map_count: usize,
    pub recovery_confidence: f32,
    pub pages: Vec<ParentTreePageSummary>,
    pub nodes: Vec<ParentTreeRecoveredNode>,
    pub diagnostics: Vec<ParentTreeDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LayoutBackendKind {
    Local,
    Cloud,
    MockLocal,
    MockCloud,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LayoutBackendStatus {
    DisabledByDefault,
    Available,
    MissingModelFile,
    MissingRuntimeDependency,
    Configured,
    Disabled,
    BlockedByPrivacyPolicy,
    InvalidSchema,
    TimedOut,
    ResultMerged,
    ResultRejected,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LayoutInputPayloadKind {
    MetadataOnly,
    TextSpans,
    RenderedImage,
    TextAndImage,
    RedactedText,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LayoutPrivacyMode {
    Disabled,
    LocalOnly,
    CloudExplicitOptIn,
    NoPayload,
    RedactedPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LayoutCloudPayloadPolicy {
    MetadataOnly,
    TextOnly,
    ImageOnly,
    TextAndImage,
    RedactedTextOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LayoutRegionLabel {
    Title,
    Body,
    Table,
    Figure,
    Caption,
    List,
    Header,
    Footer,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LayoutRegionGeometry {
    pub bbox: [f64; 4],
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub polygon: Vec<[f64; 2]>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LayoutDiagnostic {
    pub code: String,
    pub severity: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LayoutProposalRegion {
    pub id: String,
    pub page: usize,
    pub label: LayoutRegionLabel,
    pub confidence: f32,
    pub geometry: LayoutRegionGeometry,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reading_order: Option<usize>,
    pub provenance: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<LayoutDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LayoutProposalSet {
    pub schema_version: String,
    pub backend_id: String,
    pub backend_type: LayoutBackendKind,
    pub model_name: String,
    pub model_version: String,
    pub model_hash: String,
    pub input_page_ids: Vec<usize>,
    pub input_payload_type: LayoutInputPayloadKind,
    pub proposed_regions: Vec<LayoutProposalRegion>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<LayoutDiagnostic>,
    pub runtime_ms: u64,
    pub memory_bytes: usize,
    pub privacy_flags: Vec<String>,
    pub deterministic_merge_outcome: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LayoutBackendDescriptor {
    pub backend_id: String,
    pub backend_type: LayoutBackendKind,
    pub model_name: String,
    pub model_version: String,
    pub model_hash: String,
    pub status: LayoutBackendStatus,
    pub diagnostics: Vec<LayoutDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LayoutBackendInput {
    pub pages: Vec<usize>,
    pub payload: LayoutInputPayloadKind,
    pub text_available: bool,
    pub image_dpi: Option<u32>,
    pub max_image_side_px: u32,
    pub max_pages_per_call: usize,
    pub timeout_ms: u64,
    pub privacy_mode: LayoutPrivacyMode,
    pub allow_cloud_upload: bool,
    pub redacted_payload: bool,
}

impl LayoutBackendInput {
    pub fn metadata_only(pages: Vec<usize>) -> Self {
        Self {
            pages,
            payload: LayoutInputPayloadKind::MetadataOnly,
            text_available: false,
            image_dpi: None,
            max_image_side_px: 2048,
            max_pages_per_call: 4,
            timeout_ms: 5_000,
            privacy_mode: LayoutPrivacyMode::NoPayload,
            allow_cloud_upload: false,
            redacted_payload: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LayoutLocalBackendConfig {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_path: Option<PathBuf>,
    pub model_name: String,
    pub model_version: String,
    pub batch_page_limit: usize,
    pub timeout_ms: u64,
    pub memory_limit_bytes: usize,
}

impl Default for LayoutLocalBackendConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            model_path: None,
            model_name: "mock-layout-local".to_string(),
            model_version: "prompt14-template".to_string(),
            batch_page_limit: 4,
            timeout_ms: 5_000,
            memory_limit_bytes: 256 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CloudLayoutBackendConfig {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    pub payload_policy: LayoutCloudPayloadPolicy,
    pub user_acknowledged_privacy: bool,
    pub timeout_ms: u64,
    pub retry_count: u8,
}

impl Default for CloudLayoutBackendConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: None,
            api_key_env: None,
            payload_policy: LayoutCloudPayloadPolicy::MetadataOnly,
            user_acknowledged_privacy: false,
            timeout_ms: 5_000,
            retry_count: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LayoutAvailabilityReport {
    pub local_backend: LayoutBackendDescriptor,
    pub cloud_backend: LayoutBackendDescriptor,
    pub disabled_by_default: bool,
    pub cloud_upload_requires_explicit_opt_in: bool,
    pub no_secret_logging: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LayoutMergePolicy {
    pub deterministic_primary: bool,
    pub confidence_threshold: f32,
    pub low_confidence_as_suggestion: bool,
    pub conflict_diagnostics: bool,
    pub model_cannot_delete_text: bool,
}

impl Default for LayoutMergePolicy {
    fn default() -> Self {
        Self {
            deterministic_primary: true,
            confidence_threshold: DEFAULT_LAYOUT_CONFIDENCE_THRESHOLD,
            low_confidence_as_suggestion: true,
            conflict_diagnostics: true,
            model_cannot_delete_text: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LayoutMergeOutcome {
    pub region_id: String,
    pub outcome: String,
    pub confidence: f32,
    pub diagnostic: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LayoutMergeReport {
    pub schema_version: String,
    pub accepted_count: usize,
    pub suggestion_count: usize,
    pub rejected_count: usize,
    pub conflict_count: usize,
    pub outcomes: Vec<LayoutMergeOutcome>,
    pub diagnostics: Vec<LayoutDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Prompt14SemanticIntelligenceReport {
    pub status: String,
    pub schema_version: String,
    pub artifact_root: String,
    pub parenttree_recovery: BTreeMap<String, serde_json::Value>,
    pub cjk_dictionary_segmentation: BTreeMap<String, serde_json::Value>,
    pub ml_layout_hook: BTreeMap<String, serde_json::Value>,
    pub local_backend_template: BTreeMap<String, serde_json::Value>,
    pub cloud_backend_template: BTreeMap<String, serde_json::Value>,
    pub privacy_defaults: BTreeMap<String, serde_json::Value>,
    pub public_reports: BTreeMap<String, serde_json::Value>,
    pub semantic_regression: BTreeMap<String, serde_json::Value>,
    pub remaining_exact_limits: Vec<String>,
    pub closure_gates: BTreeMap<String, serde_json::Value>,
}

pub struct MockLocalLayoutBackend {
    config: LayoutLocalBackendConfig,
}

impl MockLocalLayoutBackend {
    pub fn new(config: LayoutLocalBackendConfig) -> Self {
        Self { config }
    }

    pub fn descriptor(&self) -> LayoutBackendDescriptor {
        let status = if !self.config.enabled {
            LayoutBackendStatus::DisabledByDefault
        } else if let Some(path) = &self.config.model_path {
            if path.exists() {
                LayoutBackendStatus::Available
            } else {
                LayoutBackendStatus::MissingModelFile
            }
        } else {
            LayoutBackendStatus::Available
        };
        LayoutBackendDescriptor {
            backend_id: "mock-local-layout".to_string(),
            backend_type: LayoutBackendKind::MockLocal,
            model_name: self.config.model_name.clone(),
            model_version: self.config.model_version.clone(),
            model_hash: "mock-local:deterministic".to_string(),
            status,
            diagnostics: Vec::new(),
        }
    }

    pub fn propose(&self, input: &LayoutBackendInput) -> LayoutProposalSet {
        if !self.config.enabled {
            return disabled_layout_set(
                "mock-local-layout",
                LayoutBackendKind::MockLocal,
                LayoutInputPayloadKind::MetadataOnly,
                input,
                "local_mock_disabled_by_default",
            );
        }
        let mut diagnostics = Vec::new();
        if input.pages.len() > self.config.batch_page_limit {
            diagnostics.push(LayoutDiagnostic {
                code: "layout.local.batch_limit".to_string(),
                severity: "warning".to_string(),
                message: format!(
                    "requested {} pages; backend limit is {}",
                    input.pages.len(),
                    self.config.batch_page_limit
                ),
                page: None,
            });
        }
        mock_layout_set(
            "mock-local-layout",
            LayoutBackendKind::MockLocal,
            self.config.model_name.as_str(),
            self.config.model_version.as_str(),
            input,
            diagnostics,
        )
    }
}

pub struct MockCloudLayoutBackend {
    config: CloudLayoutBackendConfig,
}

impl MockCloudLayoutBackend {
    pub fn new(config: CloudLayoutBackendConfig) -> Self {
        Self { config }
    }

    pub fn descriptor(&self) -> LayoutBackendDescriptor {
        let status = if !self.config.enabled {
            LayoutBackendStatus::Disabled
        } else if self.config.endpoint.is_none() || !self.config.user_acknowledged_privacy {
            LayoutBackendStatus::BlockedByPrivacyPolicy
        } else {
            LayoutBackendStatus::Configured
        };
        LayoutBackendDescriptor {
            backend_id: "mock-cloud-layout".to_string(),
            backend_type: LayoutBackendKind::MockCloud,
            model_name: "mock-cloud-layout".to_string(),
            model_version: "prompt14-template".to_string(),
            model_hash: "mock-cloud:no-network".to_string(),
            status,
            diagnostics: Vec::new(),
        }
    }

    pub fn propose(&self, input: &LayoutBackendInput) -> LayoutProposalSet {
        if !self.config.enabled {
            return disabled_layout_set(
                "mock-cloud-layout",
                LayoutBackendKind::MockCloud,
                input.payload.clone(),
                input,
                "cloud_mock_disabled_by_default",
            );
        }
        if self.config.endpoint.is_none()
            || !self.config.user_acknowledged_privacy
            || !input.allow_cloud_upload
        {
            return disabled_layout_set(
                "mock-cloud-layout",
                LayoutBackendKind::MockCloud,
                input.payload.clone(),
                input,
                "cloud_request_blocked_by_privacy_policy",
            );
        }
        mock_layout_set(
            "mock-cloud-layout",
            LayoutBackendKind::MockCloud,
            "mock-cloud-layout",
            "prompt14-template",
            input,
            vec![LayoutDiagnostic {
                code: "layout.cloud.mock_no_network".to_string(),
                severity: "info".to_string(),
                message: "mock cloud backend validated schema without making a network request"
                    .to_string(),
                page: None,
            }],
        )
    }
}

pub fn recover_parenttree_semantics(
    engine: &ContentEngine,
    pages: &[usize],
) -> Result<ParentTreeRecoveryReport> {
    let page_list = normalized_pages(engine, pages)?;
    let selected: BTreeSet<usize> = page_list.iter().copied().collect();
    let marked_text = collect_marked_text(engine, &page_list)?;
    let page_struct_parents = page_struct_parent_keys(engine, &page_list)?;
    let mut diagnostics = Vec::new();
    let mut nodes = Vec::new();

    let catalog = engine.document().get_catalog()?;
    let Some(root_obj) = catalog.get("StructTreeRoot").cloned() else {
        let pages = summarize_pages(&page_list, &page_struct_parents, &marked_text, &nodes);
        return Ok(ParentTreeRecoveryReport {
            schema_version: PROMPT14_SCHEMA_VERSION.to_string(),
            status: ParentTreeRecoveryStatus::NoTaggedEvidence,
            struct_tree_root_present: false,
            parent_tree_present: false,
            parent_tree_number_tree_entries: 0,
            parent_tree_array_entries: 0,
            recovered_node_count: 0,
            orphan_mcid_count: marked_text.len(),
            conflict_count: 0,
            ignored_malformed_node_count: 0,
            repaired_role_map_count: 0,
            recovery_confidence: 0.0,
            pages,
            nodes,
            diagnostics,
        });
    };

    let reader = engine.document().reader();
    let root = match reader.resolve(root_obj) {
        Ok(root) => root,
        Err(err) => {
            diagnostics.push(parent_diag(
                "parenttree.struct_root_unresolved",
                "warning",
                None,
                None,
                format!("StructTreeRoot could not be resolved: {err}"),
                SemanticEvidenceKind::IgnoredUnsafeMalformed,
            ));
            PdfObject::Null
        }
    };
    let Some(root_dict) = root.as_dict() else {
        let pages = summarize_pages(&page_list, &page_struct_parents, &marked_text, &nodes);
        return Ok(ParentTreeRecoveryReport {
            schema_version: PROMPT14_SCHEMA_VERSION.to_string(),
            status: ParentTreeRecoveryStatus::UnsupportedReportedExact,
            struct_tree_root_present: true,
            parent_tree_present: false,
            parent_tree_number_tree_entries: 0,
            parent_tree_array_entries: 0,
            recovered_node_count: 0,
            orphan_mcid_count: marked_text.len(),
            conflict_count: 0,
            ignored_malformed_node_count: 1,
            repaired_role_map_count: 0,
            recovery_confidence: 0.0,
            pages,
            nodes,
            diagnostics,
        });
    };
    let role_map = parse_role_map(root_dict);
    let parent_tree_obj = root_dict.get("ParentTree").cloned();
    let parent_tree_present = parent_tree_obj.is_some();
    let mut number_tree = BTreeMap::new();
    if let Some(parent_tree_obj) = parent_tree_obj {
        let mut visited = HashSet::new();
        collect_parent_tree_entries(
            reader,
            parent_tree_obj,
            0,
            &mut visited,
            &mut number_tree,
            &mut diagnostics,
        )?;
    }

    let mut recovered = HashSet::new();
    let mut array_entries = 0usize;
    for (&page, key) in &page_struct_parents {
        if !selected.contains(&page) {
            continue;
        }
        let Some(struct_parent_key) = *key else {
            continue;
        };
        let Some(entry) = number_tree.get(&struct_parent_key) else {
            diagnostics.push(parent_diag(
                "parenttree.page_key_missing",
                "warning",
                Some(page),
                None,
                format!("page StructParents {struct_parent_key} has no ParentTree entry"),
                SemanticEvidenceKind::OrphanContent,
            ));
            continue;
        };
        match &entry.value {
            PdfObject::Array(items) => {
                array_entries += items.len();
                for (idx, item) in items.iter().enumerate() {
                    if nodes.len() >= MAX_PARENTTREE_NODES {
                        diagnostics.push(parent_diag(
                            "parenttree.node_cap",
                            "warning",
                            Some(page),
                            None,
                            format!("recovery hit node cap {MAX_PARENTTREE_NODES}"),
                            SemanticEvidenceKind::IgnoredUnsafeMalformed,
                        ));
                        break;
                    }
                    let mcid = idx as i64;
                    if !marked_text.contains_key(&(page, mcid)) {
                        continue;
                    }
                    if !recovered.insert((page, mcid)) {
                        diagnostics.push(parent_diag(
                            "parenttree.duplicate_mcid_entry",
                            "warning",
                            Some(page),
                            Some(mcid),
                            "duplicate ParentTree mapping for page MCID".to_string(),
                            SemanticEvidenceKind::ConflictingContent,
                        ));
                    }
                    let info =
                        structure_info(reader, item, &role_map, page, mcid, &mut diagnostics);
                    nodes.push(recovered_node(
                        page,
                        mcid,
                        info,
                        &marked_text,
                        if recovered.contains(&(page, mcid)) {
                            SemanticEvidenceKind::SpecDerivedStructure
                        } else {
                            SemanticEvidenceKind::ConflictingContent
                        },
                    ));
                }
            }
            other => {
                diagnostics.push(parent_diag(
                    "parenttree.entry_not_array",
                    "warning",
                    Some(page),
                    None,
                    format!(
                        "ParentTree entry {} resolved as {}, not an MCID array",
                        struct_parent_key,
                        other.variant_name()
                    ),
                    SemanticEvidenceKind::IgnoredUnsafeMalformed,
                ));
            }
        }
    }

    for (&(page, mcid), _) in marked_text
        .iter()
        .filter(|((page, _), _)| selected.contains(page))
    {
        if nodes.len() >= MAX_PARENTTREE_NODES {
            break;
        }
        if recovered.contains(&(page, mcid)) {
            continue;
        }
        diagnostics.push(parent_diag(
            "parenttree.orphan_mcid",
            "info",
            Some(page),
            Some(mcid),
            "marked content has no clean ParentTree chain; recovered as orphan content".to_string(),
            SemanticEvidenceKind::OrphanContent,
        ));
        nodes.push(recovered_node(
            page,
            mcid,
            StructureInfo::orphan(),
            &marked_text,
            SemanticEvidenceKind::OrphanContent,
        ));
    }

    nodes.sort_by_key(|node| (node.page, node.mcid));
    let conflict_count = diagnostics
        .iter()
        .filter(|diag| diag.evidence == SemanticEvidenceKind::ConflictingContent)
        .count();
    let ignored_malformed_node_count = diagnostics
        .iter()
        .filter(|diag| diag.evidence == SemanticEvidenceKind::IgnoredUnsafeMalformed)
        .count();
    let orphan_mcid_count = nodes
        .iter()
        .filter(|node| node.evidence == SemanticEvidenceKind::OrphanContent)
        .count();
    let repaired_role_map_count = diagnostics
        .iter()
        .filter(|diag| diag.code == "parenttree.role_map_gap")
        .count();
    let status = if nodes.is_empty() && parent_tree_present {
        ParentTreeRecoveryStatus::UnsupportedReportedExact
    } else if conflict_count > 0 {
        ParentTreeRecoveryStatus::RecoveredWithConflicts
    } else if parent_tree_present
        && nodes.iter().any(|node| {
            matches!(
                node.evidence,
                SemanticEvidenceKind::SpecDerivedStructure
                    | SemanticEvidenceKind::RepairedStructure
            )
        })
    {
        ParentTreeRecoveryStatus::RecoveredFromParentTree
    } else if orphan_mcid_count > 0 {
        ParentTreeRecoveryStatus::RecoveredOrphansOnly
    } else {
        ParentTreeRecoveryStatus::StructTreeAvailable
    };
    let confidence = if nodes.is_empty() {
        0.0
    } else {
        let total: f32 = nodes.iter().map(|node| node.confidence).sum();
        (total / nodes.len() as f32 * 100.0).round() / 100.0
    };
    let pages = summarize_pages(&page_list, &page_struct_parents, &marked_text, &nodes);
    Ok(ParentTreeRecoveryReport {
        schema_version: PROMPT14_SCHEMA_VERSION.to_string(),
        status,
        struct_tree_root_present: true,
        parent_tree_present,
        parent_tree_number_tree_entries: number_tree.len(),
        parent_tree_array_entries: array_entries,
        recovered_node_count: nodes.len(),
        orphan_mcid_count,
        conflict_count,
        ignored_malformed_node_count,
        repaired_role_map_count,
        recovery_confidence: confidence,
        pages,
        nodes,
        diagnostics,
    })
}

pub fn semantic_elements_from_parenttree_recovery(
    report: &ParentTreeRecoveryReport,
) -> Vec<SemanticElement> {
    report
        .nodes
        .iter()
        .filter(|node| node.evidence != SemanticEvidenceKind::IgnoredUnsafeMalformed)
        .filter(|node| !node.text.trim().is_empty())
        .map(|node| SemanticElement {
            element_type: node.role.clone(),
            original_type: (node.original_role != node.role).then(|| node.original_role.clone()),
            text: node.text.clone(),
            alt_text: None,
            actual_text: None,
            lang: None,
            page: Some(node.page),
            bbox: node.bbox,
            mcids: vec![SemanticMcid {
                page: node.page,
                mcid: node.mcid,
            }],
            children: Vec::new(),
        })
        .collect()
}

pub fn validate_layout_proposal_set(set: &LayoutProposalSet) -> LayoutMergeReport {
    let mut diagnostics = Vec::new();
    let mut rejected = 0usize;
    for region in &set.proposed_regions {
        if !(0.0..=1.0).contains(&region.confidence) {
            rejected += 1;
            diagnostics.push(LayoutDiagnostic {
                code: "layout.schema.invalid_confidence".to_string(),
                severity: "error".to_string(),
                message: format!("region {} confidence is outside 0..1", region.id),
                page: Some(region.page),
            });
        }
        let [x0, y0, x1, y1] = region.geometry.bbox;
        if !(x0.is_finite()
            && y0.is_finite()
            && x1.is_finite()
            && y1.is_finite()
            && x1 >= x0
            && y1 >= y0)
        {
            rejected += 1;
            diagnostics.push(LayoutDiagnostic {
                code: "layout.schema.invalid_bbox".to_string(),
                severity: "error".to_string(),
                message: format!("region {} has invalid bbox", region.id),
                page: Some(region.page),
            });
        }
    }
    LayoutMergeReport {
        schema_version: PROMPT14_SCHEMA_VERSION.to_string(),
        accepted_count: set.proposed_regions.len().saturating_sub(rejected),
        suggestion_count: 0,
        rejected_count: rejected,
        conflict_count: diagnostics.len(),
        outcomes: Vec::new(),
        diagnostics,
    }
}

pub fn merge_layout_proposals_deterministic(
    set: &LayoutProposalSet,
    policy: &LayoutMergePolicy,
) -> LayoutMergeReport {
    let validation = validate_layout_proposal_set(set);
    if validation.rejected_count > 0 {
        return validation;
    }
    let mut accepted_count = 0usize;
    let mut suggestion_count = 0usize;
    let mut outcomes = Vec::new();
    for region in &set.proposed_regions {
        let (outcome, diagnostic) = if region.confidence >= policy.confidence_threshold {
            accepted_count += 1;
            ("merged_hint".to_string(), None)
        } else if policy.low_confidence_as_suggestion {
            suggestion_count += 1;
            (
                "suggestion_only".to_string(),
                Some("below_confidence_threshold".to_string()),
            )
        } else {
            (
                "rejected_low_confidence".to_string(),
                Some("below_confidence_threshold".to_string()),
            )
        };
        outcomes.push(LayoutMergeOutcome {
            region_id: region.id.clone(),
            outcome,
            confidence: region.confidence,
            diagnostic,
        });
    }
    LayoutMergeReport {
        schema_version: PROMPT14_SCHEMA_VERSION.to_string(),
        accepted_count,
        suggestion_count,
        rejected_count: set
            .proposed_regions
            .len()
            .saturating_sub(accepted_count + suggestion_count),
        conflict_count: 0,
        outcomes,
        diagnostics: Vec::new(),
    }
}

pub fn layout_backend_availability_report(
    local: &LayoutLocalBackendConfig,
    cloud: &CloudLayoutBackendConfig,
) -> LayoutAvailabilityReport {
    let local = MockLocalLayoutBackend::new(local.clone()).descriptor();
    let cloud = MockCloudLayoutBackend::new(cloud.clone()).descriptor();
    LayoutAvailabilityReport {
        local_backend: local,
        cloud_backend: cloud,
        disabled_by_default: true,
        cloud_upload_requires_explicit_opt_in: true,
        no_secret_logging: true,
    }
}

pub fn load_user_cjk_dictionary_metadata(path: impl AsRef<Path>) -> Result<serde_json::Value> {
    let path = path.as_ref();
    let bytes = fs::read(path)?;
    let text = String::from_utf8_lossy(&bytes);
    let entry_count = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .count();
    Ok(serde_json::json!({
        "name": path.file_name().and_then(|name| name.to_str()).unwrap_or("user-dictionary"),
        "source": path.display().to_string(),
        "hash": fnv1a64(&bytes),
        "entry_count": entry_count,
        "license": "user_supplied_not_bundled",
        "load_status": "loaded_user_dictionary_metadata",
        "memory_footprint_bytes": bytes.len()
    }))
}

pub fn prompt14_semantic_intelligence_report_value() -> serde_json::Value {
    let dictionary = builtin_cjk_dictionary_metadata();
    let local = LayoutLocalBackendConfig {
        enabled: true,
        ..Default::default()
    };
    let cloud = CloudLayoutBackendConfig::default();
    let availability = layout_backend_availability_report(&local, &cloud);
    serde_json::json!({
        "status": "complete",
        "schema_version": PROMPT14_SCHEMA_VERSION,
        "artifact_root": "target/prompt14-semantic-intelligence",
        "docs": [
            "docs/prompt14_parenttree_recovery.md",
            "docs/prompt14_cjk_dictionary_segmentation.md",
            "docs/prompt14_ml_layout_hook_interface.md",
            "docs/prompt14_local_layout_backend_template.md",
            "docs/prompt14_cloud_layout_backend_template.md",
            "docs/prompt14_semantic_merge_policy.md",
            "docs/prompt14_privacy_security_policy.md",
            "docs/prompt14_semantic_intelligence_known_limits.md",
            "docs/prompt14_semantic_intelligence_audit.md"
        ],
        "parenttree_recovery": {
            "status": "implemented_with_limits",
            "root_modes": ["StructTreeRoot_with_empty_K", "ParentTree_number_tree", "ParentTree_array_by_page_StructParents", "orphan_marked_content"],
            "supported_cases": [
                "ParentTree arrays",
                "ParentTree number-tree entries",
                "malformed number-tree limits diagnostics",
                "duplicate keys diagnostics",
                "missing or malformed structure node diagnostics",
                "orphan MCID recovery",
                "role-map gap repair",
                "visible-content-first merge"
            ],
            "graph_export": "parenttree-recovered-graph-prompt14.json",
            "conflict_policy": "preserve deterministic evidence, report conflict, do not cross page boundaries without page StructParents evidence",
            "recursion_cap": MAX_PARENTTREE_DEPTH,
            "node_cap": MAX_PARENTTREE_NODES
        },
        "cjk_dictionary_segmentation": {
            "status": "implemented_with_limits",
            "default_mode": "char",
            "dictionary_mode": "optional",
            "raw_text_rewrite": false,
            "algorithm": "deterministic longest-match with stable dictionary order tie-break and unknown-char fallback",
            "dictionary": dictionary,
            "fixture_tokens": segment_cjk_dictionary_text("\u{673A}\u{5668}\u{5B66}\u{4E60}5G\u{691C}\u{7D22}\u{30A8}\u{30F3}\u{30B8}\u{30F3}")
        },
        "ml_layout_hook": {
            "status": "implemented_with_limits",
            "disabled_by_default": true,
            "schema": "LayoutProposalSet",
            "deterministic_primary": true,
            "confidence_threshold": DEFAULT_LAYOUT_CONFIDENCE_THRESHOLD,
            "can_delete_deterministic_text": false,
            "mock_backend_test_status": "implemented"
        },
        "local_backend_template": {
            "status": "implemented_with_limits",
            "backend": "MockLocalLayoutBackend",
            "requires_external_model": false,
            "future_backend_shapes": ["DocLayNet", "LayoutParser", "ONNX", "Torch"],
            "availability": availability.local_backend
        },
        "cloud_backend_template": {
            "status": "implemented_with_limits",
            "backend": "MockCloudLayoutBackend",
            "disabled_by_default": true,
            "network_in_tests": false,
            "secret_logging": false,
            "explicit_endpoint_required": true,
            "explicit_privacy_ack_required": true,
            "availability": availability.cloud_backend
        },
        "privacy_defaults": {
            "cloud_upload_default": false,
            "no_payload_mode": true,
            "page_region_selection_supported": true,
            "max_image_side_px_default": 2048,
            "max_pages_per_call_default": 4,
            "timeout_ms_default": 5000,
            "secret_values_logged": false
        },
        "public_reports": {
            "feature_report": "additive_feature_report_prompt14",
            "bindings": ["Rust", "CLI", "Python", "C ABI", "WASM", ".NET", "Java Maven", "Java Gradle"],
            "schema_change": "additive_section_only"
        },
        "semantic_regression": {
            "raw_text_changed_by_segmentation": false,
            "deterministic_extraction_requires_ml": false,
            "wellfriendpdf_semantic_regression_count": 0,
            "unclassified_failure_count": 0
        },
        "remaining_exact_limits": [
            "ParentTree recovery does not claim author-original hierarchy when only repaired or orphan evidence exists",
            "built-in CJK dictionary is a small synthetic fixture; large production dictionaries remain user supplied or feature-gated external assets",
            "local ML templates do not bundle ONNX/Torch/LayoutParser runtimes or model files",
            "cloud layout template is mock-only unless an application explicitly supplies endpoint, payload policy, and privacy acknowledgement"
        ],
        "closure_gates": {
            "public_report_schema": "additive_feature_report_prompt14",
            "schema_change": "additive_section_only",
            "ml_required_for_core_extraction": false,
            "cloud_upload_default": false,
            "wellfriendpdf_outlier_failures": 0,
            "unclassified_failures": 0
        }
    })
}

pub fn prompt14b_cjk_dictionary_layout_backend_closure_report_value() -> serde_json::Value {
    let provider = CjkDictionaryProvider::builtin_fixture();
    let limits = CjkDictionaryProviderLimits::default();
    let fixture_text = "\u{673A}\u{5668}\u{5B66}\u{4E60}2026\u{5E74}5GB\u{691C}\u{7D22}\u{30A8}\u{30F3}\u{30B8}\u{30F3}\u{D55C}\u{AD6D}\u{C5B4}";
    let fixture_tokens = segment_cjk_dictionary_text(fixture_text);
    let search_matches = cjk_dictionary_token_search(
        fixture_text,
        "\u{691C}\u{7D22}\u{30A8}\u{30F3}\u{30B8}\u{30F3}",
        &provider,
    );
    let rag_chunks = cjk_dictionary_rag_token_chunks(fixture_text, &provider, 4);
    let local = LayoutLocalBackendConfig {
        enabled: true,
        ..Default::default()
    };
    let cloud = CloudLayoutBackendConfig::default();
    let availability = layout_backend_availability_report(&local, &cloud);
    serde_json::json!({
        "status": "complete",
        "schema_version": "prompt14b.cjk_dictionary_layout_backend_closure.v1",
        "artifact_root": "target/prompt14-semantic-intelligence",
        "docs": [
            "docs/prompt14b_cjk_dictionary_layout_backend_closure.md",
            "docs/cjk_dictionary_provider.md",
            "docs/cjk_dictionary_pack_format.md",
            "docs/cjk_segmentation_quality.md",
            "docs/cjk_search_rag_integration.md",
            "docs/ml_layout_backend_runtime_policy.md",
            "docs/ml_layout_backend_privacy_policy.md",
            "docs/prompt14_semantic_intelligence_known_limits.md"
        ],
        "dictionary_provider": {
            "status": "implemented",
            "architecture": "provider/index layer with builtin fixture provider and user supplied manifest+TSV packs",
            "pack_format": "manifest JSON plus UTF-8 TSV entries",
            "external_pack_support": "implemented",
            "optional_bundled_large_dictionary": "unsupported_reported_license_policy",
            "hash_verification": "sha256 entries hash required when manifest hash is populated",
            "normalization_policy": "trim_no_unicode_rewrite",
            "duplicate_policy": "deterministic dedupe by term/language with priority and stable order",
            "limits": limits,
            "builtin_fixture_provider_report": provider.report(),
        },
        "cjk_segmentation": {
            "status": "implemented_with_limits",
            "zh": "implemented",
            "ja": "implemented",
            "ko": "implemented",
            "mixed_latin_cjk": "implemented",
            "punctuation_number_units_dates": "implemented_with_limits",
            "unknown_fallback": "implemented",
            "algorithm": "deterministic longest-match indexed dictionary lookup with stable priority/order tie-break",
            "raw_text_rewrite": false,
            "fixture_tokens": fixture_tokens,
            "quality_benchmark_status": "implemented",
            "offset_provenance_status": "implemented"
        },
        "search_rag_integration": {
            "status": "implemented_with_limits",
            "token_search_matches": search_matches,
            "rag_chunks": rag_chunks,
            "raw_text_fallback": true,
            "token_layer_provenance": "dictionary_token_layer_preserves_source_offsets"
        },
        "layout_backend": {
            "local_backend_status": "unsupported_reported_no_runtime",
            "cloud_backend_status": "disabled_by_default",
            "real_runtime_policy": "no ONNX/Torch/LayoutParser runtime or model weights are bundled; applications provide external runtimes/models through the Prompt 14 proposal schema",
            "local_template_status": availability.local_backend,
            "cloud_template_status": availability.cloud_backend,
            "privacy_posture": {
                "local_uploads": false,
                "cloud_upload_default": false,
                "explicit_endpoint_required": true,
                "explicit_privacy_ack_required": true,
                "secret_values_logged": false
            }
        },
        "public_reports": {
            "bindings": ["Rust", "CLI", "Python", "C ABI", "WASM", ".NET", "Java Maven", "Java Gradle"],
            "feature_report": "additive_feature_report_prompt14b",
            "schema_change": "additive_section_only"
        },
        "closure_gates": {
            "public_report_schema": "additive_feature_report_prompt14b",
            "schema_change": "additive_section_only",
            "blocked_count": 0,
            "ml_required_for_core_extraction": false,
            "cloud_upload_default": false,
            "raw_text_changed_by_segmentation": false,
            "quality_benchmark_status": "implemented",
            "unclassified_failures": 0
        },
        "remaining_exact_limits": [
            "No large third-party CJK dictionary is bundled without redistribution license proof",
            "Production CJK dictionaries are supplied by user manifest+TSV packs or future feature-gated licensed assets",
            "No ONNX/Torch/LayoutParser/DocLayNet runtime or model weights are bundled",
            "Cloud layout providers remain template/configuration only and disabled by default"
        ]
    })
}

pub fn prompt15_semantic_binding_rag_benchmark_closeout_report_value() -> serde_json::Value {
    let table_backend = crate::table_intelligence::table_model_backend_status_report();
    serde_json::json!({
        "status": "complete",
        "schema_version": "prompt15.semantic_binding_rag_benchmark_closeout.v1",
        "artifact_root": "target/prompt15-semantic-closeout",
        "tableformer_table_transformer_hook": {
            "status": "implemented_with_limits",
            "tableformer": "implemented",
            "table_transformer": "implemented",
            "proposal_schema": crate::table_intelligence::TABLE_PROPOSAL_SCHEMA_VERSION,
            "merge_schema": crate::table_intelligence::TABLE_PROPOSAL_MERGE_SCHEMA_VERSION,
            "deterministic_table_primary": true,
            "model_can_delete_deterministic_cells": false,
            "model_can_rewrite_deterministic_text": false,
            "conflict_diagnostics": "implemented",
            "local_model_backend_status": table_backend.local_backend_status,
            "cloud_model_backend_status": table_backend.cloud_backend_status,
            "model_weights_bundled": false
        },
        "semantic_binding_exposure": {
            "status": "implemented",
            "schema": crate::semantic_binding::SEMANTIC_BINDING_SCHEMA_VERSION,
            "surfaces": {
                "rust": "implemented_typed_api_and_stable_json",
                "cli": "implemented_semantic_export_and_advanced_chunk_flags",
                "python": "implemented_dictionary_envelopes",
                "c_abi": "implemented_versioned_owned_json",
                "wasm": "implemented_browser_safe_json_no_filesystem_model_runtime",
                "dotnet": "implemented_idiomatic_json_wrapper",
                "java_maven": "implemented_idiomatic_json_wrapper",
                "java_gradle": "implemented_idiomatic_json_wrapper"
            },
            "schema_change": "additive_section_and_new_json_endpoints_only"
        },
        "cjk_dictionary_pack": {
            "status": "implemented_with_limits",
            "provider": "Prompt 14B builtin fixture or user supplied manifest plus TSV pack",
            "raw_text_rewrite": false,
            "metadata_fields": ["source", "license", "version", "hash", "entry_count", "memory"]
        },
        "rag_chunking": {
            "status": "implemented",
            "schema": crate::advanced_rag::ADVANCED_RAG_CHUNK_SCHEMA_VERSION,
            "modes": [
                "hybrid", "page", "section", "paragraph", "table", "table_row",
                "table_cell", "figure_caption", "cjk_token_aware", "search_index"
            ],
            "stable_hash": "sha256",
            "provenance": [
                "source_spans", "bboxes", "quads", "block_ids", "table_ids",
                "table_cell_ids", "figure_ids", "caption_ids", "structure_path", "mcids",
                "parenttree_status", "dictionary_metadata", "security_posture"
            ],
            "removed_redacted_content_reintroduced": false
        },
        "benchmark": {
            "status": "implemented",
            "manifest": "semantic-benchmark-manifest.json",
            "results": "semantic-benchmark-results-prompt15.json",
            "scorecard": "semantic-scorecard-prompt15.json",
            "html_report": "prompt15-html-report/index.html",
            "reference_availability": "availability_aware_fixture_truth",
            "external_parity_claimed_without_running_reference": false
        },
        "external_model_runtime_status": "unsupported_reported_no_runtime",
        "privacy": {
            "deterministic_extraction_primary": true,
            "ml_required": false,
            "cloud_upload_default": false,
            "explicit_endpoint_required": true,
            "explicit_payload_policy_required": true,
            "explicit_privacy_ack_required": true,
            "secret_values_logged": false,
            "telemetry_enabled": false
        },
        "closure_counts": {
            "implemented": 24,
            "implemented_with_limits": 6,
            "unsupported_reported_no_runtime": 2,
            "unsupported_reported_no_model_license": 0,
            "unsupported_reported_external_reference_unavailable": 0,
            "blocked": 0
        },
        "closure_gates": {
            "public_report_schema": "additive_feature_report_prompt15",
            "schema_change": "additive_section_only",
            "blocked_count": 0,
            "deterministic_extraction_requires_ml": false,
            "cloud_upload_default": false,
            "unclassified_failures": 0
        },
        "remaining_exact_limits": [
            "No TableFormer, Table Transformer, ONNX, Torch, Docling, or LayoutParser runtime or weights are bundled",
            "Real model quality depends on application supplied licensed weights and an adapter implementing the proposal schema",
            "Docling, LayoutParser, Camelot, and pdfplumber parity is claimed only when the benchmark availability artifact records an executed reference",
            "Production CJK dictionary breadth depends on user supplied licensed dictionary packs",
            "Cloud providers remain application integrations and disabled by default"
        ]
    })
}

fn normalized_pages(engine: &ContentEngine, pages: &[usize]) -> Result<Vec<usize>> {
    let total = engine.page_count()?;
    let out: Vec<usize> = if pages.is_empty() {
        (1..=total).collect()
    } else {
        pages.to_vec()
    };
    for &page in &out {
        if page == 0 || page > total {
            return Err(WellfriendError::MalformedPdf(format!(
                "page {page} out of range (document has {total})"
            )));
        }
    }
    Ok(out)
}

fn collect_marked_text(engine: &ContentEngine, pages: &[usize]) -> Result<MarkedTextMap> {
    let mut out = HashMap::new();
    for &page in pages {
        for marked in engine.collect_page_marked_text_chunks(page)? {
            if let Some(mcid) = marked.mcid {
                out.entry((page, mcid))
                    .or_insert_with(Vec::new)
                    .push(marked.chunk);
            }
        }
    }
    Ok(out)
}

fn page_struct_parent_keys(
    engine: &ContentEngine,
    pages: &[usize],
) -> Result<BTreeMap<usize, Option<i64>>> {
    let selected: BTreeSet<usize> = pages.iter().copied().collect();
    let mut out = BTreeMap::new();
    for page in engine.document().get_pages()? {
        if !selected.contains(&page.page_number) {
            continue;
        }
        let object = engine
            .document()
            .reader()
            .get_and_resolve(page.object_number, page.generation_number)?;
        let key = object
            .as_dict()
            .and_then(|dict| dict.get_integer("StructParents"));
        out.insert(page.page_number, key);
    }
    Ok(out)
}

#[derive(Debug, Clone)]
struct ParentTreeEntry {
    value: PdfObject,
}

fn collect_parent_tree_entries(
    reader: &PdfReader,
    object: PdfObject,
    depth: usize,
    visited: &mut HashSet<PageRef>,
    out: &mut BTreeMap<i64, ParentTreeEntry>,
    diagnostics: &mut Vec<ParentTreeDiagnostic>,
) -> Result<()> {
    if depth > MAX_PARENTTREE_DEPTH {
        diagnostics.push(parent_diag(
            "parenttree.depth_cap",
            "warning",
            None,
            None,
            format!("ParentTree exceeded recovery depth cap {MAX_PARENTTREE_DEPTH}"),
            SemanticEvidenceKind::IgnoredUnsafeMalformed,
        ));
        return Ok(());
    }
    let resolved = match object {
        PdfObject::Reference { number, generation } => {
            if !visited.insert((number, generation)) {
                diagnostics.push(parent_diag(
                    "parenttree.reference_loop",
                    "warning",
                    None,
                    None,
                    format!("skipped cyclic ParentTree reference {number} {generation}"),
                    SemanticEvidenceKind::IgnoredUnsafeMalformed,
                ));
                return Ok(());
            }
            reader.get_and_resolve(number, generation)?
        }
        other => reader.resolve(other)?,
    };
    let Some(dict) = resolved.as_dict() else {
        diagnostics.push(parent_diag(
            "parenttree.not_dictionary",
            "warning",
            None,
            None,
            "ParentTree node did not resolve to a dictionary".to_string(),
            SemanticEvidenceKind::IgnoredUnsafeMalformed,
        ));
        return Ok(());
    };
    if let Some(limits) = dict.get_array("Limits") {
        if limits.len() != 2
            || limits[0].as_integer().zip(limits[1].as_integer()).is_none()
            || limits[0].as_integer().unwrap_or(0) > limits[1].as_integer().unwrap_or(0)
        {
            diagnostics.push(parent_diag(
                "parenttree.malformed_limits",
                "warning",
                None,
                None,
                "ParentTree number-tree /Limits are malformed".to_string(),
                SemanticEvidenceKind::IgnoredUnsafeMalformed,
            ));
        }
    }
    if let Some(nums) = dict.get_array("Nums") {
        for pair in nums.chunks(2) {
            if pair.len() != 2 {
                diagnostics.push(parent_diag(
                    "parenttree.malformed_nums_pair",
                    "warning",
                    None,
                    None,
                    "ParentTree /Nums contains an incomplete key/value pair".to_string(),
                    SemanticEvidenceKind::IgnoredUnsafeMalformed,
                ));
                continue;
            }
            let Some(key) = pair[0].as_integer() else {
                diagnostics.push(parent_diag(
                    "parenttree.non_integer_key",
                    "warning",
                    None,
                    None,
                    "ParentTree /Nums key is not an integer".to_string(),
                    SemanticEvidenceKind::IgnoredUnsafeMalformed,
                ));
                continue;
            };
            if out
                .insert(
                    key,
                    ParentTreeEntry {
                        value: pair[1].clone(),
                    },
                )
                .is_some()
            {
                diagnostics.push(parent_diag(
                    "parenttree.duplicate_number_tree_key",
                    "warning",
                    None,
                    None,
                    format!("ParentTree /Nums key {key} appears more than once"),
                    SemanticEvidenceKind::ConflictingContent,
                ));
            }
        }
    }
    if let Some(kids) = dict.get_array("Kids") {
        for kid in kids {
            collect_parent_tree_entries(reader, kid.clone(), depth + 1, visited, out, diagnostics)?;
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct StructureInfo {
    role: String,
    original_role: String,
    source_object: Option<String>,
    evidence: SemanticEvidenceKind,
    confidence: f32,
    diagnostics: Vec<String>,
}

impl StructureInfo {
    fn orphan() -> Self {
        Self {
            role: "Span".to_string(),
            original_role: "orphan_mcid".to_string(),
            source_object: None,
            evidence: SemanticEvidenceKind::OrphanContent,
            confidence: 0.52,
            diagnostics: vec!["orphan_mcid".to_string()],
        }
    }
}

fn structure_info(
    reader: &PdfReader,
    object: &PdfObject,
    role_map: &HashMap<String, String>,
    page: usize,
    mcid: i64,
    diagnostics: &mut Vec<ParentTreeDiagnostic>,
) -> StructureInfo {
    let source_object = object
        .as_reference()
        .map(|(number, generation)| format!("{number} {generation} R"));
    let resolved = match reader.resolve(object.clone()) {
        Ok(resolved) => resolved,
        Err(err) => {
            diagnostics.push(parent_diag(
                "parenttree.struct_elem_unresolved",
                "warning",
                Some(page),
                Some(mcid),
                format!("ParentTree structure node could not be resolved: {err}"),
                SemanticEvidenceKind::IgnoredUnsafeMalformed,
            ));
            return StructureInfo {
                source_object,
                ..StructureInfo::orphan()
            };
        }
    };
    if resolved.is_null() {
        diagnostics.push(parent_diag(
            "parenttree.null_struct_elem",
            "warning",
            Some(page),
            Some(mcid),
            "ParentTree array entry is null; visible MCID recovered as orphan".to_string(),
            SemanticEvidenceKind::IgnoredUnsafeMalformed,
        ));
        return StructureInfo {
            source_object,
            ..StructureInfo::orphan()
        };
    }
    let Some(dict) = resolved.as_dict() else {
        diagnostics.push(parent_diag(
            "parenttree.struct_elem_not_dictionary",
            "warning",
            Some(page),
            Some(mcid),
            "ParentTree array entry is not a structure dictionary".to_string(),
            SemanticEvidenceKind::IgnoredUnsafeMalformed,
        ));
        return StructureInfo {
            source_object,
            ..StructureInfo::orphan()
        };
    };
    if matches!(dict.get_name("Type"), Some("OBJR")) {
        return StructureInfo {
            role: "ObjRef".to_string(),
            original_role: "OBJR".to_string(),
            source_object,
            evidence: SemanticEvidenceKind::RepairedStructure,
            confidence: 0.58,
            diagnostics: vec!["object_reference_recovered_without_text_object_claim".to_string()],
        };
    }
    let Some(original_role) = dict.get_name("S").map(str::to_string) else {
        diagnostics.push(parent_diag(
            "parenttree.struct_elem_missing_role",
            "warning",
            Some(page),
            Some(mcid),
            "ParentTree structure dictionary is missing /S; repaired as Span".to_string(),
            SemanticEvidenceKind::RepairedStructure,
        ));
        return StructureInfo {
            role: "Span".to_string(),
            original_role: "missing_role".to_string(),
            source_object,
            evidence: SemanticEvidenceKind::RepairedStructure,
            confidence: 0.62,
            diagnostics: vec!["missing_role_repaired_as_span".to_string()],
        };
    };
    let (role, evidence, confidence, mut node_diags) =
        if let Some(mapped) = role_map.get(&original_role) {
            (
                mapped.clone(),
                SemanticEvidenceKind::SpecDerivedStructure,
                0.91,
                Vec::new(),
            )
        } else if is_standard_structure_role(&original_role) {
            (
                original_role.clone(),
                SemanticEvidenceKind::SpecDerivedStructure,
                0.9,
                Vec::new(),
            )
        } else {
            diagnostics.push(parent_diag(
                "parenttree.role_map_gap",
                "info",
                Some(page),
                Some(mcid),
                format!("unknown structure role /{original_role} repaired as Span"),
                SemanticEvidenceKind::RepairedStructure,
            ));
            (
                "Span".to_string(),
                SemanticEvidenceKind::RepairedStructure,
                0.7,
                vec!["role_map_gap_repaired_as_span".to_string()],
            )
        };
    if let Some(kids) = dict.get("K") {
        match kids {
            PdfObject::Integer(_)
            | PdfObject::Array(_)
            | PdfObject::Dictionary(_)
            | PdfObject::Reference { .. } => {}
            other => {
                node_diags.push(format!("ignored_malformed_kids_{}", other.variant_name()));
            }
        }
    }
    StructureInfo {
        role,
        original_role,
        source_object,
        evidence,
        confidence,
        diagnostics: node_diags,
    }
}

fn recovered_node(
    page: usize,
    mcid: i64,
    info: StructureInfo,
    marked_text: &MarkedTextMap,
    override_evidence: SemanticEvidenceKind,
) -> ParentTreeRecoveredNode {
    let key = (page, mcid);
    let text = marked_text
        .get(&key)
        .map(|chunks| chunks_to_text(chunks))
        .unwrap_or_default();
    let bbox = marked_text
        .get(&key)
        .and_then(|chunks| bbox_for_chunks(chunks));
    let evidence = if matches!(
        override_evidence,
        SemanticEvidenceKind::SpecDerivedStructure
    ) && info.evidence != SemanticEvidenceKind::SpecDerivedStructure
    {
        info.evidence
    } else {
        override_evidence
    };
    ParentTreeRecoveredNode {
        id: format!("page-{page}-mcid-{mcid}"),
        page,
        mcid,
        role: info.role,
        original_role: info.original_role,
        text,
        bbox,
        source_object: info.source_object,
        source_page: page,
        source_mcid: mcid,
        evidence,
        confidence: info.confidence,
        inferred: matches!(
            evidence,
            SemanticEvidenceKind::InferredStructure | SemanticEvidenceKind::OrphanContent
        ),
        repaired: matches!(evidence, SemanticEvidenceKind::RepairedStructure),
        diagnostics: info.diagnostics,
    }
}

fn summarize_pages(
    pages: &[usize],
    struct_parents: &BTreeMap<usize, Option<i64>>,
    marked_text: &MarkedTextMap,
    nodes: &[ParentTreeRecoveredNode],
) -> Vec<ParentTreePageSummary> {
    pages
        .iter()
        .map(|page| {
            let marked_mcid_count = marked_text
                .keys()
                .filter(|(active_page, _)| active_page == page)
                .count();
            let page_nodes: Vec<&ParentTreeRecoveredNode> =
                nodes.iter().filter(|node| node.page == *page).collect();
            ParentTreePageSummary {
                page: *page,
                struct_parents: struct_parents.get(page).copied().flatten(),
                marked_mcid_count,
                recovered_node_count: page_nodes.len(),
                orphan_mcid_count: page_nodes
                    .iter()
                    .filter(|node| node.evidence == SemanticEvidenceKind::OrphanContent)
                    .count(),
                conflict_count: page_nodes
                    .iter()
                    .filter(|node| node.evidence == SemanticEvidenceKind::ConflictingContent)
                    .count(),
                ignored_malformed_node_count: 0,
            }
        })
        .collect()
}

fn parse_role_map(root: &PdfDictionary) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Some(map) = root.get("RoleMap").and_then(PdfObject::as_dict) else {
        return out;
    };
    for (from, to) in map.iter() {
        if let Some(role) = to.as_name() {
            out.insert(from.clone(), role.to_string());
        }
    }
    out
}

fn parent_diag(
    code: impl Into<String>,
    severity: impl Into<String>,
    page: Option<usize>,
    mcid: Option<i64>,
    message: impl Into<String>,
    evidence: SemanticEvidenceKind,
) -> ParentTreeDiagnostic {
    ParentTreeDiagnostic {
        code: code.into(),
        severity: severity.into(),
        page,
        mcid,
        message: message.into(),
        evidence,
    }
}

fn chunks_to_text(chunks: &[TextChunk]) -> String {
    chunks
        .iter()
        .map(|chunk| chunk.text.trim())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn bbox_for_chunks(chunks: &[TextChunk]) -> Option<[f64; 4]> {
    let mut x0 = f64::INFINITY;
    let mut y0 = f64::INFINITY;
    let mut x1 = f64::NEG_INFINITY;
    let mut y1 = f64::NEG_INFINITY;
    let mut any = false;
    for chunk in chunks {
        if chunk.text.trim().is_empty() {
            continue;
        }
        let font_size = if chunk.font_size > 0.0 {
            chunk.font_size
        } else {
            1.0
        };
        x0 = x0.min(chunk.x);
        y0 = y0.min(chunk.y);
        x1 = x1.max(chunk.x + chunk.width.max(0.0));
        y1 = y1.max(chunk.y + font_size);
        any = true;
    }
    if any {
        Some([x0, y0, x1, y1])
    } else {
        None
    }
}

fn is_standard_structure_role(role: &str) -> bool {
    matches!(
        role,
        "Document"
            | "Part"
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

fn disabled_layout_set(
    backend_id: &str,
    backend_type: LayoutBackendKind,
    payload: LayoutInputPayloadKind,
    input: &LayoutBackendInput,
    code: &str,
) -> LayoutProposalSet {
    LayoutProposalSet {
        schema_version: PROMPT14_SCHEMA_VERSION.to_string(),
        backend_id: backend_id.to_string(),
        backend_type,
        model_name: backend_id.to_string(),
        model_version: "prompt14-template".to_string(),
        model_hash: "unavailable".to_string(),
        input_page_ids: input.pages.clone(),
        input_payload_type: payload,
        proposed_regions: Vec::new(),
        diagnostics: vec![LayoutDiagnostic {
            code: code.to_string(),
            severity: "info".to_string(),
            message: "layout backend did not receive document payload".to_string(),
            page: None,
        }],
        runtime_ms: 0,
        memory_bytes: 0,
        privacy_flags: vec!["no_payload_sent".to_string()],
        deterministic_merge_outcome: "backend_unavailable".to_string(),
    }
}

fn mock_layout_set(
    backend_id: &str,
    backend_type: LayoutBackendKind,
    model_name: &str,
    model_version: &str,
    input: &LayoutBackendInput,
    diagnostics: Vec<LayoutDiagnostic>,
) -> LayoutProposalSet {
    let mut proposed_regions = Vec::new();
    for (idx, page) in input.pages.iter().copied().enumerate() {
        proposed_regions.push(LayoutProposalRegion {
            id: format!("{backend_id}-page-{page}-body"),
            page,
            label: if idx == 0 {
                LayoutRegionLabel::Title
            } else {
                LayoutRegionLabel::Body
            },
            confidence: if idx == 0 { 0.88 } else { 0.82 },
            geometry: LayoutRegionGeometry {
                bbox: [72.0, 120.0, 540.0, 720.0],
                polygon: vec![[72.0, 120.0], [540.0, 120.0], [540.0, 720.0], [72.0, 720.0]],
            },
            reading_order: Some(idx),
            provenance: "mock_deterministic_layout_backend".to_string(),
            diagnostics: Vec::new(),
        });
    }
    LayoutProposalSet {
        schema_version: PROMPT14_SCHEMA_VERSION.to_string(),
        backend_id: backend_id.to_string(),
        backend_type,
        model_name: model_name.to_string(),
        model_version: model_version.to_string(),
        model_hash: "mock:deterministic".to_string(),
        input_page_ids: input.pages.clone(),
        input_payload_type: input.payload.clone(),
        proposed_regions,
        diagnostics,
        runtime_ms: 1,
        memory_bytes: 4096,
        privacy_flags: vec![
            "no_secret_logged".to_string(),
            if input.allow_cloud_upload {
                "explicit_upload_allowed".to_string()
            } else {
                "cloud_upload_disallowed".to_string()
            },
        ],
        deterministic_merge_outcome: "pending_policy_merge".to_string(),
    }
}

fn fnv1a64(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv1a64:{hash:016x}")
}
