//! Optional table-model proposal schema and deterministic merge overlays.
//!
//! The table detector in [`crate::analysis::tables`] remains authoritative.
//! TableFormer/Table Transformer style backends may propose regions and grid
//! structure through this module, but proposals never rewrite deterministic
//! cells or text. Accepted proposals are attached as provenance-bearing
//! overlays; conflicts and low-confidence results remain visible suggestions.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::analysis::tables::Table;
use crate::semantic_intelligence::{
    LayoutBackendKind, LayoutDiagnostic, LayoutInputPayloadKind, LayoutPrivacyMode,
    LayoutRegionGeometry,
};

pub const TABLE_PROPOSAL_SCHEMA_VERSION: &str = "prompt15.table_proposal.v1";
pub const TABLE_PROPOSAL_MERGE_SCHEMA_VERSION: &str = "prompt15.table_merge.v1";

const MAX_TABLE_PROPOSALS: usize = 4_096;
const MAX_BOUNDARIES_PER_TABLE: usize = 8_192;
const MAX_CELLS_PER_TABLE: usize = 250_000;
const MAX_TABLE_MODEL_RUNTIME_MS: u64 = 5_000;
const MAX_TABLE_MODEL_MEMORY_BYTES: usize = 256 * 1024 * 1024;
const MAX_TABLE_MODEL_PAGES: usize = 4;
const MAX_TABLE_IMAGE_SIDE_PX: u32 = 2_048;
const MAX_TABLE_IMAGE_DPI: u32 = 1_200;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum TableSectionRole {
    Header,
    Body,
    Footer,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum TableBoundaryKind {
    Row,
    Column,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TableModelMetadata {
    pub backend_id: String,
    pub backend_type: LayoutBackendKind,
    pub model_name: String,
    pub model_version: String,
    pub model_hash: String,
    pub model_source: String,
    pub model_license: String,
    pub runtime: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TableCoordinateTransform {
    /// Affine transform `[a,b,c,d,e,f]` from model coordinates to PDF page
    /// coordinates: `x'=a*x+c*y+e`, `y'=b*x+d*y+f`.
    pub model_to_pdf: [f64; 6],
    pub pdf_page_bbox: [f64; 4],
    pub input_width_px: u32,
    pub input_height_px: u32,
    pub input_dpi: u32,
}

impl TableCoordinateTransform {
    pub fn identity(pdf_page_bbox: [f64; 4], width: u32, height: u32, dpi: u32) -> Self {
        Self {
            model_to_pdf: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            pdf_page_bbox,
            input_width_px: width,
            input_height_px: height,
            input_dpi: dpi,
        }
    }

    pub fn map_point(&self, point: [f64; 2]) -> [f64; 2] {
        let [a, b, c, d, e, f] = self.model_to_pdf;
        [
            a * point[0] + c * point[1] + e,
            b * point[0] + d * point[1] + f,
        ]
    }

    pub fn map_bbox(&self, bbox: [f64; 4]) -> [f64; 4] {
        let corners = [
            self.map_point([bbox[0], bbox[1]]),
            self.map_point([bbox[0], bbox[3]]),
            self.map_point([bbox[2], bbox[1]]),
            self.map_point([bbox[2], bbox[3]]),
        ];
        corners.iter().fold(
            [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ],
            |mut acc, point| {
                acc[0] = acc[0].min(point[0]);
                acc[1] = acc[1].min(point[1]);
                acc[2] = acc[2].max(point[0]);
                acc[3] = acc[3].max(point[1]);
                acc
            },
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TablePreprocessingMetadata {
    pub renderer: String,
    pub color_space: String,
    pub resize_policy: String,
    pub normalization: String,
    pub max_image_side_px: u32,
    pub coordinate_transform: TableCoordinateTransform,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TableProposalProvenance {
    pub page: usize,
    pub method: String,
    pub backend_id: String,
    pub model_name: String,
    pub model_version: String,
    pub model_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_region_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_span_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcids: Vec<i64>,
    pub inferred: bool,
    pub author_original: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TableBoundaryProposal {
    pub id: String,
    pub kind: TableBoundaryKind,
    pub index: usize,
    pub geometry: LayoutRegionGeometry,
    pub confidence: f32,
    pub provenance: TableProposalProvenance,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TableCellProposal {
    pub id: String,
    pub row: usize,
    pub column: usize,
    pub rowspan: usize,
    pub colspan: usize,
    pub role: TableSectionRole,
    pub geometry: LayoutRegionGeometry,
    pub confidence: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposed_text: Option<String>,
    pub provenance: TableProposalProvenance,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<LayoutDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TableStructureProposal {
    pub id: String,
    pub page: usize,
    pub geometry: LayoutRegionGeometry,
    pub confidence: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reading_order: Option<usize>,
    pub section_role: TableSectionRole,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub row_boundaries: Vec<TableBoundaryProposal>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub column_boundaries: Vec<TableBoundaryProposal>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cells: Vec<TableCellProposal>,
    pub provenance: TableProposalProvenance,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<LayoutDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TableProposalSet {
    pub schema_version: String,
    pub model: TableModelMetadata,
    pub input_page_ids: Vec<usize>,
    pub input_payload_type: LayoutInputPayloadKind,
    pub preprocessing: TablePreprocessingMetadata,
    pub privacy_mode: LayoutPrivacyMode,
    pub allow_cloud_upload: bool,
    pub user_acknowledged_privacy: bool,
    pub runtime_ms: u64,
    pub memory_bytes: usize,
    pub proposals: Vec<TableStructureProposal>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<LayoutDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeterministicTableEvidence {
    pub table_id: String,
    pub page: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_id: Option<usize>,
    pub table: Table,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_span_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcids: Vec<i64>,
    pub provenance: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TableProposalMergePolicy {
    pub deterministic_primary: bool,
    pub region_confidence_threshold: f32,
    pub element_confidence_threshold: f32,
    pub association_iou_threshold: f64,
    pub competing_proposal_iou_threshold: f64,
    pub low_confidence_as_hint: bool,
    pub preserve_deterministic_text: bool,
    pub preserve_deterministic_cells: bool,
}

impl Default for TableProposalMergePolicy {
    fn default() -> Self {
        Self {
            deterministic_primary: true,
            region_confidence_threshold: 0.82,
            element_confidence_threshold: 0.78,
            association_iou_threshold: 0.20,
            competing_proposal_iou_threshold: 0.70,
            low_confidence_as_hint: true,
            preserve_deterministic_text: true,
            preserve_deterministic_cells: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TableProposalMergeOutcomeKind {
    EnrichedDeterministicTable,
    CandidateRegion,
    SuggestionOnly,
    RejectedInvalidSchema,
    RejectedCompetingProposal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TableProposalMergeOutcome {
    pub proposal_id: String,
    pub page: usize,
    pub outcome: TableProposalMergeOutcomeKind,
    pub confidence: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deterministic_table_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accepted_row_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accepted_column_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accepted_cell_ids: Vec<String>,
    pub deterministic_text_preserved: bool,
    pub deterministic_cells_preserved: bool,
    pub author_original: bool,
    pub provenance: TableProposalProvenance,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<LayoutDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MergedTableOverlay {
    pub proposal_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deterministic_table: Option<DeterministicTableEvidence>,
    pub proposal_geometry: LayoutRegionGeometry,
    pub outcome: TableProposalMergeOutcomeKind,
    pub proposal_provenance: TableProposalProvenance,
    pub deterministic_evidence_preserved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TableProposalValidationReport {
    pub schema_version: String,
    pub valid: bool,
    pub accepted_proposal_count: usize,
    pub rejected_proposal_count: usize,
    pub diagnostics: Vec<LayoutDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TableProposalMergeReport {
    pub schema_version: String,
    pub deterministic_primary: bool,
    pub deterministic_table_count: usize,
    pub accepted_count: usize,
    pub suggestion_count: usize,
    pub rejected_count: usize,
    pub conflict_count: usize,
    pub outcomes: Vec<TableProposalMergeOutcome>,
    pub overlays: Vec<MergedTableOverlay>,
    pub diagnostics: Vec<LayoutDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TableModelBackendStatusReport {
    pub schema_version: String,
    pub tableformer_hook: String,
    pub table_transformer_hook: String,
    pub local_backend_status: String,
    pub cloud_backend_status: String,
    pub model_weights_bundled: bool,
    pub external_model_path_required: bool,
    pub local_uploads: bool,
    pub cloud_upload_default: bool,
    pub explicit_endpoint_required: bool,
    pub explicit_privacy_ack_required: bool,
    pub secret_values_logged: bool,
    pub deterministic_preprocessing: bool,
    pub response_schema_validated: bool,
    pub malformed_response_fail_closed: bool,
    pub local_timeout_ms: u64,
    pub local_memory_limit_bytes: usize,
    pub local_max_pages_per_call: usize,
    pub local_max_image_side_px: u32,
    pub cloud_timeout_ms: u64,
    pub cloud_retry_count: u8,
    pub remaining_exact_limits: Vec<String>,
}

pub fn table_model_backend_status_report() -> TableModelBackendStatusReport {
    TableModelBackendStatusReport {
        schema_version: TABLE_PROPOSAL_SCHEMA_VERSION.to_string(),
        tableformer_hook: "implemented".to_string(),
        table_transformer_hook: "implemented".to_string(),
        local_backend_status: "unsupported_reported_no_runtime".to_string(),
        cloud_backend_status: "disabled_by_default".to_string(),
        model_weights_bundled: false,
        external_model_path_required: true,
        local_uploads: false,
        cloud_upload_default: false,
        explicit_endpoint_required: true,
        explicit_privacy_ack_required: true,
        secret_values_logged: false,
        deterministic_preprocessing: true,
        response_schema_validated: true,
        malformed_response_fail_closed: true,
        local_timeout_ms: MAX_TABLE_MODEL_RUNTIME_MS,
        local_memory_limit_bytes: MAX_TABLE_MODEL_MEMORY_BYTES,
        local_max_pages_per_call: MAX_TABLE_MODEL_PAGES,
        local_max_image_side_px: MAX_TABLE_IMAGE_SIDE_PX,
        cloud_timeout_ms: 5_000,
        cloud_retry_count: 0,
        remaining_exact_limits: vec![
            "No ONNX, Torch, TableFormer, or Table Transformer runtime is bundled".to_string(),
            "No model weights are bundled without explicit model license evidence".to_string(),
            "Cloud table providers require application-owned endpoint, credentials, payload policy, and privacy acknowledgement".to_string(),
        ],
    }
}

pub fn validate_table_proposal_set(set: &TableProposalSet) -> TableProposalValidationReport {
    let mut diagnostics = Vec::new();
    let mut rejected_ids = BTreeSet::new();

    if set.schema_version != TABLE_PROPOSAL_SCHEMA_VERSION {
        diagnostics.push(diag(
            "table.schema.unsupported_version",
            "error",
            format!("unsupported table proposal schema {}", set.schema_version),
            None,
        ));
    }
    if set.proposals.len() > MAX_TABLE_PROPOSALS {
        diagnostics.push(diag(
            "table.schema.proposal_cap_exceeded",
            "error",
            format!(
                "{} proposals exceed cap {MAX_TABLE_PROPOSALS}",
                set.proposals.len()
            ),
            None,
        ));
    }
    validate_set_metadata(set, &mut diagnostics);
    if matches!(
        set.model.backend_type,
        LayoutBackendKind::Cloud | LayoutBackendKind::MockCloud
    ) && (!set.allow_cloud_upload || !set.user_acknowledged_privacy)
    {
        diagnostics.push(diag(
            "table.privacy.cloud_not_authorized",
            "error",
            "cloud table proposal payload lacks explicit upload permission and privacy acknowledgement",
            None,
        ));
    }

    let mut ids = BTreeSet::new();
    for proposal in &set.proposals {
        let before = diagnostics.len();
        validate_proposal(proposal, &set.model, &mut ids, &mut diagnostics);
        if diagnostics.len() > before {
            rejected_ids.insert(proposal.id.clone());
        }
    }

    let global_error = diagnostics
        .iter()
        .any(|item| item.severity == "error" && item.page.is_none());
    let rejected_proposal_count = if global_error {
        set.proposals.len()
    } else {
        rejected_ids.len()
    };
    TableProposalValidationReport {
        schema_version: TABLE_PROPOSAL_SCHEMA_VERSION.to_string(),
        valid: diagnostics.iter().all(|item| item.severity != "error"),
        accepted_proposal_count: set.proposals.len().saturating_sub(rejected_proposal_count),
        rejected_proposal_count,
        diagnostics,
    }
}

fn validate_set_metadata(set: &TableProposalSet, diagnostics: &mut Vec<LayoutDiagnostic>) {
    let required_model_fields = [
        ("backend_id", set.model.backend_id.as_str()),
        ("model_name", set.model.model_name.as_str()),
        ("model_version", set.model.model_version.as_str()),
        ("model_hash", set.model.model_hash.as_str()),
        ("model_source", set.model.model_source.as_str()),
        ("model_license", set.model.model_license.as_str()),
        ("runtime", set.model.runtime.as_str()),
    ];
    for (field, value) in required_model_fields {
        if value.trim().is_empty() {
            diagnostics.push(diag(
                "table.schema.missing_model_metadata",
                "error",
                format!("model metadata field {field} must not be empty"),
                None,
            ));
        }
    }

    if set.input_page_ids.is_empty() {
        diagnostics.push(diag(
            "table.schema.missing_input_pages",
            "error",
            "table proposal input_page_ids must not be empty",
            None,
        ));
    }
    if set.input_page_ids.len() > MAX_TABLE_MODEL_PAGES {
        diagnostics.push(diag(
            "table.schema.page_cap_exceeded",
            "error",
            format!(
                "{} input pages exceed cap {MAX_TABLE_MODEL_PAGES}",
                set.input_page_ids.len()
            ),
            None,
        ));
    }
    let mut input_pages = BTreeSet::new();
    for page in &set.input_page_ids {
        if *page == 0 || !input_pages.insert(*page) {
            diagnostics.push(diag(
                "table.schema.invalid_input_page",
                "error",
                format!("input page {page} is zero or duplicated"),
                None,
            ));
        }
    }
    for proposal in &set.proposals {
        if !input_pages.contains(&proposal.page) {
            diagnostics.push(diag(
                "table.schema.proposal_page_not_in_input",
                "error",
                format!(
                    "proposal {} page {} is absent from input_page_ids",
                    proposal.id, proposal.page
                ),
                Some(proposal.page),
            ));
        }
    }

    if set.runtime_ms > MAX_TABLE_MODEL_RUNTIME_MS {
        diagnostics.push(diag(
            "table.schema.runtime_cap_exceeded",
            "error",
            format!(
                "model runtime {} ms exceeds cap {MAX_TABLE_MODEL_RUNTIME_MS} ms",
                set.runtime_ms
            ),
            None,
        ));
    }
    if set.memory_bytes > MAX_TABLE_MODEL_MEMORY_BYTES {
        diagnostics.push(diag(
            "table.schema.memory_cap_exceeded",
            "error",
            format!(
                "model memory {} bytes exceeds cap {MAX_TABLE_MODEL_MEMORY_BYTES}",
                set.memory_bytes
            ),
            None,
        ));
    }

    let transform = &set.preprocessing.coordinate_transform;
    let max_input_side = transform.input_width_px.max(transform.input_height_px);
    if set.preprocessing.max_image_side_px == 0
        || set.preprocessing.max_image_side_px > MAX_TABLE_IMAGE_SIDE_PX
        || max_input_side == 0
        || max_input_side > set.preprocessing.max_image_side_px
    {
        diagnostics.push(diag(
            "table.schema.image_cap_exceeded",
            "error",
            format!(
                "input image {}x{} with declared max side {} violates cap {MAX_TABLE_IMAGE_SIDE_PX}",
                transform.input_width_px,
                transform.input_height_px,
                set.preprocessing.max_image_side_px
            ),
            None,
        ));
    }
    if transform.input_dpi == 0 || transform.input_dpi > MAX_TABLE_IMAGE_DPI {
        diagnostics.push(diag(
            "table.schema.invalid_input_dpi",
            "error",
            format!(
                "input DPI {} is outside 1..={MAX_TABLE_IMAGE_DPI}",
                transform.input_dpi
            ),
            None,
        ));
    }
    if transform
        .model_to_pdf
        .iter()
        .any(|value| !value.is_finite())
        || !valid_area_bbox(transform.pdf_page_bbox)
    {
        diagnostics.push(diag(
            "table.schema.invalid_coordinate_transform",
            "error",
            "model-to-PDF transform or PDF page bounds are invalid",
            None,
        ));
    }
}

pub fn merge_table_proposals_deterministic(
    deterministic: &[DeterministicTableEvidence],
    set: &TableProposalSet,
    policy: &TableProposalMergePolicy,
) -> TableProposalMergeReport {
    let validation = validate_table_proposal_set(set);
    if !validation.valid {
        let outcomes = set
            .proposals
            .iter()
            .map(|proposal| TableProposalMergeOutcome {
                proposal_id: proposal.id.clone(),
                page: proposal.page,
                outcome: TableProposalMergeOutcomeKind::RejectedInvalidSchema,
                confidence: proposal.confidence,
                deterministic_table_id: None,
                accepted_row_ids: Vec::new(),
                accepted_column_ids: Vec::new(),
                accepted_cell_ids: Vec::new(),
                deterministic_text_preserved: true,
                deterministic_cells_preserved: true,
                author_original: false,
                provenance: proposal.provenance.clone(),
                diagnostics: validation.diagnostics.clone(),
            })
            .collect();
        return TableProposalMergeReport {
            schema_version: TABLE_PROPOSAL_MERGE_SCHEMA_VERSION.to_string(),
            deterministic_primary: true,
            deterministic_table_count: deterministic.len(),
            accepted_count: 0,
            suggestion_count: 0,
            rejected_count: set.proposals.len(),
            conflict_count: validation.diagnostics.len(),
            outcomes,
            overlays: Vec::new(),
            diagnostics: validation.diagnostics,
        };
    }

    let mut ordered: Vec<&TableStructureProposal> = set.proposals.iter().collect();
    ordered.sort_by(|left, right| {
        left.page
            .cmp(&right.page)
            .then_with(|| right.confidence.total_cmp(&left.confidence))
            .then_with(|| left.id.cmp(&right.id))
    });

    let mut accepted_regions: Vec<&TableStructureProposal> = Vec::new();
    let mut outcomes = Vec::new();
    let mut overlays = Vec::new();
    let mut diagnostics = Vec::new();
    if !policy.deterministic_primary
        || !policy.preserve_deterministic_text
        || !policy.preserve_deterministic_cells
    {
        diagnostics.push(diag(
            "table.merge.policy_hardened",
            "warning",
            "unsafe merge-policy flags were ignored; deterministic tables, cells, text, and provenance remain authoritative",
            None,
        ));
    }

    for proposal in ordered {
        let competing = accepted_regions.iter().find(|accepted| {
            accepted.page == proposal.page
                && bbox_iou(accepted.geometry.bbox, proposal.geometry.bbox)
                    >= policy.competing_proposal_iou_threshold
        });
        if let Some(winner) = competing {
            let diagnostic = diag(
                "table.merge.competing_proposal",
                "warning",
                format!(
                    "proposal {} overlaps higher-priority proposal {}",
                    proposal.id, winner.id
                ),
                Some(proposal.page),
            );
            diagnostics.push(diagnostic.clone());
            outcomes.push(outcome(
                proposal,
                TableProposalMergeOutcomeKind::RejectedCompetingProposal,
                None,
                Vec::new(),
                Vec::new(),
                Vec::new(),
                vec![diagnostic],
            ));
            continue;
        }

        let associated =
            best_deterministic_table(deterministic, proposal, policy.association_iou_threshold);
        let high_confidence = proposal.confidence >= policy.region_confidence_threshold;
        let mut proposal_diagnostics = Vec::new();

        let (kind, accepted_rows, accepted_columns, accepted_cells) = if high_confidence {
            accepted_regions.push(proposal);
            if let Some(table) = associated {
                let (rows, columns, cells, conflicts) = accepted_elements(proposal, table, policy);
                proposal_diagnostics.extend(conflicts);
                (
                    TableProposalMergeOutcomeKind::EnrichedDeterministicTable,
                    rows,
                    columns,
                    cells,
                )
            } else {
                (
                    TableProposalMergeOutcomeKind::CandidateRegion,
                    accepted_boundary_ids(
                        &proposal.row_boundaries,
                        policy.element_confidence_threshold,
                    ),
                    accepted_boundary_ids(
                        &proposal.column_boundaries,
                        policy.element_confidence_threshold,
                    ),
                    accepted_cell_ids(&proposal.cells, policy.element_confidence_threshold),
                )
            }
        } else {
            if !policy.low_confidence_as_hint {
                proposal_diagnostics.push(diag(
                    "table.merge.low_confidence_rejected",
                    "info",
                    format!("proposal {} is below merge threshold", proposal.id),
                    Some(proposal.page),
                ));
            }
            (
                TableProposalMergeOutcomeKind::SuggestionOnly,
                Vec::new(),
                Vec::new(),
                Vec::new(),
            )
        };

        diagnostics.extend(proposal_diagnostics.clone());
        outcomes.push(outcome(
            proposal,
            kind,
            associated.map(|item| item.table_id.clone()),
            accepted_rows,
            accepted_columns,
            accepted_cells,
            proposal_diagnostics,
        ));
        overlays.push(MergedTableOverlay {
            proposal_id: proposal.id.clone(),
            deterministic_table: associated.cloned(),
            proposal_geometry: proposal.geometry.clone(),
            outcome: kind,
            proposal_provenance: proposal.provenance.clone(),
            deterministic_evidence_preserved: true,
        });
    }

    outcomes.sort_by(|left, right| {
        left.page
            .cmp(&right.page)
            .then_with(|| left.proposal_id.cmp(&right.proposal_id))
    });
    overlays.sort_by(|left, right| left.proposal_id.cmp(&right.proposal_id));
    diagnostics.sort_by(|left, right| {
        left.page
            .cmp(&right.page)
            .then_with(|| left.code.cmp(&right.code))
            .then_with(|| left.message.cmp(&right.message))
    });

    let accepted_count = outcomes
        .iter()
        .filter(|item| {
            matches!(
                item.outcome,
                TableProposalMergeOutcomeKind::EnrichedDeterministicTable
                    | TableProposalMergeOutcomeKind::CandidateRegion
            )
        })
        .count();
    let suggestion_count = outcomes
        .iter()
        .filter(|item| item.outcome == TableProposalMergeOutcomeKind::SuggestionOnly)
        .count();
    let rejected_count = outcomes
        .len()
        .saturating_sub(accepted_count + suggestion_count);
    let conflict_count = diagnostics
        .iter()
        .filter(|item| item.code.starts_with("table.merge."))
        .count();

    TableProposalMergeReport {
        schema_version: TABLE_PROPOSAL_MERGE_SCHEMA_VERSION.to_string(),
        deterministic_primary: true,
        deterministic_table_count: deterministic.len(),
        accepted_count,
        suggestion_count,
        rejected_count,
        conflict_count,
        outcomes,
        overlays,
        diagnostics,
    }
}

pub fn mock_tableformer_proposal_set(page: usize) -> TableProposalSet {
    let model = TableModelMetadata {
        backend_id: "mock-tableformer-local".to_string(),
        backend_type: LayoutBackendKind::MockLocal,
        model_name: "tableformer-contract-fixture".to_string(),
        model_version: "prompt15".to_string(),
        model_hash: "sha256:mock-tableformer-no-weights".to_string(),
        model_source: "generated contract fixture".to_string(),
        model_license: "CC0-1.0 synthetic fixture".to_string(),
        runtime: "deterministic_mock_no_ml_dependency".to_string(),
    };
    let provenance = TableProposalProvenance {
        page,
        method: "mock_tableformer_contract".to_string(),
        backend_id: model.backend_id.clone(),
        model_name: model.model_name.clone(),
        model_version: model.model_version.clone(),
        model_hash: model.model_hash.clone(),
        source_region_id: Some("layout-table-1".to_string()),
        source_span_ids: vec!["page-1-span-0".to_string()],
        mcids: vec![0],
        inferred: true,
        author_original: false,
    };
    let rows = [0.0, 40.0, 80.0]
        .into_iter()
        .enumerate()
        .map(|(index, y)| TableBoundaryProposal {
            id: format!("row-{index}"),
            kind: TableBoundaryKind::Row,
            index,
            geometry: LayoutRegionGeometry {
                bbox: [0.0, y, 200.0, y],
                polygon: vec![[0.0, y], [200.0, y]],
            },
            confidence: 0.91,
            provenance: provenance.clone(),
        })
        .collect();
    let columns = [0.0, 100.0, 200.0]
        .into_iter()
        .enumerate()
        .map(|(index, x)| TableBoundaryProposal {
            id: format!("column-{index}"),
            kind: TableBoundaryKind::Column,
            index,
            geometry: LayoutRegionGeometry {
                bbox: [x, 0.0, x, 80.0],
                polygon: vec![[x, 0.0], [x, 80.0]],
            },
            confidence: 0.90,
            provenance: provenance.clone(),
        })
        .collect();
    let mut cells = Vec::new();
    for row in 0..2 {
        for column in 0..2 {
            cells.push(TableCellProposal {
                id: format!("cell-{row}-{column}"),
                row,
                column,
                rowspan: 1,
                colspan: 1,
                role: if row == 0 {
                    TableSectionRole::Header
                } else {
                    TableSectionRole::Body
                },
                geometry: LayoutRegionGeometry {
                    bbox: [
                        column as f64 * 100.0,
                        row as f64 * 40.0,
                        (column + 1) as f64 * 100.0,
                        (row + 1) as f64 * 40.0,
                    ],
                    polygon: Vec::new(),
                },
                confidence: 0.89,
                proposed_text: None,
                provenance: provenance.clone(),
                diagnostics: Vec::new(),
            });
        }
    }
    TableProposalSet {
        schema_version: TABLE_PROPOSAL_SCHEMA_VERSION.to_string(),
        model,
        input_page_ids: vec![page],
        input_payload_type: LayoutInputPayloadKind::RenderedImage,
        preprocessing: TablePreprocessingMetadata {
            renderer: "oxide_renderer".to_string(),
            color_space: "srgb".to_string(),
            resize_policy: "fit_within_2048".to_string(),
            normalization: "none_mock_fixture".to_string(),
            max_image_side_px: 2048,
            coordinate_transform: TableCoordinateTransform::identity(
                [0.0, 0.0, 612.0, 792.0],
                1224,
                1584,
                144,
            ),
        },
        privacy_mode: LayoutPrivacyMode::LocalOnly,
        allow_cloud_upload: false,
        user_acknowledged_privacy: false,
        runtime_ms: 1,
        memory_bytes: 4_096,
        proposals: vec![TableStructureProposal {
            id: "table-proposal-1".to_string(),
            page,
            geometry: LayoutRegionGeometry {
                bbox: [0.0, 0.0, 200.0, 80.0],
                polygon: Vec::new(),
            },
            confidence: 0.94,
            reading_order: Some(0),
            section_role: TableSectionRole::Body,
            row_boundaries: rows,
            column_boundaries: columns,
            cells,
            provenance,
            diagnostics: Vec::new(),
        }],
        diagnostics: Vec::new(),
    }
}

fn validate_proposal(
    proposal: &TableStructureProposal,
    model: &TableModelMetadata,
    ids: &mut BTreeSet<String>,
    diagnostics: &mut Vec<LayoutDiagnostic>,
) {
    if !ids.insert(proposal.id.clone()) {
        diagnostics.push(diag(
            "table.schema.duplicate_id",
            "error",
            format!("duplicate proposal id {}", proposal.id),
            Some(proposal.page),
        ));
    }
    validate_confidence(
        proposal.confidence,
        &proposal.id,
        proposal.page,
        diagnostics,
    );
    validate_geometry(
        &proposal.geometry,
        &proposal.id,
        proposal.page,
        false,
        diagnostics,
    );
    if proposal.page == 0 {
        diagnostics.push(diag(
            "table.schema.invalid_page",
            "error",
            format!("proposal {} uses page 0", proposal.id),
            Some(proposal.page),
        ));
    }
    validate_provenance(&proposal.provenance, proposal.page, model, diagnostics);
    if proposal.row_boundaries.len() > MAX_BOUNDARIES_PER_TABLE
        || proposal.column_boundaries.len() > MAX_BOUNDARIES_PER_TABLE
    {
        diagnostics.push(diag(
            "table.schema.boundary_cap_exceeded",
            "error",
            format!("proposal {} exceeds boundary cap", proposal.id),
            Some(proposal.page),
        ));
    }
    if proposal.cells.len() > MAX_CELLS_PER_TABLE {
        diagnostics.push(diag(
            "table.schema.cell_cap_exceeded",
            "error",
            format!("proposal {} exceeds cell cap", proposal.id),
            Some(proposal.page),
        ));
    }

    let mut boundary_indices: BTreeMap<TableBoundaryKind, BTreeSet<usize>> = BTreeMap::new();
    for boundary in &proposal.row_boundaries {
        if boundary.kind != TableBoundaryKind::Row {
            diagnostics.push(diag(
                "table.schema.boundary_kind_mismatch",
                "error",
                format!("row boundary {} is labeled as a column", boundary.id),
                Some(proposal.page),
            ));
        }
    }
    for boundary in &proposal.column_boundaries {
        if boundary.kind != TableBoundaryKind::Column {
            diagnostics.push(diag(
                "table.schema.boundary_kind_mismatch",
                "error",
                format!("column boundary {} is labeled as a row", boundary.id),
                Some(proposal.page),
            ));
        }
    }
    for boundary in proposal
        .row_boundaries
        .iter()
        .chain(proposal.column_boundaries.iter())
    {
        if !ids.insert(boundary.id.clone()) {
            diagnostics.push(diag(
                "table.schema.duplicate_id",
                "error",
                format!("duplicate boundary id {}", boundary.id),
                Some(proposal.page),
            ));
        }
        if !boundary_indices
            .entry(boundary.kind)
            .or_default()
            .insert(boundary.index)
        {
            diagnostics.push(diag(
                "table.schema.duplicate_boundary_index",
                "error",
                format!(
                    "duplicate {:?} boundary index {}",
                    boundary.kind, boundary.index
                ),
                Some(proposal.page),
            ));
        }
        validate_confidence(
            boundary.confidence,
            &boundary.id,
            proposal.page,
            diagnostics,
        );
        validate_geometry(
            &boundary.geometry,
            &boundary.id,
            proposal.page,
            true,
            diagnostics,
        );
        validate_provenance(&boundary.provenance, proposal.page, model, diagnostics);
    }

    let mut occupied = BTreeSet::new();
    for cell in &proposal.cells {
        if !ids.insert(cell.id.clone()) {
            diagnostics.push(diag(
                "table.schema.duplicate_id",
                "error",
                format!("duplicate cell id {}", cell.id),
                Some(proposal.page),
            ));
        }
        validate_confidence(cell.confidence, &cell.id, proposal.page, diagnostics);
        validate_geometry(&cell.geometry, &cell.id, proposal.page, false, diagnostics);
        validate_provenance(&cell.provenance, proposal.page, model, diagnostics);
        if cell.rowspan == 0 || cell.colspan == 0 {
            diagnostics.push(diag(
                "table.schema.invalid_span",
                "error",
                format!("cell {} has a zero row or column span", cell.id),
                Some(proposal.page),
            ));
            continue;
        }
        for row in cell.row..cell.row.saturating_add(cell.rowspan) {
            for column in cell.column..cell.column.saturating_add(cell.colspan) {
                if !occupied.insert((row, column)) {
                    diagnostics.push(diag(
                        "table.schema.overlapping_cells",
                        "error",
                        format!("cell {} overlaps another proposed cell", cell.id),
                        Some(proposal.page),
                    ));
                }
            }
        }
    }
}

fn validate_provenance(
    provenance: &TableProposalProvenance,
    page: usize,
    model: &TableModelMetadata,
    diagnostics: &mut Vec<LayoutDiagnostic>,
) {
    let consistent = provenance.page == page
        && provenance.backend_id == model.backend_id
        && provenance.model_name == model.model_name
        && provenance.model_version == model.model_version
        && provenance.model_hash == model.model_hash
        && provenance.inferred
        && !provenance.author_original
        && !provenance.method.trim().is_empty();
    if !consistent {
        diagnostics.push(diag(
            "table.schema.invalid_proposal_provenance",
            "error",
            "proposal provenance must match model/page metadata, be inferred, and never be author-original",
            Some(page),
        ));
    }
}

fn validate_confidence(
    confidence: f32,
    id: &str,
    page: usize,
    diagnostics: &mut Vec<LayoutDiagnostic>,
) {
    if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
        diagnostics.push(diag(
            "table.schema.invalid_confidence",
            "error",
            format!("{id} confidence is outside 0..1"),
            Some(page),
        ));
    }
}

fn validate_geometry(
    geometry: &LayoutRegionGeometry,
    id: &str,
    page: usize,
    allow_line: bool,
    diagnostics: &mut Vec<LayoutDiagnostic>,
) {
    let [x0, y0, x1, y1] = geometry.bbox;
    let valid = [x0, y0, x1, y1].iter().all(|value| value.is_finite())
        && if allow_line {
            x1 >= x0 && y1 >= y0 && (x1 > x0 || y1 > y0)
        } else {
            x1 > x0 && y1 > y0
        };
    if !valid {
        diagnostics.push(diag(
            "table.schema.invalid_geometry",
            "error",
            format!("{id} has invalid geometry"),
            Some(page),
        ));
    }
    if geometry
        .polygon
        .iter()
        .flatten()
        .any(|value| !value.is_finite())
    {
        diagnostics.push(diag(
            "table.schema.invalid_polygon",
            "error",
            format!("{id} polygon contains a non-finite coordinate"),
            Some(page),
        ));
    }
}

fn best_deterministic_table<'a>(
    deterministic: &'a [DeterministicTableEvidence],
    proposal: &TableStructureProposal,
    threshold: f64,
) -> Option<&'a DeterministicTableEvidence> {
    deterministic
        .iter()
        .filter(|table| table.page == proposal.page)
        .filter_map(|table| {
            let iou = bbox_iou(table.table.bbox, proposal.geometry.bbox);
            (iou >= threshold).then_some((table, iou))
        })
        .max_by(|(left_table, left_iou), (right_table, right_iou)| {
            left_iou
                .partial_cmp(right_iou)
                .unwrap_or(Ordering::Equal)
                .then_with(|| right_table.table_id.cmp(&left_table.table_id))
        })
        .map(|(table, _)| table)
}

fn accepted_elements(
    proposal: &TableStructureProposal,
    deterministic: &DeterministicTableEvidence,
    policy: &TableProposalMergePolicy,
) -> (Vec<String>, Vec<String>, Vec<String>, Vec<LayoutDiagnostic>) {
    let rows = accepted_boundary_ids(
        &proposal.row_boundaries,
        policy.element_confidence_threshold,
    );
    let columns = accepted_boundary_ids(
        &proposal.column_boundaries,
        policy.element_confidence_threshold,
    );
    let mut cells = Vec::new();
    let mut diagnostics = Vec::new();
    let max_rows = deterministic.table.num_rows();
    let max_columns = deterministic.table.num_cols();
    for cell in &proposal.cells {
        if cell.confidence < policy.element_confidence_threshold {
            continue;
        }
        let row_end = cell.row.saturating_add(cell.rowspan);
        let column_end = cell.column.saturating_add(cell.colspan);
        if row_end > max_rows || column_end > max_columns {
            diagnostics.push(diag(
                "table.merge.cell_grid_conflict",
                "warning",
                format!(
                    "proposal cell {} exceeds deterministic {}x{} grid",
                    cell.id, max_rows, max_columns
                ),
                Some(proposal.page),
            ));
            continue;
        }
        if let Some(proposed_text) = cell.proposed_text.as_deref() {
            let deterministic_text = deterministic
                .table
                .cells
                .iter()
                .find(|item| item.row == cell.row && item.col == cell.column)
                .map(|item| item.text.as_str())
                .or_else(|| {
                    deterministic
                        .table
                        .rows
                        .get(cell.row)
                        .and_then(|row| row.get(cell.column))
                        .map(String::as_str)
                });
            if deterministic_text.is_some_and(|text| text != proposed_text) {
                diagnostics.push(diag(
                    "table.merge.proposed_text_conflict",
                    "warning",
                    format!(
                        "proposal cell {} text conflicts with deterministic text",
                        cell.id
                    ),
                    Some(proposal.page),
                ));
            }
        }
        cells.push(cell.id.clone());
    }
    (rows, columns, cells, diagnostics)
}

fn accepted_boundary_ids(boundaries: &[TableBoundaryProposal], threshold: f32) -> Vec<String> {
    let mut accepted: Vec<&TableBoundaryProposal> = boundaries
        .iter()
        .filter(|item| item.confidence >= threshold)
        .collect();
    accepted.sort_by(|left, right| {
        left.index
            .cmp(&right.index)
            .then_with(|| left.id.cmp(&right.id))
    });
    accepted.into_iter().map(|item| item.id.clone()).collect()
}

fn accepted_cell_ids(cells: &[TableCellProposal], threshold: f32) -> Vec<String> {
    let mut accepted: Vec<&TableCellProposal> = cells
        .iter()
        .filter(|item| item.confidence >= threshold)
        .collect();
    accepted.sort_by(|left, right| {
        left.row
            .cmp(&right.row)
            .then_with(|| left.column.cmp(&right.column))
            .then_with(|| left.id.cmp(&right.id))
    });
    accepted.into_iter().map(|item| item.id.clone()).collect()
}

fn outcome(
    proposal: &TableStructureProposal,
    kind: TableProposalMergeOutcomeKind,
    deterministic_table_id: Option<String>,
    accepted_row_ids: Vec<String>,
    accepted_column_ids: Vec<String>,
    accepted_cell_ids: Vec<String>,
    diagnostics: Vec<LayoutDiagnostic>,
) -> TableProposalMergeOutcome {
    TableProposalMergeOutcome {
        proposal_id: proposal.id.clone(),
        page: proposal.page,
        outcome: kind,
        confidence: proposal.confidence,
        deterministic_table_id,
        accepted_row_ids,
        accepted_column_ids,
        accepted_cell_ids,
        deterministic_text_preserved: true,
        deterministic_cells_preserved: true,
        author_original: false,
        provenance: proposal.provenance.clone(),
        diagnostics,
    }
}

fn bbox_iou(left: [f64; 4], right: [f64; 4]) -> f64 {
    if !valid_area_bbox(left) || !valid_area_bbox(right) {
        return 0.0;
    }
    let x0 = left[0].max(right[0]);
    let y0 = left[1].max(right[1]);
    let x1 = left[2].min(right[2]);
    let y1 = left[3].min(right[3]);
    let intersection = (x1 - x0).max(0.0) * (y1 - y0).max(0.0);
    let left_area = (left[2] - left[0]) * (left[3] - left[1]);
    let right_area = (right[2] - right[0]) * (right[3] - right[1]);
    let union = left_area + right_area - intersection;
    if union <= 0.0 {
        0.0
    } else {
        intersection / union
    }
}

fn valid_area_bbox(bbox: [f64; 4]) -> bool {
    bbox.iter().all(|value| value.is_finite()) && bbox[2] > bbox[0] && bbox[3] > bbox[1]
}

fn diag(
    code: impl Into<String>,
    severity: impl Into<String>,
    message: impl Into<String>,
    page: Option<usize>,
) -> LayoutDiagnostic {
    LayoutDiagnostic {
        code: code.into(),
        severity: severity.into(),
        message: message.into(),
        page,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::tables::{TableCell, TableSource};

    fn deterministic_table() -> DeterministicTableEvidence {
        DeterministicTableEvidence {
            table_id: "det-table-1".to_string(),
            page: 1,
            block_id: Some(7),
            table: Table {
                rows: vec![
                    vec!["Name".to_string(), "Value".to_string()],
                    vec!["Alpha".to_string(), "1".to_string()],
                ],
                cells: vec![TableCell {
                    row: 0,
                    col: 0,
                    rowspan: 1,
                    colspan: 1,
                    text: "Name".to_string(),
                    bbox: [0.0, 0.0, 100.0, 40.0],
                    is_header: true,
                    header_scope: None,
                    nested_tables: Vec::new(),
                }],
                header_hierarchy: Vec::new(),
                source: TableSource::Ruled,
                confidence: 0.97,
                bbox: [0.0, 0.0, 200.0, 80.0],
                notes: Vec::new(),
            },
            source_span_ids: vec!["span-1".to_string()],
            mcids: vec![4],
            provenance: "deterministic_ruled_table".to_string(),
        }
    }

    #[test]
    fn table_proposal_merge_preserves_deterministic_table() {
        let deterministic = vec![deterministic_table()];
        let before = deterministic.clone();
        let set = mock_tableformer_proposal_set(1);
        let report = merge_table_proposals_deterministic(
            &deterministic,
            &set,
            &TableProposalMergePolicy::default(),
        );
        assert_eq!(deterministic, before);
        assert_eq!(report.accepted_count, 1);
        assert_eq!(report.rejected_count, 0);
        assert_eq!(
            report.outcomes[0].outcome,
            TableProposalMergeOutcomeKind::EnrichedDeterministicTable
        );
        assert!(report.outcomes[0].deterministic_text_preserved);
        assert!(!report.outcomes[0].author_original);
    }

    #[test]
    fn malformed_table_proposal_fails_closed() {
        let mut set = mock_tableformer_proposal_set(1);
        set.proposals[0].cells[0].confidence = 2.0;
        let report = merge_table_proposals_deterministic(
            &[deterministic_table()],
            &set,
            &TableProposalMergePolicy::default(),
        );
        assert_eq!(report.accepted_count, 0);
        assert_eq!(report.rejected_count, 1);
        assert!(report
            .diagnostics
            .iter()
            .any(|item| item.code == "table.schema.invalid_confidence"));
    }

    #[test]
    fn malformed_model_preprocessing_and_provenance_fail_closed() {
        let mut set = mock_tableformer_proposal_set(1);
        set.model.model_license.clear();
        set.runtime_ms = MAX_TABLE_MODEL_RUNTIME_MS + 1;
        set.preprocessing.coordinate_transform.input_width_px = MAX_TABLE_IMAGE_SIDE_PX + 1;
        set.proposals[0].cells[0].provenance.author_original = true;
        let validation = validate_table_proposal_set(&set);
        assert!(!validation.valid);
        for code in [
            "table.schema.missing_model_metadata",
            "table.schema.runtime_cap_exceeded",
            "table.schema.image_cap_exceeded",
            "table.schema.invalid_proposal_provenance",
        ] {
            assert!(
                validation.diagnostics.iter().any(|item| item.code == code),
                "missing diagnostic {code}"
            );
        }
    }

    #[test]
    fn cloud_table_proposal_requires_explicit_privacy_ack() {
        let mut set = mock_tableformer_proposal_set(1);
        set.model.backend_type = LayoutBackendKind::MockCloud;
        let validation = validate_table_proposal_set(&set);
        assert!(!validation.valid);
        assert!(validation
            .diagnostics
            .iter()
            .any(|item| item.code == "table.privacy.cloud_not_authorized"));
    }

    #[test]
    fn competing_table_proposals_resolve_stably() {
        let mut set = mock_tableformer_proposal_set(1);
        let mut lower = set.proposals[0].clone();
        lower.id = "table-proposal-2".to_string();
        lower.confidence = 0.90;
        lower.provenance.source_region_id = Some("layout-table-2".to_string());
        for boundary in &mut lower.row_boundaries {
            boundary.id.push_str("-p2");
        }
        for boundary in &mut lower.column_boundaries {
            boundary.id.push_str("-p2");
        }
        for cell in &mut lower.cells {
            cell.id.push_str("-p2");
        }
        set.proposals.push(lower);
        let report = merge_table_proposals_deterministic(
            &[deterministic_table()],
            &set,
            &TableProposalMergePolicy::default(),
        );
        assert_eq!(report.accepted_count, 1);
        assert_eq!(report.rejected_count, 1);
        assert!(report
            .outcomes
            .iter()
            .any(|item| item.outcome == TableProposalMergeOutcomeKind::RejectedCompetingProposal));
    }

    #[test]
    fn unsafe_policy_flags_cannot_demote_deterministic_evidence() {
        let policy = TableProposalMergePolicy {
            deterministic_primary: false,
            preserve_deterministic_text: false,
            preserve_deterministic_cells: false,
            ..Default::default()
        };
        let report = merge_table_proposals_deterministic(
            &[deterministic_table()],
            &mock_tableformer_proposal_set(1),
            &policy,
        );
        assert!(report.deterministic_primary);
        assert!(report.outcomes[0].deterministic_text_preserved);
        assert!(report.outcomes[0].deterministic_cells_preserved);
        assert!(report
            .diagnostics
            .iter()
            .any(|item| item.code == "table.merge.policy_hardened"));
    }
}
