//! Prompt 33 geometric and semantic reflow closure.
//!
//! This module extends the Prompt 31/32 source-linked editing stack. It is a
//! bounded production adapter: it models geometric regions, paragraph/style
//! facts, line breaking, overflow, semantic region graphs, reading order and
//! flow edges, then compiles supported edits back through Prompt 31/32 source
//! mutation and canonical writer paths. It never creates an overlay-only edit
//! engine or a second scene/semantic/font stack.

use crate::authoring::{PageSize as AuthorPageSize, PdfBuilder, TextStyle};
use crate::filters::decode_stream_lossless;
use crate::prompt20::{
    analyze_multi_run_text_range, edit_advanced_text_pdf_with_positioned_visual_layout,
    edit_advanced_text_pdf_with_visual_layout, edit_multi_run_text_range, edit_vector_object,
    list_vector_objects, move_link_annotation_rect_pdf, AdvancedTextEditOptions, AdvancedTextMode,
    ExplicitLayoutLine, GeneratedTextAlignment, MultiRunStylePolicy, MultiRunTextRangeRequest,
    PositionedExplicitLayoutLine, SharedFormEditPolicy, TextOverflowPolicy, VectorEditOperation,
    VectorEditOptions,
};
use crate::prompt31::{operator_text_provenance, TrueEditingMode};
use crate::prompt32::{
    build_document_snapshot, build_scene_graph, dirty_region_report, text_identity_report,
    undo_restoration_report, DocumentSnapshot, EditTransactionReport, EditableSceneGraph,
    SceneTextEditRequest, TransactionState,
};
use crate::writer::append_authored_page_preserving_catalog;
#[cfg(test)]
use crate::writer::build_merged;
use crate::{interactive_report, ContentEngine, Result, WellfriendError};
use cassowary::strength::{MEDIUM, REQUIRED, STRONG, WEAK};
use cassowary::WeightedRelation::*;
use cassowary::{Solver, Variable};
use hyphenation::{Hyphenator, Language, Load, Standard};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;
use unicode_bidi::BidiInfo;
use unicode_linebreak::{break_property, linebreaks, BreakOpportunity};
use unicode_segmentation::UnicodeSegmentation;

pub const PROMPT33_SCHEMA_VERSION: &str = "prompt33.geometric-semantic-reflow.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Prompt33Status {
    Implemented,
    ImplementedWithLimits,
    Verified,
    VerifiedWithLimits,
    DeferredPrompt34,
    DeferredPrompt35,
    UnsupportedExact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Prompt33EvidenceKind {
    ExactSourceFact,
    DeterministicGeometry,
    DeterministicFontShaping,
    StructureTreeFact,
    HeuristicInference,
    UserCorrection,
    Ambiguous,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NeighborPolicy {
    Locked,
    FlowLinkedMovable,
    FreelyMovableWithinRegion,
    AtomicObstacle,
    AnchoredToParagraph,
    AnchoredToPage,
    BackgroundDecorative,
    UnknownUnsafe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverflowStatus {
    FitInRegion,
    FitAfterSpacingAdjustment,
    FitAfterRegionExpansion,
    FitAfterDownstreamFlow,
    FitAfterColumnFlow,
    FitAfterPageFlow,
    FontReductionAvailableNotApplied,
    UnresolvedOverflow,
    ConstraintsInfeasible,
    SemanticDocumentRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfidenceDecision {
    AutoApply,
    ApplyWithWarning,
    ReviewRequired,
    Refuse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflowConfidencePolicy {
    pub auto_apply: f64,
    pub apply_with_warning: f64,
    pub review_required: f64,
    pub refuse_below: f64,
}

impl Default for ReflowConfidencePolicy {
    fn default() -> Self {
        Self {
            auto_apply: 0.90,
            apply_with_warning: 0.80,
            review_required: 0.70,
            refuse_below: 0.70,
        }
    }
}

pub fn evaluate_reflow_confidence(
    dimensions: &Value,
    requested_mode: TrueEditingMode,
    policy: &ReflowConfidencePolicy,
) -> Value {
    let keys: &[&str] = if requested_mode == TrueEditingMode::SemanticDocument {
        &[
            "geometry",
            "text_mapping",
            "font_identity",
            "reading_order",
            "semantic_type",
            "cross_page_flow",
        ]
    } else {
        &["geometry", "text_mapping", "font_identity"]
    };
    let evidence = keys
        .iter()
        .map(|key| {
            (
                *key,
                dimensions.get(*key).and_then(Value::as_f64).unwrap_or(0.0),
            )
        })
        .collect::<Vec<_>>();
    let minimum = evidence
        .iter()
        .map(|(_, value)| *value)
        .fold(1.0_f64, f64::min);
    let decision = if minimum >= policy.auto_apply {
        ConfidenceDecision::AutoApply
    } else if minimum >= policy.apply_with_warning {
        ConfidenceDecision::ApplyWithWarning
    } else if minimum >= policy.review_required {
        ConfidenceDecision::ReviewRequired
    } else {
        ConfidenceDecision::Refuse
    };
    json!({
        "policy": policy,
        "requested_mode": requested_mode,
        "relevant_dimensions": evidence.into_iter().map(|(key, value)| json!({"name": key, "value": value})).collect::<Vec<_>>(),
        "minimum_relevant_confidence": minimum,
        "decision": decision,
        "deterministic": true,
    })
}

/// An explicit, source-linked downstream vector movement.  It is deliberately
/// a narrow extension of Prompt 20's canonical vector mutator: callers name a
/// stable vector identity and an approved semantic dependency rather than
/// asking Prompt 33 to move nearby/unknown artwork.  The current transaction
/// boundary accepts a bounded, collision-free set of same-page path objects so
/// preimage undo and collision proof remain exact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownstreamVectorMove {
    pub vector_stable_id: String,
    pub relationship: String,
    pub dependency_edge_id: String,
    pub dx: f64,
    pub dy: f64,
    #[serde(default)]
    pub shared_form_policy: SharedFormEditPolicy,
}

/// One explicit Link annotation that follows the caller-selected source text
/// region. This is deliberately an identity/geometry transaction, not a
/// nearest-annotation heuristic: the expected preimage rectangle prevents a
/// stale page index or unrelated Link from moving silently.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownstreamLinkMove {
    pub annotation_index: usize,
    pub expected_rect: [f64; 4],
    pub relationship: String,
    pub dependency_edge_id: String,
    pub dx: f64,
    pub dy: f64,
}

/// A caller-supplied layout constraint evaluated by the same bounded
/// Cassowary instance that guards source rewriting.  The deliberately small
/// variable vocabulary keeps a request from becoming a second layout engine:
/// every variable is an observed property of the resolved source region.
///
/// `priority` is one of `required`, `strong`, `medium`, or `weak`; `relation`
/// is `eq`, `le`, or `ge`. Required constraints can refuse the transaction;
/// soft constraints are retained with their measured residuals.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutConstraint {
    pub constraint_id: String,
    pub variable: String,
    pub relation: String,
    pub value: f64,
    #[serde(default = "default_constraint_priority")]
    pub priority: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeometricReflowRequest {
    #[serde(default = "default_geometric_mode")]
    pub requested_mode: TrueEditingMode,
    #[serde(default = "default_page")]
    pub page: usize,
    pub source_text: String,
    pub replacement_text: String,
    #[serde(default)]
    pub region: Option<[f64; 4]>,
    #[serde(default)]
    pub allowed_expansion_region: Option<[f64; 4]>,
    /// Explicit, user-approved downstream region.  It is considered only by
    /// SemanticDocument flow after the local-region stages are exhausted.
    #[serde(default)]
    pub next_region: Option<[f64; 4]>,
    /// Explicit, user-approved next-column rectangle. It is intentionally
    /// separate from `next_region` so a caller cannot silently change an
    /// ordinary downstream flow into a column transition.
    #[serde(default)]
    pub next_column: Option<[f64; 4]>,
    /// Explicit source-linked downstream vector movement. Unknown scene
    /// neighbors are never inferred as movable. This currently supports a
    /// bounded collision-free same-page path set through Prompt 20's canonical
    /// vector mutator; images, annotations, and text objects remain typed
    /// limits until they have equivalent source-level movement transactions.
    #[serde(default)]
    pub downstream_vector_moves: Vec<DownstreamVectorMove>,
    /// Explicit source-associated Link annotation rectangles that should move
    /// with the edited source text. A bounded same-page /Link set with exact
    /// expected source rects is currently supported; all other annotations stay
    /// locked until equivalent source-linked repair transactions exist.
    #[serde(default)]
    pub downstream_link_moves: Vec<DownstreamLinkMove>,
    /// Explicit hard/soft constraints over the resolved region metrics. They
    /// are evaluated before any source mutation and are bounded to avoid an
    /// untrusted request creating an unbounded global optimization problem.
    #[serde(default)]
    pub layout_constraints: Vec<LayoutConstraint>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub direction: Option<String>,
    #[serde(default = "default_font_policy")]
    pub font_policy: String,
    #[serde(default = "default_alignment")]
    pub alignment: String,
    #[serde(default)]
    pub justify_last_line: bool,
    #[serde(default)]
    pub hyphenation: bool,
    #[serde(default)]
    pub allow_page_creation: bool,
    #[serde(default)]
    pub allow_font_reduction: bool,
    #[serde(default)]
    pub approve_low_confidence_structure: bool,
    #[serde(default)]
    pub signature_policy_override: bool,
    #[serde(default = "default_line_height")]
    pub line_height: f64,
    #[serde(default = "default_max_downstream_blocks")]
    pub max_downstream_blocks: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct GeometricTextRegion {
    pub schema_version: String,
    pub region_id: String,
    pub source_scene_nodes: Vec<String>,
    pub source_semantic_nodes: Vec<String>,
    pub source_instructions: Vec<String>,
    pub page_id: usize,
    pub page_box: [f64; 4],
    pub writing_mode: String,
    pub base_direction: String,
    pub language: String,
    pub polygon_or_rect: Vec<[f64; 2]>,
    pub padding: [f64; 4],
    pub transforms: Vec<String>,
    pub clipping: String,
    pub style_runs: Vec<Value>,
    pub paragraph_ids: Vec<String>,
    pub allowed_expansion_region: [f64; 4],
    pub locked_neighbors: Vec<String>,
    pub movable_neighbors: Vec<String>,
    pub exclusion_zones: Vec<Value>,
    pub downstream_flow_targets: Vec<String>,
    pub born_digital_ocr_or_inferred: String,
    pub confidence: Value,
    pub edit_policy: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct ParagraphStyleModel {
    pub schema_version: String,
    pub paragraph_id: String,
    pub source_semantic_links: Vec<String>,
    pub source_provenance_links: Vec<String>,
    pub language: String,
    pub script_runs: Vec<Value>,
    pub base_direction: String,
    pub writing_mode: String,
    pub style_runs: Vec<Value>,
    pub font_identity: Value,
    pub line_height: f64,
    pub alignment: String,
    pub first_line_indent: f64,
    pub hanging_indent: f64,
    pub start_end_indents: [f64; 2],
    pub margins: [f64; 4],
    pub spacing_before_after: [f64; 2],
    pub tab_stops: Vec<f64>,
    pub hyphenation_policy: String,
    pub widow_orphan_policy: String,
    pub keep_with_next: bool,
    pub keep_together: bool,
    pub list_relationship: Option<Value>,
    pub baseline_grid: Option<f64>,
    pub source_region: String,
    pub allowed_expansion_region: String,
    pub locked_neighbors: Vec<String>,
    pub flow_successor_predecessor: [Option<String>; 2],
    pub confidence: Value,
    pub evidence: Vec<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LayoutLine {
    pub line_id: String,
    /// Logical replacement text for this line. Mandatory source separators
    /// remain here even though they are not painted as glyphs.
    pub text: String,
    pub visual_text: String,
    pub grapheme_range: [usize; 2],
    pub advance: f64,
    pub baseline: f64,
    pub bidi_visual_order: Vec<usize>,
    pub hyphen_inserted: bool,
    pub source_link_status: Prompt33EvidenceKind,
}

/// A source-linked UAX #14 candidate boundary. Offsets are UTF-8 byte offsets
/// because that is the stable boundary representation produced by the Unicode
/// implementation; the accompanying grapheme index is the only boundary that
/// may be selected for a reflow line.
#[derive(Debug, Clone, Serialize)]
pub struct LineBreakRecord {
    pub logical_offset_utf8: usize,
    pub grapheme_index: Option<usize>,
    pub shaping_cluster_utf8: Option<usize>,
    pub source_location: String,
    pub break_class: String,
    pub disposition: String,
    pub penalty: i64,
    pub hyphenation_source: String,
    pub inserted_visual_glyph_behavior: String,
    pub extraction_behavior: String,
    /// Whether this candidate can currently be serialized by the canonical
    /// Prompt 20 source writer without changing logical extraction. Supported
    /// dictionary candidates paint one visible hyphen CID with an empty
    /// ToUnicode mapping; soft-hyphen source handling remains fail-closed.
    pub source_output_supported: bool,
    pub confidence: Prompt33EvidenceKind,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LineBreakingResult {
    pub schema_version: String,
    pub pipeline: Vec<String>,
    pub preview_algorithm: String,
    pub final_algorithm: String,
    pub preview_lines: Vec<LayoutLine>,
    pub lines: Vec<LayoutLine>,
    pub break_records: Vec<LineBreakRecord>,
    pub final_cost: f64,
    pub overflow_status: OverflowStatus,
    pub overflow_amount: f64,
    pub grapheme_safe: bool,
    pub bidi_source_visual_separated: bool,
    pub hyphenation: Value,
    pub justification: Value,
    pub exact_limits: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConstraintSolverReport {
    pub schema_version: String,
    pub solver: String,
    pub deterministic: bool,
    pub bounded_runtime: bool,
    pub constraints: Vec<Value>,
    pub hard_constraints: Vec<Value>,
    pub soft_constraints: Vec<Value>,
    pub unsatisfied_soft_constraints: Vec<Value>,
    pub fixed_constraint_count: usize,
    pub infeasible: bool,
    pub infeasibility_explanation: Vec<String>,
    pub locked_objects_moved: usize,
    pub unknown_objects_locked_by_default: bool,
    pub no_nan_or_infinite_geometry: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SemanticRegionNode {
    pub node_id: String,
    pub node_type: String,
    pub page: usize,
    pub source_scene_nodes: Vec<String>,
    pub source_instructions: Vec<String>,
    pub bounds: [f64; 4],
    pub text_hash: String,
    pub evidence_kind: Prompt33EvidenceKind,
    pub confidence: Value,
    pub coordinate_space: String,
    pub source_evidence: Value,
    pub alternatives: Vec<Value>,
    pub transaction_revision: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SemanticRegionEdge {
    pub edge_id: String,
    pub source: String,
    pub target: String,
    pub relationship: String,
    pub confidence: f64,
    pub exact_inferred_or_user_supplied: Prompt33EvidenceKind,
    pub source_evidence: Value,
    pub alternatives: Vec<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SemanticLayoutReport {
    pub schema_version: String,
    pub document_id: String,
    pub nodes: Vec<SemanticRegionNode>,
    pub edges: Vec<SemanticRegionEdge>,
    pub algorithms_used: Vec<String>,
    pub exact_vs_inferred: Value,
    pub reading_order: Value,
    pub flow_graph: Value,
    /// Executable graph integrity and invalidation facts. This is computed from
    /// the canonical Prompt 06/32 projections on every analysis, rather than
    /// being a documentation-only promise.
    pub region_graph_invariants: Value,
    pub review_required: Vec<Value>,
    pub prompt34_boundaries: Vec<String>,
    pub prompt35_boundaries: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReflowTransactionReport {
    pub schema_version: String,
    pub transaction_id: String,
    pub input_snapshot: DocumentSnapshot,
    pub requested_mode: TrueEditingMode,
    pub eligible_modes: Vec<TrueEditingMode>,
    pub applied_mode: Option<TrueEditingMode>,
    pub escalation_reason: Option<String>,
    pub scope_of_movement: String,
    pub confidence: Value,
    pub region: GeometricTextRegion,
    pub paragraph: ParagraphStyleModel,
    pub line_breaking: LineBreakingResult,
    pub constraints: ConstraintSolverReport,
    pub overflow_status: OverflowStatus,
    pub semantic_layout: SemanticLayoutReport,
    pub objects_moved: Vec<String>,
    pub pages_columns_affected: Vec<Value>,
    pub source_instructions_regenerated: Vec<String>,
    pub fonts_resources_changed: Vec<String>,
    pub flow_graph_changes: Vec<Value>,
    pub reading_order_changes: Vec<Value>,
    pub structure_changes: Vec<Value>,
    pub signature_impact: Value,
    pub conformance_impact: Value,
    pub validation_evidence: Value,
    pub inverse_operation: Option<Value>,
    pub undo_proof: Value,
    pub refusal: Option<Value>,
    pub prompt32_transaction: Option<EditTransactionReport>,
}

/// Outcome of an executable Prompt 33 undo.  The session stores no replacement
/// PDF bytes in its report; it validates and truncates the canonical
/// incremental revision, which is exact for the currently supported
/// source-linked reflow operations.
#[derive(Debug, Clone, Serialize)]
pub struct ReflowUndoReport {
    pub schema_version: String,
    pub transaction_id: String,
    pub undone: bool,
    pub atomic: bool,
    pub before_sha256: String,
    pub edited_sha256: String,
    pub restored_sha256: String,
    pub byte_exact_restoration: bool,
    pub output_reopened: bool,
    pub restored_page_count: usize,
    pub conflict: Option<String>,
}

#[derive(Debug, Clone)]
struct ReflowMutationCheckpoint {
    transaction_id: String,
    before_bytes: usize,
    before_sha256: String,
    after_sha256: String,
    before_page_count: usize,
    retained_preimage: Option<Vec<u8>>,
}

/// In-memory transaction owner for supported Prompt 33 source reflows.
///
/// Supported reflow output is written as a canonical incremental revision, so
/// undo can restore the exact preimage by validated truncation.  Operations
/// that later require a non-append page-tree rewrite must use a different
/// checkpoint representation and are refused rather than being recorded here.
#[derive(Debug, Clone)]
pub struct ReflowMutationSession {
    current: Vec<u8>,
    checkpoints: Vec<ReflowMutationCheckpoint>,
    cursor: usize,
    max_operations: usize,
}

impl ReflowMutationSession {
    pub fn new(input: Vec<u8>) -> Result<Self> {
        ContentEngine::open_bytes(input.clone())?;
        Ok(Self {
            current: input,
            checkpoints: Vec::new(),
            cursor: 0,
            max_operations: 1_024,
        })
    }

    pub fn with_max_operations(input: Vec<u8>, max_operations: usize) -> Result<Self> {
        if max_operations == 0 || max_operations > 100_000 {
            return Err(WellfriendError::ResourceLimit(
                "prompt33 reflow session operation limit must be between 1 and 100000".to_string(),
            ));
        }
        let mut session = Self::new(input)?;
        session.max_operations = max_operations;
        Ok(session)
    }

    pub fn bytes(&self) -> &[u8] {
        &self.current
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn apply_geometric(
        &mut self,
        request: &GeometricReflowRequest,
    ) -> Result<ReflowTransactionReport> {
        let (output, report) = apply_reflow_region(&self.current, request)?;
        self.commit(output, &report)?;
        Ok(report)
    }

    pub fn apply_semantic(
        &mut self,
        request: &GeometricReflowRequest,
    ) -> Result<ReflowTransactionReport> {
        let (output, report) = apply_reflow_document(&self.current, request)?;
        self.commit(output, &report)?;
        Ok(report)
    }

    pub fn undo_reflow(&mut self) -> Result<ReflowUndoReport> {
        let Some(checkpoint) = self.checkpoints.get(self.cursor.saturating_sub(1)).cloned() else {
            return Ok(ReflowUndoReport {
                schema_version: PROMPT33_SCHEMA_VERSION.to_string(),
                transaction_id: String::new(),
                undone: false,
                atomic: true,
                before_sha256: String::new(),
                edited_sha256: digest_hex(&self.current),
                restored_sha256: digest_hex(&self.current),
                byte_exact_restoration: true,
                output_reopened: ContentEngine::open_bytes(self.current.clone()).is_ok(),
                restored_page_count: ContentEngine::open_bytes(self.current.clone())?
                    .page_count()?,
                conflict: None,
            });
        };
        let edited_sha256 = digest_hex(&self.current);
        if edited_sha256 != checkpoint.after_sha256 || checkpoint.before_bytes > self.current.len()
        {
            return Err(WellfriendError::MalformedPdf(
                "prompt33 stale_snapshot_conflict: reflow undo checkpoint does not match current output"
                    .to_string(),
            ));
        }
        let restored = checkpoint
            .retained_preimage
            .clone()
            .unwrap_or_else(|| self.current[..checkpoint.before_bytes].to_vec());
        let restored_sha256 = digest_hex(&restored);
        if restored_sha256 != checkpoint.before_sha256 {
            return Err(WellfriendError::MalformedPdf(
                "prompt33 undo_validation_failed: incremental truncation did not match recorded preimage"
                    .to_string(),
            ));
        }
        let reopened = ContentEngine::open_bytes(restored.clone())?;
        let page_count = reopened.page_count()?;
        if page_count != checkpoint.before_page_count {
            return Err(WellfriendError::MalformedPdf(
                "prompt33 undo_validation_failed: restored page count does not match checkpoint"
                    .to_string(),
            ));
        }
        // All validation completed before assignment: a failure leaves the
        // current session byte-for-byte untouched.
        self.current = restored;
        self.cursor -= 1;
        Ok(ReflowUndoReport {
            schema_version: PROMPT33_SCHEMA_VERSION.to_string(),
            transaction_id: checkpoint.transaction_id,
            undone: true,
            atomic: true,
            before_sha256: checkpoint.before_sha256,
            edited_sha256,
            restored_sha256,
            byte_exact_restoration: true,
            output_reopened: true,
            restored_page_count: page_count,
            conflict: None,
        })
    }

    fn commit(&mut self, output: Vec<u8>, report: &ReflowTransactionReport) -> Result<()> {
        if self.cursor >= self.max_operations {
            return Err(WellfriendError::ResourceLimit(format!(
                "prompt33 resource_limit_exceeded: reflow session operation cap {} reached",
                self.max_operations
            )));
        }
        let before_engine = ContentEngine::open_bytes(self.current.clone())?;
        // Canonical incremental output is restored without retaining duplicate
        // bytes.  The narrow page-tree merge path is canonical but not
        // incremental, so its session retains an in-memory preimage and still
        // verifies it before any state change.
        let retained_preimage = (!output.starts_with(&self.current)).then(|| self.current.clone());
        let checkpoint = ReflowMutationCheckpoint {
            transaction_id: report.transaction_id.clone(),
            before_bytes: self.current.len(),
            before_sha256: digest_hex(&self.current),
            after_sha256: digest_hex(&output),
            before_page_count: before_engine.page_count()?,
            retained_preimage,
        };
        if self.cursor < self.checkpoints.len() {
            self.checkpoints.truncate(self.cursor);
        }
        self.current = output;
        self.checkpoints.push(checkpoint);
        self.cursor += 1;
        Ok(())
    }
}

/// Execute a binding-safe Prompt 33 undo from immutable byte inputs.  Public
/// bindings normally return a new PDF byte buffer for an apply operation, so
/// they cannot retain a Rust-only `ReflowMutationSession` handle.  This helper
/// reconstructs that session from the exact preimage, replays the requested
/// canonical operation, compares the resulting bytes with the caller's
/// candidate output, and only then executes the session's atomic undo.
///
/// A mismatch is a stale-snapshot conflict: no bytes are returned and no
/// caller-owned data is modified.  This remains an executable inverse proof,
/// including the page-tree preimage path, instead of treating an input buffer
/// as an unverified "undo" result.
pub fn undo_reflow_from_replay(
    input: &[u8],
    edited: &[u8],
    request: &GeometricReflowRequest,
) -> Result<(Vec<u8>, ReflowUndoReport)> {
    let mut session = ReflowMutationSession::new(input.to_vec())?;
    if request.requested_mode == TrueEditingMode::SemanticDocument {
        session.apply_semantic(request)?;
    } else {
        session.apply_geometric(request)?;
    }
    if session.bytes() != edited {
        return Err(WellfriendError::MalformedPdf(
            "prompt33 stale_snapshot_conflict: supplied output does not match deterministic reflow replay"
                .to_string(),
        ));
    }
    let undo = session.undo_reflow()?;
    if !undo.undone || !undo.byte_exact_restoration || session.bytes() != input {
        return Err(WellfriendError::MalformedPdf(
            "prompt33 undo_validation_failed: replay session did not restore the exact preimage"
                .to_string(),
        ));
    }
    Ok((session.bytes().to_vec(), undo))
}

fn default_geometric_mode() -> TrueEditingMode {
    TrueEditingMode::GeometricBlock
}

fn default_page() -> usize {
    1
}

fn default_font_policy() -> String {
    "rebuild_subset_or_generated_type0".to_string()
}

fn default_alignment() -> String {
    "left".to_string()
}

fn default_line_height() -> f64 {
    14.0
}

fn default_max_downstream_blocks() -> usize {
    8
}

fn default_constraint_priority() -> String {
    "required".to_string()
}

fn stable_id(kind: &str, values: &[impl AsRef<[u8]>]) -> String {
    let mut digest = Sha256::new();
    digest.update(kind.as_bytes());
    digest.update([0]);
    for value in values {
        digest.update(value.as_ref());
        digest.update([0]);
    }
    let encoded = format!("{:x}", digest.finalize());
    format!("{kind}-{}", &encoded[..24])
}

fn digest_hex(data: impl AsRef<[u8]>) -> String {
    let mut digest = Sha256::new();
    digest.update(data.as_ref());
    format!("{:x}", digest.finalize())
}

fn layout_extraction_equivalent(extracted: &str, expected: &str) -> bool {
    extracted.split_whitespace().collect::<String>()
        == expected.split_whitespace().collect::<String>()
}

fn unaffected_content_proof(
    input: &[u8],
    output: &[u8],
    affected_page: usize,
    source_text: &str,
    replacement_text: &str,
    expected_link_rects: &[(usize, [f64; 4])],
    maximum_changed_existing_streams: usize,
) -> Value {
    let Ok(before) = ContentEngine::open_bytes(input.to_vec()) else {
        return json!({"status": "unavailable", "reason": "input_reopen_failed"});
    };
    let Ok(after) = ContentEngine::open_bytes(output.to_vec()) else {
        return json!({"status": "unavailable", "reason": "output_reopen_failed"});
    };
    let page_count_before = before.page_count().unwrap_or(0);
    let page_count_after = after.page_count().unwrap_or(0);
    let source_page_before = before.get_page_text(affected_page).unwrap_or_default();
    let source_page_after = after.get_page_text(affected_page).unwrap_or_default();
    let source_occurrences = source_page_before.matches(source_text).count();
    let expected_source_page = source_page_before.replacen(source_text, replacement_text, 1);
    let source_extraction_exact_under_layout_policy = source_occurrences == 1
        && layout_extraction_equivalent(&source_page_after, &expected_source_page);
    let page_stream_hashes = |engine: &ContentEngine, page: usize| -> Result<Vec<Value>> {
        let page = engine.document().get_page(page)?;
        page.contents
            .iter()
            .map(|(number, generation)| {
                let object = engine
                    .document()
                    .reader()
                    .get_object(*number, *generation)?;
                let decoded = decode_stream_lossless(&object, engine.document().reader())?;
                Ok(json!({
                    "object": [number, generation],
                    "decode_status": decoded.status,
                    "decoded_sha256": digest_hex(&decoded.data),
                }))
            })
            .collect()
    };
    let mut untouched_pages = Vec::new();
    let mut untouched_pages_proven = page_count_before == page_count_after;
    if page_count_before == page_count_after {
        for page in 1..=page_count_before {
            if page == affected_page {
                continue;
            }
            let before_text = before.get_page_text(page).unwrap_or_default();
            let after_text = after.get_page_text(page).unwrap_or_default();
            let box_same = before.page_box(page).ok() == after.page_box(page).ok();
            let content_references_same = before
                .document()
                .get_page(page)
                .ok()
                .map(|item| item.contents)
                == after
                    .document()
                    .get_page(page)
                    .ok()
                    .map(|item| item.contents);
            let before_streams = page_stream_hashes(&before, page).unwrap_or_default();
            let after_streams = page_stream_hashes(&after, page).unwrap_or_default();
            let decoded_streams_unchanged =
                !before_streams.is_empty() && before_streams == after_streams;
            let unchanged = before_text == after_text
                && box_same
                && content_references_same
                && decoded_streams_unchanged;
            untouched_pages_proven &= unchanged;
            untouched_pages.push(json!({
                "page": page,
                "text_sha256_before": digest_hex(before_text.as_bytes()),
                "text_sha256_after": digest_hex(after_text.as_bytes()),
                "page_box_unchanged": box_same,
                "content_references_unchanged": content_references_same,
                "decoded_stream_hashes": before_streams,
                "decoded_streams_unchanged": decoded_streams_unchanged,
                "unchanged": unchanged,
            }));
        }
    }
    let affected_page_stream_proof = (|| -> Result<Value> {
        let before_page = before.document().get_page(affected_page)?;
        let after_page = after.document().get_page(affected_page)?;
        let before_references = before_page
            .contents
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let after_references = after_page.contents.iter().copied().collect::<BTreeSet<_>>();
        let mut rows = Vec::new();
        let mut changed_existing = 0usize;
        let mut all_unmodified_existing_match = true;
        for reference in &before_references {
            let before_object = before
                .document()
                .reader()
                .get_object(reference.0, reference.1)?;
            let before_decoded =
                decode_stream_lossless(&before_object, before.document().reader())?;
            let before_hash = digest_hex(&before_decoded.data);
            let after_hash = if after_references.contains(reference) {
                let after_object = after
                    .document()
                    .reader()
                    .get_object(reference.0, reference.1)?;
                let after_decoded =
                    decode_stream_lossless(&after_object, after.document().reader())?;
                Some(digest_hex(&after_decoded.data))
            } else {
                None
            };
            let changed = after_hash.as_deref() != Some(before_hash.as_str());
            changed_existing += usize::from(changed);
            if !changed {
                all_unmodified_existing_match &=
                    after_hash.as_deref() == Some(before_hash.as_str());
            }
            rows.push(json!({
                "object": [reference.0, reference.1],
                "before_decoded_sha256": before_hash,
                "after_decoded_sha256": after_hash,
                "changed_by_declared_source_transaction": changed,
            }));
        }
        let generated_streams = after_references
            .difference(&before_references)
            .copied()
            .collect::<Vec<_>>();
        let changed_within_declared_bound =
            changed_existing > 0 && changed_existing <= maximum_changed_existing_streams;
        Ok(json!({
            "existing_stream_rows": rows,
            "new_generated_stream_references": generated_streams,
            "changed_existing_stream_count": changed_existing,
            "maximum_declared_changed_existing_streams": maximum_changed_existing_streams,
            "changed_existing_streams_within_declared_bound": changed_within_declared_bound,
            "unmodified_existing_streams_match": all_unmodified_existing_match,
        }))
    })();
    let affected_page_stream_proof = affected_page_stream_proof.unwrap_or_else(|error| {
        json!({
            "changed_existing_streams_within_declared_bound": false,
            "unmodified_existing_streams_match": false,
            "reason": error.to_string(),
        })
    });
    let affected_page_streams_proven = affected_page_stream_proof
        ["changed_existing_streams_within_declared_bound"]
        .as_bool()
        .unwrap_or(false)
        && affected_page_stream_proof["unmodified_existing_streams_match"]
            .as_bool()
            .unwrap_or(false);
    let annotation_proof = (|| -> Result<Value> {
        let before_annotations = interactive_report(&before)?.annotations.annotations;
        let after_annotations = interactive_report(&after)?.annotations.annotations;
        let expected = expected_link_rects
            .iter()
            .copied()
            .collect::<BTreeMap<usize, [f64; 4]>>();
        let mut rows = Vec::new();
        let mut unchanged = before_annotations.len() == after_annotations.len();
        for before_annotation in &before_annotations {
            let after_annotation = after_annotations.iter().find(|candidate| {
                candidate.page == before_annotation.page
                    && candidate.index == before_annotation.index
            });
            let expected_link_rect = (before_annotation.page == affected_page)
                .then(|| expected.get(&before_annotation.index))
                .flatten()
                .copied();
            let action_same = after_annotation.is_some_and(|candidate| {
                serde_json::to_value(&candidate.action).ok()
                    == serde_json::to_value(&before_annotation.action).ok()
            });
            let row_unchanged = match (after_annotation, expected_link_rect) {
                (Some(after_annotation), Some(expected_rect)) => {
                    before_annotation.subtype == "Link"
                        && after_annotation.subtype == "Link"
                        && action_same
                        && after_annotation
                            .rect
                            .is_some_and(|actual| rects_nearly_equal(actual, expected_rect))
                }
                (Some(after_annotation), None) => {
                    serde_json::to_value(after_annotation).ok()
                        == serde_json::to_value(before_annotation).ok()
                }
                (None, _) => false,
            };
            unchanged &= row_unchanged;
            rows.push(json!({
                "page": before_annotation.page,
                "index": before_annotation.index,
                "subtype": before_annotation.subtype,
                "expected_source_link_move": expected_link_rect.is_some(),
                "action_or_destination_unchanged": action_same,
                "unchanged_or_expectedly_moved": row_unchanged,
            }));
        }
        for (index, _) in expected {
            if !before_annotations
                .iter()
                .any(|annotation| annotation.page == affected_page && annotation.index == index)
            {
                unchanged = false;
            }
        }
        Ok(json!({
            "annotations_outside_flow_unchanged": unchanged,
            "rows": rows,
        }))
    })();
    let annotation_proof = annotation_proof.unwrap_or_else(|error| {
        json!({
            "annotations_outside_flow_unchanged": false,
            "reason": error.to_string(),
            "rows": [],
        })
    });
    let annotations_unchanged_or_expectedly_moved = annotation_proof
        ["annotations_outside_flow_unchanged"]
        .as_bool()
        .unwrap_or(false);
    json!({
        "status": if source_extraction_exact_under_layout_policy && untouched_pages_proven && affected_page_streams_proven && annotations_unchanged_or_expectedly_moved { "pass_with_documented_layout_whitespace_policy" } else { "fail" },
        "page_count_before": page_count_before,
        "page_count_after": page_count_after,
        "source_occurrences_before": source_occurrences,
        "affected_page_extraction_exact_under_layout_whitespace_policy": source_extraction_exact_under_layout_policy,
        "untouched_pages": untouched_pages,
        "untouched_pages_proven": untouched_pages_proven,
        "affected_page_stream_proof": affected_page_stream_proof,
        "annotation_proof": annotation_proof,
        "no_coverup_or_duplicate_old_source": !source_page_after.contains(source_text),
    })
}

fn expected_downstream_link_rects(request: &GeometricReflowRequest) -> Vec<(usize, [f64; 4])> {
    request
        .downstream_link_moves
        .iter()
        .map(|movement| {
            (
                movement.annotation_index,
                [
                    movement.expected_rect[0] + movement.dx,
                    movement.expected_rect[1] + movement.dy,
                    movement.expected_rect[2] + movement.dx,
                    movement.expected_rect[3] + movement.dy,
                ],
            )
        })
        .collect()
}

fn direction_label(value: Option<&str>) -> String {
    match value.unwrap_or_default() {
        "rtl" | "right_to_left" | "right-to-left" => "right_to_left".to_string(),
        "vertical" | "ttb" | "vertical_rl" => "vertical_rl".to_string(),
        _ => "left_to_right".to_string(),
    }
}

fn page_bounds(input: &[u8], page: usize) -> Result<[f64; 4]> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    Ok(engine.page_box(page).unwrap_or([0.0, 0.0, 612.0, 792.0]))
}

fn sanitize_region(region: [f64; 4]) -> Result<[f64; 4]> {
    if region.iter().any(|v| !v.is_finite()) {
        return Err(WellfriendError::MalformedPdf(
            "prompt33 region geometry contains non-finite coordinates".to_string(),
        ));
    }
    let x0 = region[0].min(region[2]);
    let x1 = region[0].max(region[2]);
    let y0 = region[1].min(region[3]);
    let y1 = region[1].max(region[3]);
    if (x1 - x0) <= 0.0 || (y1 - y0) <= 0.0 {
        return Err(WellfriendError::MalformedPdf(
            "prompt33 region geometry has zero area".to_string(),
        ));
    }
    Ok([x0, y0, x1, y1])
}

fn region_for_request(input: &[u8], request: &GeometricReflowRequest) -> Result<[f64; 4]> {
    if let Some(region) = request.region {
        sanitize_region(region)
    } else {
        let page = page_bounds(input, request.page)?;
        Ok([
            page[0] + 36.0,
            page[1] + 36.0,
            page[2] - 36.0,
            (page[1] + page[3]) / 2.0,
        ])
    }
}

fn allowed_expansion_for_request(
    input: &[u8],
    request: &GeometricReflowRequest,
    source_region: [f64; 4],
) -> Result<Option<[f64; 4]>> {
    let Some(expansion) = request.allowed_expansion_region else {
        return Ok(None);
    };
    let expansion = sanitize_region(expansion)?;
    let page = page_bounds(input, request.page)?;
    let contains_source = expansion[0] <= source_region[0]
        && expansion[1] <= source_region[1]
        && expansion[2] >= source_region[2]
        && expansion[3] >= source_region[3];
    let inside_page = expansion[0] >= page[0]
        && expansion[1] >= page[1]
        && expansion[2] <= page[2]
        && expansion[3] <= page[3];
    if !contains_source || !inside_page {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt33 constraint_infeasible: allowed expansion must contain the source region and remain inside the page box"
                .to_string(),
        ));
    }
    Ok(Some(expansion))
}

fn line_break_with_ordered_local_expansion(
    input: &[u8],
    request: &GeometricReflowRequest,
    source_region: [f64; 4],
) -> Result<LineBreakingResult> {
    let mut initial = line_break_text(
        &request.replacement_text,
        source_region[2] - source_region[0],
        source_region[3] - source_region[1],
        request.line_height,
        request.language.as_deref(),
        request.direction.as_deref(),
        request.hyphenation,
    )?;
    if initial.overflow_status != OverflowStatus::UnresolvedOverflow {
        return Ok(initial);
    }
    let Some(expansion) = allowed_expansion_for_request(input, request, source_region)? else {
        return Ok(initial);
    };
    let mut expanded = line_break_text(
        &request.replacement_text,
        expansion[2] - expansion[0],
        expansion[3] - expansion[1],
        request.line_height,
        request.language.as_deref(),
        request.direction.as_deref(),
        request.hyphenation,
    )?;
    if expanded.overflow_status == OverflowStatus::FitInRegion {
        expanded.overflow_status = OverflowStatus::FitAfterRegionExpansion;
        expanded.exact_limits.push(format!(
            "ordered_overflow_stage=explicit_allowed_region_expansion; source_region={source_region:?}; expansion_region={expansion:?}"
        ));
        return Ok(expanded);
    }
    initial.exact_limits.push(format!(
        "ordered_overflow_stage=explicit_allowed_region_expansion_attempted_but_insufficient; expansion_region={expansion:?}"
    ));
    Ok(initial)
}

fn effective_region_for_report(
    input: &[u8],
    request: &GeometricReflowRequest,
    overflow_status: OverflowStatus,
) -> Result<[f64; 4]> {
    let source = region_for_request(input, request)?;
    if overflow_status == OverflowStatus::FitAfterRegionExpansion {
        return allowed_expansion_for_request(input, request, source)?.ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "prompt33 expanded layout lacks an explicit validated expansion region".to_string(),
            )
        });
    }
    Ok(source)
}

const MAX_FINAL_LAYOUT_CANDIDATE_SPANS: usize = 2_048;

fn shaping_mode(direction: &str) -> AdvancedTextMode {
    match direction {
        "right_to_left" => AdvancedTextMode::ParagraphReflowRtl,
        "vertical_rl" => AdvancedTextMode::ParagraphReflowVertical,
        _ => AdvancedTextMode::ParagraphReflowHorizontal,
    }
}

/// Measure the exact shaped advance used by the Prompt 20 generated-Type0
/// writer.  This is deliberately per candidate span: contextual scripts are
/// shaped in their final candidate-line context instead of summing isolated
/// characters.
fn shaped_advance(text: &str, direction: &str, font_size: f64) -> Result<f64> {
    let analysis = crate::prompt20::analyze_advanced_text_reflow(
        text,
        shaping_mode(direction),
        None,
        crate::prompt20::TextReflowLimits::default(),
    )?;
    if !analysis.missing_glyph_clusters.is_empty() {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt33 shaping_failed: selected canonical generated font lacks one or more candidate clusters"
                .to_string(),
        ));
    }
    Ok(analysis
        .glyphs
        .iter()
        .map(|glyph| glyph.advance_1000.abs())
        .sum::<f64>()
        / 1000.0
        * font_size)
}

fn grapheme_boundaries(text: &str) -> Vec<usize> {
    let mut boundaries = text
        .grapheme_indices(true)
        .map(|(offset, _)| offset)
        .collect::<Vec<_>>();
    if boundaries.first().copied() != Some(0) {
        boundaries.insert(0, 0);
    }
    if boundaries.last().copied() != Some(text.len()) {
        boundaries.push(text.len());
    }
    boundaries
}

fn scalar_offset_for_byte(text: &str, byte_offset: usize) -> usize {
    text.get(..byte_offset)
        .map(|prefix| prefix.chars().count())
        .unwrap_or(0)
}

fn paragraph_source_style_runs(input: &[u8], request: &GeometricReflowRequest) -> Vec<Value> {
    if request.source_text.is_empty() {
        return Vec::new();
    }
    let Ok(model) = crate::prompt20::analyze_multi_run_text_range(input, request.page) else {
        return Vec::new();
    };
    if model.logical_text.matches(&request.source_text).count() != 1 {
        return Vec::new();
    }
    let Some(byte_start) = model.logical_text.find(&request.source_text) else {
        return Vec::new();
    };
    let scalar_start = scalar_offset_for_byte(&model.logical_text, byte_start);
    let scalar_end = scalar_start + request.source_text.chars().count();
    let selected_text = &model.logical_text[byte_start..byte_start + request.source_text.len()];
    let grapheme_offsets = selected_text
        .grapheme_indices(true)
        .map(|(offset, _)| offset)
        .collect::<Vec<_>>();
    model
        .source_spans
        .iter()
        .filter(|span| {
            span.logical_range[0] < scalar_end && span.logical_range[1] > scalar_start
        })
        .map(|span| {
            let overlap_start = span.logical_range[0].max(scalar_start) - scalar_start;
            let overlap_end = span.logical_range[1].min(scalar_end) - scalar_start;
            let grapheme_start = grapheme_offsets
                .iter()
                .filter(|offset| scalar_offset_for_byte(selected_text, **offset) < overlap_start)
                .count();
            let grapheme_end = grapheme_offsets
                .iter()
                .filter(|offset| scalar_offset_for_byte(selected_text, **offset) < overlap_end)
                .count();
            json!({
                "style_run_id": stable_id("style-run", &[span.span_id.as_bytes(), request.source_text.as_bytes()]),
                "source_span_id": span.span_id,
                "stream": {"object": span.stream_object, "generation": span.stream_generation},
                "operator": span.operator,
                "tj_element": span.tj_element,
                "font_identity": {"resource": span.font_resource, "preservation_status": if request.font_policy == "preserve_original_per_run" { "replayed_by_canonical_preserve_per_segment_serializer_when_eligible" } else { "generated_type0_output" }},
                "unicode_scalar_range": [span.logical_range[0], span.logical_range[1]],
                "selected_grapheme_range": [grapheme_start, grapheme_end],
                "marked_content_depth": span.marked_content_depth,
                "writing_mode": span.writing_mode,
                "source_text_hash": digest_hex(span.text.as_bytes()),
                "evidence": Prompt33EvidenceKind::ExactSourceFact,
                "exact_limits": ["preserve_original_per_run replays font resource, size, DeviceGray/RGB/CMYK paint state, text rendering mode, spacing, horizontal scaling, and rise for horizontal source selections. Changed-length text assigns each complete replacement grapheme to a deterministic proportional source-style owner, preserving style order without flattening or splitting a grapheme. One text-state-only MCID BDC containing exactly the selected source spans is relocated with its original identity while the empty source wrapper becomes Artifact; links, nested/partial tagged content, arbitrary color spaces, vertical writing, and bidi edits fail closed"],
            })
        })
        .collect()
}

fn uax14_break_records(text: &str) -> Vec<LineBreakRecord> {
    let boundaries = grapheme_boundaries(text);
    let opportunities = linebreaks(text).collect::<Vec<_>>();
    boundaries
        .iter()
        .copied()
        .skip(1)
        .map(|offset| {
            let grapheme_index = boundaries.iter().position(|boundary| *boundary == offset);
            let opportunity = opportunities
                .iter()
                .find(|(candidate, _)| *candidate == offset)
                .map(|(_, opportunity)| *opportunity);
            let previous = text[..offset].chars().next_back();
            let soft_hyphen = previous == Some('\u{00ad}');
            let disposition = match opportunity {
                Some(BreakOpportunity::Mandatory) => "mandatory",
                Some(BreakOpportunity::Allowed) => "optional",
                None => "prohibited",
            };
            LineBreakRecord {
                logical_offset_utf8: offset,
                grapheme_index,
                shaping_cluster_utf8: Some(offset),
                source_location: format!("replacement:utf8:{offset}"),
                break_class: previous
                    .map(|character| format!("{:?}", break_property(character as u32)))
                    .unwrap_or_else(|| "StartOfText".to_string()),
                disposition: disposition.to_string(),
                penalty: match opportunity {
                    Some(BreakOpportunity::Mandatory) => -10_000,
                    Some(BreakOpportunity::Allowed) if soft_hyphen => 100,
                    Some(BreakOpportunity::Allowed) => 0,
                    None => i64::MAX,
                },
                hyphenation_source: if soft_hyphen {
                    "source_soft_hyphen".to_string()
                } else {
                    "none".to_string()
                },
                inserted_visual_glyph_behavior: if soft_hyphen {
                    "source_soft_hyphen_visible_only_at_selected_break".to_string()
                } else {
                    "no_inserted_glyph".to_string()
                },
                extraction_behavior: if soft_hyphen {
                    "preserve_source_soft_hyphen".to_string()
                } else {
                    "preserve_exact_source_text".to_string()
                },
                source_output_supported: !soft_hyphen,
                confidence: Prompt33EvidenceKind::DeterministicGeometry,
                reason: "unicode_linebreak_uax14_unicode_15_default_tailoring".to_string(),
            }
        })
        .collect()
}

const HYPHENATION_PROVIDER: &str = "hyphenation-0.8.4-knuth-liang";
const HYPHENATION_MAX_WORD_GRAPHEMES: usize = 64;
const HYPHENATION_MIN_WORD_GRAPHEMES: usize = 5;
const HYPHENATION_MIN_PREFIX_GRAPHEMES: usize = 2;
const HYPHENATION_MIN_SUFFIX_GRAPHEMES: usize = 2;
/// A dictionary break paints a real visual hyphen.  Keep runs of those breaks
/// bounded in both the interactive preview and the final optimizer so a
/// narrow region cannot produce a visually distracting or semantically
/// surprising ladder of generated hyphens.
const HYPHENATION_MAX_CONSECUTIVE_HYPHENATED_LINES: usize = 2;

static EN_US_HYPHENATOR: OnceLock<Standard> = OnceLock::new();
static ES_HYPHENATOR: OnceLock<Standard> = OnceLock::new();

#[derive(Debug, Clone)]
struct HyphenationPlan {
    report: Value,
    candidates: Vec<LineBreakRecord>,
}

fn normalize_bcp47(language: Option<&str>) -> Option<String> {
    let raw = language?.trim().to_ascii_lowercase().replace('_', "-");
    (!raw.is_empty()).then_some(raw)
}

/// Prompt 33 intentionally ships only two independently reviewed pattern
/// families.  Locale fallback is explicit and stable; no language ever falls
/// back to English merely because it is Latin script.
fn hyphenation_language(language: Option<&str>) -> Option<(Language, &'static str, &'static str)> {
    match normalize_bcp47(language).as_deref() {
        Some("en") | Some("en-us") => Some((
            Language::EnglishUS,
            "en-us",
            "exact_or_primary_subtag_fallback",
        )),
        Some(tag) if tag.starts_with("en-") => Some((
            Language::EnglishUS,
            "en-us",
            "documented_en_to_en_us_fallback",
        )),
        Some("es") | Some("es-es") => {
            Some((Language::Spanish, "es", "exact_or_primary_subtag_fallback"))
        }
        Some(tag) if tag.starts_with("es-") => {
            Some((Language::Spanish, "es", "documented_es_to_es_fallback"))
        }
        _ => None,
    }
}

fn load_hyphenator(language: Language) -> Result<&'static Standard> {
    let cell = match language {
        Language::EnglishUS => &EN_US_HYPHENATOR,
        Language::Spanish => &ES_HYPHENATOR,
        _ => {
            return Err(WellfriendError::UnsupportedFeature(
                "prompt33 hyphenation_unavailable: no audited embedded dictionary for requested language"
                    .to_string(),
            ));
        }
    };
    Ok(cell.get_or_init(|| {
        Standard::from_embedded(language).expect(
            "Prompt33 embeds only audited hyphenation dictionaries selected at compile time",
        )
    }))
}

fn whitespace_segment(text: &str, offset: usize) -> &str {
    let start = text[..offset]
        .char_indices()
        .rev()
        .find(|(_, ch)| ch.is_whitespace())
        .map(|(index, ch)| index + ch.len_utf8())
        .unwrap_or(0);
    let end = text[offset..]
        .char_indices()
        .find(|(_, ch)| ch.is_whitespace())
        .map(|(index, _)| offset + index)
        .unwrap_or(text.len());
    &text[start..end]
}

fn dictionary_hyphenation_plan(
    text: &str,
    language: Option<&str>,
    enabled: bool,
) -> HyphenationPlan {
    if !enabled {
        return HyphenationPlan {
            report: json!({
                "enabled": false,
                "requested": false,
                "policy": "disabled",
                "provider": HYPHENATION_PROVIDER,
                "unknown_language_not_guessed": true,
            }),
            candidates: Vec::new(),
        };
    }
    let Some((language_id, resolved_tag, fallback)) = hyphenation_language(language) else {
        return HyphenationPlan {
            report: json!({
                "enabled": false,
                "requested": true,
                "requested_language": language.unwrap_or("und"),
                "policy": "hyphenation_unavailable",
                "typed_result": "hyphenation_unavailable",
                "provider": HYPHENATION_PROVIDER,
                "unknown_language_not_guessed": true,
            }),
            candidates: Vec::new(),
        };
    };
    let Ok(dictionary) = load_hyphenator(language_id) else {
        return HyphenationPlan {
            report: json!({
                "enabled": false,
                "requested": true,
                "requested_language": language.unwrap_or("und"),
                "resolved_language": resolved_tag,
                "policy": "hyphenation_unavailable",
                "typed_result": "hyphenation_unavailable",
                "provider": HYPHENATION_PROVIDER,
                "unknown_language_not_guessed": true,
            }),
            candidates: Vec::new(),
        };
    };
    let grapheme_offsets = grapheme_boundaries(text);
    let mut candidates = Vec::new();
    let mut word_start = None::<usize>;
    let mut visit_word = |start: usize, end: usize| {
        let word = &text[start..end];
        let word_graphemes = word.graphemes(true).count();
        if !(HYPHENATION_MIN_WORD_GRAPHEMES..=HYPHENATION_MAX_WORD_GRAPHEMES)
            .contains(&word_graphemes)
        {
            return;
        }
        let surrounding = whitespace_segment(text, start);
        if surrounding.contains("://") || surrounding.contains('@') || surrounding.contains('.') {
            return;
        }
        for byte_in_word in dictionary.hyphenate(word).breaks {
            let offset = start.saturating_add(byte_in_word);
            if !text.is_char_boundary(offset) || !grapheme_offsets.contains(&offset) {
                continue;
            }
            let prefix = text[start..offset].graphemes(true).count();
            let suffix = text[offset..end].graphemes(true).count();
            if prefix < HYPHENATION_MIN_PREFIX_GRAPHEMES
                || suffix < HYPHENATION_MIN_SUFFIX_GRAPHEMES
            {
                continue;
            }
            let grapheme_index = grapheme_offsets.iter().position(|item| *item == offset);
            candidates.push(LineBreakRecord {
                logical_offset_utf8: offset,
                grapheme_index,
                shaping_cluster_utf8: Some(offset),
                source_location: format!("replacement:utf8:{offset}"),
                break_class: "dictionary_hyphenation".to_string(),
                disposition: "optional".to_string(),
                penalty: 100,
                hyphenation_source: format!("dictionary:{resolved_tag}"),
                inserted_visual_glyph_behavior: "visible_end_of_line_hyphen_with_empty_tounicode_mapping".to_string(),
                extraction_behavior: "logical_source_text_must_remain_unchanged".to_string(),
                source_output_supported: true,
                confidence: Prompt33EvidenceKind::DeterministicGeometry,
                reason: format!(
                    "{HYPHENATION_PROVIDER}; language={resolved_tag}; min_word={HYPHENATION_MIN_WORD_GRAPHEMES}; min_prefix={HYPHENATION_MIN_PREFIX_GRAPHEMES}; min_suffix={HYPHENATION_MIN_SUFFIX_GRAPHEMES}"
                ),
            });
        }
    };
    for (offset, ch) in text.char_indices() {
        if ch.is_alphabetic() {
            word_start.get_or_insert(offset);
        } else if let Some(start) = word_start.take() {
            visit_word(start, offset);
        }
    }
    if let Some(start) = word_start {
        visit_word(start, text.len());
    }
    candidates.sort_by_key(|candidate| {
        (
            candidate.logical_offset_utf8,
            candidate.hyphenation_source.clone(),
        )
    });
    candidates.dedup_by_key(|candidate| candidate.logical_offset_utf8);
    HyphenationPlan {
        report: json!({
            "enabled": true,
            "requested": true,
            "requested_language": language.unwrap_or("und"),
            "resolved_language": resolved_tag,
            "locale_fallback": fallback,
            "provider": HYPHENATION_PROVIDER,
            "provider_license": "Apache-2.0 OR MIT",
            "data": [
                {"language": "en-us", "version": "hyph-utf8 2005-05-30", "license": "redistribution and modification permitted with retained notices"},
                {"language": "es", "version": "tex-hyphen-spanish 5.0 pattern source", "license": "MIT"}
            ],
            "minimum_word_graphemes": HYPHENATION_MIN_WORD_GRAPHEMES,
            "minimum_prefix_graphemes": HYPHENATION_MIN_PREFIX_GRAPHEMES,
            "minimum_suffix_graphemes": HYPHENATION_MIN_SUFFIX_GRAPHEMES,
            "maximum_consecutive_hyphenated_lines": HYPHENATION_MAX_CONSECUTIVE_HYPHENATED_LINES,
            "consecutive_hyphenated_lines_enforced_by": "greedy_preview_and_final_dynamic_optimizer",
            "candidate_count": candidates.len(),
            "cache_key": format!("{HYPHENATION_PROVIDER}:{resolved_tag}"),
            "output_application": "canonical_generated_type0_visual_hyphen_with_empty_tounicode_mapping",
        }),
        candidates,
    }
}

fn usable_break(record: &LineBreakRecord) -> bool {
    record.disposition != "prohibited" && record.source_output_supported
}

fn line_record_at(records: &[LineBreakRecord], end: usize) -> Option<&LineBreakRecord> {
    records
        .iter()
        .filter(|record| record.grapheme_index == Some(end))
        .find(|record| usable_break(record))
        .or_else(|| {
            records
                .iter()
                .find(|record| record.grapheme_index == Some(end))
        })
}

fn mandatory_boundary_after(records: &[LineBreakRecord], start: usize) -> Option<usize> {
    records
        .iter()
        .filter(|record| record.disposition == "mandatory")
        .filter_map(|record| record.grapheme_index)
        .find(|end| *end > start)
}

fn range_advance(
    graphemes: &[&str],
    start: usize,
    end: usize,
    direction: &str,
    font_size: f64,
    cache: &mut BTreeMap<(usize, usize), f64>,
    measured_spans: &mut usize,
) -> Result<f64> {
    if let Some(advance) = cache.get(&(start, end)) {
        return Ok(*advance);
    }
    if *measured_spans >= MAX_FINAL_LAYOUT_CANDIDATE_SPANS {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "prompt33 resource_limit_exceeded: final layout candidate span limit {} reached",
            MAX_FINAL_LAYOUT_CANDIDATE_SPANS
        )));
    }
    *measured_spans += 1;
    let advance = shaped_advance(&graphemes[start..end].join(""), direction, font_size)?;
    cache.insert((start, end), advance);
    Ok(advance)
}

#[allow(clippy::too_many_arguments)] // Keeps shaped-metric cache ownership explicit at the DP boundary.
fn candidate_advance(
    graphemes: &[&str],
    start: usize,
    end: usize,
    record: Option<&LineBreakRecord>,
    direction: &str,
    font_size: f64,
    cache: &mut BTreeMap<(usize, usize), f64>,
    measured_spans: &mut usize,
) -> Result<f64> {
    let mut visual = graphemes[start..end].join("");
    if record.is_some_and(is_mandatory_line_separator) {
        visual = strip_trailing_line_separators(&visual).to_string();
    }
    if record.is_some_and(|item| item.hyphenation_source.starts_with("dictionary:")) {
        if *measured_spans >= MAX_FINAL_LAYOUT_CANDIDATE_SPANS {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "prompt33 resource_limit_exceeded: final layout candidate span limit {} reached",
                MAX_FINAL_LAYOUT_CANDIDATE_SPANS
            )));
        }
        *measured_spans += 1;
        return shaped_advance(&format!("{visual}-"), direction, font_size);
    }
    if record.is_some_and(is_mandatory_line_separator) {
        if *measured_spans >= MAX_FINAL_LAYOUT_CANDIDATE_SPANS {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "prompt33 resource_limit_exceeded: final layout candidate span limit {} reached",
                MAX_FINAL_LAYOUT_CANDIDATE_SPANS
            )));
        }
        *measured_spans += 1;
        return shaped_advance(&visual, direction, font_size);
    }
    range_advance(
        graphemes,
        start,
        end,
        direction,
        font_size,
        cache,
        measured_spans,
    )
}

fn is_mandatory_line_separator(record: &LineBreakRecord) -> bool {
    record.disposition == "mandatory"
}

fn strip_trailing_line_separators(text: &str) -> &str {
    text.trim_end_matches(['\r', '\n', '\u{0085}', '\u{2028}', '\u{2029}'])
}

fn greedy_line_ranges(
    graphemes: &[&str],
    records: &[LineBreakRecord],
    region_width: f64,
    direction: &str,
    font_size: f64,
    cache: &mut BTreeMap<(usize, usize), f64>,
    measured_spans: &mut usize,
) -> Result<Vec<(usize, usize)>> {
    let mut ranges = Vec::new();
    let mut start = 0usize;
    let mut consecutive_hyphenated_lines = 0usize;
    while start < graphemes.len() {
        let mandatory_end = mandatory_boundary_after(records, start).unwrap_or(graphemes.len());
        let mut selected = None::<(usize, bool)>;
        for end in start + 1..=mandatory_end {
            let record = line_record_at(records, end);
            if candidate_advance(
                graphemes,
                start,
                end,
                record,
                direction,
                font_size,
                cache,
                measured_spans,
            )? > region_width
            {
                break;
            }
            if record.is_some_and(usable_break) {
                let dictionary_hyphen =
                    record.is_some_and(|item| item.hyphenation_source.starts_with("dictionary:"));
                if !dictionary_hyphen
                    || consecutive_hyphenated_lines < HYPHENATION_MAX_CONSECUTIVE_HYPHENATED_LINES
                {
                    selected = Some((end, dictionary_hyphen));
                }
            }
        }
        let (end, dictionary_hyphen) = selected.unwrap_or((mandatory_end, false));
        ranges.push((start, end));
        consecutive_hyphenated_lines = if dictionary_hyphen {
            consecutive_hyphenated_lines.saturating_add(1)
        } else {
            0
        };
        start = end;
    }
    Ok(ranges)
}

type LineRange = (usize, usize);
type OptimizedLineRanges = Option<(Vec<LineRange>, f64)>;

fn optimized_line_ranges(
    graphemes: &[&str],
    records: &[LineBreakRecord],
    region_width: f64,
    direction: &str,
    font_size: f64,
    cache: &mut BTreeMap<(usize, usize), f64>,
    measured_spans: &mut usize,
) -> Result<OptimizedLineRanges> {
    let count = graphemes.len();
    // The state carries the trailing generated-hyphen count rather than
    // treating it as report-only policy.  This is a bounded DAG: position
    // always increases and the secondary state has exactly three values.
    let mut best =
        vec![
            vec![None::<(f64, usize, usize)>; HYPHENATION_MAX_CONSECUTIVE_HYPHENATED_LINES + 1];
            count + 1
        ];
    best[0][0] = Some((0.0, 0, 0));
    for start in 0..count {
        if start != 0
            && line_record_at(records, start)
                .is_none_or(|record| record.disposition == "prohibited")
        {
            continue;
        }
        let mandatory_end = mandatory_boundary_after(records, start).unwrap_or(count);
        for prior_hyphen_count in 0..=HYPHENATION_MAX_CONSECUTIVE_HYPHENATED_LINES {
            let Some((prior_cost, _, _)) = best[start][prior_hyphen_count] else {
                continue;
            };
            for (end, best_at_end) in best
                .iter_mut()
                .enumerate()
                .take(mandatory_end + 1)
                .skip(start + 1)
            {
                let Some(record) = line_record_at(records, end) else {
                    continue;
                };
                if !usable_break(record) {
                    continue;
                }
                let advance = candidate_advance(
                    graphemes,
                    start,
                    end,
                    Some(record),
                    direction,
                    font_size,
                    cache,
                    measured_spans,
                )?;
                if advance > region_width {
                    break;
                }
                let dictionary_hyphen = record.hyphenation_source.starts_with("dictionary:");
                let next_hyphen_count = if dictionary_hyphen {
                    prior_hyphen_count.saturating_add(1)
                } else {
                    0
                };
                if next_hyphen_count > HYPHENATION_MAX_CONSECUTIVE_HYPHENATED_LINES {
                    continue;
                }
                let slack = (region_width - advance).max(0.0);
                let line_cost = if end == count { 0.0 } else { slack * slack };
                let candidate = prior_cost + line_cost + record.penalty.max(0) as f64;
                let replace = best_at_end[next_hyphen_count].is_none_or(
                    |(current_cost, current_start, current_hyphen_count)| {
                        candidate < current_cost - 1e-9
                            || ((candidate - current_cost).abs() <= 1e-9
                                && (start < current_start
                                    || (start == current_start
                                        && prior_hyphen_count < current_hyphen_count)))
                    },
                );
                if replace {
                    best_at_end[next_hyphen_count] = Some((candidate, start, prior_hyphen_count));
                }
            }
        }
    }
    let Some((cost, mut hyphen_count)) = (0..=HYPHENATION_MAX_CONSECUTIVE_HYPHENATED_LINES)
        .filter_map(|candidate_hyphen_count| {
            best[count][candidate_hyphen_count]
                .map(|(candidate_cost, _, _)| (candidate_cost, candidate_hyphen_count))
        })
        .min_by(
            |(left_cost, left_hyphen_count), (right_cost, right_hyphen_count)| {
                left_cost
                    .partial_cmp(right_cost)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| left_hyphen_count.cmp(right_hyphen_count))
            },
        )
    else {
        return Ok(None);
    };
    let mut ranges = Vec::new();
    let mut end = count;
    while end > 0 {
        let Some((_, start, previous_hyphen_count)) = best[end][hyphen_count] else {
            return Ok(None);
        };
        ranges.push((start, end));
        end = start;
        hyphen_count = previous_hyphen_count;
    }
    ranges.reverse();
    Ok(Some((ranges, cost)))
}

#[allow(clippy::too_many_arguments)] // Call-site keeps line context and mutable shaped-metric cache explicit.
fn layout_lines_from_ranges(
    text: &str,
    graphemes: &[&str],
    ranges: &[(usize, usize)],
    records: &[LineBreakRecord],
    direction: &str,
    line_height: f64,
    font_size: f64,
    cache: &mut BTreeMap<(usize, usize), f64>,
    measured_spans: &mut usize,
) -> Result<Vec<LayoutLine>> {
    ranges
        .iter()
        .enumerate()
        .map(|(index, (start, end))| -> Result<LayoutLine> {
            let line_text = graphemes[*start..*end].join("");
            let break_record = line_record_at(records, *end);
            let visual_text = if break_record.is_some_and(is_mandatory_line_separator) {
                strip_trailing_line_separators(&line_text).to_string()
            } else {
                line_text.clone()
            };
            let hyphen_inserted = break_record
                .is_some_and(|record| record.hyphenation_source.starts_with("dictionary:"));
            let bidi = BidiInfo::new(&visual_text, None);
            let mut visual_order = Vec::new();
            for paragraph in &bidi.paragraphs {
                let (_, ranges) = bidi.visual_runs(paragraph, paragraph.range.clone());
                visual_order.extend(0..ranges.len());
            }
            Ok(LayoutLine {
                line_id: stable_id(
                    "line",
                    &[text.as_bytes(), &start.to_le_bytes(), &end.to_le_bytes()],
                ),
                text: line_text,
                visual_text,
                grapheme_range: [*start, *end],
                advance: candidate_advance(
                    graphemes,
                    *start,
                    *end,
                    break_record,
                    direction,
                    font_size,
                    cache,
                    measured_spans,
                )?,
                baseline: line_height * (index + 1) as f64,
                bidi_visual_order: visual_order,
                hyphen_inserted,
                source_link_status: Prompt33EvidenceKind::DeterministicGeometry,
            })
        })
        .collect()
}

pub fn line_break_text(
    text: &str,
    region_width: f64,
    region_height: f64,
    line_height: f64,
    language: Option<&str>,
    direction: Option<&str>,
    hyphenation: bool,
) -> Result<LineBreakingResult> {
    if !region_width.is_finite()
        || !region_height.is_finite()
        || !line_height.is_finite()
        || region_width <= 0.0
        || region_height <= 0.0
        || line_height <= 0.0
    {
        return Err(WellfriendError::MalformedPdf(
            "prompt33 line layout received invalid dimensions".to_string(),
        ));
    }
    let direction = direction_label(direction);
    let font_size = line_height / 1.2;
    let graphemes = text.graphemes(true).collect::<Vec<_>>();
    let mut break_records = uax14_break_records(text);
    let hyphenation_plan = dictionary_hyphenation_plan(text, language, hyphenation);
    break_records.extend(hyphenation_plan.candidates);
    break_records.sort_by_key(|record| {
        (
            record.logical_offset_utf8,
            !record.source_output_supported,
            record.hyphenation_source.clone(),
        )
    });
    let mut measured_spans = 0usize;
    let mut advances = BTreeMap::new();
    let preview_ranges = if graphemes.is_empty() {
        vec![(0, 0)]
    } else {
        greedy_line_ranges(
            &graphemes,
            &break_records,
            region_width,
            &direction,
            font_size,
            &mut advances,
            &mut measured_spans,
        )?
    };
    let optimized = if graphemes.is_empty() {
        Some((vec![(0, 0)], 0.0))
    } else {
        optimized_line_ranges(
            &graphemes,
            &break_records,
            region_width,
            &direction,
            font_size,
            &mut advances,
            &mut measured_spans,
        )?
    };
    let (final_ranges, final_cost, optimization_succeeded) = optimized
        .map(|(ranges, cost)| (ranges, cost, true))
        .unwrap_or_else(|| (preview_ranges.clone(), f64::INFINITY, false));
    let preview_lines = layout_lines_from_ranges(
        text,
        &graphemes,
        &preview_ranges,
        &break_records,
        &direction,
        line_height,
        font_size,
        &mut advances,
        &mut measured_spans,
    )?;
    let lines = layout_lines_from_ranges(
        text,
        &graphemes,
        &final_ranges,
        &break_records,
        &direction,
        line_height,
        font_size,
        &mut advances,
        &mut measured_spans,
    )?;
    let used_height = lines.len() as f64 * line_height;
    let width_overflow = lines
        .iter()
        .map(|line| (line.advance - region_width).max(0.0))
        .fold(0.0_f64, f64::max);
    let height_overflow = (used_height - region_height).max(0.0);
    // The report has one scalar overflow field for backward-compatible public
    // serialization.  Its value is the largest violated dimension; the
    // refusal path never attempts to emit a line that this preflight found to
    // exceed the region.
    let overflow_amount = width_overflow.max(height_overflow);
    let overflow_status =
        if !optimization_succeeded || width_overflow > 0.0 || height_overflow > 0.0 {
            OverflowStatus::UnresolvedOverflow
        } else if overflow_amount == 0.0 {
            OverflowStatus::FitInRegion
        } else {
            OverflowStatus::UnresolvedOverflow
        };
    Ok(LineBreakingResult {
        schema_version: PROMPT33_SCHEMA_VERSION.to_string(),
        pipeline: vec![
            "explicit_normalization_policy".into(),
            "unicode_grapheme_segmentation".into(),
            "script_language_run_resolution".into(),
            "unicode_bidi_resolution".into(),
            "uax14_unicode_linebreak_opportunities".into(),
            "language_hyphenation_provider_candidates".into(),
            "prompt20_rustybuzz_shaped_candidate_metrics".into(),
            "greedy_preview".into(),
            "bounded_dynamic_final_layout".into(),
            "source_linked_line_records".into(),
        ],
        preview_algorithm: "deterministic_greedy_uax14_grapheme_safe".into(),
        final_algorithm: "bounded_dynamic_programming_over_uax14_grapheme_safe_candidates".into(),
        preview_lines,
        lines,
        break_records,
        final_cost,
        overflow_status,
        overflow_amount,
        grapheme_safe: true,
        bidi_source_visual_separated: true,
        hyphenation: hyphenation_plan.report,
        justification: json!({
            "latin": "space_distribution_with_bounds",
            "arabic": "shape_context_preserved_kashida_reported_only_when_supported",
            "cjk": "punctuation_and_spacing_constraints_reported",
            "unsafe_universal_scaling": false,
        }),
        exact_limits: vec![
            "supported dictionary candidates are source-linked and grapheme-safe; the generated-Type0 writer paints an end-of-line hyphen with an empty ToUnicode mapping so logical extraction remains unchanged; source soft-hyphen serialization remains refused".into(),
            format!(
                "candidate spans are bounded to {}; exceeded budgets return resource_limit_exceeded",
                MAX_FINAL_LAYOUT_CANDIDATE_SPANS
            ),
            format!(
                "greedy preview and final dynamic layout enforce at most {} consecutive generated dictionary-hyphen lines; no legal sequence yields unresolved overflow",
                HYPHENATION_MAX_CONSECUTIVE_HYPHENATED_LINES
            ),
            "the preflight refuses height or width overflow; it never asks the writer to clip an over-wide unbreakable line".into(),
        ],
    })
}

pub fn analyze_geometric_region(
    input: &[u8],
    request: &GeometricReflowRequest,
) -> Result<GeometricTextRegion> {
    let graph = build_scene_graph(input, &[request.page])?;
    let region = region_for_request(input, request)?;
    let page_box = page_bounds(input, request.page)?;
    let provenance = operator_text_provenance(
        input,
        request.page,
        &request.source_text,
        &request.replacement_text,
    )
    .ok();
    let mut source_instructions = provenance
        .as_ref()
        .map(|report| {
            report
                .source_instructions
                .iter()
                .map(|item| item.instruction_id.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if source_instructions.is_empty() && request.font_policy == "preserve_original_per_run" {
        if let Ok(model) = analyze_multi_run_text_range(input, request.page) {
            if let Ok([start, end]) = unique_scalar_range(&model.logical_text, &request.source_text)
            {
                let selected = model
                    .source_spans
                    .iter()
                    .filter(|span| span.logical_range[0] >= start && span.logical_range[1] <= end)
                    .collect::<Vec<_>>();
                if selected
                    .first()
                    .is_some_and(|span| span.logical_range[0] == start)
                    && selected
                        .last()
                        .is_some_and(|span| span.logical_range[1] == end)
                {
                    source_instructions = selected
                        .into_iter()
                        .map(|span| {
                            format!(
                                "multirun:p{}:o{}g{}:{}..{}",
                                request.page,
                                span.stream_object,
                                span.stream_generation,
                                span.byte_range[0],
                                span.byte_range[1]
                            )
                        })
                        .collect();
                }
            }
        }
    }
    let source_mapping_resolved = !source_instructions.is_empty();
    let source_scene_nodes = graph
        .nodes
        .iter()
        .filter(|node| node.page == request.page)
        .map(|node| node.node_id.clone())
        .take(16)
        .collect::<Vec<_>>();
    let region_id = stable_id(
        "geometric-region",
        &[
            input,
            &request.page.to_le_bytes(),
            request.source_text.as_bytes(),
            request.replacement_text.as_bytes(),
        ],
    );
    Ok(GeometricTextRegion {
        schema_version: PROMPT33_SCHEMA_VERSION.to_string(),
        region_id: region_id.clone(),
        source_scene_nodes,
        source_semantic_nodes: vec![stable_id("semantic-node", &[region_id.as_bytes()])],
        source_instructions,
        page_id: request.page,
        page_box,
        writing_mode: if direction_label(request.direction.as_deref()) == "vertical_rl" {
            "vertical_rl"
        } else {
            "horizontal_tb"
        }
        .into(),
        base_direction: direction_label(request.direction.as_deref()),
        language: request.language.clone().unwrap_or_else(|| "und".into()),
        polygon_or_rect: vec![
            [region[0], region[1]],
            [region[2], region[1]],
            [region[2], region[3]],
            [region[0], region[3]],
        ],
        padding: [0.0, 0.0, 0.0, 0.0],
        transforms: vec!["page_user_space_after_crop_rotation".into()],
        clipping: "safe_if_source_text_is_not_text_clipping; otherwise refusal".into(),
        style_runs: vec![json!({
            "source": "Prompt32 font identity and scene style summary",
            "font_policy": request.font_policy,
            "evidence": Prompt33EvidenceKind::DeterministicFontShaping,
        })],
        paragraph_ids: vec![stable_id("paragraph", &[region_id.as_bytes()])],
        allowed_expansion_region: request.allowed_expansion_region.unwrap_or(region),
        locked_neighbors: vec!["unknown_objects_locked_by_default".into()],
        // Only callers that name a Prompt20 stable vector identity plus an
        // explicit dependency edge can enter the movement transaction. All
        // other nearby/unknown scene objects remain locked.
        movable_neighbors: request
            .downstream_vector_moves
            .iter()
            .map(|movement| format!("explicit_vector:{}", movement.vector_stable_id))
            .chain(
                request.downstream_link_moves.iter().map(|movement| {
                    format!("explicit_link_annotation:{}", movement.annotation_index)
                }),
            )
            .collect(),
        exclusion_zones: vec![json!({
            "policy": NeighborPolicy::UnknownUnsafe,
            "locked": true,
            "reason": "Prompt33 does not move unknown obstacles",
        })],
        downstream_flow_targets: {
            let mut targets = vec!["same_region".to_string()];
            if request.next_region.is_some() {
                targets.push("explicit_next_region".to_string());
            }
            if request.next_column.is_some() {
                targets.push("explicit_next_column".to_string());
            }
            if !request.downstream_vector_moves.is_empty() {
                targets.push("explicit_dependency_linked_vector_movement".to_string());
            }
            if !request.downstream_link_moves.is_empty() {
                targets.push("explicit_source_link_annotation_movement".to_string());
            }
            targets
        },
        born_digital_ocr_or_inferred: "born_digital_or_inferred_from_source_text_operators".into(),
        confidence: json!({
            "geometry": 0.86,
            "reading_order": 0.82,
            "semantic_type": 0.76,
            "text_mapping": if provenance.is_some() { 0.98 } else if source_mapping_resolved { 0.94 } else { 0.0 },
            "font_identity": 0.80,
            "cross_page_flow": 0.68,
            "overall": if provenance.is_some() { 0.83 } else if source_mapping_resolved { 0.81 } else { 0.42 },
        }),
        edit_policy: json!({
            "requested_mode": request.requested_mode,
            "unknown_objects_locked_by_default": true,
            "font_reduction_not_automatic": true,
            "clipping_never_success": true,
            "overlay_forbidden": true,
            "page_creation_requires_policy": request.allow_page_creation,
        }),
    })
}

pub fn paragraph_style_model(
    input: &[u8],
    request: &GeometricReflowRequest,
) -> Result<ParagraphStyleModel> {
    let region = analyze_geometric_region(input, request)?;
    let source_style_runs = paragraph_source_style_runs(input, request);
    let font_identity =
        text_identity_report(&request.replacement_text, request.direction.as_deref())
            .map(|report| json!(report))
            .unwrap_or_else(|err| {
                json!({
                    "status": "unsupported_exact",
                    "reason": err.to_string(),
                })
            });
    Ok(ParagraphStyleModel {
        schema_version: PROMPT33_SCHEMA_VERSION.to_string(),
        paragraph_id: region.paragraph_ids[0].clone(),
        source_semantic_links: region.source_semantic_nodes.clone(),
        source_provenance_links: region.source_instructions.clone(),
        language: request.language.clone().unwrap_or_else(|| "und".into()),
        script_runs: vec![json!({
            "script": "detected_from_unicode_run_or_user_policy",
            "direction": direction_label(request.direction.as_deref()),
            "evidence": Prompt33EvidenceKind::DeterministicFontShaping,
        })],
        base_direction: direction_label(request.direction.as_deref()),
        writing_mode: region.writing_mode,
        style_runs: if source_style_runs.is_empty() {
            region.style_runs
        } else {
            source_style_runs
        },
        font_identity,
        line_height: request.line_height,
        alignment: "preserve_source_or_left_start_default".into(),
        first_line_indent: 0.0,
        hanging_indent: 0.0,
        start_end_indents: [0.0, 0.0],
        margins: [0.0, 0.0, 0.0, 0.0],
        spacing_before_after: [0.0, 0.0],
        tab_stops: Vec::new(),
        hyphenation_policy: if request.hyphenation {
            "explicit_language_policy".into()
        } else {
            "disabled".into()
        },
        widow_orphan_policy: "reported_not_enforced_for_single_region_preview".into(),
        keep_with_next: false,
        keep_together: false,
        list_relationship: None,
        baseline_grid: None,
        source_region: region.region_id.clone(),
        allowed_expansion_region: "same_as_geometric_region_unless_policy_supplies_expansion"
            .into(),
        locked_neighbors: region.locked_neighbors,
        flow_successor_predecessor: [None, None],
        confidence: region.confidence,
        evidence: vec![
            json!({"kind": Prompt33EvidenceKind::ExactSourceFact, "source": "Prompt31 provenance when source text resolves"}),
            json!({"kind": Prompt33EvidenceKind::HeuristicInference, "source": "paragraph grouping from source-linked scene text"}),
        ],
    })
}

pub fn preview_reflow(
    input: &[u8],
    request: &GeometricReflowRequest,
) -> Result<ReflowTransactionReport> {
    if request.requested_mode == TrueEditingMode::OperatorPreserving {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt33 owns GeometricBlock and SemanticDocument; OperatorPreserving remains Prompt31"
                .to_string(),
        ));
    }
    let snapshot = build_document_snapshot(input, None)?;
    let region = analyze_geometric_region(input, request)?;
    let paragraph = paragraph_style_model(input, request)?;
    let rect = region_for_request(input, request)?;
    let line_breaking = line_break_with_ordered_local_expansion(input, request, rect)?;
    let planned_downstream_vector_moves = validate_downstream_vector_moves(input, request)?;
    let planned_downstream_link_moves = validate_downstream_link_moves(input, request)?;
    let effective_region =
        effective_region_for_report(input, request, line_breaking.overflow_status)?;
    let constraints = constraint_solver_report(&region, request, &line_breaking);
    let semantic_layout = analyze_semantic_layout(input, Some(request))?;
    let eligible = eligibility_modes(&region, &line_breaking, request);
    let confidence = evaluate_reflow_confidence(
        &region.confidence,
        request.requested_mode,
        &ReflowConfidencePolicy::default(),
    );
    let confidence_decision =
        serde_json::from_value::<ConfidenceDecision>(confidence["decision"].clone())
            .unwrap_or(ConfidenceDecision::Refuse);
    let refusal = refusal_for(&region, &line_breaking, &constraints, request).or_else(|| {
        if matches!(
            confidence_decision,
            ConfidenceDecision::ReviewRequired | ConfidenceDecision::Refuse
        ) && !request.approve_low_confidence_structure
        {
            Some(json!({
                "code": if confidence_decision == ConfidenceDecision::Refuse { "refuse" } else { "review_required" },
                "message": "prompt33 confidence policy does not permit an unreviewed apply",
                "confidence": confidence.clone(),
                "no_change_proof": true,
            }))
        } else {
            None
        }
    });
    let applied_mode = if refusal.is_none() {
        Some(request.requested_mode)
    } else {
        None
    };
    let scope = if request.requested_mode == TrueEditingMode::SemanticDocument {
        "semantic_analysis_only_application_refused"
    } else {
        "single_geometric_region_and_declared_flow_linked_neighbors"
    };
    Ok(ReflowTransactionReport {
        schema_version: PROMPT33_SCHEMA_VERSION.to_string(),
        transaction_id: stable_id(
            "reflow-transaction",
            &[
                input,
                request.source_text.as_bytes(),
                request.replacement_text.as_bytes(),
            ],
        ),
        input_snapshot: snapshot,
        requested_mode: request.requested_mode,
        eligible_modes: eligible,
        applied_mode,
        escalation_reason: refusal
            .as_ref()
            .and_then(|value| value.get("recommended_mode").and_then(Value::as_str))
            .map(str::to_string),
        scope_of_movement: scope.into(),
        confidence,
        region,
        paragraph,
        overflow_status: line_breaking.overflow_status,
        line_breaking,
        constraints,
        semantic_layout,
        objects_moved: Vec::new(),
        pages_columns_affected: vec![json!({
            "page": request.page,
            "kind": "local_region",
            "source_region": rect,
            "effective_region": effective_region,
        })],
        source_instructions_regenerated: Vec::new(),
        fonts_resources_changed: Vec::new(),
        flow_graph_changes: planned_downstream_vector_moves
            .iter()
            .map(|move_plan| json!({"kind": "planned_explicit_downstream_vector_move", "plan": move_plan}))
            .chain(planned_downstream_link_moves.iter().map(|move_plan| {
                json!({"kind": "planned_explicit_source_link_annotation_move", "plan": move_plan})
            }))
            .collect(),
        reading_order_changes: Vec::new(),
        structure_changes: Vec::new(),
        signature_impact: signature_impact(request),
        conformance_impact: conformance_impact(request),
        validation_evidence: json!({
            "preview_does_not_mutate": true,
            "no_overlay": true,
            "no_silent_clipping": true,
            "independent_tools_required_after_apply": ["qpdf", "Poppler", "MuPDF"],
        }),
        inverse_operation: None,
        undo_proof: json!({"preview_no_change": true}),
        refusal,
        prompt32_transaction: None,
    })
}

fn eligibility_modes(
    region: &GeometricTextRegion,
    line_breaking: &LineBreakingResult,
    request: &GeometricReflowRequest,
) -> Vec<TrueEditingMode> {
    if region.source_instructions.is_empty() {
        Vec::new()
    } else if line_breaking.overflow_status == OverflowStatus::UnresolvedOverflow {
        // SemanticDocument has bounded explicit target-flow adapters, but a
        // generic unresolved plan remains ineligible until the caller supplies
        // an approved target or page-creation policy. Returning it as broadly
        // eligible would be a silent promise of a mode that cannot execute.
        Vec::new()
    } else {
        vec![request.requested_mode]
    }
}

fn refusal_for(
    region: &GeometricTextRegion,
    line_breaking: &LineBreakingResult,
    constraints: &ConstraintSolverReport,
    request: &GeometricReflowRequest,
) -> Option<Value> {
    if region.source_instructions.is_empty() {
        return Some(json!({
            "code": "source_not_resolved",
            "message": "requested text could not be linked to Prompt31 source instructions",
            "recommended_mode": "manual_review",
            "no_change_proof": true,
        }));
    }
    if constraints.infeasible {
        return Some(json!({
            "code": "constraints_infeasible",
            "message": "mandatory geometric constraints cannot be satisfied without moving locked or unknown objects",
            "recommended_mode": "manual_review",
            "no_change_proof": true,
        }));
    }
    if line_breaking.overflow_status == OverflowStatus::UnresolvedOverflow {
        return Some(json!({
            "code": "unresolved_overflow",
            "message": "text does not fit under configured overflow policy; clipping and silent font reduction are forbidden",
            "recommended_mode": "manual_review",
            "no_change_proof": true,
            "font_reduction_available_not_applied": request.allow_font_reduction,
        }));
    }
    None
}

const MAX_LAYOUT_CONSTRAINTS: usize = 64;

fn layout_constraint_strength(priority: &str) -> Option<f64> {
    match priority {
        "required" => Some(REQUIRED),
        "strong" => Some(STRONG),
        "medium" => Some(MEDIUM),
        "weak" => Some(WEAK),
        _ => None,
    }
}

fn layout_constraint_satisfied(actual: f64, relation: &str, target: f64) -> Option<bool> {
    const EPSILON: f64 = 1e-8;
    match relation {
        "eq" => Some((actual - target).abs() <= EPSILON),
        "le" => Some(actual <= target + EPSILON),
        "ge" => Some(actual + EPSILON >= target),
        _ => None,
    }
}

fn add_layout_constraint(
    solver: &mut Solver,
    variable: Variable,
    relation: &str,
    strength: f64,
    value: f64,
) -> std::result::Result<(), String> {
    let result = match relation {
        "eq" => solver.add_constraint(variable | EQ(strength) | value),
        "le" => solver.add_constraint(variable | LE(strength) | value),
        "ge" => solver.add_constraint(variable | GE(strength) | value),
        _ => return Err("unsupported_relation".to_string()),
    };
    result.map_err(|error| format!("{error:?}"))
}

fn constraint_solver_report(
    region: &GeometricTextRegion,
    request: &GeometricReflowRequest,
    line_breaking: &LineBreakingResult,
) -> ConstraintSolverReport {
    let bounds = region.polygon_or_rect.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |acc, point| {
            [
                acc[0].min(point[0]),
                acc[1].min(point[1]),
                acc[2].max(point[0]),
                acc[3].max(point[1]),
            ]
        },
    );
    let effective_bounds =
        if line_breaking.overflow_status == OverflowStatus::FitAfterRegionExpansion {
            region.allowed_expansion_region
        } else {
            bounds
        };
    let width = effective_bounds[2] - effective_bounds[0];
    let height = effective_bounds[3] - effective_bounds[1];
    let content_height = line_breaking.lines.len() as f64 * request.line_height;
    let baseline_target = (content_height / request.line_height).round() * request.line_height;
    let left = Variable::new();
    let right = Variable::new();
    let top = Variable::new();
    let bottom = Variable::new();
    let used_height = Variable::new();
    let available_width = Variable::new();
    let available_height = Variable::new();
    let line_count = Variable::new();
    let line_height = Variable::new();
    let mut solver = Solver::new();
    let base_solve = solver.add_constraints(&[
        left | EQ(REQUIRED) | effective_bounds[0],
        right | EQ(REQUIRED) | effective_bounds[2],
        top | EQ(REQUIRED) | effective_bounds[3],
        bottom | EQ(REQUIRED) | effective_bounds[1],
        (right - left) | GE(REQUIRED) | 0.0,
        (top - bottom) | GE(REQUIRED) | 0.0,
        used_height | EQ(REQUIRED) | content_height,
        used_height | LE(REQUIRED) | height,
        available_width | EQ(REQUIRED) | width,
        available_height | EQ(REQUIRED) | height,
        line_count | EQ(REQUIRED) | line_breaking.lines.len() as f64,
        line_height | EQ(REQUIRED) | request.line_height,
        used_height | EQ(WEAK) | baseline_target,
    ]);
    let metric_variables = BTreeMap::from([
        ("region_left", (left, effective_bounds[0])),
        ("region_right", (right, effective_bounds[2])),
        ("region_top", (top, effective_bounds[3])),
        ("region_bottom", (bottom, effective_bounds[1])),
        ("content_height", (used_height, content_height)),
        ("region_width", (available_width, width)),
        ("region_height", (available_height, height)),
        ("line_count", (line_count, line_breaking.lines.len() as f64)),
        ("line_height", (line_height, request.line_height)),
    ]);
    let mut user_hard_constraints = Vec::new();
    let mut user_soft_constraints = Vec::new();
    let mut user_constraint_errors = Vec::new();
    if request.layout_constraints.len() > MAX_LAYOUT_CONSTRAINTS {
        user_constraint_errors.push(format!(
            "constraint_count_exceeds_limit:{}>{MAX_LAYOUT_CONSTRAINTS}",
            request.layout_constraints.len()
        ));
    } else {
        for constraint in &request.layout_constraints {
            let priority = layout_constraint_strength(&constraint.priority);
            let metric = metric_variables.get(constraint.variable.as_str()).copied();
            let finite = constraint.value.is_finite();
            let supported_relation =
                layout_constraint_satisfied(0.0, &constraint.relation, 0.0).is_some();
            let mut record = json!({
                "kind": "caller_layout_constraint",
                "constraint_id": constraint.constraint_id,
                "variable": constraint.variable,
                "relation": constraint.relation,
                "value": constraint.value,
                "priority": constraint.priority,
                "required": constraint.priority == "required",
                "source": "explicit_request",
            });
            let error = if constraint.constraint_id.is_empty() {
                Some("missing_constraint_id".to_string())
            } else if !finite {
                Some("non_finite_constraint_value".to_string())
            } else if priority.is_none() {
                Some("unsupported_priority".to_string())
            } else if !supported_relation {
                Some("unsupported_relation".to_string())
            } else if metric.is_none() {
                Some("unsupported_variable".to_string())
            } else {
                None
            };
            if let Some(error) = error {
                record["status"] = Value::String("invalid".to_string());
                record["error"] = Value::String(error.clone());
                // Invalid constraints are never ignored. A malformed soft
                // request is a refusal as well because otherwise the caller
                // cannot distinguish it from a satisfied preference.
                user_constraint_errors.push(format!("{}:{}", constraint.constraint_id, error));
            } else if let Some((variable, actual)) = metric {
                let strength = priority.expect("checked above");
                let satisfied =
                    layout_constraint_satisfied(actual, &constraint.relation, constraint.value)
                        .expect("checked above");
                record["actual"] = json!(actual);
                record["satisfied"] = json!(satisfied);
                record["residual"] = json!(actual - constraint.value);
                match add_layout_constraint(
                    &mut solver,
                    variable,
                    &constraint.relation,
                    strength,
                    constraint.value,
                ) {
                    Ok(()) => {
                        record["status"] = Value::String(
                            if satisfied { "satisfied" } else { "softened" }.to_string(),
                        )
                    }
                    Err(error) => {
                        record["status"] = Value::String("infeasible".to_string());
                        record["solver_error"] = Value::String(error.clone());
                        user_constraint_errors
                            .push(format!("{}:{error}", constraint.constraint_id));
                    }
                }
            }
            if constraint.priority == "required" {
                user_hard_constraints.push(record);
            } else {
                user_soft_constraints.push(record);
            }
        }
    }
    let explicit_target = request
        .next_region
        .map(|region| ("next_region", region))
        .or_else(|| request.next_column.map(|region| ("next_column", region)));
    let mut target_constraint_error = None::<String>;
    let mut target_constraint_record = None::<Value>;
    if base_solve.is_ok() {
        if let Some((kind, raw_target)) = explicit_target {
            match sanitize_region(raw_target) {
                Ok(target) => {
                    let target_left = Variable::new();
                    let target_right = Variable::new();
                    let target_top = Variable::new();
                    let target_bottom = Variable::new();
                    let target_solve = solver.add_constraints(&[
                        target_left | EQ(REQUIRED) | target[0],
                        target_right | EQ(REQUIRED) | target[2],
                        target_top | EQ(REQUIRED) | target[3],
                        target_bottom | EQ(REQUIRED) | target[1],
                        target_left | GE(REQUIRED) | region.page_box[0],
                        target_bottom | GE(REQUIRED) | region.page_box[1],
                        target_right | LE(REQUIRED) | region.page_box[2],
                        target_top | LE(REQUIRED) | region.page_box[3],
                        (target_right - target_left) | GE(REQUIRED) | 0.0,
                        (target_top - target_bottom) | GE(REQUIRED) | 0.0,
                    ]);
                    if let Err(error) = target_solve {
                        target_constraint_error = Some(format!("{error:?}"));
                    } else {
                        let order_solve = if kind == "next_region" {
                            // Strictly below the source in page user space.
                            solver.add_constraint(target_top | LE(REQUIRED) | bottom)
                        } else if direction_label(request.direction.as_deref()) == "right_to_left" {
                            // RTL stories progress into the geometrically
                            // leftward column while retaining the source
                            // reading band.  The direction is explicit in the
                            // request; we never infer it from x coordinates.
                            solver.add_constraints(&[
                                target_right | LE(REQUIRED) | left,
                                target_bottom | LE(REQUIRED) | top,
                                target_top | GE(REQUIRED) | bottom,
                            ])
                        } else {
                            // The supported LTR column primitive follows the
                            // source right edge and shares its reading band.
                            solver.add_constraints(&[
                                target_left | GE(REQUIRED) | right,
                                target_bottom | LE(REQUIRED) | top,
                                target_top | GE(REQUIRED) | bottom,
                            ])
                        };
                        if let Err(error) = order_solve {
                            target_constraint_error = Some(format!("{error:?}"));
                        }
                    }
                    target_constraint_record = Some(json!({
                        "kind": kind,
                        "required": true,
                        "target": target,
                        "page_box": region.page_box,
                        "reading_order_relation": if kind == "next_region" {
                            "target_above_or_at_source_bottom"
                        } else if direction_label(request.direction.as_deref()) == "right_to_left" {
                            "target_left_of_source_same_band_rtl"
                        } else {
                            "target_right_of_source_same_band_ltr"
                        },
                    }));
                }
                Err(error) => {
                    target_constraint_error = Some(error.to_string());
                    target_constraint_record = Some(json!({
                        "kind": kind,
                        "required": true,
                        "invalid_target": raw_target,
                    }));
                }
            }
        }
    }
    let infeasible = base_solve.is_err()
        || target_constraint_error.is_some()
        || !user_constraint_errors.is_empty()
        || width <= 0.0
        || height <= 0.0;
    let mut hard_constraints = vec![
        json!({"kind": "inside_region", "required": true, "bounds": effective_bounds, "source_bounds": bounds}),
        json!({"kind": "content_height", "required": true, "value": content_height, "available": height}),
        json!({"kind": "locked_objects_do_not_move", "required": true}),
        json!({"kind": "no_forbidden_overlap", "required": true}),
        json!({"kind": "font_reduction_not_automatic", "required": true}),
        json!({"kind": "page_creation_policy", "allowed": request.allow_page_creation, "reason": "only the explicit catalog-preserving one-page SemanticDocument append boundary may use this policy"}),
    ];
    let mut soft_constraints = vec![json!({
        "kind": "baseline_grid_preference",
        "required": false,
        "priority": "weak",
        "target": baseline_target,
        "resolved": content_height,
    })];
    hard_constraints.extend(user_hard_constraints);
    soft_constraints.extend(user_soft_constraints);
    if let Some(target) = target_constraint_record {
        hard_constraints.push(target);
    }
    let mut constraints = hard_constraints.clone();
    constraints.extend(soft_constraints.clone());
    let unsatisfied_soft_constraints = soft_constraints
        .iter()
        .filter(|constraint| {
            constraint["target"]
                .as_f64()
                .zip(constraint["resolved"].as_f64())
                .is_some_and(|(target, resolved)| (target - resolved).abs() > 1e-9)
                || constraint["satisfied"] == Value::Bool(false)
                || constraint["status"] == Value::String("invalid".to_string())
        })
        .cloned()
        .collect::<Vec<_>>();
    let fixed_constraint_count = hard_constraints.len() + soft_constraints.len();
    ConstraintSolverReport {
        schema_version: PROMPT33_SCHEMA_VERSION.to_string(),
        solver: "cassowary-0.3.0_bounded_region_and_explicit_flow_target_feasibility".into(),
        deterministic: true,
        bounded_runtime: true,
        constraints,
        hard_constraints,
        soft_constraints,
        unsatisfied_soft_constraints,
        fixed_constraint_count,
        infeasible,
        infeasibility_explanation: if infeasible {
            vec![
                format!(
                    "required single-region constraints are infeasible: content_height={content_height}, available_height={height}, solver_error={}",
                    base_solve
                        .err()
                        .map(|error| format!("{error:?}"))
                    .or(target_constraint_error)
                    .or_else(|| user_constraint_errors.first().cloned())
                        .unwrap_or_else(|| "invalid_region_geometry".to_string())
                ),
            ]
        } else {
            Vec::new()
        },
        locked_objects_moved: 0,
        unknown_objects_locked_by_default: region
            .locked_neighbors
            .iter()
            .any(|item| item.contains("unknown")),
        no_nan_or_infinite_geometry: effective_bounds.iter().all(|value| value.is_finite())
            && request
                .layout_constraints
                .iter()
                .all(|constraint| constraint.value.is_finite()),
    }
}

fn signature_impact(request: &GeometricReflowRequest) -> Value {
    json!({
        "status": if request.signature_policy_override {
            "allowed_but_invalidates_certification"
        } else {
            "profile_revalidation_required"
        },
        "doc_mdp_field_mdp_checked_before_apply": true,
        "no_false_valid_signature_claim": true,
    })
}

fn conformance_impact(_request: &GeometricReflowRequest) -> Value {
    json!({
        "pdfa": "profile_revalidation_required",
        "pdfua": "tagged_structure_preserved_or_prompt35_boundary_reported",
        "pdfx": "profile_revalidation_required",
        "linearization": "invalidated_by_rewrite_if_present",
    })
}

fn source_reflow_mode(request: &GeometricReflowRequest) -> AdvancedTextMode {
    match direction_label(request.direction.as_deref()).as_str() {
        "right_to_left" => AdvancedTextMode::ParagraphReflowRtl,
        "vertical_rl" => AdvancedTextMode::ParagraphReflowVertical,
        _ => AdvancedTextMode::ParagraphReflowHorizontal,
    }
}

fn source_reflow_options(
    request: &GeometricReflowRequest,
    region: [f64; 4],
) -> Result<AdvancedTextEditOptions> {
    if !matches!(
        request.font_policy.as_str(),
        "rebuild_subset_or_generated_type0" | "preserve_original_per_run"
    ) {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt33 font_reconstruction_failed: supported source reflow requires font_policy=rebuild_subset_or_generated_type0 or preserve_original_per_run"
                .to_string(),
        ));
    }
    let usable_height = region[3] - region[1];
    let max_lines = (usable_height / request.line_height).floor() as usize;
    if max_lines == 0 {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt33 constraint_infeasible: region has no complete line-height slot".to_string(),
        ));
    }
    let alignment = match request.alignment.trim().to_ascii_lowercase().as_str() {
        "left" => GeneratedTextAlignment::Left,
        "right" => GeneratedTextAlignment::Right,
        "center" | "centre" => GeneratedTextAlignment::Center,
        "start" => GeneratedTextAlignment::Start,
        "end" => GeneratedTextAlignment::End,
        "justify" | "justified" | "full_justify" => GeneratedTextAlignment::Justify,
        other => {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "prompt33 unsupported_alignment: {other}; supported values are left, right, center, start, end, justify"
            )))
        }
    };
    if alignment == GeneratedTextAlignment::Justify
        && request.language.as_deref().is_some_and(|language| {
            language.eq_ignore_ascii_case("ar") || language.to_ascii_lowercase().starts_with("ar-")
        })
    {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt33 shaping_failed: Arabic full justification is refused until the canonical source writer can serialize a shaped kashida feature without changing extraction semantics"
                .to_string(),
        ));
    }
    Ok(AdvancedTextEditOptions {
        region,
        // The canonical Prompt 20 writer uses this as the glyph-scale source
        // for Type0 text positioning. Prompt 33's line-height is deliberately
        // preserved independently of this point size.
        font_size: request.line_height / 1.2,
        line_spacing: 1.2,
        max_lines_or_columns: max_lines,
        overflow_policy: TextOverflowPolicy::Error,
        signature_policy_override: request.signature_policy_override,
        deterministic: true,
        alignment,
        justify_last_line: request.justify_last_line,
        max_word_spacing: 0.5,
        max_character_spacing: 0.05,
    })
}

fn source_output_lines(lines: &[LayoutLine]) -> Vec<ExplicitLayoutLine> {
    lines
        .iter()
        .map(|line| ExplicitLayoutLine {
            logical_text: line.text.clone(),
            visual_text: if line.hyphen_inserted {
                format!("{}-", line.visual_text)
            } else {
                line.visual_text.clone()
            },
            inserted_visual_hyphen: line.hyphen_inserted,
        })
        .collect()
}

fn unique_scalar_range(haystack: &str, needle: &str) -> Result<[usize; 2]> {
    if needle.is_empty() {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt33 paragraph_not_resolved: preserve_original_per_run requires nonempty source text"
                .to_string(),
        ));
    }
    let matches = haystack.match_indices(needle).collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "prompt33 paragraph_not_resolved: preserve_original_per_run requires one unambiguous page logical source range; found {}",
            matches.len()
        )));
    }
    let (byte_start, _) = matches[0];
    let byte_end = byte_start.saturating_add(needle.len());
    Ok([
        haystack[..byte_start].chars().count(),
        haystack[..byte_end].chars().count(),
    ])
}

pub fn apply_reflow_region(
    input: &[u8],
    request: &GeometricReflowRequest,
) -> Result<(Vec<u8>, ReflowTransactionReport)> {
    apply_source_linked_reflow(input, request, TrueEditingMode::GeometricBlock)
}

fn apply_source_linked_reflow(
    input: &[u8],
    request: &GeometricReflowRequest,
    required_mode: TrueEditingMode,
) -> Result<(Vec<u8>, ReflowTransactionReport)> {
    if request.requested_mode != required_mode {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt33 requested edit mode does not match the explicit source-linked application path"
                .to_string(),
        ));
    }
    let mut report = preview_reflow(input, request)?;
    if let Some(refusal) = report.refusal.as_ref() {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "prompt33 {}: {}",
            refusal["code"], refusal["message"]
        )));
    }
    // Move explicitly linked non-text occurrences before the text serializer.
    // Both Prompt20 mutations are pure byte-to-byte transformations until this
    // function returns, so a later text/font failure returns no partial output.
    // The vector move runs first to retain its stable source identity, followed
    // by the caller-associated Link rectangle update; unknown neighbors never
    // participate in either transaction.
    let (vector_mutation_input, downstream_vector_moves) =
        apply_downstream_vector_moves(input, request)?;
    let (mutation_input, downstream_link_moves) =
        apply_downstream_link_moves(&vector_mutation_input, request)?;
    let source_region = effective_region_for_report(input, request, report.overflow_status)?;
    let options = source_reflow_options(request, source_region)?;
    let final_lines = source_output_lines(&report.line_breaking.lines);
    let (
        output,
        removed_old_reachable_content,
        old_text_absent,
        generated_text_reopens_and_extracts,
        fonts_resources_changed,
        source_rewrite_detail,
        line_adjustments,
    ) = if request.font_policy == "preserve_original_per_run" {
        let model = analyze_multi_run_text_range(&mutation_input, request.page)?;
        let [logical_start, logical_end] =
            unique_scalar_range(&model.logical_text, &request.source_text)?;
        let multi_request = MultiRunTextRangeRequest {
            page: request.page,
            logical_start,
            logical_end,
            replacement_text: request.replacement_text.clone(),
            mode: source_reflow_mode(request),
            style_policy: MultiRunStylePolicy::PreservePerSegment,
            options: options.clone(),
            final_lines: Some(final_lines.clone()),
        };
        let (output, apply) = edit_multi_run_text_range(&mutation_input, &multi_request, None)?;
        let fonts = apply
            .selected_source_spans
            .iter()
            .map(|span| format!("preserved_source_font_resource:{}", span.font_resource))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        (
            output,
            apply.reachable_source_tokens_removed,
            apply.old_selected_text_absent,
            apply.output_reopened && apply.replacement_extracts,
            fonts,
            json!({"path": "prompt20_multi_run_preserve_per_segment_source_serializer", "detail": apply}),
            Value::Array(Vec::new()),
        )
    } else {
        let (output, apply) = edit_advanced_text_pdf_with_visual_layout(
            &mutation_input,
            request.page,
            &request.source_text,
            &request.replacement_text,
            source_reflow_mode(request),
            &options,
            None,
            &final_lines,
        )?;
        (
            output,
            apply.removed_old_reachable_content,
            apply.old_text_absent,
            apply.output_reopened && apply.replacement_extracts,
            vec![format!(
                "generated_type0_font_resource:{}",
                apply.font_resource
            )],
            json!({"path": "prompt20_source_token_removal_plus_generated_type0_text_stream", "detail": apply}),
            json!(apply.line_adjustments.clone()),
        )
    };
    if ContentEngine::open_bytes(output.clone()).is_err() {
        return Err(WellfriendError::MalformedPdf(
            "prompt33 output_reopen_failed after source reflow rewrite".to_string(),
        ));
    }
    let prompt32_request = SceneTextEditRequest {
        requested_mode: TrueEditingMode::OperatorPreserving,
        page: request.page,
        source_text: request.source_text.clone(),
        replacement_text: request.replacement_text.clone(),
        signature_policy_override: request.signature_policy_override,
        font_policy: request.font_policy.clone(),
        normalization_policy: Some("preserve_exact_sequence".into()),
        direction: request.direction.clone(),
    };
    let dirty = dirty_region_report(&mutation_input, &prompt32_request).unwrap_or_else(|err| {
        json!({
            "status": "unavailable",
            "reason": err.to_string(),
        })
    });
    let unaffected_proof = unaffected_content_proof(
        input,
        &output,
        request.page,
        &request.source_text,
        &request.replacement_text,
        &expected_downstream_link_rects(request),
        1 + request.downstream_vector_moves.len(),
    );
    if unaffected_proof["status"]
        != Value::String("pass_with_documented_layout_whitespace_policy".to_string())
    {
        return Err(WellfriendError::MalformedPdf(
            "prompt33 unaffected-content proof failed after source rewrite".to_string(),
        ));
    }
    report.applied_mode = Some(request.requested_mode);
    report.refusal = None;
    report.source_instructions_regenerated = report.region.source_instructions.clone();
    report.fonts_resources_changed = fonts_resources_changed;
    report.objects_moved = downstream_vector_moves
        .iter()
        .filter_map(|move_report| move_report["vector_stable_id"].as_str())
        .map(|stable_id| format!("vector:{stable_id}"))
        .chain(downstream_link_moves.iter().filter_map(|move_report| {
            move_report["annotation_index"]
                .as_u64()
                .map(|index| format!("link_annotation:{}:{index}", request.page))
        }))
        .collect();
    report.constraints.locked_objects_moved = 0;
    if !downstream_vector_moves.is_empty() {
        let movement_constraint = json!({
            "kind": "explicit_dependency_linked_vector_movement",
            "required": true,
            "moved_object_count": downstream_vector_moves.len(),
            "unknown_neighbors_remain_locked": true,
        });
        report
            .constraints
            .constraints
            .push(movement_constraint.clone());
        report
            .constraints
            .hard_constraints
            .push(movement_constraint);
        report.constraints.fixed_constraint_count =
            report.constraints.hard_constraints.len() + report.constraints.soft_constraints.len();
        report.pages_columns_affected.push(json!({
            "page": request.page,
            "kind": "explicit_dependency_linked_vector_movement",
            "moved_objects": report.objects_moved.clone(),
        }));
    }
    if !downstream_link_moves.is_empty() {
        let movement_constraint = json!({
            "kind": "explicit_source_link_annotation_movement",
            "required": true,
            "moved_object_count": downstream_link_moves.len(),
            "unknown_neighbors_remain_locked": true,
            "action_destination_preserved": true,
        });
        report
            .constraints
            .constraints
            .push(movement_constraint.clone());
        report
            .constraints
            .hard_constraints
            .push(movement_constraint);
        report.constraints.fixed_constraint_count =
            report.constraints.hard_constraints.len() + report.constraints.soft_constraints.len();
        report.pages_columns_affected.push(json!({
            "page": request.page,
            "kind": "explicit_source_link_annotation_movement",
            "moved_objects": report.objects_moved.clone(),
        }));
    }
    report.line_breaking.justification = json!({
        "status": "output_driving_text_state_spacing",
        "alignment": request.alignment,
        "justify_last_line": request.justify_last_line,
        "lines": line_adjustments,
        "arabic_kashida": "not selected unless a canonical shaping feature serializer is available",
        "cjk": "bounded character spacing only; no outline scaling",
        "unsafe_universal_scaling": false,
    });
    report.flow_graph_changes.push(json!({
        "kind": "local_region_reflow",
        "status": report.overflow_status,
    }));
    report.reading_order_changes =
        vec![json!({"status": "stable_for_single_region_source_rewrite"})];
    report.validation_evidence = json!({
        "output_reopened": true,
        "source_rewrite": source_rewrite_detail,
        "downstream_vector_moves": downstream_vector_moves,
        "downstream_link_moves": downstream_link_moves,
        "dirty_region": dirty,
        "no_overlay_no_clipping": removed_old_reachable_content,
        "old_source_text_absent_from_target_extraction": old_text_absent,
        "source_text_token_removed": removed_old_reachable_content,
        "generated_text_reopens_and_extracts": generated_text_reopens_and_extracts,
        "unaffected_content_proof": unaffected_proof,
        "independent_tools": "pending_prompt33_vps_validation",
    });
    report.inverse_operation = Some(json!({
        "kind": if downstream_vector_moves.is_empty() && downstream_link_moves.is_empty() { "replace_text" } else { "replace_text_and_restore_explicit_dependency_preimages" },
        "page": request.page,
        "source_text": request.replacement_text,
        "replacement_text": request.source_text,
        "mode": request.requested_mode,
        "dependent_vector_moves": downstream_vector_moves,
        "dependent_link_moves": downstream_link_moves,
        "atomic_restore": "ReflowMutationSession retained preimage",
    }));
    let prompt32_proxy = EditTransactionReport {
        schema_version: crate::prompt32::PROMPT32_SCHEMA_VERSION.to_string(),
        transaction_id: stable_id("prompt32-proxy", &[request.source_text.as_bytes()]),
        base_snapshot_id: report.input_snapshot.snapshot_id.clone(),
        requested_mode: TrueEditingMode::OperatorPreserving,
        applied_mode: Some(TrueEditingMode::OperatorPreserving),
        lifecycle: vec![TransactionState::ReopenedValidated],
        preconditions: Vec::new(),
        read_set: report
            .region
            .source_instructions
            .iter()
            .cloned()
            .chain(report.objects_moved.iter().cloned())
            .collect(),
        write_set: report
            .region
            .source_instructions
            .iter()
            .cloned()
            .chain(report.objects_moved.iter().cloned())
            .collect(),
        affected_objects: report.objects_moved.clone(),
        affected_pages: vec![request.page],
        affected_scene_nodes: report
            .region
            .source_scene_nodes
            .iter()
            .cloned()
            .chain(
                downstream_vector_moves
                    .iter()
                    .filter_map(|move_report| move_report["scene_node_id"].as_str())
                    .map(str::to_string),
            )
            .collect(),
        cloned_resources: Vec::new(),
        dirty_regions: Vec::new(),
        signature_impact: signature_impact(request),
        conformance_impact: conformance_impact(request),
        validation_plan: Vec::new(),
        inverse_operations: report.inverse_operation.clone().into_iter().collect(),
        commit_policy: "prompt33_proxy_for_undo_proof".into(),
        operation_log_hash: digest_hex(request.replacement_text.as_bytes()),
        deterministic: true,
        refusal: None,
        prompt31_operation: None,
    };
    report.undo_proof = undo_restoration_report(input, &output, &prompt32_proxy);
    report.prompt32_transaction = Some(prompt32_proxy);
    Ok((output, report))
}

/// Continue a paragraph in one explicit, semantically proven-empty target
/// rectangle. The source and target are both proven through the existing
/// semantic/scene models; all non-text target objects stay locked. This is a
/// deliberately bounded source-linked flow primitive, not general pagination.
#[allow(clippy::too_many_arguments)]
fn apply_single_paragraph_existing_target_flow(
    input: &[u8],
    request: &GeometricReflowRequest,
    mut report: ReflowTransactionReport,
    target_page: usize,
    target_region: [f64; 4],
    relationship: &str,
    scope_of_movement: &str,
) -> Result<(Vec<u8>, ReflowTransactionReport)> {
    if request.requested_mode != TrueEditingMode::SemanticDocument
        || !request.approve_low_confidence_structure
    {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt33 review_required: existing-next-page flow requires explicit SemanticDocument review approval"
                .to_string(),
        ));
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    if target_page == 0 || target_page > engine.page_count()? {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt33 next_region_unavailable: the explicit downstream target page is unavailable"
                .to_string(),
        ));
    }
    let source_region = region_for_request(input, request)?;
    let target_semantic = engine
        .extract_text_semantic_model(&[target_page], crate::text::TextSemanticOptions::default())?;
    if target_semantic
        .pages
        .first()
        .is_some_and(|item| item.structure.enabled)
    {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt33 structure_update_failed: existing-next-page flow refuses tagged structure until a source-linked structure repair transaction exists"
                .to_string(),
        ));
    }
    if target_semantic
        .pages
        .iter()
        .flat_map(|page| page.blocks.iter())
        .flat_map(|block| block.lines.iter())
        .any(|line| rects_intersect(quad_bounds(line.quad), target_region))
    {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt33 locked_object_conflict: canonical semantic text geometry occupies the proposed downstream target region"
                .to_string(),
        ));
    }
    let target_scene = build_scene_graph(input, &[target_page])?;
    if let Some(node) = target_scene.nodes.iter().find(|node| {
        node.page == target_page
            && node.visibility != "hidden"
            && node.node_kind != crate::prompt32::SceneNodeKind::TextObject
            && rects_intersect(node.bounds_user_space, target_region)
    }) {
        return Err(WellfriendError::UnsupportedFeature(
            format!(
                "prompt33 locked_object_conflict: the proposed downstream target region intersects visible {} scene node {}",
                format!("{:?}", node.node_kind).to_ascii_lowercase(),
                node.node_id
            ),
        ));
    }
    let source_semantic = engine.extract_text_semantic_model(
        &[request.page],
        crate::text::TextSemanticOptions::default(),
    )?;
    if source_semantic
        .pages
        .first()
        .is_some_and(|item| item.structure.enabled)
    {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt33 structure_update_failed: existing-target flow refuses tagged structure until a source-linked structure repair transaction exists"
                .to_string(),
        ));
    }
    let layout = line_break_text(
        &request.replacement_text,
        source_region[2] - source_region[0],
        source_region[3] - source_region[1],
        request.line_height,
        request.language.as_deref(),
        request.direction.as_deref(),
        request.hyphenation,
    )?;
    let max_lines = ((source_region[3] - source_region[1]) / request.line_height).floor() as usize;
    if max_lines == 0 || layout.lines.len() <= max_lines {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt33 next_region_unavailable: requested paragraph does not have eligible bounded downstream overflow"
                .to_string(),
        ));
    }
    let continuation_count = layout.lines.len().saturating_sub(max_lines);
    let target_max_lines =
        ((target_region[3] - target_region[1]) / request.line_height).floor() as usize;
    if target_max_lines == 0
        || continuation_count > target_max_lines
        || layout
            .lines
            .iter()
            .take(max_lines)
            .any(|line| line.advance > source_region[2] - source_region[0])
        || layout
            .lines
            .iter()
            .skip(max_lines)
            .any(|line| line.advance > target_region[2] - target_region[0])
    {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt33 overflow_unresolved: downstream target capacity cannot hold the shaped continuation"
                .to_string(),
        ));
    }
    let first_lines = source_output_lines(&layout.lines[..max_lines]);
    let continuation_lines = source_output_lines(&layout.lines[max_lines..]);
    let first_text = first_lines
        .iter()
        .map(|line| line.logical_text.as_str())
        .collect::<String>();
    let continuation_text = continuation_lines
        .iter()
        .map(|line| line.logical_text.as_str())
        .collect::<String>();
    if format!("{first_text}{continuation_text}") != request.replacement_text {
        return Err(WellfriendError::MalformedPdf(
            "prompt33 downstream split did not preserve exact logical replacement text".to_string(),
        ));
    }
    let options = source_reflow_options(request, source_region)?;
    let target_options = source_reflow_options(request, target_region)?;
    let (output, first_apply, continuation_evidence, continuation_resource) =
        if target_page == request.page {
            // A same-page story must be one generated source stream: separate
            // incremental generated streams are extracted newest-first by the
            // canonical reader. Per-line rectangles let Prompt 20 emit both
            // fragments in logical order while preserving their distinct
            // geometric target regions.
            let font_size = request.line_height / 1.2;
            let line_advance = font_size * 1.2;
            let positioned_lines = layout
                .lines
                .iter()
                .enumerate()
                .map(|(index, line)| {
                    let (region, local_index) = if index < max_lines {
                        (source_region, index)
                    } else {
                        (target_region, index - max_lines)
                    };
                    let baseline = region[3] - font_size - local_index as f64 * line_advance;
                    PositionedExplicitLayoutLine {
                        line: source_output_lines(std::slice::from_ref(line))
                            .into_iter()
                            .next()
                            .expect("one source output line"),
                        region: [region[0], baseline, region[2], baseline + font_size],
                    }
                })
                .collect::<Vec<_>>();
            let (output, first_apply) = edit_advanced_text_pdf_with_positioned_visual_layout(
                input,
                request.page,
                &request.source_text,
                &request.replacement_text,
                source_reflow_mode(request),
                &options,
                None,
                &positioned_lines,
            )?;
            (
                output,
                first_apply.clone(),
                json!({
                    "operation": "single_canonical_positioned_source_rewrite",
                    "line_count": positioned_lines.len(),
                    "target_region": target_region,
                    "output_sha256": first_apply.output_sha256,
                }),
                format!(
                    "generated_type0_font_resource:{}",
                    first_apply.font_resource
                ),
            )
        } else {
            let insertion = MultiRunTextRangeRequest {
                page: target_page,
                logical_start: 0,
                logical_end: 0,
                replacement_text: continuation_text.clone(),
                mode: source_reflow_mode(request),
                style_policy: MultiRunStylePolicy::InheritLeading,
                options: target_options,
                final_lines: Some(continuation_lines),
            };
            let (first_output, first_apply) = edit_advanced_text_pdf_with_visual_layout(
                input,
                request.page,
                &request.source_text,
                &first_text,
                source_reflow_mode(request),
                &options,
                None,
                &first_lines,
            )?;
            let (output, continuation_apply) =
                edit_multi_run_text_range(&first_output, &insertion, None)?;
            let continuation_resource = format!(
                "generated_type0_font_resource:{}",
                continuation_apply.output_sha256
            );
            (
                output,
                first_apply,
                json!(continuation_apply),
                continuation_resource,
            )
        };
    let reopened = ContentEngine::open_bytes(output.clone())?;
    if reopened.page_count()? != engine.page_count()? {
        return Err(WellfriendError::MalformedPdf(
            "prompt33 output_reopen_failed: existing-target flow changed page count".to_string(),
        ));
    }
    if reopened
        .get_page_text(request.page)?
        .contains(&request.source_text)
    {
        return Err(WellfriendError::MalformedPdf(
            "prompt33 extraction_validation_failed: source paragraph remained reachable after downstream flow"
                .to_string(),
        ));
    }
    report.applied_mode = Some(TrueEditingMode::SemanticDocument);
    report.refusal = None;
    report.scope_of_movement = scope_of_movement.to_string();
    report.line_breaking = layout;
    report.overflow_status = match relationship {
        "next_page" => OverflowStatus::FitAfterPageFlow,
        "next_column" => OverflowStatus::FitAfterColumnFlow,
        _ => OverflowStatus::FitAfterDownstreamFlow,
    };
    report.constraints.infeasible = false;
    report.constraints.infeasibility_explanation.clear();
    report.pages_columns_affected = vec![
        json!({"page": request.page, "kind": "source_region", "lines": max_lines}),
        json!({"page": target_page, "kind": "existing_empty_target_region", "region": target_region, "lines": continuation_count}),
    ];
    report.source_instructions_regenerated = report.region.source_instructions.clone();
    report.fonts_resources_changed = vec![
        format!(
            "generated_type0_font_resource:{}",
            first_apply.font_resource
        ),
        continuation_resource,
    ];
    report.flow_graph_changes = vec![json!({
        "kind": relationship,
        "source_page": request.page,
        "target_page": target_page,
        "target_proven_empty": true,
        "source_linked": true,
        "base_direction": direction_label(request.direction.as_deref()),
    })];
    report.reading_order_changes = vec![json!({"relationship": relationship, "confidence": 0.92})];
    report.validation_evidence = json!({
        "output_reopened": true,
        "page_count_preserved": true,
        "source_rewrite": first_apply,
        "continuation_source_insertion": continuation_evidence,
        "target_region_was_proven_empty": true,
        "target_region": target_region,
        "no_overlay_no_clipping": true,
    });
    report.inverse_operation = Some(json!({
        "kind": "truncate_incremental_revisions",
        "scope": "ReflowMutationSession exact preimage",
        "pages": [request.page, target_page],
    }));
    Ok((output, report))
}

/// Flow into a distinct, explicitly approved rectangle on the same page.  The
/// caller supplies this relationship in the SemanticDocument request; it is
/// never inferred from arbitrary nearby scene objects.
fn apply_single_paragraph_existing_next_region_flow(
    input: &[u8],
    request: &GeometricReflowRequest,
    report: ReflowTransactionReport,
) -> Result<(Vec<u8>, ReflowTransactionReport)> {
    let source_region = region_for_request(input, request)?;
    let target_region = request.next_region.ok_or_else(|| {
        WellfriendError::UnsupportedFeature(
            "prompt33 next_region_unavailable: no explicit next_region was supplied".to_string(),
        )
    })?;
    let target_region = sanitize_region(target_region)?;
    let page = page_bounds(input, request.page)?;
    let inside_page = target_region[0] >= page[0]
        && target_region[1] >= page[1]
        && target_region[2] <= page[2]
        && target_region[3] <= page[3];
    // This bounded primitive only supports ordinary top-to-bottom same-column
    // continuation. A target above or level with the source would make the
    // canonical geometry-based extractor reverse story order; cross-column,
    // RTL-column, and arbitrary graph transitions remain typed limits.
    let follows_source_in_reading_order = target_region[3] <= source_region[1];
    if !inside_page
        || rects_intersect(source_region, target_region)
        || !follows_source_in_reading_order
    {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt33 reading_order_ambiguous: next_region must be a disjoint below-source rectangle inside the source page box"
                .to_string(),
        ));
    }
    apply_single_paragraph_existing_target_flow(
        input,
        request,
        report,
        request.page,
        target_region,
        "next_region",
        "semantic_single_paragraph_existing_next_region_flow",
    )
}

/// Flow into a distinct, explicitly approved next-column rectangle. The
/// caller's explicit base direction selects the geometric progression: right
/// for LTR and left for RTL. The narrow same-band rule gives the positioned
/// source stream a deterministic story relationship without guessing a column
/// transition from nearby artwork or paint order.
fn apply_single_paragraph_existing_next_column_flow(
    input: &[u8],
    request: &GeometricReflowRequest,
    report: ReflowTransactionReport,
) -> Result<(Vec<u8>, ReflowTransactionReport)> {
    let direction = direction_label(request.direction.as_deref());
    if !matches!(direction.as_str(), "left_to_right" | "right_to_left") {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt33 unsupported_writing_mode: explicit next-column flow supports only horizontal left-to-right or right-to-left text"
                .to_string(),
        ));
    }
    let source_region = region_for_request(input, request)?;
    let target_region = request.next_column.ok_or_else(|| {
        WellfriendError::UnsupportedFeature(
            "prompt33 next_column_unavailable: no explicit next_column was supplied".to_string(),
        )
    })?;
    let target_region = sanitize_region(target_region)?;
    let page = page_bounds(input, request.page)?;
    let inside_page = target_region[0] >= page[0]
        && target_region[1] >= page[1]
        && target_region[2] <= page[2]
        && target_region[3] <= page[3];
    let same_reading_band =
        target_region[1] < source_region[3] && target_region[3] > source_region[1];
    let follows_source_in_column_order = if direction == "right_to_left" {
        target_region[2] <= source_region[0]
    } else {
        target_region[0] >= source_region[2]
    };
    if !inside_page
        || rects_intersect(source_region, target_region)
        || !same_reading_band
        || !follows_source_in_column_order
    {
        return Err(WellfriendError::UnsupportedFeature(
            format!(
                "prompt33 reading_order_ambiguous: next_column must be a disjoint {} rectangle overlapping the source reading band",
                if direction == "right_to_left" { "leftward RTL" } else { "rightward LTR" }
            ),
        ));
    }
    apply_single_paragraph_existing_target_flow(
        input,
        request,
        report,
        request.page,
        target_region,
        "next_column",
        "semantic_single_paragraph_existing_next_column_flow",
    )
}

/// Continue a paragraph in the corresponding rectangle of the immediate next
/// page. This wrapper keeps the target relation exact and delegates all
/// mutation to the shared source-linked target-flow primitive.
fn apply_single_paragraph_existing_next_page_flow(
    input: &[u8],
    request: &GeometricReflowRequest,
    report: ReflowTransactionReport,
) -> Result<(Vec<u8>, ReflowTransactionReport)> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let target_page = request.page.saturating_add(1);
    if target_page > engine.page_count()? {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt33 next_page_unavailable: no existing next page is available for this bounded flow"
                .to_string(),
        ));
    }
    let source_region = region_for_request(input, request)?;
    if page_bounds(input, request.page)? != page_bounds(input, target_page)? {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt33 next_page_unavailable: existing-next-page flow requires identical page boxes"
                .to_string(),
        ));
    }
    apply_single_paragraph_existing_target_flow(
        input,
        request,
        report,
        target_page,
        source_region,
        "next_page",
        "semantic_single_paragraph_existing_next_page_flow",
    )
}

/// A bounded canonical page-flow adapter.  The continuation is authored from
/// the final shaped lines, then appended by the canonical writer while the
/// source catalog and existing object graph are preserved.  Existing named
/// destinations, outlines, labels, forms, annotations, and links remain
/// source-object linked; an appended page does not require retargeting an
/// existing destination.  New continuation content intentionally has no
/// inferred association with those pre-existing interactive objects.
fn apply_single_paragraph_page_creation(
    input: &[u8],
    request: &GeometricReflowRequest,
    mut report: ReflowTransactionReport,
) -> Result<(Vec<u8>, ReflowTransactionReport)> {
    if request.requested_mode != TrueEditingMode::SemanticDocument || !request.allow_page_creation {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt33 page_creation_not_permitted: SemanticDocument plus explicit allow_page_creation is required"
                .to_string(),
        ));
    }
    if !request.approve_low_confidence_structure {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt33 review_required: page creation requires explicit approval of the semantic structure"
                .to_string(),
        ));
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    if engine.page_count()? != 1 || request.page != 1 {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt33 next_page_unavailable: the current canonical page-flow boundary supports exactly one source page and one appended continuation page"
                .to_string(),
        ));
    }
    if !engine.verify_signatures()?.is_empty() {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt33 signature_permission_violation: page-tree rebuilding is refused for signed documents"
                .to_string(),
        ));
    }
    let interactive_before = interactive_report(&engine)?;
    let page_ops = &interactive_before.page_operations;
    let page_info = page_ops.pages.first().ok_or_else(|| {
        WellfriendError::MalformedPdf(
            "prompt33 page creation could not resolve page box".to_string(),
        )
    })?;
    if page_info.rotate != 0
        || page_info.media_box != page_info.crop_box
        || page_info.media_box[0] != 0.0
        || page_info.media_box[1] != 0.0
    {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt33 unsupported_writing_mode: narrow page creation requires an unrotated zero-origin MediaBox/CropBox"
                .to_string(),
        ));
    }
    let semantic =
        engine.extract_text_semantic_model(&[1], crate::text::TextSemanticOptions::default())?;
    if semantic
        .pages
        .first()
        .is_some_and(|page| page.structure.enabled)
    {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt33 structure_update_failed: tagged structure repair is not available for page-tree rebuilding"
                .to_string(),
        ));
    }
    if report.region.source_instructions.is_empty() {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt33 source_not_resolved: page flow requires exact Prompt31 source provenance"
                .to_string(),
        ));
    }
    let region = region_for_request(input, request)?;
    let width = region[2] - region[0];
    let height = region[3] - region[1];
    let line_breaking = line_break_text(
        &request.replacement_text,
        width,
        height,
        request.line_height,
        request.language.as_deref(),
        request.direction.as_deref(),
        request.hyphenation,
    )?;
    let max_lines = (height / request.line_height).floor() as usize;
    if max_lines == 0 || line_breaking.lines.len() <= max_lines {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt33 page_creation_not_required: no eligible page overflow exists for the explicit policy"
                .to_string(),
        ));
    }
    if line_breaking.lines.iter().any(|line| line.advance > width) {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt33 overflow_unresolved: page creation cannot repair an over-wide unbreakable line"
                .to_string(),
        ));
    }
    if line_breaking.lines.iter().any(|line| line.hyphen_inserted) {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt33 hyphenation_unavailable: explicit page creation refuses dictionary-hyphenated lines until the canonical continuation-page writer can retain logical extraction"
                .to_string(),
        ));
    }
    let first_lines = line_breaking.lines[..max_lines]
        .iter()
        .map(|line| line.text.clone())
        .collect::<Vec<_>>();
    let continuation_lines = line_breaking.lines[max_lines..]
        .iter()
        .map(|line| line.text.clone())
        .collect::<Vec<_>>();
    let first_text = first_lines.concat();
    let continuation_text = continuation_lines.concat();
    if first_text.is_empty()
        || continuation_text.is_empty()
        || format!("{first_text}{continuation_text}") != request.replacement_text
    {
        return Err(WellfriendError::MalformedPdf(
            "prompt33 page flow split did not preserve exact replacement text".to_string(),
        ));
    }
    let first_options = source_reflow_options(request, region)?;
    let first_output_lines = source_output_lines(&line_breaking.lines[..max_lines]);
    let (first_output, first_apply) = edit_advanced_text_pdf_with_visual_layout(
        input,
        1,
        &request.source_text,
        &first_text,
        source_reflow_mode(request),
        &first_options,
        None,
        &first_output_lines,
    )?;
    let page_width = page_info.media_box[2] - page_info.media_box[0];
    let page_height = page_info.media_box[3] - page_info.media_box[1];
    let mut continuation_builder = PdfBuilder::new();
    let continuation_page =
        continuation_builder.add_page(AuthorPageSize::custom(page_width, page_height));
    let style = TextStyle::unicode(first_options.font_size);
    for (index, line) in continuation_lines.iter().enumerate() {
        let baseline = region[3]
            - first_options.font_size
            - index as f64 * first_options.font_size * first_options.line_spacing;
        continuation_page.draw_text(line, region[0], baseline, &style)?;
    }
    let continuation_bytes = continuation_builder.to_bytes()?;
    let first_engine = ContentEngine::open_bytes(first_output)?;
    let continuation_engine = ContentEngine::open_bytes(continuation_bytes)?;
    let output = append_authored_page_preserving_catalog(
        first_engine.document(),
        continuation_engine.document(),
    )?;
    let reopened = ContentEngine::open_bytes(output.clone())?;
    if reopened.page_count()? != 2 {
        return Err(WellfriendError::MalformedPdf(
            "prompt33 output_reopen_failed: appended page count was not preserved".to_string(),
        ));
    }
    let extracted = format!(
        "{}{}",
        reopened.get_page_text(1)?,
        reopened.get_page_text(2)?
    );
    if !layout_extraction_equivalent(&extracted, &request.replacement_text)
        || extracted.contains(&request.source_text)
    {
        return Err(WellfriendError::MalformedPdf(
            "prompt33 extraction_validation_failed after explicit page creation".to_string(),
        ));
    }
    report.applied_mode = Some(TrueEditingMode::SemanticDocument);
    report.refusal = None;
    report.scope_of_movement = "semantic_single_paragraph_explicit_new_page_flow".to_string();
    report.line_breaking = line_breaking;
    report.overflow_status = OverflowStatus::FitAfterPageFlow;
    report.constraints.infeasible = false;
    report.constraints.infeasibility_explanation.clear();
    let page_creation_constraint = json!({
        "kind": "explicit_page_creation",
        "required": true,
        "policy": "allow_page_creation",
        "page_size": [page_width, page_height],
        "pages_inserted": 1,
    });
    report
        .constraints
        .constraints
        .push(page_creation_constraint.clone());
    report
        .constraints
        .hard_constraints
        .push(page_creation_constraint);
    report.constraints.fixed_constraint_count =
        report.constraints.hard_constraints.len() + report.constraints.soft_constraints.len();
    report.pages_columns_affected = vec![
        json!({"page": 1, "kind": "source_region", "lines": first_lines.len()}),
        json!({"page": 2, "kind": "created_continuation_page", "lines": continuation_lines.len()}),
    ];
    report.source_instructions_regenerated = report.region.source_instructions.clone();
    report.fonts_resources_changed = vec![
        format!(
            "generated_type0_font_resource:{}",
            first_apply.font_resource
        ),
        "continuation_page_authoring_unicode_font".to_string(),
    ];
    report.flow_graph_changes = vec![json!({
        "kind": "next_page",
        "source_page": 1,
        "target_page": 2,
        "text_split": {"first_page": first_text, "second_page": continuation_text},
        "source_linked": true,
    })];
    report.reading_order_changes = vec![json!({"relationship": "next_page", "confidence": 1.0})];
    let interactive_after = interactive_report(&reopened)?;
    let catalog_reference_preservation = json!({
        "forms_preserved": interactive_before.forms.has_acroform == interactive_after.forms.has_acroform,
        "annotations_preserved": interactive_before.annotations.annotations.len() == interactive_after.annotations.annotations.len(),
        "outlines_preserved": page_ops.outlines_present == interactive_after.page_operations.outlines_present,
        "page_labels_preserved": page_ops.page_labels_present == interactive_after.page_operations.page_labels_present,
        "named_destinations_preserved": page_ops.named_destinations_present == interactive_after.page_operations.named_destinations_present,
        "embedded_files_preserved": page_ops.embedded_files_present == interactive_after.page_operations.embedded_files_present,
        "repair_scope": "append_only: existing page references retain their copied page identity; inserted-page renumbering and non-append destination repair remain refused",
    });
    report.validation_evidence = json!({
        "output_reopened": true,
        "page_count": 2,
        "source_rewrite": first_apply,
        "page_tree_writer": "canonical_writer_append_authored_page_preserving_catalog",
        "extraction_exact_under_layout_whitespace_policy": true,
        "original_source_text_absent": true,
        "catalog_reference_preservation": catalog_reference_preservation,
        "unaffected_content_proof": "all pre-existing catalog objects are copied through the canonical writer; continuation has no inferred association with source interactive objects",
    });
    report.inverse_operation = Some(json!({
        "kind": "exact_preimage_restore",
        "scope": "ReflowMutationSession retained preimage for non-incremental canonical page-tree output",
        "page_count_before": 1,
        "page_count_after": 2,
    }));
    report.undo_proof = json!({
        "status": "requires_reflow_mutation_session_execution",
        "preimage_retained_in_memory": true,
        "atomic": true,
    });
    Ok((output, report))
}

pub fn apply_reflow_document(
    input: &[u8],
    request: &GeometricReflowRequest,
) -> Result<(Vec<u8>, ReflowTransactionReport)> {
    if request.requested_mode != TrueEditingMode::SemanticDocument {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt33 apply_semantic_reflow requires requested_mode=semantic_document".to_string(),
        ));
    }
    let preliminary = preview_reflow(input, request)?;
    if preliminary.overflow_status == OverflowStatus::UnresolvedOverflow {
        // The preserved-style writer deliberately replays existing source
        // instructions in their original local text context.  Continuation
        // flow instead emits one positioned canonical stream, so routing a
        // preserve-per-run request through it would silently downgrade the
        // requested source-style guarantee.  Refuse before any mutation.
        if request.font_policy == "preserve_original_per_run"
            && (request.next_region.is_some()
                || request.next_column.is_some()
                || request.page < ContentEngine::open_bytes(input.to_vec())?.page_count()?
                || request.allow_page_creation)
        {
            return Err(WellfriendError::UnsupportedFeature(
                "prompt33 font_reconstruction_failed: preserve_original_per_run is supported only for a single local region; downstream, column, page, and page-creation flow require the generated Type0 font policy"
                    .to_string(),
            ));
        }
        if request.next_region.is_some() {
            return apply_single_paragraph_existing_next_region_flow(input, request, preliminary);
        }
        if request.next_column.is_some() {
            return apply_single_paragraph_existing_next_column_flow(input, request, preliminary);
        }
        let engine = ContentEngine::open_bytes(input.to_vec())?;
        if request.page < engine.page_count()? {
            return apply_single_paragraph_existing_next_page_flow(input, request, preliminary);
        }
        if request.allow_page_creation {
            return apply_single_paragraph_page_creation(input, request, preliminary);
        }
    }
    let semantic = analyze_semantic_layout(input, Some(request))?;
    let source_region = analyze_geometric_region(input, request)?;
    // Prompt 06's exact glyph/word spans are deliberately not re-labelled as
    // Prompt 31 instruction IDs.  The application boundary therefore uses the
    // existing exact Prompt 31 provenance for *this requested source text* and
    // separately requires one deterministic semantic paragraph group on the
    // selected page.  Counting every paragraph in the document would make a
    // perfectly unambiguous local edit fail merely because unrelated body text
    // exists elsewhere, which is both needlessly restrictive and contrary to
    // Prompt 33's incremental invalidation boundary.
    let source_text_hash = digest_hex(request.source_text.as_bytes());
    let paragraph_nodes = semantic
        .nodes
        .iter()
        .filter(|node| {
            node.node_type == "paragraph"
                && node.page == request.page
                && node.text_hash == source_text_hash
        })
        .collect::<Vec<_>>();
    // An exact text hash is deliberately not a nearest-neighbor heuristic:
    // duplicated paragraph text remains ambiguous and refuses until a stable
    // semantic-node selector is added to the public request model.
    if source_region.source_instructions.is_empty() || paragraph_nodes.len() != 1 {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt33 paragraph_not_resolved: SemanticDocument local application requires exactly one page-local semantic paragraph whose exact text matches the provenance-resolved source selection; duplicate or partial paragraph selections remain review-required"
                .to_string(),
        ));
    }
    let (output, mut report) =
        apply_source_linked_reflow(input, request, TrueEditingMode::SemanticDocument)?;
    report.scope_of_movement = "semantic_single_paragraph_single_region_no_flow".to_string();
    report.flow_graph_changes = vec![json!({
        "kind": "semantic_single_paragraph_reflow",
        "cross_column_or_page_flow": false,
        "source_semantic_node": paragraph_nodes[0].node_id,
        "source_instruction_ids": source_region.source_instructions,
    })];
    Ok((output, report))
}

pub fn analyze_semantic_layout(
    input: &[u8],
    request: Option<&GeometricReflowRequest>,
) -> Result<SemanticLayoutReport> {
    // A local GeometricBlock preview invalidates only its selected page.  A
    // SemanticDocument request, in contrast, needs the bounded document-wide
    // graph to evaluate repeated headers/footers and explicit page-flow
    // candidates.  Keeping the distinction here prevents a silent mode
    // upgrade while avoiding full-document analysis for keystroke previews.
    let graph_pages = match request {
        Some(item) if item.requested_mode == TrueEditingMode::GeometricBlock => vec![item.page],
        _ => Vec::new(),
    };
    let graph = build_scene_graph(input, &graph_pages)?;
    semantic_layout_from_graph(input, &graph, request)
}

fn quad_bounds(quad: crate::text::TextQuad) -> [f64; 4] {
    [quad.x0, quad.y0, quad.x1, quad.y1]
}

fn semantic_role_node_type(role: crate::text::TextRole, fallback: &str) -> &str {
    match role {
        crate::text::TextRole::BodyText => fallback,
        crate::text::TextRole::Heading => "heading",
        crate::text::TextRole::List => "list",
        crate::text::TextRole::TableCandidate => "table_placeholder",
        crate::text::TextRole::FigureCaption => "caption",
        crate::text::TextRole::Header => "header",
        crate::text::TextRole::Footer => "footer",
        crate::text::TextRole::Footnote => "footnote_body",
        crate::text::TextRole::Marginalia => "sidebar",
        crate::text::TextRole::Unknown => fallback,
    }
}

fn scene_node_kind_for_figure(kind: crate::prompt32::SceneNodeKind) -> Option<&'static str> {
    match kind {
        crate::prompt32::SceneNodeKind::ImageObject => Some("image"),
        crate::prompt32::SceneNodeKind::PathObject => Some("path"),
        _ => None,
    }
}

fn rect_gap(left: [f64; 4], right: [f64; 4]) -> f64 {
    let dx = if left[2] < right[0] {
        right[0] - left[2]
    } else if right[2] < left[0] {
        left[0] - right[2]
    } else {
        0.0
    };
    let dy = if left[3] < right[1] {
        right[1] - left[3]
    } else if right[3] < left[1] {
        left[1] - right[3]
    } else {
        0.0
    };
    dx.hypot(dy)
}

fn rects_intersect(left: [f64; 4], right: [f64; 4]) -> bool {
    left[0] < right[2] && left[2] > right[0] && left[1] < right[3] && left[3] > right[1]
}

fn semantic_region_graph_invariants(
    nodes: &[SemanticRegionNode],
    edges: &[SemanticRegionEdge],
    graph_pages: &[usize],
    request: Option<&GeometricReflowRequest>,
) -> Value {
    const MAX_REGION_GRAPH_EDGES: usize = 32_768;
    let ids = nodes
        .iter()
        .map(|node| node.node_id.as_str())
        .collect::<BTreeSet<_>>();
    let stable_ids_unique = ids.len() == nodes.len();
    let finite_nonempty_bounds = nodes.iter().all(|node| {
        node.bounds.iter().all(|value| value.is_finite())
            && node.bounds[2] >= node.bounds[0]
            && node.bounds[3] >= node.bounds[1]
    });
    let no_dangling_edges = edges
        .iter()
        .all(|edge| ids.contains(edge.source.as_str()) && ids.contains(edge.target.as_str()));
    let edge_ids_unique = edges
        .iter()
        .map(|edge| edge.edge_id.as_str())
        .collect::<BTreeSet<_>>()
        .len()
        == edges.len();
    let bounded_edge_count = edges.len() <= MAX_REGION_GRAPH_EDGES;
    let analyzed_pages = nodes.iter().map(|node| node.page).collect::<BTreeSet<_>>();
    let invalidated_pages = if graph_pages.is_empty() {
        analyzed_pages.iter().copied().collect::<Vec<_>>()
    } else {
        graph_pages.to_vec()
    };
    json!({
        "stable_node_ids_unique": stable_ids_unique,
        "stable_edge_ids_unique": edge_ids_unique,
        "no_dangling_edges": no_dangling_edges,
        "finite_nonempty_node_bounds": finite_nonempty_bounds,
        "bounded_edge_count": bounded_edge_count,
        "edge_count_limit": MAX_REGION_GRAPH_EDGES,
        "node_count": nodes.len(),
        "edge_count": edges.len(),
        "analysis_pages": analyzed_pages,
        "incremental_invalidation": {
            "invalidated_pages": invalidated_pages,
            "mode": request.map(|item| item.requested_mode),
            "geometric_block_is_page_local": request.is_some_and(|item| item.requested_mode == TrueEditingMode::GeometricBlock),
            "semantic_document_uses_document_scope_only_when_requested": request.is_none_or(|item| item.requested_mode == TrueEditingMode::SemanticDocument),
            // Only the explicit local GeometricBlock path can reuse pages.
            // A SemanticDocument request intentionally reconstructs the bounded
            // document scope so its cross-page inference is not presented as
            // an incremental result.
            "unaffected_pages_reused_without_full_page_analysis": request.is_some_and(|item| item.requested_mode == TrueEditingMode::GeometricBlock),
        },
        "valid": stable_ids_unique && edge_ids_unique && no_dangling_edges && finite_nonempty_bounds && bounded_edge_count,
    })
}

/// A bounded, deterministic column candidate derived from the canonical
/// semantic block quads.  Prompt 33 deliberately keeps this as a geometry
/// derivation layered on the existing Prompt 06 layout model: it does not
/// tokenize content again or treat paint order as a universal reading order.
#[derive(Debug, Clone)]
struct SemanticColumnCandidate {
    bounds: [f64; 4],
    block_indices: Vec<usize>,
    confidence: f64,
    method: &'static str,
}

/// Derive at most eight column bands from non-spanning canonical text blocks.
///
/// Blocks that span most of the page width (for example a document title or a
/// full-width figure caption) intentionally remain children of the page
/// region rather than being guessed into every column.  This preserves the
/// ambiguity needed by the semantic reflow policy.  A one-column page still
/// receives one explicit Column node so callers can use a uniform region graph
/// without inferring an absent column from a PageRegion.
fn semantic_column_candidates(
    page_box: [f64; 4],
    blocks: &[(usize, [f64; 4])],
) -> Vec<SemanticColumnCandidate> {
    const MAX_COLUMNS: usize = 8;
    let page_width = page_box[2] - page_box[0];
    if !page_width.is_finite() || page_width <= 0.0 {
        return Vec::new();
    }
    let mut valid = blocks
        .iter()
        .copied()
        .filter(|(_, bounds)| {
            bounds.iter().all(|value| value.is_finite())
                && bounds[2] > bounds[0]
                && bounds[3] > bounds[1]
        })
        .collect::<Vec<_>>();
    if valid.is_empty() {
        return vec![SemanticColumnCandidate {
            bounds: page_box,
            block_indices: Vec::new(),
            confidence: 0.50,
            method: "empty_page_fallback",
        }];
    }
    // A modest gap is sufficient to recognize ordinary newspaper columns,
    // while overlap keeps paragraphs in a shared x band together.  The cap
    // keeps adversarial sets from producing unbounded graph edges.
    let projection_gap = (page_width * 0.03).clamp(8.0, 24.0);
    let spanning_width = page_width * 0.72;
    let mut non_spanning = valid
        .iter()
        .copied()
        .filter(|(_, bounds)| bounds[2] - bounds[0] < spanning_width)
        .collect::<Vec<_>>();
    non_spanning.sort_by(|left, right| {
        left.1[0]
            .partial_cmp(&right.1[0])
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.0.cmp(&right.0))
    });

    let mut groups = Vec::<Vec<(usize, [f64; 4])>>::new();
    for block in non_spanning {
        let joins_last = groups.last().is_some_and(|group| {
            let right = group
                .iter()
                .map(|(_, bounds)| bounds[2])
                .fold(f64::NEG_INFINITY, f64::max);
            block.1[0] <= right + projection_gap
        });
        if joins_last {
            groups.last_mut().expect("last group exists").push(block);
        } else if groups.len() < MAX_COLUMNS {
            groups.push(vec![block]);
        }
    }

    // One or zero narrow groups do not constitute evidence for a multi-column
    // page.  Keep every source block linked to the single page-wide column.
    if groups.len() <= 1 {
        valid.sort_by_key(|(index, _)| *index);
        return vec![SemanticColumnCandidate {
            bounds: page_box,
            block_indices: valid.into_iter().map(|(index, _)| index).collect(),
            confidence: 0.82,
            method: "single_column_page_geometry",
        }];
    }

    groups
        .into_iter()
        .map(|group| {
            let left = group
                .iter()
                .map(|(_, bounds)| bounds[0])
                .fold(f64::INFINITY, f64::min)
                .max(page_box[0]);
            let right = group
                .iter()
                .map(|(_, bounds)| bounds[2])
                .fold(f64::NEG_INFINITY, f64::max)
                .min(page_box[2]);
            SemanticColumnCandidate {
                bounds: [left, page_box[1], right, page_box[3]],
                block_indices: group.into_iter().map(|(index, _)| index).collect(),
                confidence: 0.90,
                method: "canonical_block_x_projection_clusters",
            }
        })
        .collect()
}

/// An explicit dependency list is still bounded so a local reflow cannot turn
/// into an unbounded page reconstruction.  Every selected path is source
/// resolved and all unselected scene objects remain hard obstacles.
const MAX_EXECUTABLE_DOWNSTREAM_VECTOR_MOVES: usize = 8;

fn approved_downstream_vector_relationship(relationship: &str) -> bool {
    matches!(
        relationship,
        "keep_with_next"
            | "heading_for"
            | "list_continuation"
            | "caption_of"
            | "footnote_of"
            | "source_link"
            | "next_region"
            | "next_column"
            | "next_page"
    )
}

fn rects_nearly_equal(left: [f64; 4], right: [f64; 4]) -> bool {
    left.iter()
        .zip(right)
        .all(|(first, second)| (first - second).abs() <= 0.01)
}

/// Validate the narrow executable movement boundary before a text transaction
/// is started.  This uses the existing Prompt 20 vector inventory and Prompt
/// 32 scene graph, so identity, clipping, transform, shared-Form ownership,
/// and collision facts remain tied to source occurrences rather than inferred
/// from a replacement layer.
fn validate_downstream_vector_moves(
    input: &[u8],
    request: &GeometricReflowRequest,
) -> Result<Vec<Value>> {
    if request.downstream_vector_moves.is_empty() {
        return Ok(Vec::new());
    }
    if request.downstream_vector_moves.len() > MAX_EXECUTABLE_DOWNSTREAM_VECTOR_MOVES
        || request.downstream_vector_moves.len() > request.max_downstream_blocks
    {
        return Err(WellfriendError::ResourceLimit(format!(
            "prompt33 resource_limit_exceeded: executable downstream movement supports at most {MAX_EXECUTABLE_DOWNSTREAM_VECTOR_MOVES} explicitly linked path object per transaction"
        )));
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let interactive = interactive_report(&engine)?;
    if interactive.forms.has_acroform || !interactive.annotations.annotations.is_empty() {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt33 destination_update_failed: downstream vector movement refuses documents with forms or annotations until a source-linked association/rectangle repair transaction is available"
                .to_string(),
        ));
    }
    let page_box = engine.page_box(request.page).map_err(|_| {
        WellfriendError::UnsupportedFeature(
            "prompt33 region_not_resolved: downstream movement source page is unavailable"
                .to_string(),
        )
    })?;
    let source_region = region_for_request(input, request)?;
    let scene = build_scene_graph(input, &[request.page])?;
    let semantic = engine.extract_text_semantic_model(
        &[request.page],
        crate::text::TextSemanticOptions::default(),
    )?;
    let inventory = list_vector_objects(input, request.page)?;
    let selected_vector_ids = request
        .downstream_vector_moves
        .iter()
        .map(|movement| movement.vector_stable_id.as_str())
        .collect::<BTreeSet<_>>();
    let selected_source_bounds = inventory
        .objects
        .iter()
        .filter(|candidate| selected_vector_ids.contains(candidate.stable_id.as_str()))
        .map(|candidate| candidate.bbox)
        .collect::<Vec<_>>();
    let mut seen = BTreeSet::<String>::new();
    let mut plans = Vec::new();
    for movement in &request.downstream_vector_moves {
        if movement.vector_stable_id.trim().is_empty()
            || movement.dependency_edge_id.trim().is_empty()
        {
            return Err(WellfriendError::UnsupportedFeature(
                "prompt33 locked_object_conflict: downstream movement requires a non-empty source vector stable ID and explicit dependency edge ID"
                    .to_string(),
            ));
        }
        if !seen.insert(movement.vector_stable_id.clone()) {
            return Err(WellfriendError::UnsupportedFeature(
                "prompt33 locked_object_conflict: one source vector may not be moved twice in the same transaction"
                    .to_string(),
            ));
        }
        if !approved_downstream_vector_relationship(&movement.relationship) {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "prompt33 ambiguous_layout: unsupported downstream dependency relationship {}; explicit approved relationships are keep_with_next, heading_for, list_continuation, caption_of, footnote_of, next_region, next_column, and next_page",
                movement.relationship
            )));
        }
        if !movement.dx.is_finite()
            || !movement.dy.is_finite()
            || (movement.dx.abs() <= f64::EPSILON && movement.dy.abs() <= f64::EPSILON)
        {
            return Err(WellfriendError::MalformedPdf(
                "prompt33 constraint_infeasible: downstream movement requires finite non-zero dx or dy"
                    .to_string(),
            ));
        }
        let vector = inventory
            .objects
            .iter()
            .find(|candidate| candidate.stable_id == movement.vector_stable_id)
            .ok_or_else(|| {
                WellfriendError::UnsupportedFeature(format!(
                    "prompt33 locked_object_conflict: explicit downstream vector {} is not a source-resolved editable path on page {}",
                    movement.vector_stable_id, request.page
                ))
            })?;
        if vector.clipping_path
            || vector.clipping_context
            || vector.provenance.marked_content_depth != 0
            || vector.provenance.ocg_context.is_some()
        {
            return Err(WellfriendError::UnsupportedFeature(
                "prompt33 structure_update_failed: downstream movement refuses vector paths in clipping, marked-content, or optional-content contexts"
                    .to_string(),
            ));
        }
        if vector.provenance.form_invocation.is_some()
            && movement.shared_form_policy == SharedFormEditPolicy::Reject
        {
            return Err(WellfriendError::UnsupportedFeature(
                "prompt33 locked_object_conflict: a downstream vector owned by a Form XObject requires an explicit Prompt20 shared_form_policy"
                    .to_string(),
            ));
        }
        let own_scene_nodes = scene
            .nodes
            .iter()
            .filter(|node| {
                node.node_kind == crate::prompt32::SceneNodeKind::PathObject
                    && rects_nearly_equal(node.bounds_user_space, vector.bbox)
            })
            .collect::<Vec<_>>();
        if own_scene_nodes.len() != 1 {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "prompt33 ambiguous_layout: vector {} does not have exactly one matching Prompt32 scene occurrence",
                movement.vector_stable_id
            )));
        }
        let target = [
            vector.bbox[0] + movement.dx,
            vector.bbox[1] + movement.dy,
            vector.bbox[2] + movement.dx,
            vector.bbox[3] + movement.dy,
        ];
        if target[0] < page_box[0]
            || target[1] < page_box[1]
            || target[2] > page_box[2]
            || target[3] > page_box[3]
        {
            return Err(WellfriendError::UnsupportedFeature(
                "prompt33 constraint_infeasible: downstream vector target lies outside the page box"
                    .to_string(),
            ));
        }
        if rects_intersect(target, source_region) {
            return Err(WellfriendError::UnsupportedFeature(
                "prompt33 locked_object_conflict: downstream vector target overlaps the edited text region"
                    .to_string(),
            ));
        }
        if let Some(node) = scene.nodes.iter().find(|node| {
            node.node_id != own_scene_nodes[0].node_id
                && node.node_kind != crate::prompt32::SceneNodeKind::TextObject
                && node.visibility != "hidden"
                && !selected_source_bounds
                    .iter()
                    .any(|bounds| rects_nearly_equal(node.bounds_user_space, *bounds))
                && rects_intersect(node.bounds_user_space, target)
        }) {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "prompt33 locked_object_conflict: downstream vector target overlaps source-linked {} scene node {}",
                format!("{:?}", node.node_kind).to_ascii_lowercase(),
                node.node_id
            )));
        }
        if let Some(line) = semantic
            .pages
            .iter()
            .flat_map(|page| page.blocks.iter())
            .flat_map(|block| block.lines.iter())
            .find(|line| {
                line.text != request.source_text && rects_intersect(quad_bounds(line.quad), target)
            })
        {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "prompt33 locked_object_conflict: downstream vector target overlaps source-linked text line with hash {}",
                digest_hex(line.text.as_bytes())
            )));
        }
        plans.push(json!({
            "vector_stable_id": movement.vector_stable_id,
            "scene_node_id": own_scene_nodes[0].node_id,
            "source_stream_object": vector.provenance.object_number,
            "source_operation_byte_start": vector.provenance.operation_byte_start,
            "relationship": movement.relationship,
            "dependency_edge_id": movement.dependency_edge_id,
            "before_bounds": vector.bbox,
            "after_bounds": target,
            "shared_form_policy": movement.shared_form_policy,
            "evidence": Prompt33EvidenceKind::UserCorrection,
        }));
    }
    for (index, left) in plans.iter().enumerate() {
        let Some(left_bounds) = left["after_bounds"].as_array() else {
            return Err(WellfriendError::MalformedPdf(
                "prompt33 constraint_infeasible: downstream vector plan has invalid target bounds"
                    .to_string(),
            ));
        };
        let left_bounds = [
            left_bounds[0].as_f64().unwrap_or(f64::NAN),
            left_bounds[1].as_f64().unwrap_or(f64::NAN),
            left_bounds[2].as_f64().unwrap_or(f64::NAN),
            left_bounds[3].as_f64().unwrap_or(f64::NAN),
        ];
        for right in plans.iter().skip(index + 1) {
            let Some(right_bounds) = right["after_bounds"].as_array() else {
                return Err(WellfriendError::MalformedPdf(
                    "prompt33 constraint_infeasible: downstream vector plan has invalid target bounds"
                        .to_string(),
                ));
            };
            let right_bounds = [
                right_bounds[0].as_f64().unwrap_or(f64::NAN),
                right_bounds[1].as_f64().unwrap_or(f64::NAN),
                right_bounds[2].as_f64().unwrap_or(f64::NAN),
                right_bounds[3].as_f64().unwrap_or(f64::NAN),
            ];
            if rects_intersect(left_bounds, right_bounds) {
                return Err(WellfriendError::UnsupportedFeature(
                    "prompt33 locked_object_conflict: two explicitly movable downstream vectors have overlapping target bounds"
                        .to_string(),
                ));
            }
        }
    }
    Ok(plans)
}

fn apply_downstream_vector_moves(
    input: &[u8],
    request: &GeometricReflowRequest,
) -> Result<(Vec<u8>, Vec<Value>)> {
    let plans = validate_downstream_vector_moves(input, request)?;
    if plans.is_empty() {
        return Ok((input.to_vec(), plans));
    }
    let mut output = input.to_vec();
    let mut applied = plans;
    // Prompt20 vector IDs include their decoded source operation range.  For
    // two operations in the same stream, execute the later range first so its
    // replacement cannot shift the provenance offset of an earlier selected
    // path. Different streams are source-independent.
    let mut execution = request
        .downstream_vector_moves
        .iter()
        .enumerate()
        .collect::<Vec<_>>();
    execution.sort_by(|(left_index, _), (right_index, _)| {
        let left_stream = applied[*left_index]["source_stream_object"]
            .as_u64()
            .unwrap_or(0);
        let right_stream = applied[*right_index]["source_stream_object"]
            .as_u64()
            .unwrap_or(0);
        let left_offset = applied[*left_index]["source_operation_byte_start"]
            .as_u64()
            .unwrap_or(0);
        let right_offset = applied[*right_index]["source_operation_byte_start"]
            .as_u64()
            .unwrap_or(0);
        left_stream
            .cmp(&right_stream)
            .then_with(|| right_offset.cmp(&left_offset))
    });
    for (index, movement) in execution {
        let (next_output, report) = edit_vector_object(
            &output,
            request.page,
            &movement.vector_stable_id,
            VectorEditOperation::Move {
                dx: movement.dx,
                dy: movement.dy,
            },
            &VectorEditOptions {
                signature_policy_override: request.signature_policy_override,
                deterministic: true,
                shared_form_policy: movement.shared_form_policy,
            },
        )?;
        let reopened = ContentEngine::open_bytes(next_output.clone())?;
        if reopened.page_count()? == 0 {
            return Err(WellfriendError::MalformedPdf(
                "prompt33 output_reopen_failed: downstream vector movement output has no page"
                    .to_string(),
            ));
        }
        output = next_output;
        applied[index]["vector_edit"] = json!(report);
        applied[index]["output_reopened"] = Value::Bool(true);
    }
    Ok((output, applied))
}

const MAX_EXECUTABLE_DOWNSTREAM_LINK_MOVES: usize = 8;

/// Validate an explicit source-associated Link annotation before a reflow
/// transaction starts. Annotation proximity is never treated as sufficient
/// evidence: the caller supplies the source preimage rect, an approved
/// dependency edge, and a delta, while the current page annotation inventory
/// proves it is still the same `/Link` occurrence.
fn validate_downstream_link_moves(
    input: &[u8],
    request: &GeometricReflowRequest,
) -> Result<Vec<Value>> {
    if request.downstream_link_moves.is_empty() {
        return Ok(Vec::new());
    }
    if request.downstream_link_moves.len() > MAX_EXECUTABLE_DOWNSTREAM_LINK_MOVES
        || request.downstream_link_moves.len() > request.max_downstream_blocks
    {
        return Err(WellfriendError::ResourceLimit(format!(
            "prompt33 resource_limit_exceeded: executable source-linked Link movement supports at most {MAX_EXECUTABLE_DOWNSTREAM_LINK_MOVES} annotation per transaction"
        )));
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let source_region = region_for_request(input, request)?;
    let page_box = engine.page_box(request.page).map_err(|_| {
        WellfriendError::UnsupportedFeature(
            "prompt33 region_not_resolved: source-associated Link page is unavailable".to_string(),
        )
    })?;
    let interactive = interactive_report(&engine)?;
    let selected_annotation_indexes = request
        .downstream_link_moves
        .iter()
        .map(|movement| movement.annotation_index)
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    let mut plans = Vec::new();
    for movement in &request.downstream_link_moves {
        if movement.dependency_edge_id.trim().is_empty()
            || !approved_downstream_vector_relationship(&movement.relationship)
        {
            return Err(WellfriendError::UnsupportedFeature(
                "prompt33 ambiguous_layout: source-associated Link movement requires a non-empty dependency edge and an approved relationship"
                    .to_string(),
            ));
        }
        if !movement.expected_rect.iter().all(|value| value.is_finite())
            || !movement.dx.is_finite()
            || !movement.dy.is_finite()
            || (movement.dx.abs() <= f64::EPSILON && movement.dy.abs() <= f64::EPSILON)
        {
            return Err(WellfriendError::MalformedPdf(
                "prompt33 constraint_infeasible: source-associated Link movement requires finite expected geometry and a non-zero finite delta"
                    .to_string(),
            ));
        }
        if !seen.insert(movement.annotation_index) {
            return Err(WellfriendError::UnsupportedFeature(
                "prompt33 locked_object_conflict: one Link annotation may not be moved twice in the same transaction"
                    .to_string(),
            ));
        }
        let annotation = interactive
            .annotations
            .annotations
            .iter()
            .find(|annotation| {
                annotation.page == request.page && annotation.index == movement.annotation_index
            })
            .ok_or_else(|| {
                WellfriendError::UnsupportedFeature(format!(
                    "prompt33 locked_object_conflict: Link annotation {} is not present on source page {}",
                    movement.annotation_index, request.page
                ))
            })?;
        if annotation.subtype != "Link" {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "prompt33 locked_object_conflict: annotation {} is {}, not a source-associated Link",
                movement.annotation_index, annotation.subtype
            )));
        }
        let actual = annotation.rect.ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "prompt33 structure_update_failed: source-associated Link has no finite /Rect"
                    .to_string(),
            )
        })?;
        if !rects_nearly_equal(actual, movement.expected_rect) {
            return Err(WellfriendError::UnsupportedFeature(
                "prompt33 stale_snapshot: source-associated Link rectangle differs from the request preimage"
                    .to_string(),
            ));
        }
        if !rects_intersect(actual, source_region) {
            return Err(WellfriendError::UnsupportedFeature(
                "prompt33 ambiguous_layout: explicit Link rectangle does not overlap the selected source region"
                    .to_string(),
            ));
        }
        let target = [
            actual[0] + movement.dx,
            actual[1] + movement.dy,
            actual[2] + movement.dx,
            actual[3] + movement.dy,
        ];
        if target[0] < page_box[0]
            || target[1] < page_box[1]
            || target[2] > page_box[2]
            || target[3] > page_box[3]
        {
            return Err(WellfriendError::UnsupportedFeature(
                "prompt33 constraint_infeasible: source-associated Link target lies outside the page box"
                    .to_string(),
            ));
        }
        if let Some(other) = interactive.annotations.annotations.iter().find(|other| {
            other.page == request.page
                && other.index != movement.annotation_index
                && !selected_annotation_indexes.contains(&other.index)
                && other
                    .rect
                    .is_some_and(|other_rect| rects_intersect(other_rect, target))
        }) {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "prompt33 locked_object_conflict: source-associated Link target overlaps locked annotation {} on page {}",
                other.index, request.page
            )));
        }
        plans.push(json!({
            "annotation_index": movement.annotation_index,
            "annotation_object": annotation.object,
            "relationship": movement.relationship,
            "dependency_edge_id": movement.dependency_edge_id,
            "before_rect": actual,
            "after_rect": target,
            "action": annotation.action,
            "quad_points_count": annotation.quad_points.len(),
            "evidence": Prompt33EvidenceKind::UserCorrection,
        }));
    }
    for (index, left) in plans.iter().enumerate() {
        let left_rect = left["after_rect"].as_array().ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "prompt33 constraint_infeasible: source-associated Link plan has invalid target geometry"
                    .to_string(),
            )
        })?;
        let left_rect = [
            left_rect[0].as_f64().unwrap_or(f64::NAN),
            left_rect[1].as_f64().unwrap_or(f64::NAN),
            left_rect[2].as_f64().unwrap_or(f64::NAN),
            left_rect[3].as_f64().unwrap_or(f64::NAN),
        ];
        for right in plans.iter().skip(index + 1) {
            let right_rect = right["after_rect"].as_array().ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "prompt33 constraint_infeasible: source-associated Link plan has invalid target geometry"
                        .to_string(),
                )
            })?;
            let right_rect = [
                right_rect[0].as_f64().unwrap_or(f64::NAN),
                right_rect[1].as_f64().unwrap_or(f64::NAN),
                right_rect[2].as_f64().unwrap_or(f64::NAN),
                right_rect[3].as_f64().unwrap_or(f64::NAN),
            ];
            if rects_intersect(left_rect, right_rect) {
                return Err(WellfriendError::UnsupportedFeature(
                    "prompt33 locked_object_conflict: two explicitly movable source-associated Links have overlapping target rectangles"
                        .to_string(),
                ));
            }
        }
    }
    Ok(plans)
}

fn apply_downstream_link_moves(
    input: &[u8],
    request: &GeometricReflowRequest,
) -> Result<(Vec<u8>, Vec<Value>)> {
    let plans = validate_downstream_link_moves(input, request)?;
    if plans.is_empty() {
        return Ok((input.to_vec(), plans));
    }
    let mut output = input.to_vec();
    let mut applied = plans;
    for (index, movement) in request.downstream_link_moves.iter().enumerate() {
        let (next_output, report) = move_link_annotation_rect_pdf(
            &output,
            request.page,
            movement.annotation_index,
            movement.expected_rect,
            movement.dx,
            movement.dy,
            request.signature_policy_override,
        )?;
        ContentEngine::open_bytes(next_output.clone())?;
        output = next_output;
        applied[index]["link_annotation_move"] = json!(report);
        applied[index]["output_reopened"] = Value::Bool(true);
    }
    Ok((output, applied))
}

/// Normalize only enough to recognize repeated running regions. Digits are
/// intentionally collapsed so page-number variants can be matched, while a
/// position-band and multi-page requirement prevent a coincidental body repeat
/// from becoming an artifact classification.
fn repeated_region_key(text: &str) -> String {
    let mut normalized = String::new();
    let mut previous_space = false;
    for character in text.chars() {
        if character.is_ascii_digit() {
            normalized.push('#');
            previous_space = false;
        } else if character.is_alphanumeric() {
            normalized.extend(character.to_lowercase());
            previous_space = false;
        } else if !previous_space {
            normalized.push(' ');
            previous_space = true;
        }
    }
    normalized.trim().to_string()
}

fn footnote_body_label(text: &str) -> Option<String> {
    let trimmed = text.trim_start();
    let first = trimmed.chars().next()?;
    if matches!(first, '*' | '†' | '‡') {
        return Some(first.to_string());
    }
    let digits = trimmed
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>();
    if digits.is_empty() {
        return None;
    }
    let tail = trimmed[digits.len()..].chars().next();
    tail.is_none_or(|character| character.is_whitespace() || matches!(character, '.' | ')' | ']'))
        .then_some(digits)
}

fn inline_footnote_marker_label(text: &str) -> Option<(String, usize)> {
    text.char_indices().find_map(|(offset, character)| {
        let label = match character {
            '⁰' => "0",
            '¹' => "1",
            '²' => "2",
            '³' => "3",
            '⁴' => "4",
            '⁵' => "5",
            '⁶' => "6",
            '⁷' => "7",
            '⁸' => "8",
            '⁹' => "9",
            '*' => "*",
            '†' => "†",
            '‡' => "‡",
            _ => return None,
        };
        Some((label.to_string(), offset))
    })
}

fn nearest_figure(figures: &[(String, [f64; 4])], caption: [f64; 4]) -> Option<(String, f64)> {
    figures
        .iter()
        .map(|(id, bounds)| (id.clone(), rect_gap(*bounds, caption)))
        .min_by(|left, right| {
            left.1
                .partial_cmp(&right.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.0.cmp(&right.0))
        })
}

/// Resolve the precedence portion of the canonical region graph.  Geometry and
/// structure may supply conflicting preferences, so cycles are never ignored:
/// the lowest-confidence edge is removed with a stable ID tie-break and the
/// decision is retained for review.  Containment edges are deliberately not
/// part of the precedence DAG.
fn resolve_reading_order_graph(
    nodes: &[SemanticRegionNode],
    edges: &[SemanticRegionEdge],
) -> Value {
    let node_ids = nodes
        .iter()
        .map(|node| node.node_id.clone())
        .collect::<BTreeSet<_>>();
    let mut active = edges
        .iter()
        .filter(|edge| {
            matches!(
                edge.relationship.as_str(),
                "next_reading" | "next_region" | "next_column" | "next_page"
            ) && edge.source != edge.target
                && node_ids.contains(&edge.source)
                && node_ids.contains(&edge.target)
        })
        .cloned()
        .collect::<Vec<_>>();
    active.sort_by(|left, right| left.edge_id.cmp(&right.edge_id));
    let mut removed = Vec::<Value>::new();
    let mut order = Vec::<String>::new();
    let mut completed = BTreeSet::<String>::new();
    while completed.len() < node_ids.len() {
        let remaining = node_ids
            .iter()
            .filter(|node| !completed.contains(*node))
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut incoming = BTreeMap::<String, usize>::new();
        for node in &remaining {
            incoming.insert(node.clone(), 0);
        }
        for edge in &active {
            if remaining.contains(&edge.source) && remaining.contains(&edge.target) {
                *incoming.entry(edge.target.clone()).or_default() += 1;
            }
        }
        let ready = incoming
            .iter()
            .filter(|(_, degree)| **degree == 0)
            .map(|(node, _)| node.clone())
            .collect::<BTreeSet<_>>();
        if let Some(node) = ready.into_iter().next() {
            completed.insert(node.clone());
            order.push(node);
            continue;
        }
        let candidate = active
            .iter()
            .filter(|edge| remaining.contains(&edge.source) && remaining.contains(&edge.target))
            .min_by(|left, right| {
                left.confidence
                    .partial_cmp(&right.confidence)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| left.edge_id.cmp(&right.edge_id))
            })
            .cloned();
        let Some(edge) = candidate else {
            // A malformed graph with no candidate edges still has a stable,
            // complete machine order and cannot loop forever.
            let node = remaining
                .into_iter()
                .next()
                .expect("nonempty remaining nodes");
            completed.insert(node.clone());
            order.push(node);
            continue;
        };
        active.retain(|item| item.edge_id != edge.edge_id);
        removed.push(json!({
            "edge_id": edge.edge_id,
            "source": edge.source,
            "target": edge.target,
            "relationship": edge.relationship,
            "confidence": edge.confidence,
            "reason": "lowest_confidence_edge_in_detected_precedence_cycle",
        }));
    }
    let review_required = removed
        .iter()
        .any(|edge| edge["confidence"].as_f64().unwrap_or(0.0) < 0.70);
    json!({
        "machine_order": order,
        "active_precedence_edge_count": active.len(),
        "removed_cycle_edges": removed,
        "cycle_count": removed.len(),
        "cycle_break_policy": "remove_lowest_confidence_then_edge_id",
        "review_required": review_required,
        "deterministic": true,
    })
}

/// Score a resolved reading order against an explicitly annotated fixture.
///
/// Production documents do not carry a ground-truth order, so the runtime
/// report never invents an accuracy number.  This helper is instead used by
/// owned fixture tests and validation harnesses: the expected node order,
/// column sequence, and footnote IDs are supplied by the fixture author.  It
/// deliberately ignores unknown IDs rather than silently treating them as
/// correct, and records malformed annotations in the returned evidence.
pub fn score_reading_order_fixture(
    expected: &[String],
    actual: &[String],
    expected_columns: &[Vec<String>],
    footnote_ids: &[String],
) -> Value {
    let expected_ids = expected.iter().collect::<BTreeSet<_>>();
    let actual_ids = actual.iter().collect::<BTreeSet<_>>();
    let expected_unique = expected_ids.len() == expected.len();
    let actual_unique = actual_ids.len() == actual.len();
    let actual_positions = actual
        .iter()
        .enumerate()
        .map(|(index, id)| (id.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    let mut comparable_pairs = 0usize;
    let mut concordant_pairs = 0usize;
    let mut discordant_pairs = 0usize;
    for (left_index, left) in expected.iter().enumerate() {
        let Some(left_actual) = actual_positions.get(left.as_str()) else {
            continue;
        };
        for right in expected.iter().skip(left_index + 1) {
            let Some(right_actual) = actual_positions.get(right.as_str()) else {
                continue;
            };
            comparable_pairs += 1;
            if left_actual < right_actual {
                concordant_pairs += 1;
            } else {
                discordant_pairs += 1;
            }
        }
    }
    let kendall_style_correlation = if comparable_pairs == 0 {
        if expected == actual {
            1.0
        } else {
            0.0
        }
    } else {
        (concordant_pairs as f64 - discordant_pairs as f64) / comparable_pairs as f64
    };

    let mut column_pairs = 0usize;
    let mut correct_column_pairs = 0usize;
    for (left_column_index, left_column) in expected_columns.iter().enumerate() {
        for right_column in expected_columns.iter().skip(left_column_index + 1) {
            for left in left_column {
                let Some(left_actual) = actual_positions.get(left.as_str()) else {
                    continue;
                };
                for right in right_column {
                    let Some(right_actual) = actual_positions.get(right.as_str()) else {
                        continue;
                    };
                    column_pairs += 1;
                    if left_actual < right_actual {
                        correct_column_pairs += 1;
                    }
                }
            }
        }
    }
    let column_order_accuracy = if column_pairs == 0 {
        1.0
    } else {
        correct_column_pairs as f64 / column_pairs as f64
    };

    let footnotes = footnote_ids.iter().collect::<BTreeSet<_>>();
    let mut footnote_pairs = 0usize;
    let mut correctly_placed_footnotes = 0usize;
    for footnote in footnote_ids {
        let Some(footnote_actual) = actual_positions.get(footnote.as_str()) else {
            continue;
        };
        for body in expected {
            if footnotes.contains(body) {
                continue;
            }
            let Some(body_actual) = actual_positions.get(body.as_str()) else {
                continue;
            };
            footnote_pairs += 1;
            if body_actual < footnote_actual {
                correctly_placed_footnotes += 1;
            }
        }
    }
    let footnote_placement_accuracy = if footnote_pairs == 0 {
        1.0
    } else {
        correctly_placed_footnotes as f64 / footnote_pairs as f64
    };
    let expected_coverage = expected
        .iter()
        .filter(|id| actual_positions.contains_key(id.as_str()))
        .count();
    let annotation_valid = expected_unique
        && actual_unique
        && expected_columns
            .iter()
            .flatten()
            .all(|id| expected_ids.contains(id));
    json!({
        "annotation_valid": annotation_valid,
        "expected_node_count": expected.len(),
        "actual_node_count": actual.len(),
        "expected_coverage": expected_coverage,
        "exact_order_accuracy": if expected == actual { 1.0 } else { 0.0 },
        "kendall_style_correlation": kendall_style_correlation,
        "comparable_pair_count": comparable_pairs,
        "concordant_pair_count": concordant_pairs,
        "discordant_pair_count": discordant_pairs,
        "column_order_accuracy": column_order_accuracy,
        "column_pair_count": column_pairs,
        "footnote_placement_accuracy": footnote_placement_accuracy,
        "footnote_pair_count": footnote_pairs,
        "deterministic": true,
    })
}

fn semantic_layout_from_graph(
    input: &[u8],
    graph: &EditableSceneGraph,
    request: Option<&GeometricReflowRequest>,
) -> Result<SemanticLayoutReport> {
    const MAX_RUNTIME_SEMANTIC_NODES: usize = 16_384;
    const MAX_RUNTIME_SEMANTIC_EDGES: usize = 32_768;

    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let requested_pages = match request {
        Some(item) if item.requested_mode == TrueEditingMode::GeometricBlock => vec![item.page],
        _ => (1..=graph.page_count.min(64)).collect(),
    };
    // This is the existing Prompt 06 semantic model and XY-cut layout engine,
    // not a Prompt 33 parser.  It carries geometry, text, word/char spans,
    // MCID/structure facts, role evidence, bidi direction, and bounded limits.
    let semantic = engine.extract_text_semantic_model(
        &requested_pages,
        crate::text::TextSemanticOptions::default(),
    )?;
    // Preserve the exact analysis scope before the canonical semantic pages
    // are consumed to construct runtime nodes below.
    let analyzed_graph_pages = semantic
        .pages
        .iter()
        .map(|page| page.page)
        .collect::<Vec<_>>();
    // Repeated headers/footers are a deterministic ensemble member layered on
    // the canonical semantic blocks, not a second layout parser. A candidate
    // must recur on multiple pages in the same top/bottom page band.
    let mut repeated_candidates = BTreeMap::<String, Vec<(usize, usize, usize, String)>>::new();
    for page in &semantic.pages {
        let page_height = page.page_box[3] - page.page_box[1];
        if !page_height.is_finite() || page_height <= 0.0 {
            continue;
        }
        for block in &page.blocks {
            for paragraph in &block.paragraphs {
                let bounds = quad_bounds(paragraph.quad);
                let placement = if bounds[3] >= page.page_box[3] - page_height * 0.18 {
                    Some("header")
                } else if bounds[1] <= page.page_box[1] + page_height * 0.18 {
                    Some("footer")
                } else {
                    None
                };
                let Some(placement) = placement else {
                    continue;
                };
                let key = repeated_region_key(&paragraph.text);
                if key.len() >= 2 {
                    repeated_candidates.entry(key).or_default().push((
                        page.page,
                        block.block_index,
                        paragraph.paragraph_index,
                        placement.to_string(),
                    ));
                }
            }
        }
    }
    let mut repeated_header_footer = BTreeMap::<(usize, usize, usize), String>::new();
    for candidates in repeated_candidates.into_values() {
        let pages = candidates
            .iter()
            .map(|(page, _, _, _)| *page)
            .collect::<BTreeSet<_>>();
        if pages.len() < 2 {
            continue;
        }
        let first_placement = &candidates[0].3;
        if candidates
            .iter()
            .all(|(_, _, _, placement)| placement == first_placement)
        {
            for (page, block_index, paragraph_index, placement) in candidates {
                repeated_header_footer.insert((page, block_index, paragraph_index), placement);
            }
        }
    }
    let mut nodes = Vec::<SemanticRegionNode>::new();
    let mut edges = Vec::<SemanticRegionEdge>::new();
    let mut reading_nodes = Vec::<String>::new();
    let mut footnote_markers = Vec::<(usize, String, String)>::new();
    let mut footnote_bodies = Vec::<(usize, String, String)>::new();
    let mut add_node = |node: SemanticRegionNode| -> Result<String> {
        if nodes.len() >= MAX_RUNTIME_SEMANTIC_NODES {
            return Err(WellfriendError::ResourceLimit(format!(
                "prompt33 resource_limit_exceeded: semantic runtime node cap {MAX_RUNTIME_SEMANTIC_NODES}"
            )));
        }
        let id = node.node_id.clone();
        nodes.push(node);
        Ok(id)
    };
    let mut add_edge = |source: String,
                        target: String,
                        relationship: &str,
                        confidence: f64,
                        evidence: Prompt33EvidenceKind,
                        source_evidence: Value|
     -> Result<()> {
        let edge_id = stable_id(
            "semantic-edge",
            &[
                source.as_bytes(),
                target.as_bytes(),
                relationship.as_bytes(),
            ],
        );
        // Multiple deterministic ensemble members can independently support
        // the same relationship. Keep one canonical edge ID and preserve the
        // additional evidence as alternatives instead of emitting duplicate
        // IDs that make graph serialization and incremental updates unsafe.
        if let Some(existing) = edges.iter_mut().find(|edge| edge.edge_id == edge_id) {
            let evidence_changed = existing.source_evidence != source_evidence;
            if confidence > existing.confidence {
                existing.alternatives.push(json!({
                    "confidence": existing.confidence,
                    "evidence_kind": existing.exact_inferred_or_user_supplied,
                    "source_evidence": existing.source_evidence.clone(),
                    "reason": "lower_confidence_canonical_evidence_replaced",
                }));
                existing.confidence = confidence;
                existing.exact_inferred_or_user_supplied = evidence;
                existing.source_evidence = source_evidence;
            } else if evidence_changed {
                existing.alternatives.push(json!({
                    "confidence": confidence,
                    "evidence_kind": evidence,
                    "source_evidence": source_evidence,
                    "reason": "duplicate_relation_evidence_merged",
                }));
            }
            return Ok(());
        }
        if edges.len() >= MAX_RUNTIME_SEMANTIC_EDGES {
            return Err(WellfriendError::ResourceLimit(format!(
                "prompt33 resource_limit_exceeded: semantic runtime edge cap {MAX_RUNTIME_SEMANTIC_EDGES}"
            )));
        }
        edges.push(SemanticRegionEdge {
            edge_id,
            source,
            target,
            relationship: relationship.to_string(),
            confidence,
            exact_inferred_or_user_supplied: evidence,
            source_evidence,
            alternatives: Vec::new(),
        });
        Ok(())
    };

    for page in semantic.pages {
        let page_scene_nodes = graph
            .nodes
            .iter()
            .filter(|node| node.page == page.page)
            .map(|node| node.node_id.clone())
            .collect::<Vec<_>>();
        let page_source_instructions = graph
            .nodes
            .iter()
            .filter(|node| {
                node.page == page.page
                    && node.node_kind == crate::prompt32::SceneNodeKind::TextObject
            })
            .flat_map(|node| node.source_instruction_ids.clone())
            .collect::<Vec<_>>();
        let page_region_id = stable_id(
            "page-region",
            &[graph.document_id.as_bytes(), &page.page.to_le_bytes()],
        );
        let page_region = add_node(SemanticRegionNode {
            node_id: page_region_id.clone(),
            node_type: "page_region".to_string(),
            page: page.page,
            source_scene_nodes: page_scene_nodes.clone(),
            source_instructions: page_source_instructions.clone(),
            bounds: page.page_box,
            text_hash: digest_hex(page.text().as_bytes()),
            evidence_kind: Prompt33EvidenceKind::DeterministicGeometry,
            confidence: json!({"geometry": 1.0, "reading_order": f64::from(page.confidence), "semantic_type": 0.94, "overall": f64::from(page.confidence)}),
            coordinate_space: "page_user_space".to_string(),
            source_evidence: json!({"semantic_model_strategy": page.strategy, "structure": page.structure, "scene_revision": graph.revision_id}),
            alternatives: Vec::new(),
            transaction_revision: graph.revision_id.clone(),
        })?;
        // Materialize the column layer of the runtime region graph from the
        // canonical semantic block geometry before classifying individual
        // blocks.  This gives reflow planning explicit, provenance-linked
        // column ownership without promoting an ambiguous wide title/sidebar
        // into an arbitrary story flow.
        let column_candidates = semantic_column_candidates(
            page.page_box,
            &page
                .blocks
                .iter()
                .map(|block| (block.block_index, quad_bounds(block.quad)))
                .collect::<Vec<_>>(),
        );
        let mut column_for_block = BTreeMap::<usize, String>::new();
        let mut ordered_columns = Vec::<String>::new();
        for (column_index, candidate) in column_candidates.iter().enumerate() {
            let column_id = stable_id(
                "semantic-column",
                &[
                    graph.document_id.as_bytes(),
                    &page.page.to_le_bytes(),
                    &column_index.to_le_bytes(),
                    &candidate.bounds[0].to_le_bytes(),
                    &candidate.bounds[2].to_le_bytes(),
                ],
            );
            let column = add_node(SemanticRegionNode {
                node_id: column_id.clone(),
                node_type: "column".to_string(),
                page: page.page,
                source_scene_nodes: page_scene_nodes.clone(),
                source_instructions: page_source_instructions.clone(),
                bounds: candidate.bounds,
                text_hash: digest_hex(
                    candidate
                        .block_indices
                        .iter()
                        .flat_map(|index| index.to_le_bytes())
                        .collect::<Vec<_>>()
                        .as_slice(),
                ),
                evidence_kind: Prompt33EvidenceKind::DeterministicGeometry,
                confidence: json!({
                    "geometry": candidate.confidence,
                    "column_grouping": candidate.confidence,
                    "overall": candidate.confidence,
                }),
                coordinate_space: "page_user_space".to_string(),
                source_evidence: json!({
                    "method": candidate.method,
                    "block_indices": candidate.block_indices,
                    "wide_spanning_blocks_remain_page_children": true,
                    "projection_gap": "bounded_3_percent_page_width_clamped_to_8_24_points",
                }),
                alternatives: vec![json!({
                    "type": "single_story_or_sidebar_partition",
                    "confidence": 1.0 - candidate.confidence,
                })],
                transaction_revision: graph.revision_id.clone(),
            })?;
            add_edge(
                page_region.clone(),
                column.clone(),
                "contains",
                candidate.confidence,
                Prompt33EvidenceKind::DeterministicGeometry,
                json!({"method": candidate.method, "parent": "page_region"}),
            )?;
            for block_index in &candidate.block_indices {
                column_for_block.insert(*block_index, column.clone());
            }
            ordered_columns.push(column);
        }
        // Directionality is an explicit request policy where supplied.  In
        // analysis-only mode, use the canonical document's left-to-right
        // geometric convention and preserve the right-to-left alternative in
        // the edge evidence rather than silently guessing it.
        if direction_label(request.and_then(|item| item.direction.as_deref())) == "right_to_left" {
            ordered_columns.reverse();
        }
        for pair in ordered_columns.windows(2) {
            add_edge(
                pair[0].clone(),
                pair[1].clone(),
                "next_column",
                0.74,
                Prompt33EvidenceKind::DeterministicGeometry,
                json!({
                    "method": "column_projection_order",
                    "direction": direction_label(request.and_then(|item| item.direction.as_deref())),
                    "review_required_without_explicit_direction": request.is_none(),
                }),
            )?;
        }
        let mut previous_paragraph = None::<String>;
        let mut active_heading = None::<String>;
        let mut page_figures = Vec::<(String, [f64; 4])>::new();
        for scene_node in graph.nodes.iter().filter(|node| {
            node.page == page.page
                && matches!(
                    scene_node_kind_for_figure(node.node_kind),
                    Some("image") | Some("path")
                )
        }) {
            let figure_id = stable_id(
                "semantic-figure",
                &[
                    graph.document_id.as_bytes(),
                    &page.page.to_le_bytes(),
                    scene_node.node_id.as_bytes(),
                ],
            );
            let figure = add_node(SemanticRegionNode {
                node_id: figure_id.clone(),
                node_type: "figure".to_string(),
                page: page.page,
                source_scene_nodes: vec![scene_node.node_id.clone()],
                source_instructions: scene_node.source_instruction_ids.clone(),
                bounds: scene_node.bounds_user_space,
                text_hash: digest_hex(scene_node.node_id.as_bytes()),
                evidence_kind: Prompt33EvidenceKind::ExactSourceFact,
                confidence: json!({"geometry": 0.98, "semantic_type": 0.72, "overall": 0.72}),
                coordinate_space: "page_user_space".to_string(),
                source_evidence: json!({
                    "scene_node_kind": scene_node.node_kind,
                    "scene_node": scene_node.node_id,
                    "z_order": scene_node.z_order,
                    "clipping": scene_node.clipping,
                }),
                alternatives: vec![json!({"type": "decorative_artifact", "confidence": 0.28})],
                transaction_revision: graph.revision_id.clone(),
            })?;
            add_edge(
                page_region.clone(),
                figure.clone(),
                "contains",
                0.98,
                Prompt33EvidenceKind::ExactSourceFact,
                json!({"parent": "page_region", "source": "prompt32_scene_occurrence"}),
            )?;
            page_figures.push((figure, scene_node.bounds_user_space));
        }
        for block in page.blocks {
            let block_kind = semantic_role_node_type(block.role, "block");
            let block_id = stable_id(
                "semantic-block",
                &[
                    graph.document_id.as_bytes(),
                    &page.page.to_le_bytes(),
                    &block.block_index.to_le_bytes(),
                    block.text.as_bytes(),
                ],
            );
            let block_node = add_node(SemanticRegionNode {
                node_id: block_id.clone(),
                node_type: block_kind.to_string(),
                page: page.page,
                source_scene_nodes: page_scene_nodes.clone(),
                source_instructions: page_source_instructions.clone(),
                bounds: quad_bounds(block.quad),
                text_hash: digest_hex(block.text.as_bytes()),
                evidence_kind: Prompt33EvidenceKind::DeterministicGeometry,
                confidence: json!({"geometry": 0.98, "semantic_type": f64::from(block.role_confidence), "overall": f64::from(block.confidence), "role_source": block.role_source}),
                coordinate_space: "page_user_space".to_string(),
                source_evidence: json!({"semantic_role": block.role, "role_source": block.role_source, "mcids": block.mcids, "structure_role": block.struct_role, "original_role": block.original_role}),
                alternatives: Vec::new(),
                transaction_revision: graph.revision_id.clone(),
            })?;
            add_edge(
                page_region.clone(),
                block_node.clone(),
                "contains",
                0.98,
                Prompt33EvidenceKind::DeterministicGeometry,
                json!({"parent": "page_region", "geometry": "semantic_block_quad"}),
            )?;
            if let Some(column) = column_for_block.get(&block.block_index) {
                add_edge(
                    column.clone(),
                    block_node.clone(),
                    "contains",
                    0.90,
                    Prompt33EvidenceKind::DeterministicGeometry,
                    json!({"method": "canonical_block_x_projection_membership"}),
                )?;
            }
            // The canonical block classifier can conservatively retain a
            // mixed block as body text even when its line/paragraph evidence
            // identifies list items. Materialize one bounded List parent in
            // that case, rather than leaving list-item nodes attached only to
            // a generic block. This remains an inference with the exact
            // source block/paragraph provenance retained on both nodes.
            let list_parent = if block_kind == "list" {
                Some(block_node.clone())
            } else if block.paragraphs.iter().any(|paragraph| {
                matches!(paragraph.role, crate::text::TextRole::List)
                    || crate::text::semantic_model::is_canonical_list_item_text(&paragraph.text)
            }) {
                let list_id = stable_id(
                    "semantic-list",
                    &[block_node.as_bytes(), block.text.as_bytes()],
                );
                let list_node = add_node(SemanticRegionNode {
                    node_id: list_id.clone(),
                    node_type: "list".to_string(),
                    page: page.page,
                    source_scene_nodes: page_scene_nodes.clone(),
                    source_instructions: page_source_instructions.clone(),
                    bounds: quad_bounds(block.quad),
                    text_hash: digest_hex(block.text.as_bytes()),
                    evidence_kind: Prompt33EvidenceKind::HeuristicInference,
                    confidence: json!({
                        "geometry": 0.98,
                        "semantic_type": f64::from(block.role_confidence),
                        "list_relationship": 0.72,
                        "overall": f64::from(block.confidence),
                    }),
                    coordinate_space: "page_user_space".to_string(),
                    source_evidence: json!({
                        "method": "canonical_paragraph_list_item_evidence_inside_body_block",
                        "source_block": block_node.clone(),
                        "paragraph_count": block.paragraphs.len(),
                    }),
                    alternatives: vec![json!({"type": "body_block", "confidence": 0.28})],
                    transaction_revision: graph.revision_id.clone(),
                })?;
                add_edge(
                    block_node.clone(),
                    list_node.clone(),
                    "contains",
                    0.72,
                    Prompt33EvidenceKind::HeuristicInference,
                    json!({"method": "canonical_paragraph_list_item_evidence_inside_body_block"}),
                )?;
                Some(list_node)
            } else {
                None
            };
            if block_kind == "caption" {
                if let Some((figure, distance)) =
                    nearest_figure(&page_figures, quad_bounds(block.quad))
                {
                    add_edge(
                        block_node.clone(),
                        figure,
                        "caption_of",
                        if distance <= 24.0 { 0.88 } else { 0.68 },
                        Prompt33EvidenceKind::HeuristicInference,
                        json!({"method": "nearest_scene_figure_candidate", "distance": distance}),
                    )?;
                }
            }
            let mut paragraph_ids_by_index = BTreeMap::<usize, String>::new();
            for paragraph in &block.paragraphs {
                let repeated_kind = repeated_header_footer
                    .get(&(page.page, block.block_index, paragraph.paragraph_index))
                    .map(String::as_str);
                let paragraph_bounds = quad_bounds(paragraph.quad);
                let footnote_label = footnote_body_label(&paragraph.text);
                let page_height = page.page_box[3] - page.page_box[1];
                let in_bottom_footnote_zone = page_height.is_finite()
                    && page_height > 0.0
                    && paragraph_bounds[1] <= page.page_box[1] + page_height * 0.20;
                let paragraph_kind = if let Some(kind) = repeated_kind {
                    kind
                } else if matches!(block_kind, "header" | "footer") {
                    block_kind
                } else if matches!(paragraph.role, crate::text::TextRole::Footnote)
                    || (footnote_label.is_some() && in_bottom_footnote_zone)
                {
                    "footnote_body"
                } else if matches!(paragraph.role, crate::text::TextRole::List)
                    || crate::text::semantic_model::is_canonical_list_item_text(&paragraph.text)
                {
                    "list_item"
                } else if crate::text::semantic_model::is_canonical_caption_text(&paragraph.text) {
                    "caption"
                } else {
                    semantic_role_node_type(paragraph.role, "paragraph")
                };
                let paragraph_id = stable_id(
                    "semantic-paragraph",
                    &[
                        block_node.as_bytes(),
                        &paragraph.paragraph_index.to_le_bytes(),
                        paragraph.text.as_bytes(),
                    ],
                );
                let paragraph_node = add_node(SemanticRegionNode {
                    node_id: paragraph_id.clone(),
                    node_type: paragraph_kind.to_string(),
                    page: page.page,
                    source_scene_nodes: page_scene_nodes.clone(),
                    source_instructions: page_source_instructions.clone(),
                    bounds: paragraph_bounds,
                    text_hash: digest_hex(paragraph.text.as_bytes()),
                    evidence_kind: if page_source_instructions.is_empty() {
                        Prompt33EvidenceKind::DeterministicGeometry
                    } else {
                        Prompt33EvidenceKind::ExactSourceFact
                    },
                    confidence: json!({"geometry": 0.98, "paragraph_grouping": f64::from(paragraph.confidence), "semantic_type": f64::from(paragraph.role_confidence), "source_mapping": if page_source_instructions.is_empty() { 0.0 } else { 0.82 }, "overall": f64::from(paragraph.confidence)}),
                    coordinate_space: "page_user_space".to_string(),
                    source_evidence: json!({"semantic_role": paragraph.role, "role_source": paragraph.role_source, "line_range": paragraph.line_range, "source_mapping_scope": if page_source_instructions.is_empty() { "unavailable" } else { "page_text_scene_node_only" }, "repeated_region_detection": repeated_kind.map(|kind| json!({"classification": kind, "method": "same_normalized_text_same_page_band_across_multiple_pages", "artifact_candidate": true})).unwrap_or(Value::Null)}),
                    alternatives: Vec::new(),
                    transaction_revision: graph.revision_id.clone(),
                })?;
                add_edge(
                    if paragraph_kind == "list_item" {
                        list_parent.clone().unwrap_or_else(|| block_node.clone())
                    } else {
                        block_node.clone()
                    },
                    paragraph_node.clone(),
                    if paragraph_kind == "list_item" {
                        "list_parent"
                    } else {
                        "contains"
                    },
                    f64::from(paragraph.confidence),
                    Prompt33EvidenceKind::DeterministicGeometry,
                    json!({"line_range": paragraph.line_range}),
                )?;
                if paragraph_kind == "caption" {
                    if let Some((figure, distance)) =
                        nearest_figure(&page_figures, paragraph_bounds)
                    {
                        add_edge(
                            paragraph_node.clone(),
                            figure,
                            "caption_of",
                            if distance <= 24.0 { 0.88 } else { 0.68 },
                            Prompt33EvidenceKind::HeuristicInference,
                            json!({
                                "method": "canonical_paragraph_caption_label_plus_nearest_scene_figure",
                                "distance": distance,
                            }),
                        )?;
                    }
                }
                if paragraph_kind == "footnote_body" {
                    if let Some(label) = footnote_label.as_ref() {
                        footnote_bodies.push((page.page, label.clone(), paragraph_node.clone()));
                    }
                } else if let Some((label, marker_offset)) =
                    inline_footnote_marker_label(&paragraph.text)
                {
                    let marker_id = stable_id(
                        "semantic-footnote-marker",
                        &[
                            paragraph_node.as_bytes(),
                            label.as_bytes(),
                            &marker_offset.to_le_bytes(),
                        ],
                    );
                    let marker_node = add_node(SemanticRegionNode {
                        node_id: marker_id.clone(),
                        node_type: "footnote_marker".to_string(),
                        page: page.page,
                        source_scene_nodes: page_scene_nodes.clone(),
                        source_instructions: page_source_instructions.clone(),
                        bounds: paragraph_bounds,
                        text_hash: digest_hex(label.as_bytes()),
                        evidence_kind: Prompt33EvidenceKind::HeuristicInference,
                        confidence: json!({"geometry": f64::from(paragraph.confidence), "footnote_association": 0.68, "overall": 0.68}),
                        coordinate_space: "page_user_space".to_string(),
                        source_evidence: json!({"method": "inline_superscript_or_symbol_marker", "label": label, "utf8_offset": marker_offset, "paragraph": paragraph_id}),
                        alternatives: vec![
                            json!({"type": "ordinary_symbol_or_literal", "confidence": 0.28}),
                        ],
                        transaction_revision: graph.revision_id.clone(),
                    })?;
                    add_edge(
                        paragraph_node.clone(),
                        marker_node.clone(),
                        "contains",
                        0.68,
                        Prompt33EvidenceKind::HeuristicInference,
                        json!({"method": "inline_marker_character"}),
                    )?;
                    footnote_markers.push((page.page, label, marker_id));
                }
                if !matches!(paragraph_kind, "header" | "footer") {
                    if let Some(previous) = previous_paragraph.replace(paragraph_node.clone()) {
                        add_edge(
                            previous,
                            paragraph_node.clone(),
                            "next_reading",
                            f64::from(page.confidence),
                            Prompt33EvidenceKind::DeterministicGeometry,
                            json!({"strategy": page.strategy}),
                        )?;
                    }
                    if paragraph_kind == "heading" {
                        active_heading = Some(paragraph_node.clone());
                    } else if let Some(heading) = active_heading.take() {
                        if paragraph_kind == "paragraph" || paragraph_kind == "list_item" {
                            add_edge(
                                heading,
                                paragraph_node.clone(),
                                "heading_for",
                                0.78,
                                Prompt33EvidenceKind::HeuristicInference,
                                json!({"method": "next_reading_after_heading"}),
                            )?;
                        }
                    }
                    reading_nodes.push(paragraph_node.clone());
                }
                paragraph_ids_by_index.insert(paragraph.paragraph_index, paragraph_node);
            }
            for line in &block.lines {
                let parent = block
                    .paragraphs
                    .iter()
                    .find(|paragraph| {
                        line.line_index >= paragraph.line_range[0]
                            && line.line_index < paragraph.line_range[1]
                    })
                    .and_then(|paragraph| paragraph_ids_by_index.get(&paragraph.paragraph_index))
                    .cloned()
                    .unwrap_or_else(|| block_node.clone());
                let line_id = stable_id(
                    "semantic-line",
                    &[
                        parent.as_bytes(),
                        &line.line_index.to_le_bytes(),
                        line.text.as_bytes(),
                    ],
                );
                let line_node = add_node(SemanticRegionNode {
                    node_id: line_id.clone(),
                    node_type: "line".to_string(),
                    page: page.page,
                    source_scene_nodes: page_scene_nodes.clone(),
                    source_instructions: page_source_instructions.clone(),
                    bounds: quad_bounds(line.quad),
                    text_hash: digest_hex(line.text.as_bytes()),
                    evidence_kind: Prompt33EvidenceKind::DeterministicGeometry,
                    confidence: json!({"geometry": f64::from(line.confidence), "semantic_type": f64::from(line.role_confidence), "overall": f64::from(line.confidence)}),
                    coordinate_space: "page_user_space".to_string(),
                    source_evidence: json!({"role": line.role, "role_source": line.role_source, "mcids": line.mcids, "direction": line.direction}),
                    alternatives: Vec::new(),
                    transaction_revision: graph.revision_id.clone(),
                })?;
                add_edge(
                    parent,
                    line_node.clone(),
                    "contains",
                    f64::from(line.confidence),
                    Prompt33EvidenceKind::DeterministicGeometry,
                    json!({"line_index": line.line_index}),
                )?;
                for word in &line.words {
                    let word_id = stable_id(
                        "semantic-word",
                        &[
                            line_node.as_bytes(),
                            &word.word_index.to_le_bytes(),
                            word.text.as_bytes(),
                        ],
                    );
                    let word_node = add_node(SemanticRegionNode {
                        node_id: word_id.clone(),
                        node_type: "word".to_string(),
                        page: page.page,
                        source_scene_nodes: page_scene_nodes.clone(),
                        source_instructions: page_source_instructions.clone(),
                        bounds: quad_bounds(word.quad),
                        text_hash: digest_hex(word.text.as_bytes()),
                        evidence_kind: Prompt33EvidenceKind::DeterministicGeometry,
                        confidence: json!({"geometry": f64::from(word.confidence), "overall": f64::from(word.confidence)}),
                        coordinate_space: "page_user_space".to_string(),
                        source_evidence: json!({"char_range": word.char_range, "mcids": word.mcids, "provenance": word.provenance_summary}),
                        alternatives: Vec::new(),
                        transaction_revision: graph.revision_id.clone(),
                    })?;
                    add_edge(
                        line_node.clone(),
                        word_node.clone(),
                        "contains",
                        f64::from(word.confidence),
                        Prompt33EvidenceKind::DeterministicGeometry,
                        json!({"word_index": word.word_index}),
                    )?;
                }
                for character in &line.chars {
                    let glyph_id = stable_id(
                        "semantic-glyph",
                        &[
                            line_node.as_bytes(),
                            &character.char_index.to_le_bytes(),
                            character.text.as_bytes(),
                        ],
                    );
                    let glyph_node = add_node(SemanticRegionNode {
                        node_id: glyph_id.clone(),
                        node_type: "glyph".to_string(),
                        page: page.page,
                        source_scene_nodes: page_scene_nodes.clone(),
                        source_instructions: page_source_instructions.clone(),
                        bounds: quad_bounds(character.quad),
                        text_hash: digest_hex(character.text.as_bytes()),
                        evidence_kind: Prompt33EvidenceKind::DeterministicFontShaping,
                        confidence: json!({"geometry": f64::from(character.confidence), "text_mapping": f64::from(character.confidence), "overall": f64::from(character.confidence)}),
                        coordinate_space: "page_user_space".to_string(),
                        source_evidence: json!({"unicode": character.unicode, "font_name": character.font_name, "font_size": character.font_size, "mapping_source": character.mapping_source, "mcid": character.mcid}),
                        alternatives: Vec::new(),
                        transaction_revision: graph.revision_id.clone(),
                    })?;
                    add_edge(
                        line_node.clone(),
                        glyph_node,
                        "contains",
                        f64::from(character.confidence),
                        Prompt33EvidenceKind::DeterministicFontShaping,
                        json!({"char_index": character.char_index}),
                    )?;
                }
            }
        }
    }
    for (page, label, marker) in &footnote_markers {
        let bodies = footnote_bodies
            .iter()
            .filter(|(body_page, body_label, _)| body_page == page && body_label == label)
            .collect::<Vec<_>>();
        for (_, _, body) in &bodies {
            add_edge(
                marker.clone(),
                (*body).clone(),
                "footnote_of",
                if bodies.len() == 1 { 0.86 } else { 0.52 },
                Prompt33EvidenceKind::HeuristicInference,
                json!({
                    "method": "matching_inline_marker_and_bottom_zone_body_label",
                    "label": label,
                    "same_page": true,
                    "candidate_body_count": bodies.len(),
                    "review_required": bodies.len() != 1,
                }),
            )?;
        }
    }
    for pair in reading_nodes.windows(2) {
        if pair[0] == pair[1] {
            continue;
        }
        let source_page = nodes
            .iter()
            .find(|node| node.node_id == pair[0])
            .map(|node| node.page)
            .unwrap_or(0);
        let target_page = nodes
            .iter()
            .find(|node| node.node_id == pair[1])
            .map(|node| node.page)
            .unwrap_or(0);
        add_edge(
            pair[0].clone(),
            pair[1].clone(),
            if source_page == target_page {
                "next_reading"
            } else {
                "next_page"
            },
            if source_page == target_page {
                0.84
            } else {
                0.62
            },
            Prompt33EvidenceKind::DeterministicGeometry,
            json!({"cross_page": source_page != target_page, "application": "analysis_only_until_source_linked_flow_is_available"}),
        )?;
    }
    let reading_resolution = resolve_reading_order_graph(&nodes, &edges);
    let body_ids = reading_nodes.iter().cloned().collect::<BTreeSet<_>>();
    let body_nodes = nodes
        .iter()
        .filter(|node| body_ids.contains(&node.node_id))
        .cloned()
        .collect::<Vec<_>>();
    let body_edges = edges
        .iter()
        .filter(|edge| {
            body_ids.contains(&edge.source)
                && body_ids.contains(&edge.target)
                && matches!(
                    edge.relationship.as_str(),
                    "next_reading" | "next_page" | "heading_for"
                )
        })
        .cloned()
        .collect::<Vec<_>>();
    let body_reading_resolution = resolve_reading_order_graph(&body_nodes, &body_edges);
    let cycle_count = reading_resolution["cycle_count"].as_u64().unwrap_or(0) as usize;
    let review_required = if nodes
        .iter()
        .any(|node| node.confidence["overall"].as_f64().unwrap_or(0.0) < 0.7)
    {
        vec![json!({
            "code": "low_confidence_semantic_structure",
            "requires_user_review": !request.map(|r| r.approve_low_confidence_structure).unwrap_or(false),
        })]
    } else {
        Vec::new()
    };
    let mut review_required = review_required;
    if reading_resolution["review_required"] == Value::Bool(true) {
        review_required.push(json!({
            "code": "reading_order_cycle_resolved_with_low_confidence_edge_removal",
            "requires_user_review": true,
            "removed_edges": reading_resolution["removed_cycle_edges"],
        }));
    }
    let repeated_header_footer_count = nodes
        .iter()
        .filter(|node| {
            matches!(node.node_type.as_str(), "header" | "footer")
                && !node.source_evidence["repeated_region_detection"].is_null()
        })
        .count();
    let region_graph_invariants =
        semantic_region_graph_invariants(&nodes, &edges, &analyzed_graph_pages, request);
    Ok(SemanticLayoutReport {
        schema_version: PROMPT33_SCHEMA_VERSION.to_string(),
        document_id: stable_id("document", &[input]),
        nodes,
        edges,
        algorithms_used: vec![
            "prompt06_semantic_text_model".into(),
            "canonical_xy_cut_projection_profiles".into(),
            "canonical_docstrum_style_spacing_estimate".into(),
            "prompt32_scene_graph_source_occurrence_links".into(),
        ],
        exact_vs_inferred: json!({
            "source_structure_facts": "Prompt06 MCID/structure context is retained where present",
            "deterministic_geometry": "Prompt06 text semantic quads and canonical XY-cut layout",
            "heuristic_inference": "existing Prompt06 role classifier and layout fallback only",
            "model_inference": "not_enabled_by_default",
            "user_correction": "not implemented",
        }),
        reading_order: json!({
            "candidate_chain_is_acyclic": true,
            "cycle_count": cycle_count,
            "cycle_break_policy": reading_resolution["cycle_break_policy"],
            "machine_order": body_reading_resolution["machine_order"],
            "all_runtime_node_order": reading_resolution["machine_order"],
            "removed_cycle_edges": reading_resolution["removed_cycle_edges"],
            "review_required": reading_resolution["review_required"],
            "stable_deterministic_candidate_sort": true,
            "ambiguity_reported": true,
            "body_node_count": body_nodes.len(),
            "artifact_nodes_excluded_from_body_order": true,
        }),
        flow_graph: json!({
            "cross_column_flow": "one approved explicit same-page next_column boundary is source-linked and executable: LTR rightward or RTL leftward, each in the same reading band and semantically/scene-proven empty; canonical XY-cut candidates remain analysis-only",
            "cross_page_flow": "typed next_page candidates; one approved source-linked existing-empty-next-page boundary and one catalog-preserving append-only new-page boundary are implemented",
            "page_creation": "implemented_only_for_explicitly_approved_catalog_preserving_single_page_semantic_paragraph_overflow",
            "tables_formulas": "not_classified",
            "headers_footers": {
                "deterministic_repeated_page_band_detection": true,
                "repeated_runtime_nodes": repeated_header_footer_count,
                "body_reading_order_excludes_repeated_artifact_candidates": true,
            },
        }),
        region_graph_invariants,
        review_required,
        prompt34_boundaries: vec![
            "editable tables".into(),
            "editable formulas/math layout".into(),
            "OCR correction pipeline".into(),
        ],
        prompt35_boundaries: vec![
            "full PDF/UA repair after broad reflow".into(),
            "final accessibility/tag remediation".into(),
        ],
    })
}

pub fn reading_order_report(input: &[u8]) -> Result<Value> {
    let semantic = analyze_semantic_layout(input, None)?;
    Ok(json!({
        "schema_version": PROMPT33_SCHEMA_VERSION,
        "status": "implemented_with_limits",
        "node_count": semantic.nodes.len(),
        "edge_count": semantic.edges.len(),
        "dag": semantic.reading_order["candidate_chain_is_acyclic"],
        "cycle_policy": semantic.reading_order["cycle_break_policy"],
        "machine_order": semantic.reading_order["machine_order"],
        "removed_cycle_edges": semantic.reading_order["removed_cycle_edges"],
        "review_required": semantic.reading_order["review_required"],
        "confidence_edges": semantic.edges,
        "region_graph_invariants": semantic.region_graph_invariants,
        "stable_deterministic": semantic.reading_order["stable_deterministic_candidate_sort"],
    }))
}

pub fn flow_graph_report(input: &[u8]) -> Result<Value> {
    let semantic = analyze_semantic_layout(input, None)?;
    Ok(json!({
        "schema_version": PROMPT33_SCHEMA_VERSION,
        "status": "analysis_candidates_with_bounded_source_linked_flow_adapters",
        "nodes": semantic.nodes,
        "edges": semantic.edges,
        "region_graph_invariants": semantic.region_graph_invariants,
        "relationships": [
            "paragraph_to_paragraph",
            "column_to_column",
            "page_body_to_next_page_body",
            "heading_to_body",
            "list_item_to_next_item",
            "footnote_marker_to_body",
            "caption_to_figure_table",
            "paragraph_to_anchor"
        ],
        "source_linked_application": {
            "same_page_next_region": "implemented_only_for_explicit_below_source_proven_empty_target",
            "same_page_next_column": "implemented_only_for_explicit_horizontal_ltr_rightward_or_rtl_leftward_same_band_proven_empty_target",
            "existing_next_page": "implemented_only_for_identical_box_proven_empty_target",
            "page_creation": "implemented_with_catalog_preserving_append_only_limits",
            "same_page_dependency_movement": "implemented_only_for_up_to_eight caller-named pairwise-collision-free source-resolved vector paths through Prompt20; text, image, annotation, form, ambiguous, and generic-neighbor movement refuse",
            "same_page_source_link_annotation_movement": "implemented_only_for_up_to_eight caller-named /Link annotations whose expected source rectangles overlap the selected text; /Rect and existing /QuadPoints move while each /A or /Dest stays unchanged",
        },
    }))
}

pub fn approve_structure_correction(input: &[u8], correction_json: &str) -> Result<Value> {
    let _correction: Value = serde_json::from_str(correction_json)
        .map_err(|err| WellfriendError::invalid_input(format!("invalid correction JSON: {err}")))?;
    let _ = input;
    Err(WellfriendError::UnsupportedFeature(
        "prompt33 structure_update_failed: semantic structure correction has no executable source-linked transaction yet"
            .to_string(),
    ))
}

pub fn no_overlay_no_clipping_report(
    input: &[u8],
    request: &GeometricReflowRequest,
) -> Result<Value> {
    let preview = preview_reflow(input, request)?;
    Ok(json!({
        "schema_version": PROMPT33_SCHEMA_VERSION,
        "overlay_detection": {
            "old_text_reachable_under_cover": false,
            "white_rectangle_coverup": false,
            "duplicate_hidden_text": false,
            "source_rewrite_required": true,
        },
        "clipping": {
            "silent_clipping": false,
            "overflow_status": preview.overflow_status,
            "refusal_leaves_document_unchanged": preview.refusal.is_some(),
        },
        "status": if preview.refusal.is_some() { "refused_no_change" } else { "planned_source_rewrite_requires_apply_validation" },
    }))
}

/// Query the ordered overflow state generated by the canonical preview engine.
/// This does not mutate a document or promise that an unavailable downstream
/// stage can apply; callers receive the exact currently planned/refused state.
pub fn query_overflow(input: &[u8], request: &GeometricReflowRequest) -> Result<Value> {
    let preview = preview_reflow(input, request)?;
    Ok(json!({
        "schema_version": PROMPT33_SCHEMA_VERSION,
        "requested_mode": request.requested_mode,
        "preview_only": true,
        "overflow_status": preview.overflow_status,
        "overflow_amount": preview.line_breaking.overflow_amount,
        "final_line_count": preview.line_breaking.lines.len(),
        "final_cost": preview.line_breaking.final_cost,
        "hyphenation": preview.line_breaking.hyphenation,
        "ordered_stage_evidence": preview.line_breaking.exact_limits,
        "refusal": preview.refusal,
        "no_change_proof": true,
    }))
}

/// Query the bounded hard/soft Cassowary report produced for a reflow request.
pub fn query_constraints(input: &[u8], request: &GeometricReflowRequest) -> Result<Value> {
    let preview = preview_reflow(input, request)?;
    Ok(json!({
        "schema_version": PROMPT33_SCHEMA_VERSION,
        "requested_mode": request.requested_mode,
        "preview_only": true,
        "constraints": preview.constraints,
        "refusal": preview.refusal,
        "no_change_proof": true,
    }))
}

/// Query central confidence/review enforcement for a proposed reflow.
pub fn query_confidence(input: &[u8], request: &GeometricReflowRequest) -> Result<Value> {
    let preview = preview_reflow(input, request)?;
    Ok(json!({
        "schema_version": PROMPT33_SCHEMA_VERSION,
        "requested_mode": request.requested_mode,
        "preview_only": true,
        "confidence": preview.confidence,
        "refusal": preview.refusal,
        "no_change_proof": true,
    }))
}

/// Validate a completed supported local reflow against reopen and unchanged
/// page/stream/extraction facts. Cross-page/page-tree operations provide their
/// own narrower transaction evidence and are not silently treated as this
/// local proof.
pub fn validate_reflow_output(
    input: &[u8],
    output: &[u8],
    request: &GeometricReflowRequest,
) -> Result<Value> {
    let output_reopened = ContentEngine::open_bytes(output.to_vec()).is_ok();
    let proof = unaffected_content_proof(
        input,
        output,
        request.page,
        &request.source_text,
        &request.replacement_text,
        &expected_downstream_link_rects(request),
        1 + request.downstream_vector_moves.len(),
    );
    Ok(json!({
        "schema_version": PROMPT33_SCHEMA_VERSION,
        "output_reopened": output_reopened,
        "unaffected_content_proof": proof,
        "valid": output_reopened && proof["status"] == "pass_with_documented_layout_whitespace_policy",
        "exact_scope": "single-source-page local reflow; cross-page/page-tree flows require their transaction-specific validation evidence",
    }))
}

pub fn transaction_undo_report(input: &[u8], request: &GeometricReflowRequest) -> Result<Value> {
    let preview = preview_reflow(input, request)?;
    if let Some(refusal) = preview.refusal {
        return Ok(json!({
            "schema_version": PROMPT33_SCHEMA_VERSION,
            "transaction_id": preview.transaction_id,
            "status": "refused_no_change",
            "atomic": true,
            "undo_executed": false,
            "refusal": refusal,
            "input_sha256": digest_hex(input),
        }));
    }
    let mut session = ReflowMutationSession::new(input.to_vec())?;
    let transaction = match request.requested_mode {
        TrueEditingMode::GeometricBlock => session.apply_geometric(request)?,
        TrueEditingMode::SemanticDocument => session.apply_semantic(request)?,
        TrueEditingMode::OperatorPreserving => {
            return Err(WellfriendError::UnsupportedFeature(
                "prompt33 owns only geometric_block and semantic_document reflow undo".to_string(),
            ));
        }
    };
    let edited_sha256 = digest_hex(session.bytes());
    let undo = session.undo_reflow()?;
    Ok(json!({
        "schema_version": PROMPT33_SCHEMA_VERSION,
        "transaction_id": transaction.transaction_id,
        "status": "executed_incremental_undo_verified",
        "atomic": undo.atomic,
        "undo_restores_original_layout_source_semantic_state": undo.byte_exact_restoration,
        "redo_deterministic": "not_exposed_by_prompt33_session",
        "refusal_no_change": false,
        "edited_sha256": edited_sha256,
        "restored_sha256": undo.restored_sha256,
        "undo": undo,
        "states": ["created", "planned", "applied", "serialized", "reopened_validated", "undo_reopened_validated"],
    }))
}

pub fn prompt33_report(input: &[u8]) -> Result<Value> {
    let scene = build_scene_graph(input, &[])?;
    let semantic = semantic_layout_from_graph(input, &scene, None)?;
    Ok(json!({
        "schema_version": PROMPT33_SCHEMA_VERSION,
        "status": "implemented_with_open_prompt33_gates",
        "product": "Wellfriend PDF SDK",
        "technical_namespace": "wellfriendpdf",
        "modes_owned": [TrueEditingMode::GeometricBlock, TrueEditingMode::SemanticDocument],
        "operator_preserving_owner": "Prompt31",
        "no_duplicate_architecture": true,
        "canonical_inputs": [
            "Prompt31 provenance/source selection",
            "Prompt32 scene/snapshot/transaction/font identity",
            "canonical writer and Prompt20/31 source mutation"
        ],
        "scene_node_count": scene.nodes.len(),
        "semantic_node_count": semantic.nodes.len(),
        "geometric_reflow": "implemented_with_limits",
        "semantic_document_flow": "single_provenance_resolved_paragraph_only_with_explicit_confidence_approval",
        "no_silent_escalation": true,
        "no_overlay_no_clipping": true,
        "unknown_objects_locked_by_default": true,
        "tables_formulas_boundary": "deferred_prompt34",
        "accessibility_repair_boundary": "deferred_prompt35",
        "exact_limits": [
            "supported GeometricBlock apply removes one provenance-resolved source string and writes shaped Type0 text through the canonical incremental writer",
            "an explicit GeometricBlock transaction may move a bounded collision-free source-resolved same-page vector path set through Prompt20 when every stable identity and approved dependency edge is supplied; text/image/form movement, unknown neighbors, collisions, and reference repair remain exact refusals",
            "an explicit GeometricBlock transaction may move a bounded caller-associated same-page /Link set when every exact expected rectangle overlaps the selected source region; /Rect and existing /QuadPoints move, /A or /Dest is preserved, and stale, generic, reply, widget, form, cross-page, or collision movement refuses",
            "SemanticDocument may rewrite one exact page-local semantic paragraph whose deterministic text identity matches Prompt31 provenance after explicit confidence approval; it may flow to one explicit, below-source same-page next_region or one explicit horizontal next_column through a positioned canonical source stream (LTR rightward or RTL leftward), to one identical-box existing next-page region, or append one continuation page through the catalog-preserving direct-root-Kids writer; duplicate/partial selections, inferred cross-column flow, non-append insertion, and catalog-reference repair remain exact refusals",
            "full editable tables, formulas, OCR, and final accessibility repair remain Prompt34/35"
        ],
    }))
}

pub fn prompt33_feature_matrix() -> Value {
    json!({
        "schema_version": PROMPT33_SCHEMA_VERSION,
        "rows": [
            {"area": "geometric_text_regions", "status": Prompt33Status::ImplementedWithLimits, "canonical_extension": "Prompt32 scene nodes plus Prompt31 source provenance"},
            {"area": "paragraph_style_model", "status": Prompt33Status::ImplementedWithLimits, "source_linked": true, "executable_output_boundary": "preserve_original_per_run replays existing CMap/font size/text state/DeviceGray-RGB-CMYK paint for horizontal LTR ranges. Changed-length text assigns each complete replacement grapheme to a deterministic proportional source-style owner without flattening style order or splitting a grapheme; one fully selected text-state-only MCID BDC is relocated without duplicate MCID ownership. Links, nested/partial tags, clipping, arbitrary paint spaces, vertical writing, and bidi remain exact refusals"},
            {"area": "unicode_line_breaking", "status": Prompt33Status::VerifiedWithLimits, "grapheme_safe": true},
            {"area": "hyphenation", "status": Prompt33Status::ImplementedWithLimits, "provider": HYPHENATION_PROVIDER, "languages": ["en-us", "es"], "unknown_language_not_guessed": true, "inserted_hyphen_source_output": "canonical_generated_type0_visual_hyphen_with_empty_tounicode_mapping", "source_soft_hyphen": "refused"},
            {"area": "preview_layout", "status": Prompt33Status::Implemented, "algorithm": "deterministic_greedy"},
            {"area": "final_layout", "status": Prompt33Status::ImplementedWithLimits, "algorithm": "bounded_knuth_plass_style_dp"},
            {"area": "constraint_solver", "status": Prompt33Status::ImplementedWithLimits, "unknown_and_locked_objects_never_move": true, "explicit_source_linked_path_movement": "bounded collision-free same-page vector path set through Prompt20; all other objects refuse", "explicit_source_linked_link_annotation_movement": "bounded same-page /Link set with caller-provided source rectangles and dependency edges; action/destination stays unchanged"},
            {"area": "overflow_policy", "status": Prompt33Status::ImplementedWithLimits, "silent_clipping": false, "font_reduction_not_first": true, "implemented_flow_stages": ["explicit_same_page_dependency_linked_vector_path_move_set", "explicit_same_page_next_region", "explicit_same_page_ltr_or_rtl_next_column", "identical_box_existing_next_page", "catalog_preserving_explicit_page_append"], "remaining_stages": "broad_dependency_movement_and_inferred_cross_column_flow_unavailable"},
            {"area": "semantic_reconstruction", "status": Prompt33Status::ImplementedWithLimits, "canonical_extension": "Prompt06 semantic text model plus Prompt32 occurrence graph", "runtime_nodes": ["page_region", "figure", "block", "paragraph", "heading", "list", "list_item", "caption", "footnote_marker", "footnote_body", "header", "footer", "sidebar", "line", "word", "glyph"], "application_boundary": "one exact page-local semantic paragraph matched to Prompt31 provenance; duplicate/partial selections refuse and inferred semantic types require the report confidence/review policy"},
            {"area": "reading_order", "status": Prompt33Status::ImplementedWithLimits, "cycle_resolution": "remove_lowest_confidence_then_edge_id", "accuracy_metrics": "annotated two-column/footnote cycle fixture scores exact order, Kendall-style agreement, column order, and footnote placement; corpus coverage still incomplete"},
            {"area": "cross_column_cross_page_flow", "status": Prompt33Status::ImplementedWithLimits, "page_creation_policy_required": true, "implemented_boundary": ["one approved SemanticDocument paragraph to one explicit below-source semantically proven-empty same-page next_region through one positioned canonical source stream", "one approved horizontal SemanticDocument paragraph to one explicit same-band semantically proven-empty next_column through one positioned canonical source stream: LTR rightward or RTL leftward", "one approved SemanticDocument paragraph to one semantically proven-empty existing next-page region", "one catalog-preserving direct-root-Kids single-page SemanticDocument paragraph to one explicit appended page", "up to eight explicit source-associated same-page /Link rectangles with unchanged actions/destinations"], "remaining": "broad dependency movement, inferred columns, insertion/retargeting repair, non-Link annotations, tags"},
            {"area": "table_formula_editing", "status": Prompt33Status::DeferredPrompt34},
            {"area": "accessibility_repair", "status": Prompt33Status::DeferredPrompt35},
            {"area": "bindings", "status": Prompt33Status::ImplementedWithLimits, "surfaces": ["Rust", "CLI", "Python", "C ABI", "WASM", ".NET", "Java"], "rust_cli_queries": ["overflow", "constraints", "confidence", "local_output_validation"], "full_runtime_parity": "still_open"},
        ],
        "no_blocked_prompt33_rows": false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writer::{write_incremental_update, IncrementalObject, OutputObject, PdfWriter};
    use crate::PdfObject;

    fn fixture(content: &[u8]) -> Vec<u8> {
        let mut catalog = crate::PdfDictionary::empty();
        catalog.insert("Type", PdfObject::Name("Catalog".into()));
        catalog.insert(
            "Pages",
            PdfObject::Reference {
                number: 2,
                generation: 0,
            },
        );
        let mut pages = crate::PdfDictionary::empty();
        pages.insert("Type", PdfObject::Name("Pages".into()));
        pages.insert("Count", PdfObject::Integer(1));
        pages.insert(
            "Kids",
            PdfObject::Array(vec![PdfObject::Reference {
                number: 3,
                generation: 0,
            }]),
        );
        let mut font = crate::PdfDictionary::empty();
        font.insert("Type", PdfObject::Name("Font".into()));
        font.insert("Subtype", PdfObject::Name("Type1".into()));
        font.insert("BaseFont", PdfObject::Name("Helvetica".into()));
        font.insert("Encoding", PdfObject::Name("WinAnsiEncoding".into()));
        let mut font2 = crate::PdfDictionary::empty();
        font2.insert("Type", PdfObject::Name("Font".into()));
        font2.insert("Subtype", PdfObject::Name("Type1".into()));
        font2.insert("BaseFont", PdfObject::Name("Times-Roman".into()));
        font2.insert("Encoding", PdfObject::Name("WinAnsiEncoding".into()));
        let mut fonts = crate::PdfDictionary::empty();
        fonts.insert(
            "F1",
            PdfObject::Reference {
                number: 5,
                generation: 0,
            },
        );
        fonts.insert(
            "F2",
            PdfObject::Reference {
                number: 6,
                generation: 0,
            },
        );
        let mut resources = crate::PdfDictionary::empty();
        resources.insert("Font", PdfObject::Dictionary(fonts));
        let mut page = crate::PdfDictionary::empty();
        page.insert("Type", PdfObject::Name("Page".into()));
        page.insert(
            "Parent",
            PdfObject::Reference {
                number: 2,
                generation: 0,
            },
        );
        page.insert(
            "MediaBox",
            PdfObject::Array(vec![
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                PdfObject::Integer(300),
                PdfObject::Integer(300),
            ]),
        );
        page.insert("Resources", PdfObject::Dictionary(resources));
        page.insert(
            "Contents",
            PdfObject::Reference {
                number: 4,
                generation: 0,
            },
        );
        let mut stream = crate::PdfDictionary::empty();
        stream.insert("Length", PdfObject::Integer(content.len() as i64));
        PdfWriter::new(
            vec![
                OutputObject {
                    number: 1,
                    object: PdfObject::Dictionary(catalog),
                },
                OutputObject {
                    number: 2,
                    object: PdfObject::Dictionary(pages),
                },
                OutputObject {
                    number: 3,
                    object: PdfObject::Dictionary(page),
                },
                OutputObject {
                    number: 4,
                    object: PdfObject::Stream {
                        dict: stream,
                        raw: content.to_vec(),
                    },
                },
                OutputObject {
                    number: 5,
                    object: PdfObject::Dictionary(font),
                },
                OutputObject {
                    number: 6,
                    object: PdfObject::Dictionary(font2),
                },
            ],
            1,
        )
        .write()
        .expect("fixture")
    }

    fn fixture_with_catalog_reference_roots(content: &[u8]) -> Vec<u8> {
        let input = fixture(content);
        let engine = ContentEngine::open_bytes(input.clone()).expect("catalog fixture open");
        let reader = engine.document().reader();
        let (root_number, root_generation) = reader.root_reference().expect("catalog root");
        let mut catalog = reader
            .get_object(root_number, root_generation)
            .expect("catalog object")
            .as_dict()
            .cloned()
            .expect("catalog dictionary");
        let mut labels = crate::PdfDictionary::empty();
        labels.insert(
            "Nums",
            PdfObject::Array(vec![
                PdfObject::Integer(0),
                PdfObject::Dictionary({
                    let mut item = crate::PdfDictionary::empty();
                    item.insert("S", PdfObject::Name("D".to_string()));
                    item
                }),
            ]),
        );
        let mut destinations = crate::PdfDictionary::empty();
        destinations.insert(
            "source-page",
            PdfObject::Array(vec![
                PdfObject::Reference {
                    number: 3,
                    generation: 0,
                },
                PdfObject::Name("Fit".to_string()),
            ]),
        );
        catalog.insert(
            "Outlines",
            PdfObject::Reference {
                number: 11,
                generation: 0,
            },
        );
        catalog.insert("PageLabels", PdfObject::Dictionary(labels));
        catalog.insert("Dests", PdfObject::Dictionary(destinations));
        let mut outlines = crate::PdfDictionary::empty();
        outlines.insert("Type", PdfObject::Name("Outlines".to_string()));
        outlines.insert("Count", PdfObject::Integer(0));
        write_incremental_update(
            reader,
            vec![
                IncrementalObject {
                    number: root_number,
                    generation: root_generation,
                    object: PdfObject::Dictionary(catalog),
                },
                IncrementalObject {
                    number: 11,
                    generation: 0,
                    object: PdfObject::Dictionary(outlines),
                },
            ],
        )
        .expect("catalog reference fixture")
    }

    fn fixture_with_source_link(content: &[u8]) -> Vec<u8> {
        let input = fixture(content);
        let engine = ContentEngine::open_bytes(input.clone()).expect("fixture open");
        let reader = engine.document().reader();
        let page = engine.document().get_page(1).expect("fixture page");
        let mut page_dict = reader
            .get_object(page.object_number, page.generation_number)
            .expect("fixture page object")
            .as_dict()
            .cloned()
            .expect("fixture page dictionary");
        page_dict.insert(
            "Annots",
            PdfObject::Array(vec![PdfObject::Reference {
                number: 11,
                generation: 0,
            }]),
        );
        let mut action = crate::PdfDictionary::empty();
        action.insert("S", PdfObject::Name("URI".to_string()));
        action.insert(
            "URI",
            PdfObject::String(b"https://example.invalid/prompt33-source-link".to_vec()),
        );
        let mut link = crate::PdfDictionary::empty();
        link.insert("Type", PdfObject::Name("Annot".to_string()));
        link.insert("Subtype", PdfObject::Name("Link".to_string()));
        link.insert(
            "Rect",
            PdfObject::Array(vec![
                PdfObject::Integer(10),
                PdfObject::Integer(140),
                PdfObject::Integer(70),
                PdfObject::Integer(160),
            ]),
        );
        link.insert(
            "QuadPoints",
            PdfObject::Array(vec![
                PdfObject::Integer(10),
                PdfObject::Integer(160),
                PdfObject::Integer(70),
                PdfObject::Integer(160),
                PdfObject::Integer(10),
                PdfObject::Integer(140),
                PdfObject::Integer(70),
                PdfObject::Integer(140),
            ]),
        );
        link.insert("A", PdfObject::Dictionary(action));
        write_incremental_update(
            reader,
            vec![
                IncrementalObject {
                    number: page.object_number,
                    generation: page.generation_number,
                    object: PdfObject::Dictionary(page_dict),
                },
                IncrementalObject {
                    number: 11,
                    generation: 0,
                    object: PdfObject::Dictionary(link),
                },
            ],
        )
        .expect("source Link fixture")
    }

    fn fixture_with_two_source_links(content: &[u8]) -> Vec<u8> {
        let input = fixture_with_source_link(content);
        let engine = ContentEngine::open_bytes(input.clone()).expect("source Link fixture open");
        let reader = engine.document().reader();
        let page = engine.document().get_page(1).expect("fixture page");
        let mut page_dict = reader
            .get_object(page.object_number, page.generation_number)
            .expect("fixture page object")
            .as_dict()
            .cloned()
            .expect("fixture page dictionary");
        let mut annotations = page_dict
            .get("Annots")
            .and_then(PdfObject::as_array)
            .expect("first Link annotations")
            .to_vec();
        annotations.push(PdfObject::Reference {
            number: 12,
            generation: 0,
        });
        page_dict.insert("Annots", PdfObject::Array(annotations));
        let mut action = crate::PdfDictionary::empty();
        action.insert("S", PdfObject::Name("URI".to_string()));
        action.insert(
            "URI",
            PdfObject::String(b"https://example.invalid/prompt33-second-source-link".to_vec()),
        );
        let mut link = crate::PdfDictionary::empty();
        link.insert("Type", PdfObject::Name("Annot".to_string()));
        link.insert("Subtype", PdfObject::Name("Link".to_string()));
        link.insert(
            "Rect",
            PdfObject::Array(vec![
                PdfObject::Integer(90),
                PdfObject::Integer(140),
                PdfObject::Integer(150),
                PdfObject::Integer(160),
            ]),
        );
        link.insert(
            "QuadPoints",
            PdfObject::Array(vec![
                PdfObject::Integer(90),
                PdfObject::Integer(160),
                PdfObject::Integer(150),
                PdfObject::Integer(160),
                PdfObject::Integer(90),
                PdfObject::Integer(140),
                PdfObject::Integer(150),
                PdfObject::Integer(140),
            ]),
        );
        link.insert("A", PdfObject::Dictionary(action));
        write_incremental_update(
            reader,
            vec![
                IncrementalObject {
                    number: page.object_number,
                    generation: page.generation_number,
                    object: PdfObject::Dictionary(page_dict),
                },
                IncrementalObject {
                    number: 12,
                    generation: 0,
                    object: PdfObject::Dictionary(link),
                },
            ],
        )
        .expect("second source Link fixture")
    }

    fn two_page_fixture(first: &[u8], second: &[u8]) -> Vec<u8> {
        let first = ContentEngine::open_bytes(fixture(first)).expect("first page");
        let second = ContentEngine::open_bytes(fixture(second)).expect("second page");
        build_merged(&[(first.document(), vec![1]), (second.document(), vec![1])])
            .expect("two page fixture")
    }

    fn request(source: &str, replacement: &str) -> GeometricReflowRequest {
        GeometricReflowRequest {
            requested_mode: TrueEditingMode::GeometricBlock,
            page: 1,
            source_text: source.into(),
            replacement_text: replacement.into(),
            region: Some([10.0, 10.0, 260.0, 90.0]),
            allowed_expansion_region: None,
            next_region: None,
            next_column: None,
            downstream_vector_moves: Vec::new(),
            downstream_link_moves: Vec::new(),
            layout_constraints: Vec::new(),
            language: Some("en".into()),
            direction: None,
            font_policy: "rebuild_subset_or_generated_type0".into(),
            alignment: "left".into(),
            justify_last_line: false,
            hyphenation: true,
            allow_page_creation: false,
            allow_font_reduction: false,
            approve_low_confidence_structure: false,
            signature_policy_override: false,
            line_height: 14.0,
            max_downstream_blocks: 4,
        }
    }

    #[test]
    fn geometric_preview_is_grapheme_safe_and_source_linked() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
        let preview = preview_reflow(&input, &request("HELLO", "WORLD")).expect("preview");
        assert_eq!(preview.requested_mode, TrueEditingMode::GeometricBlock);
        assert_eq!(preview.applied_mode, Some(TrueEditingMode::GeometricBlock));
        assert!(preview.line_breaking.grapheme_safe);
        assert!(!preview.region.source_instructions.is_empty());
    }

    #[test]
    fn paragraph_model_retains_exact_source_style_span_evidence() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
        let paragraph =
            paragraph_style_model(&input, &request("HELLO", "WORLD")).expect("paragraph model");
        assert!(!paragraph.style_runs.is_empty());
        assert_eq!(paragraph.style_runs[0]["evidence"], "exact_source_fact");
        assert!(paragraph.style_runs[0]["stream"]["object"].is_number());
        assert!(paragraph.style_runs[0]["font_identity"]["resource"].is_string());
    }

    #[test]
    fn geometric_apply_changes_source_without_overlay() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
        let (output, report) =
            apply_reflow_region(&input, &request("HELLO", "WORLD")).expect("source reflow");
        assert!(output.starts_with(&input));
        assert_eq!(report.validation_evidence["no_overlay_no_clipping"], true);
        assert!(report.inverse_operation.is_some());
        assert_eq!(
            report.validation_evidence["source_text_token_removed"],
            serde_json::Value::Bool(true)
        );
        assert_eq!(
            report.validation_evidence["unaffected_content_proof"]["status"],
            "pass_with_documented_layout_whitespace_policy"
        );
        assert_eq!(
            report.validation_evidence["unaffected_content_proof"]["affected_page_stream_proof"]
                ["changed_existing_stream_count"],
            1
        );
        assert_eq!(
            report.validation_evidence["unaffected_content_proof"]["affected_page_stream_proof"]
                ["unmodified_existing_streams_match"],
            true
        );
        assert!(report.fonts_resources_changed[0].starts_with("generated_type0_font_resource:"));
    }

    #[test]
    fn unaffected_proof_hashes_sibling_source_streams_on_the_edited_page() {
        let input = include_bytes!("../tests/fixtures/multi_stream.pdf").to_vec();
        let mut request = request("Hello", "Updated");
        request.region = Some([50.0, 650.0, 300.0, 730.0]);
        let (output, report) =
            apply_reflow_region(&input, &request).expect("multi-stream source rewrite");
        let proof = &report.validation_evidence["unaffected_content_proof"];
        assert_eq!(
            proof["status"],
            "pass_with_documented_layout_whitespace_policy"
        );
        let stream_proof = &proof["affected_page_stream_proof"];
        assert_eq!(stream_proof["changed_existing_stream_count"], 1);
        assert_eq!(stream_proof["unmodified_existing_streams_match"], true);
        let rows = stream_proof["existing_stream_rows"]
            .as_array()
            .expect("existing source stream rows");
        assert!(rows.len() >= 2);
        assert_eq!(
            rows.iter()
                .filter(|row| row["changed_by_declared_source_transaction"] == false)
                .count(),
            rows.len() - 1
        );
        let reopened = ContentEngine::open_bytes(output).expect("multi-stream reopen");
        assert!(reopened.get_page_text(1).expect("text").contains("World"));
    }

    #[test]
    fn geometric_apply_can_preserve_explicit_multi_run_source_styles() {
        let input =
            fixture(b"BT /F1 10 Tf 0 g 10 150 Td (ONE) Tj /F2 18 Tf 1 0 0 rg (TWO) Tj ET\n");
        let mut request = request("ONETWO", "redSUN");
        request.font_policy = "preserve_original_per_run".into();
        let (output, report) = apply_reflow_region(&input, &request).expect("preserved reflow");
        assert!(output.starts_with(&input));
        assert!(report
            .fonts_resources_changed
            .iter()
            .any(|entry| entry == "preserved_source_font_resource:F1"));
        assert!(report
            .fonts_resources_changed
            .iter()
            .any(|entry| entry == "preserved_source_font_resource:F2"));
        assert_eq!(
            report.validation_evidence["source_rewrite"]["path"],
            "prompt20_multi_run_preserve_per_segment_source_serializer"
        );
        assert_eq!(
            report.validation_evidence["unaffected_content_proof"]["status"],
            "pass_with_documented_layout_whitespace_policy"
        );
    }

    #[test]
    fn geometric_apply_preserves_mixed_source_styles_for_changed_length_text() {
        let input =
            fixture(b"BT /F1 10 Tf 0 g 10 150 Td (ONE) Tj /F2 18 Tf 1 0 0 rg (TWO) Tj ET\n");
        let mut request = request("ONETWO", "summerDAY");
        request.font_policy = "preserve_original_per_run".into();
        let (output, report) =
            apply_reflow_region(&input, &request).expect("changed-length preserved reflow");
        let reopened = ContentEngine::open_bytes(output).expect("reopen changed-length reflow");
        assert!(reopened
            .get_page_text(1)
            .expect("changed-length extraction")
            .contains("summerDAY"));
        assert!(report
            .fonts_resources_changed
            .iter()
            .any(|entry| entry == "preserved_source_font_resource:F1"));
        assert!(report
            .fonts_resources_changed
            .iter()
            .any(|entry| entry == "preserved_source_font_resource:F2"));
        assert_eq!(
            report.validation_evidence["source_rewrite"]["path"],
            "prompt20_multi_run_preserve_per_segment_source_serializer"
        );
    }

    #[test]
    fn executable_reflow_undo_restores_the_exact_incremental_preimage() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
        let mut session = ReflowMutationSession::new(input.clone()).expect("session");
        let transaction = session
            .apply_geometric(&request("HELLO", "WORLD"))
            .expect("apply reflow");
        assert_ne!(session.bytes(), input.as_slice());
        let undo = session.undo_reflow().expect("undo reflow");
        assert!(undo.undone);
        assert!(undo.atomic);
        assert!(undo.byte_exact_restoration);
        assert_eq!(undo.transaction_id, transaction.transaction_id);
        assert_eq!(session.bytes(), input.as_slice());
        assert_eq!(session.cursor(), 0);
    }

    #[test]
    fn uax14_final_lines_drive_the_shaped_source_writer() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
        let mut req = request("HELLO", "ONE TWO THREE");
        req.region = Some([10.0, 10.0, 95.0, 80.0]);
        let preview = preview_reflow(&input, &req).expect("uax14 preview");
        assert!(preview.line_breaking.lines.len() >= 2);
        assert_eq!(
            preview
                .line_breaking
                .lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<String>(),
            req.replacement_text
        );
        assert!(preview
            .line_breaking
            .break_records
            .iter()
            .any(|record| record.disposition == "optional"));
        let (_output, report) = apply_reflow_region(&input, &req).expect("uax14 source reflow");
        assert_eq!(
            report.validation_evidence["source_rewrite"]["detail"]["lines_or_columns"],
            serde_json::json!(report.line_breaking.lines.len())
        );
    }

    #[test]
    fn geometric_justification_is_emitted_by_the_canonical_source_writer() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
        let mut req = request("HELLO", "ONE TWO THREE FOUR FIVE SIX");
        req.alignment = "justify".into();
        req.region = Some([10.0, 10.0, 150.0, 80.0]);
        let (_output, report) = apply_reflow_region(&input, &req).expect("justified source reflow");
        assert_eq!(
            report.line_breaking.justification["status"],
            "output_driving_text_state_spacing"
        );
        assert_eq!(report.line_breaking.justification["alignment"], "justify");
        assert!(report.line_breaking.justification["lines"].is_array());
    }

    #[test]
    fn explicit_source_linked_downstream_vector_move_rewrites_and_undoes_atomically() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n20 20 40 20 re f\n");
        let original_vector = list_vector_objects(&input, 1)
            .expect("source vector inventory")
            .objects
            .into_iter()
            .next()
            .expect("one editable source path");
        let mut req = request("HELLO", "WORLD");
        req.region = Some([10.0, 120.0, 260.0, 180.0]);
        req.downstream_vector_moves = vec![DownstreamVectorMove {
            vector_stable_id: original_vector.stable_id.clone(),
            relationship: "keep_with_next".to_string(),
            dependency_edge_id: "user-approved-test-edge".to_string(),
            dx: 0.0,
            dy: 30.0,
            shared_form_policy: SharedFormEditPolicy::Reject,
        }];
        let preview = preview_reflow(&input, &req).expect("movement plan");
        assert!(preview.flow_graph_changes.iter().any(|change| {
            change["kind"] == "planned_explicit_downstream_vector_move"
                && change["plan"]["vector_stable_id"] == original_vector.stable_id
        }));
        let mut session = ReflowMutationSession::new(input.clone()).expect("session");
        let report = session.apply_geometric(&req).expect("source-linked move");
        assert_eq!(
            report.objects_moved,
            vec![format!("vector:{}", original_vector.stable_id)]
        );
        assert_eq!(report.constraints.locked_objects_moved, 0);
        let transaction = report
            .prompt32_transaction
            .as_ref()
            .expect("Prompt32 transaction evidence");
        assert!(transaction
            .affected_objects
            .contains(&format!("vector:{}", original_vector.stable_id)));
        assert!(transaction
            .affected_scene_nodes
            .iter()
            .any(|node| node.starts_with("scene-path-")));
        let moved = list_vector_objects(session.bytes(), 1)
            .expect("moved vector inventory")
            .objects
            .into_iter()
            .find(|vector| rects_nearly_equal(vector.bbox, [20.0, 50.0, 60.0, 70.0]))
            .expect("moved vector bounds");
        assert_ne!(moved.stable_id, original_vector.stable_id);
        let undo = session.undo_reflow().expect("undo");
        assert!(undo.undone);
        assert!(undo.byte_exact_restoration);
        assert_eq!(session.bytes(), input.as_slice());
        let restored = list_vector_objects(session.bytes(), 1)
            .expect("restored inventory")
            .objects
            .into_iter()
            .next()
            .expect("restored vector");
        assert!(rects_nearly_equal(restored.bbox, original_vector.bbox));
    }

    #[test]
    fn bounded_explicit_dependency_set_moves_multiple_downstream_paths_atomically() {
        let input =
            fixture(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n20 20 20 20 re f\n60 20 20 20 re f\n");
        let vectors = list_vector_objects(&input, 1)
            .expect("source vector inventory")
            .objects;
        assert_eq!(vectors.len(), 2);
        let mut req = request("HELLO", "WORLD");
        req.region = Some([10.0, 120.0, 260.0, 180.0]);
        req.downstream_vector_moves = vectors
            .iter()
            .enumerate()
            .map(|(index, vector)| DownstreamVectorMove {
                vector_stable_id: vector.stable_id.clone(),
                relationship: "keep_with_next".to_string(),
                dependency_edge_id: format!("user-approved-multiple-path-edge-{index}"),
                dx: 0.0,
                dy: 30.0,
                shared_form_policy: SharedFormEditPolicy::Reject,
            })
            .collect();
        let mut session = ReflowMutationSession::new(input.clone()).expect("session");
        let report = session.apply_geometric(&req).expect("multi-path movement");
        assert_eq!(report.objects_moved.len(), 2);
        assert_eq!(
            report.validation_evidence["downstream_vector_moves"]
                .as_array()
                .expect("move records")
                .len(),
            2
        );
        let moved = list_vector_objects(session.bytes(), 1)
            .expect("moved inventory")
            .objects;
        assert!(moved
            .iter()
            .any(|vector| rects_nearly_equal(vector.bbox, [20.0, 50.0, 40.0, 70.0])));
        assert!(moved
            .iter()
            .any(|vector| rects_nearly_equal(vector.bbox, [60.0, 50.0, 80.0, 70.0])));
        let undo = session.undo_reflow().expect("undo");
        assert!(undo.byte_exact_restoration);
        assert_eq!(session.bytes(), input.as_slice());
    }

    #[test]
    fn explicit_source_link_annotation_moves_with_text_and_undoes_atomically() {
        let input = fixture_with_source_link(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
        let mut req = request("HELLO", "WORLD");
        req.region = Some([10.0, 120.0, 260.0, 180.0]);
        req.downstream_link_moves = vec![DownstreamLinkMove {
            annotation_index: 0,
            expected_rect: [10.0, 140.0, 70.0, 160.0],
            relationship: "source_link".to_string(),
            dependency_edge_id: "user-approved-source-link-edge".to_string(),
            dx: 20.0,
            dy: -10.0,
        }];
        let preview = preview_reflow(&input, &req).expect("Link move plan");
        assert!(preview.flow_graph_changes.iter().any(|change| {
            change["kind"] == "planned_explicit_source_link_annotation_move"
                && change["plan"]["annotation_index"] == 0
        }));
        let mut session = ReflowMutationSession::new(input.clone()).expect("session");
        let report = session.apply_geometric(&req).expect("source Link move");
        assert!(report
            .objects_moved
            .contains(&"link_annotation:1:0".to_string()));
        assert_eq!(
            report.validation_evidence["downstream_link_moves"][0]["link_annotation_move"]
                ["after_rect"],
            json!([30.0, 130.0, 90.0, 150.0])
        );
        let moved_engine = ContentEngine::open_bytes(session.bytes().to_vec()).expect("moved open");
        let moved_report = interactive_report(&moved_engine).expect("moved interactive report");
        assert_eq!(
            moved_report.annotations.annotations[0].rect,
            Some([30.0, 130.0, 90.0, 150.0])
        );
        let validation = validate_reflow_output(&input, session.bytes(), &req)
            .expect("Link-aware unaffected-content validation");
        assert_eq!(validation["valid"], true);
        let undo = session.undo_reflow().expect("undo");
        assert!(undo.undone);
        assert!(undo.byte_exact_restoration);
        assert_eq!(session.bytes(), input.as_slice());
    }

    #[test]
    fn bounded_source_link_set_moves_multiple_annotations_and_undoes_atomically() {
        let input = fixture_with_two_source_links(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
        let mut req = request("HELLO", "WORLD");
        req.region = Some([10.0, 120.0, 260.0, 180.0]);
        req.downstream_link_moves = vec![
            DownstreamLinkMove {
                annotation_index: 0,
                expected_rect: [10.0, 140.0, 70.0, 160.0],
                relationship: "source_link".to_string(),
                dependency_edge_id: "user-approved-first-link-edge".to_string(),
                dx: 20.0,
                dy: -10.0,
            },
            DownstreamLinkMove {
                annotation_index: 1,
                expected_rect: [90.0, 140.0, 150.0, 160.0],
                relationship: "source_link".to_string(),
                dependency_edge_id: "user-approved-second-link-edge".to_string(),
                dx: 20.0,
                dy: -10.0,
            },
        ];
        let mut session = ReflowMutationSession::new(input.clone()).expect("session");
        let report = session
            .apply_geometric(&req)
            .expect("two source Links move");
        assert_eq!(report.objects_moved.len(), 2);
        let moved_engine = ContentEngine::open_bytes(session.bytes().to_vec()).expect("moved open");
        let moved = interactive_report(&moved_engine)
            .expect("moved interactive")
            .annotations
            .annotations;
        assert_eq!(moved.len(), 2);
        assert_eq!(moved[0].rect, Some([30.0, 130.0, 90.0, 150.0]));
        assert_eq!(moved[1].rect, Some([110.0, 130.0, 170.0, 150.0]));
        let undo = session.undo_reflow().expect("undo");
        assert!(undo.byte_exact_restoration);
        assert_eq!(session.bytes(), input.as_slice());
    }

    #[test]
    fn source_link_movement_refuses_a_locked_annotation_collision_before_mutation() {
        let input = fixture_with_two_source_links(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
        let before = input.clone();
        let mut req = request("HELLO", "WORLD");
        req.region = Some([10.0, 120.0, 260.0, 180.0]);
        req.downstream_link_moves = vec![DownstreamLinkMove {
            annotation_index: 0,
            expected_rect: [10.0, 140.0, 70.0, 160.0],
            relationship: "source_link".to_string(),
            dependency_edge_id: "locked-second-link-collision".to_string(),
            dx: 80.0,
            dy: 0.0,
        }];
        let error = preview_reflow(&input, &req).expect_err("locked Link collision");
        assert!(error.to_string().contains("locked_object_conflict"));
        assert_eq!(
            interactive_report(&ContentEngine::open_bytes(input.clone()).expect("source open"))
                .expect("source report")
                .annotations
                .annotations
                .len(),
            2
        );
        assert_eq!(input, before);
    }

    #[test]
    fn arabic_full_justification_refuses_without_a_real_kashida_serializer() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
        let mut request = request("HELLO", "HELLO");
        request.alignment = "justify".into();
        request.language = Some("ar".into());
        let error = apply_reflow_region(&input, &request).expect_err("Arabic kashida boundary");
        assert!(error.to_string().contains("shaping_failed"));
    }

    #[test]
    fn final_line_advance_matches_prompt20_shaping() {
        let text = "office fi";
        let layout =
            line_break_text(text, 400.0, 80.0, 14.0, Some("en"), None, false).expect("layout");
        assert_eq!(layout.lines.len(), 1);
        let shaped = crate::prompt20::analyze_advanced_text_reflow(
            text,
            AdvancedTextMode::ParagraphReflowHorizontal,
            None,
            crate::prompt20::TextReflowLimits::default(),
        )
        .expect("canonical shaping");
        let expected = shaped
            .glyphs
            .iter()
            .map(|glyph| glyph.advance_1000.abs())
            .sum::<f64>()
            / 1000.0
            * (14.0 / 1.2);
        assert!((layout.lines[0].advance - expected).abs() < 1e-9);
    }

    #[test]
    fn final_optimizer_can_improve_on_the_greedy_preview_without_changing_text() {
        let optimized = [
            "a a a a a a a a a a a a a a",
            "alpha beta gamma delta epsilon zeta eta theta iota kappa",
            "one two three four five six seven eight nine ten eleven twelve",
            "small medium large small medium large small medium large small",
        ]
        .into_iter()
        .find_map(|text| {
            (20..260).map(f64::from).find_map(|width| {
                let layout =
                    line_break_text(text, width, 400.0, 14.0, Some("en"), None, false).ok()?;
                let preview = layout
                    .preview_lines
                    .iter()
                    .map(|line| line.text.as_str())
                    .collect::<Vec<_>>();
                let final_lines = layout
                    .lines
                    .iter()
                    .map(|line| line.text.as_str())
                    .collect::<Vec<_>>();
                let preview_cost = layout
                    .preview_lines
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| *index + 1 < layout.preview_lines.len())
                    .map(|(_, line)| (width - line.advance).powi(2))
                    .sum::<f64>();
                (layout.overflow_status == OverflowStatus::FitInRegion
                    && preview != final_lines
                    && layout.final_cost + 1e-9 < preview_cost)
                    .then_some((text, layout))
            })
        });
        let (text, layout) = optimized
            .expect("a deterministic corpus case where dynamic layout improves greedy filling");
        assert_eq!(
            layout
                .lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<String>(),
            text
        );
        assert!(layout.final_cost.is_finite());
        assert_ne!(
            layout
                .preview_lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            layout
                .lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn preview_and_final_optimizer_enforce_the_consecutive_dictionary_hyphen_limit() {
        let text = "aaaaaaa";
        let graphemes = text.graphemes(true).collect::<Vec<_>>();
        let record = |end: usize, dictionary_hyphen: bool| LineBreakRecord {
            logical_offset_utf8: end,
            grapheme_index: Some(end),
            shaping_cluster_utf8: Some(end),
            source_location: format!("test:{end}"),
            break_class: if dictionary_hyphen {
                "dictionary_hyphenation".to_string()
            } else {
                "test_end".to_string()
            },
            disposition: "optional".to_string(),
            penalty: 0,
            hyphenation_source: if dictionary_hyphen {
                "dictionary:en-us".to_string()
            } else {
                "none".to_string()
            },
            inserted_visual_glyph_behavior: "test".to_string(),
            extraction_behavior: "test".to_string(),
            source_output_supported: true,
            confidence: Prompt33EvidenceKind::DeterministicGeometry,
            reason: "test".to_string(),
        };
        let mut records = (1..graphemes.len())
            .map(|end| record(end, true))
            .collect::<Vec<_>>();
        records.push(record(graphemes.len(), false));

        let font_size = 14.0 / 1.2;
        let one_with_hyphen = shaped_advance("a-", "left_to_right", font_size).expect("shape");
        let two_with_hyphen = shaped_advance("aa-", "left_to_right", font_size).expect("shape");
        assert!(one_with_hyphen < two_with_hyphen);
        let width = (one_with_hyphen + two_with_hyphen) / 2.0;

        let mut preview_cache = BTreeMap::new();
        let mut preview_spans = 0;
        let preview = greedy_line_ranges(
            &graphemes,
            &records,
            width,
            "left_to_right",
            font_size,
            &mut preview_cache,
            &mut preview_spans,
        )
        .expect("bounded greedy preview");
        assert!(
            preview
                .iter()
                .filter(|(_, end)| {
                    line_record_at(&records, *end).is_some_and(|candidate| {
                        candidate.hyphenation_source.starts_with("dictionary:")
                    })
                })
                .count()
                <= HYPHENATION_MAX_CONSECUTIVE_HYPHENATED_LINES
        );

        let mut final_cache = BTreeMap::new();
        let mut final_spans = 0;
        let optimized = optimized_line_ranges(
            &graphemes,
            &records,
            width,
            "left_to_right",
            font_size,
            &mut final_cache,
            &mut final_spans,
        )
        .expect("bounded optimizer");
        assert!(
            optimized.is_none(),
            "the optimizer must refuse a layout that requires more than the configured generated-hyphen run"
        );
    }

    #[test]
    fn dictionary_hyphenation_is_language_aware_and_writer_fail_closed() {
        let english = line_break_text(
            "demonstration",
            500.0,
            80.0,
            14.0,
            Some("en-US"),
            None,
            true,
        )
        .expect("english candidate plan");
        let spanish = line_break_text(
            "extraordinario",
            500.0,
            80.0,
            14.0,
            Some("es-MX"),
            None,
            true,
        )
        .expect("spanish candidate plan");
        assert_eq!(english.hyphenation["resolved_language"], "en-us");
        assert_eq!(spanish.hyphenation["resolved_language"], "es");
        assert!(english.break_records.iter().any(|record| {
            record.hyphenation_source == "dictionary:en-us" && record.source_output_supported
        }));
        assert!(spanish.break_records.iter().any(|record| {
            record.hyphenation_source == "dictionary:es" && record.source_output_supported
        }));
        assert_eq!(english.lines[0].text, "demonstration");
        assert_eq!(
            english.hyphenation["output_application"],
            "canonical_generated_type0_visual_hyphen_with_empty_tounicode_mapping"
        );
    }

    #[test]
    fn dictionary_hyphen_is_visible_but_not_added_to_logical_extraction() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
        let selected = (36..140).map(f64::from).find_map(|width| {
            let layout = line_break_text(
                "demonstration",
                width,
                90.0,
                14.0,
                Some("en-US"),
                None,
                true,
            )
            .ok()?;
            layout
                .lines
                .iter()
                .any(|line| line.hyphen_inserted)
                .then_some((width, layout))
        });
        let (width, layout) = selected.expect("a bounded dictionary hyphen layout");
        assert_eq!(
            layout
                .lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<String>(),
            "demonstration"
        );
        assert!(layout.lines.iter().any(|line| line.hyphen_inserted));
        let mut req = request("HELLO", "demonstration");
        req.region = Some([10.0, 10.0, 10.0 + width, 90.0]);
        let (output, report) = apply_reflow_region(&input, &req).expect("hyphenated source reflow");
        assert!(report
            .line_breaking
            .lines
            .iter()
            .any(|line| line.hyphen_inserted));
        let extracted = ContentEngine::open_bytes(output)
            .expect("reopen")
            .get_page_text(1)
            .expect("extract");
        assert!(layout_extraction_equivalent(&extracted, "demonstration"));
        assert!(!extracted.contains("demonstration-"));
    }

    #[test]
    fn unsupported_hyphenation_language_is_exact_and_never_guessed() {
        let layout = line_break_text(
            "longunbreakabletoken",
            500.0,
            80.0,
            14.0,
            Some("nl-NL"),
            None,
            true,
        )
        .expect("unsupported language is an exact report, not an error");
        assert_eq!(
            layout.hyphenation["typed_result"],
            "hyphenation_unavailable"
        );
        assert!(!layout
            .break_records
            .iter()
            .any(|record| { record.hyphenation_source.starts_with("dictionary:") }));
    }

    #[test]
    fn overflow_refusal_is_no_change_and_no_silent_font_reduction() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
        let mut req = request("HELLO", "THIS IS A VERY LONG STRING THAT CANNOT FIT");
        req.region = Some([10.0, 10.0, 50.0, 24.0]);
        req.max_downstream_blocks = 0;
        let preview = preview_reflow(&input, &req).expect("preview");
        assert!(preview.refusal.is_some());
        assert_eq!(preview.applied_mode, None);
        assert_eq!(
            preview.validation_evidence["no_silent_clipping"],
            serde_json::Value::Bool(true)
        );
        assert!(preview.constraints.infeasible);
        assert!(preview.constraints.solver.starts_with("cassowary-0.3.0"));
    }

    #[test]
    fn explicit_whitespace_expansion_is_an_ordered_source_rewrite_stage() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
        let mut req = request("HELLO", "ONE TWO THREE FOUR FIVE SIX");
        req.region = Some([10.0, 10.0, 105.0, 24.0]);
        req.allowed_expansion_region = Some([10.0, 10.0, 105.0, 94.0]);
        let preview = preview_reflow(&input, &req).expect("expanded preview");
        assert_eq!(
            preview.overflow_status,
            OverflowStatus::FitAfterRegionExpansion
        );
        assert!(!preview.constraints.infeasible);
        assert_eq!(
            preview.pages_columns_affected[0]["effective_region"],
            json!([10.0, 10.0, 105.0, 94.0])
        );
        let (_output, applied) = apply_reflow_region(&input, &req).expect("expanded source reflow");
        assert_eq!(
            applied.overflow_status,
            OverflowStatus::FitAfterRegionExpansion
        );
        assert_eq!(
            applied.validation_evidence["source_rewrite"]["detail"]["lines_or_columns"],
            json!(applied.line_breaking.lines.len())
        );
    }

    #[test]
    fn cassowary_region_constraints_accept_a_fitting_layout() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
        let preview = preview_reflow(&input, &request("HELLO", "WORLD")).expect("preview");
        assert!(!preview.constraints.infeasible);
        assert!(preview
            .constraints
            .constraints
            .iter()
            .any(|constraint| constraint["kind"] == "baseline_grid_preference"));
        assert!(preview
            .constraints
            .hard_constraints
            .iter()
            .all(
                |constraint| constraint["required"] == serde_json::Value::Bool(true)
                    || constraint["kind"] == "page_creation_policy"
            ));
        assert_eq!(preview.constraints.soft_constraints[0]["priority"], "weak");
        assert!(preview.constraints.unsatisfied_soft_constraints.is_empty());
        assert_eq!(
            preview.constraints.fixed_constraint_count,
            preview.constraints.hard_constraints.len() + preview.constraints.soft_constraints.len()
        );
    }

    #[test]
    fn caller_hard_and_soft_constraints_share_the_bounded_solver() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
        let mut req = request("HELLO", "WORLD");
        req.layout_constraints = vec![
            LayoutConstraint {
                constraint_id: "prefer_taller_region".to_string(),
                variable: "region_height".to_string(),
                relation: "ge".to_string(),
                value: 500.0,
                priority: "weak".to_string(),
            },
            LayoutConstraint {
                constraint_id: "bounded_line_count".to_string(),
                variable: "line_count".to_string(),
                relation: "le".to_string(),
                value: 1.0,
                priority: "required".to_string(),
            },
        ];
        let preview = preview_reflow(&input, &req).expect("preview");
        assert!(!preview.constraints.infeasible);
        assert!(preview
            .constraints
            .soft_constraints
            .iter()
            .any(|constraint| {
                constraint["constraint_id"] == "prefer_taller_region"
                    && constraint["satisfied"] == false
            }));
        assert!(preview
            .constraints
            .unsatisfied_soft_constraints
            .iter()
            .any(|constraint| constraint["constraint_id"] == "prefer_taller_region"));
        assert!(preview
            .constraints
            .hard_constraints
            .iter()
            .any(|constraint| {
                constraint["constraint_id"] == "bounded_line_count"
                    && constraint["status"] == "satisfied"
            }));
    }

    #[test]
    fn caller_hard_constraint_conflict_refuses_before_source_mutation() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
        let mut req = request("HELLO", "WORLD");
        req.layout_constraints = vec![LayoutConstraint {
            constraint_id: "impossible_width".to_string(),
            variable: "region_width".to_string(),
            relation: "le".to_string(),
            value: 1.0,
            priority: "required".to_string(),
        }];
        let preview = preview_reflow(&input, &req).expect("preview");
        assert!(preview.constraints.infeasible);
        assert_eq!(
            preview
                .refusal
                .as_ref()
                .and_then(|value| value["code"].as_str()),
            Some("constraints_infeasible")
        );
        let error = apply_reflow_region(&input, &req).expect_err("must not mutate");
        assert!(error.to_string().contains("constraints_infeasible"));
    }

    #[test]
    fn caller_constraint_rejects_non_finite_values_without_panicking() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
        let mut req = request("HELLO", "WORLD");
        req.layout_constraints = vec![LayoutConstraint {
            constraint_id: "not_a_number".to_string(),
            variable: "region_width".to_string(),
            relation: "eq".to_string(),
            value: f64::NAN,
            priority: "required".to_string(),
        }];
        let preview = preview_reflow(&input, &req).expect("typed preview result");
        assert!(preview.constraints.infeasible);
        assert!(!preview.constraints.no_nan_or_infinite_geometry);
        assert!(preview
            .constraints
            .infeasibility_explanation
            .iter()
            .any(|reason| reason.contains("non_finite_constraint_value")));
    }

    #[test]
    fn reflow_query_apis_share_the_canonical_preview_state() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
        let request = request("HELLO", "WORLD");
        let overflow = query_overflow(&input, &request).expect("overflow query");
        let constraints = query_constraints(&input, &request).expect("constraint query");
        let confidence = query_confidence(&input, &request).expect("confidence query");
        assert_eq!(overflow["preview_only"], true);
        assert_eq!(constraints["preview_only"], true);
        assert_eq!(confidence["preview_only"], true);
        let (output, _) = apply_reflow_region(&input, &request).expect("apply");
        let validation = validate_reflow_output(&input, &output, &request).expect("validate");
        assert_eq!(validation["valid"], true);
    }

    #[test]
    fn semantic_layout_labels_inference_and_prompt34_boundary() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (Heading) Tj ET\n");
        let report = analyze_semantic_layout(&input, None).expect("semantic");
        assert!(report.exact_vs_inferred["heuristic_inference"].is_string());
        assert!(report
            .prompt34_boundaries
            .iter()
            .any(|item| item.contains("tables")));
        assert_eq!(report.reading_order["candidate_chain_is_acyclic"], true);
    }

    #[test]
    fn semantic_runtime_graph_uses_canonical_text_model_and_has_no_dangling_edges() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (HELLO WORLD) Tj ET\n");
        let report = analyze_semantic_layout(&input, None).expect("semantic runtime graph");
        let ids = report
            .nodes
            .iter()
            .map(|node| node.node_id.as_str())
            .collect::<BTreeSet<_>>();
        assert!(report
            .nodes
            .iter()
            .any(|node| node.node_type == "page_region"));
        assert!(report
            .nodes
            .iter()
            .any(|node| node.node_type == "paragraph"));
        assert!(report.nodes.iter().any(|node| node.node_type == "line"));
        assert!(report.nodes.iter().any(|node| node.node_type == "word"));
        assert!(report.nodes.iter().any(|node| node.node_type == "glyph"));
        assert!(report.nodes.iter().any(|node| node.node_type == "column"));
        assert!(report
            .edges
            .iter()
            .all(|edge| ids.contains(edge.source.as_str()) && ids.contains(edge.target.as_str())));
        assert_eq!(
            report.region_graph_invariants["valid"], true,
            "runtime graph invariants: {}",
            report.region_graph_invariants
        );
        assert_eq!(report.region_graph_invariants["no_dangling_edges"], true);
        assert_eq!(
            report.region_graph_invariants["stable_node_ids_unique"],
            true
        );
        assert_eq!(
            report.region_graph_invariants["stable_edge_ids_unique"],
            true
        );
        assert_eq!(report.region_graph_invariants["bounded_edge_count"], true);
        assert_eq!(
            report.region_graph_invariants["incremental_invalidation"]
                ["semantic_document_uses_document_scope_only_when_requested"],
            true
        );
        assert!(report
            .algorithms_used
            .iter()
            .any(|item| item == "canonical_xy_cut_projection_profiles"));
    }

    #[test]
    fn public_region_and_reading_reports_retain_runtime_graph_invariants() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (HELLO WORLD) Tj ET\n");
        let flow = flow_graph_report(&input).expect("flow graph report");
        let order = reading_order_report(&input).expect("reading order report");
        assert_eq!(flow["region_graph_invariants"]["valid"], true);
        assert_eq!(order["region_graph_invariants"]["no_dangling_edges"], true);
    }

    #[test]
    fn column_candidates_are_bounded_and_keep_spanning_blocks_out_of_ambiguous_columns() {
        let candidates = semantic_column_candidates(
            [0.0, 0.0, 400.0, 600.0],
            &[
                (0, [20.0, 320.0, 170.0, 500.0]),
                (1, [230.0, 300.0, 380.0, 490.0]),
                // A title spanning the page must remain a PageRegion child
                // rather than being duplicated into either inferred column.
                (2, [20.0, 520.0, 380.0, 560.0]),
            ],
        );
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].block_indices, vec![0]);
        assert_eq!(candidates[1].block_indices, vec![1]);
        assert!(candidates.iter().all(|candidate| {
            candidate.bounds[0].is_finite()
                && candidate.bounds[2].is_finite()
                && candidate.confidence >= 0.70
        }));

        let bounded = semantic_column_candidates(
            [0.0, 0.0, 20_000.0, 600.0],
            &(0..32)
                .map(|index| {
                    let left = index as f64 * 500.0;
                    (index, [left, 0.0, left + 100.0, 300.0])
                })
                .collect::<Vec<_>>(),
        );
        assert!(bounded.len() <= 8);
    }

    #[test]
    fn semantic_document_analysis_uses_bounded_document_scope_while_geometric_stays_local() {
        let input = two_page_fixture(
            b"BT /F1 12 Tf 10 150 Td (FIRST BODY) Tj ET\n",
            b"BT /F1 12 Tf 10 150 Td (SECOND BODY) Tj ET\n",
        );
        let mut semantic_request = request("FIRST BODY", "FIRST BODY");
        semantic_request.requested_mode = TrueEditingMode::SemanticDocument;
        let semantic =
            analyze_semantic_layout(&input, Some(&semantic_request)).expect("semantic scope");
        assert!(semantic.nodes.iter().any(|node| node.page == 1));
        assert!(semantic.nodes.iter().any(|node| node.page == 2));
        assert_eq!(
            semantic.region_graph_invariants["incremental_invalidation"]
                ["unaffected_pages_reused_without_full_page_analysis"],
            false
        );

        let geometric = analyze_semantic_layout(&input, Some(&request("FIRST BODY", "FIRST BODY")))
            .expect("geometric local scope");
        assert!(geometric.nodes.iter().all(|node| node.page == 1));
        assert_eq!(
            geometric.region_graph_invariants["incremental_invalidation"]["invalidated_pages"],
            json!([1])
        );
        assert_eq!(
            geometric.region_graph_invariants["incremental_invalidation"]
                ["unaffected_pages_reused_without_full_page_analysis"],
            true
        );
    }

    #[test]
    fn repeated_page_band_headers_become_runtime_artifact_nodes() {
        let input = two_page_fixture(
            b"BT /F1 12 Tf 10 280 Td (ACME REPORT 1) Tj ET\nBT /F1 12 Tf 10 80 Td (FIRST BODY) Tj ET\nBT /F1 10 Tf 10 20 Td (ACME FOOTER 1) Tj ET\n",
            b"BT /F1 12 Tf 10 280 Td (ACME REPORT 2) Tj ET\nBT /F1 12 Tf 10 80 Td (SECOND BODY) Tj ET\nBT /F1 10 Tf 10 20 Td (ACME FOOTER 2) Tj ET\n",
        );
        let report = analyze_semantic_layout(&input, None).expect("semantic runtime graph");
        let headers = report
            .nodes
            .iter()
            .filter(|node| {
                node.node_type == "header"
                    && node.source_evidence["repeated_region_detection"]["artifact_candidate"]
                        == Value::Bool(true)
            })
            .collect::<Vec<_>>();
        assert_eq!(headers.len(), 2);
        let footers = report
            .nodes
            .iter()
            .filter(|node| {
                node.node_type == "footer"
                    && node.source_evidence["repeated_region_detection"]["artifact_candidate"]
                        == Value::Bool(true)
            })
            .collect::<Vec<_>>();
        assert_eq!(footers.len(), 2);
        let order = report.reading_order["machine_order"]
            .as_array()
            .expect("machine order");
        assert!(headers.iter().all(|node| {
            !order
                .iter()
                .any(|value| value == &Value::String(node.node_id.clone()))
        }));
        assert!(footers.iter().all(|node| {
            !order
                .iter()
                .any(|value| value == &Value::String(node.node_id.clone()))
        }));
    }

    #[test]
    fn matching_inline_marker_and_bottom_body_create_a_reviewable_footnote_edge() {
        let input = fixture(
            b"BT /F1 12 Tf 10 220 Td (BODY\xB9) Tj ET\nBT /F1 10 Tf 10 20 Td (1 Footnote body) Tj ET\n",
        );
        let report = analyze_semantic_layout(&input, None).expect("semantic footnote graph");
        let marker = report
            .nodes
            .iter()
            .find(|node| node.node_type == "footnote_marker")
            .expect("marker");
        let body = report
            .nodes
            .iter()
            .find(|node| node.node_type == "footnote_body")
            .expect("body");
        assert!(report.edges.iter().any(|edge| {
            edge.relationship == "footnote_of"
                && edge.source == marker.node_id
                && edge.target == body.node_id
        }));
        assert!(report
            .review_required
            .iter()
            .any(|item| item["code"] == "low_confidence_semantic_structure"));
    }

    #[test]
    fn semantic_runtime_types_link_heading_list_caption_and_figure_evidence() {
        let input = fixture(
            b"q 20 80 80 40 re f Q\nBT /F1 20 Tf 10 250 Td (Section Heading) Tj ET\nBT /F1 12 Tf 10 200 Td (1. First list item) Tj ET\nBT /F1 12 Tf 10 180 Td (2. Second list item) Tj ET\nBT /F1 12 Tf 20 55 Td (Figure 1 sample) Tj ET\n",
        );
        let report = analyze_semantic_layout(&input, None).expect("semantic runtime types");
        assert!(report.nodes.iter().any(|node| node.node_type == "heading"));
        assert!(report.nodes.iter().any(|node| node.node_type == "list"));
        assert!(report
            .nodes
            .iter()
            .any(|node| node.node_type == "list_item"));
        assert!(report.nodes.iter().any(|node| node.node_type == "caption"));
        assert!(report.nodes.iter().any(|node| node.node_type == "figure"));
        assert!(report
            .edges
            .iter()
            .any(|edge| edge.relationship == "heading_for"));
        assert!(report
            .edges
            .iter()
            .any(|edge| edge.relationship == "list_parent"));
        assert!(report
            .edges
            .iter()
            .any(|edge| edge.relationship == "caption_of"));
        assert_eq!(
            report.region_graph_invariants["valid"], true,
            "runtime graph invariants: {}",
            report.region_graph_invariants
        );
    }

    #[test]
    fn reading_order_resolver_removes_lowest_confidence_cycle_edge_deterministically() {
        let node = |id: &str| SemanticRegionNode {
            node_id: id.to_string(),
            node_type: "paragraph".to_string(),
            page: 1,
            source_scene_nodes: Vec::new(),
            source_instructions: Vec::new(),
            bounds: [0.0, 0.0, 1.0, 1.0],
            text_hash: id.to_string(),
            evidence_kind: Prompt33EvidenceKind::DeterministicGeometry,
            confidence: json!({"overall": 1.0}),
            coordinate_space: "page_user_space".to_string(),
            source_evidence: json!({}),
            alternatives: Vec::new(),
            transaction_revision: "test".to_string(),
        };
        let edge = |id: &str, source: &str, target: &str, confidence: f64| SemanticRegionEdge {
            edge_id: id.to_string(),
            source: source.to_string(),
            target: target.to_string(),
            relationship: "next_reading".to_string(),
            confidence,
            exact_inferred_or_user_supplied: Prompt33EvidenceKind::HeuristicInference,
            source_evidence: json!({}),
            alternatives: Vec::new(),
        };
        let resolution = resolve_reading_order_graph(
            &[node("a"), node("b"), node("c")],
            &[
                edge("a-b", "a", "b", 0.9),
                edge("b-c", "b", "c", 0.8),
                edge("c-a", "c", "a", 0.2),
            ],
        );
        assert_eq!(resolution["cycle_count"], 1);
        assert_eq!(resolution["removed_cycle_edges"][0]["edge_id"], "c-a");
        assert_eq!(resolution["machine_order"], json!(["a", "b", "c"]));
    }

    #[test]
    fn annotated_reading_order_fixture_scores_columns_footnotes_and_cycle_resolution() {
        let node = |id: &str| SemanticRegionNode {
            node_id: id.to_string(),
            node_type: if id == "footnote" {
                "footnote_body".to_string()
            } else {
                "paragraph".to_string()
            },
            page: 1,
            source_scene_nodes: Vec::new(),
            source_instructions: Vec::new(),
            bounds: [0.0, 0.0, 1.0, 1.0],
            text_hash: id.to_string(),
            evidence_kind: Prompt33EvidenceKind::DeterministicGeometry,
            confidence: json!({"overall": 1.0}),
            coordinate_space: "page_user_space".to_string(),
            source_evidence: json!({"fixture": "annotated_two_column_footnote"}),
            alternatives: Vec::new(),
            transaction_revision: "fixture".to_string(),
        };
        let edge = |id: &str, source: &str, target: &str, confidence: f64| SemanticRegionEdge {
            edge_id: id.to_string(),
            source: source.to_string(),
            target: target.to_string(),
            relationship: "next_reading".to_string(),
            confidence,
            exact_inferred_or_user_supplied: Prompt33EvidenceKind::HeuristicInference,
            source_evidence: json!({"fixture": "annotated_two_column_footnote"}),
            alternatives: Vec::new(),
        };
        let resolution = resolve_reading_order_graph(
            &[
                node("left-heading"),
                node("left-body"),
                node("right-heading"),
                node("right-body"),
                node("footnote"),
            ],
            &[
                edge("left-heading-left-body", "left-heading", "left-body", 0.98),
                edge(
                    "left-body-right-heading",
                    "left-body",
                    "right-heading",
                    0.96,
                ),
                edge(
                    "right-heading-right-body",
                    "right-heading",
                    "right-body",
                    0.98,
                ),
                edge("right-body-footnote", "right-body", "footnote", 0.94),
                // A low-confidence conflicting preference creates a real
                // cycle; the resolver must remove this precise edge before
                // calculating the fixture metrics.
                edge("footnote-left-heading", "footnote", "left-heading", 0.15),
            ],
        );
        assert_eq!(
            resolution["removed_cycle_edges"][0]["edge_id"],
            "footnote-left-heading"
        );
        let actual = resolution["machine_order"]
            .as_array()
            .expect("machine order")
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect::<Vec<_>>();
        let expected = [
            "left-heading",
            "left-body",
            "right-heading",
            "right-body",
            "footnote",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
        let metrics = score_reading_order_fixture(
            &expected,
            &actual,
            &[
                vec!["left-heading".to_string(), "left-body".to_string()],
                vec!["right-heading".to_string(), "right-body".to_string()],
            ],
            &["footnote".to_string()],
        );
        assert_eq!(metrics["annotation_valid"], true);
        assert_eq!(metrics["exact_order_accuracy"], 1.0);
        assert_eq!(metrics["kendall_style_correlation"], 1.0);
        assert_eq!(metrics["column_order_accuracy"], 1.0);
        assert_eq!(metrics["footnote_placement_accuracy"], 1.0);
    }

    #[test]
    fn caption_association_uses_deterministic_nearest_scene_figure() {
        let figures = vec![
            ("far".to_string(), [200.0, 200.0, 260.0, 260.0]),
            ("near".to_string(), [20.0, 40.0, 100.0, 90.0]),
        ];
        let (id, gap) =
            nearest_figure(&figures, [20.0, 20.0, 100.0, 35.0]).expect("nearest figure");
        assert_eq!(id, "near");
        assert_eq!(gap, 5.0);
    }

    #[test]
    fn semantic_apply_requires_explicit_confidence_approval() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
        let mut req = request("HELLO", "WORLD");
        req.requested_mode = TrueEditingMode::SemanticDocument;
        let error = apply_reflow_document(&input, &req).expect_err("semantic apply refuses");
        assert!(error.to_string().contains("\"refuse\""));
    }

    #[test]
    fn explicit_semantic_single_paragraph_apply_preserves_its_mode() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
        let mut req = request("HELLO", "WORLD");
        req.requested_mode = TrueEditingMode::SemanticDocument;
        req.approve_low_confidence_structure = true;
        let (_output, report) = apply_reflow_document(&input, &req).expect("semantic apply");
        assert_eq!(report.requested_mode, TrueEditingMode::SemanticDocument);
        assert_eq!(report.applied_mode, Some(TrueEditingMode::SemanticDocument));
        assert_eq!(
            report.scope_of_movement,
            "semantic_single_paragraph_single_region_no_flow"
        );
    }

    #[test]
    fn semantic_local_apply_resolves_one_exact_paragraph_without_rejecting_other_body_paragraphs() {
        let input = fixture(
            b"BT /F1 12 Tf 10 180 Td (FIRST PARAGRAPH) Tj ET\nBT /F1 12 Tf 10 100 Td (SECOND PARAGRAPH) Tj ET\n",
        );
        let mut req = request("FIRST PARAGRAPH", "UPDATED PARAGRAPH");
        req.requested_mode = TrueEditingMode::SemanticDocument;
        req.approve_low_confidence_structure = true;
        req.region = Some([10.0, 140.0, 260.0, 200.0]);
        let (output, report) =
            apply_reflow_document(&input, &req).expect("page-local semantic apply");
        let reopened = ContentEngine::open_bytes(output).expect("semantic output reopen");
        let text = reopened.get_page_text(1).expect("semantic output text");
        assert!(text.contains("UPDATED PARAGRAPH"));
        assert!(text.contains("SECOND PARAGRAPH"));
        assert_eq!(
            report.flow_graph_changes[0]["source_semantic_node"],
            serde_json::Value::String(
                analyze_semantic_layout(&input, Some(&req))
                    .expect("semantic source")
                    .nodes
                    .into_iter()
                    .find(|node| {
                        node.node_type == "paragraph"
                            && node.page == 1
                            && node.text_hash == digest_hex(b"FIRST PARAGRAPH")
                    })
                    .expect("selected paragraph")
                    .node_id,
            )
        );
    }

    #[test]
    fn explicit_page_creation_flows_one_plain_semantic_paragraph_and_session_undo_restores() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
        let mut req = request(
            "HELLO",
            "ONE TWO THREE FOUR FIVE SIX SEVEN EIGHT NINE TEN ELEVEN TWELVE",
        );
        req.requested_mode = TrueEditingMode::SemanticDocument;
        req.approve_low_confidence_structure = true;
        req.allow_page_creation = true;
        req.region = Some([10.0, 10.0, 90.0, 25.0]);
        let mut session = ReflowMutationSession::new(input.clone()).expect("session");
        let report = session.apply_semantic(&req).expect("page flow apply");
        assert_eq!(report.overflow_status, OverflowStatus::FitAfterPageFlow);
        assert_eq!(report.pages_columns_affected.len(), 2);
        assert_eq!(
            ContentEngine::open_bytes(session.bytes().to_vec())
                .expect("reopen")
                .page_count()
                .expect("page count"),
            2
        );
        let undo = session.undo_reflow().expect("page flow undo");
        assert!(undo.undone);
        assert_eq!(session.bytes(), input.as_slice());
    }

    #[test]
    fn explicit_page_creation_preserves_existing_source_link_annotations() {
        let input = fixture_with_source_link(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
        let mut req = request(
            "HELLO",
            "ONE TWO THREE FOUR FIVE SIX SEVEN EIGHT NINE TEN ELEVEN TWELVE",
        );
        req.requested_mode = TrueEditingMode::SemanticDocument;
        req.approve_low_confidence_structure = true;
        req.allow_page_creation = true;
        req.region = Some([10.0, 10.0, 90.0, 25.0]);
        let (output, report) = apply_reflow_document(&input, &req).expect("page flow apply");
        let reopened = ContentEngine::open_bytes(output).expect("reopen");
        let annotations = interactive_report(&reopened)
            .expect("interactive")
            .annotations
            .annotations;
        assert_eq!(annotations.len(), 1);
        assert_eq!(annotations[0].subtype, "Link");
        assert_eq!(
            report.validation_evidence["catalog_reference_preservation"]["annotations_preserved"],
            true
        );
        assert_eq!(
            report.validation_evidence["page_tree_writer"],
            "canonical_writer_append_authored_page_preserving_catalog"
        );
    }

    #[test]
    fn explicit_page_creation_preserves_catalog_destination_outline_and_label_roots() {
        let input = fixture_with_catalog_reference_roots(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
        let before = ContentEngine::open_bytes(input.clone()).expect("catalog source open");
        let before_reader = before.document().reader();
        let (before_root, before_generation) = before_reader.root_reference().expect("source root");
        let before_catalog = before_reader
            .get_object(before_root, before_generation)
            .expect("source catalog")
            .as_dict()
            .cloned()
            .expect("source catalog dictionary");
        let mut req = request(
            "HELLO",
            "ONE TWO THREE FOUR FIVE SIX SEVEN EIGHT NINE TEN ELEVEN TWELVE",
        );
        req.requested_mode = TrueEditingMode::SemanticDocument;
        req.approve_low_confidence_structure = true;
        req.allow_page_creation = true;
        req.region = Some([10.0, 10.0, 90.0, 25.0]);
        let (output, report) = apply_reflow_document(&input, &req).expect("catalog page flow");
        let after = ContentEngine::open_bytes(output).expect("catalog output open");
        let after_reader = after.document().reader();
        let (after_root, after_generation) = after_reader.root_reference().expect("output root");
        let after_catalog_object = after_reader
            .get_object(after_root, after_generation)
            .expect("output catalog");
        let after_catalog = after_catalog_object
            .as_dict()
            .expect("output catalog dictionary");
        assert_eq!(
            format!("{:?}", before_catalog.get("PageLabels")),
            format!("{:?}", after_catalog.get("PageLabels")),
            "direct page-label policy is preserved by the catalog copy"
        );
        let (outline_number, outline_generation) = match after_catalog.get("Outlines") {
            Some(PdfObject::Reference { number, generation }) => (*number, *generation),
            other => panic!("output outline root must remain a reference, got {other:?}"),
        };
        let outline = after_reader
            .get_object(outline_number, outline_generation)
            .expect("copied outline object")
            .as_dict()
            .cloned()
            .expect("copied outline dictionary");
        assert_eq!(
            outline.get("Type"),
            Some(&PdfObject::Name("Outlines".to_string()))
        );
        assert_eq!(outline.get("Count"), Some(&PdfObject::Integer(0)));
        let destinations = after_catalog
            .get("Dests")
            .and_then(PdfObject::as_dict)
            .expect("copied named destination dictionary");
        let destination = destinations
            .get("source-page")
            .and_then(PdfObject::as_array)
            .expect("copied named destination array");
        assert!(matches!(destination.get(1), Some(PdfObject::Name(name)) if name == "Fit"));
        let (destination_page_number, destination_page_generation) = match destination.first() {
            Some(PdfObject::Reference { number, generation }) => (*number, *generation),
            other => panic!("named destination target must remain a page reference, got {other:?}"),
        };
        let first_page = after.document().get_page(1).expect("first copied page");
        assert_eq!(destination_page_number, first_page.object_number);
        assert_eq!(destination_page_generation, first_page.generation_number);
        let interactive = interactive_report(&after).expect("output interactive report");
        assert!(interactive.page_operations.outlines_present);
        assert!(interactive.page_operations.page_labels_present);
        assert!(interactive.page_operations.named_destinations_present);
        assert_eq!(
            report.validation_evidence["catalog_reference_preservation"]["outlines_preserved"],
            true
        );
        assert_eq!(
            report.validation_evidence["catalog_reference_preservation"]["page_labels_preserved"],
            true
        );
        assert_eq!(
            report.validation_evidence["catalog_reference_preservation"]
                ["named_destinations_preserved"],
            true
        );
    }

    #[test]
    fn binding_safe_undo_replay_rejects_stale_output_and_restores_exact_preimage() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
        let req = request("HELLO", "WORLD");
        let (output, _) = apply_reflow_region(&input, &req).expect("apply");
        let (restored, undo) =
            undo_reflow_from_replay(&input, &output, &req).expect("binding-safe undo");
        assert!(undo.undone);
        assert!(undo.byte_exact_restoration);
        assert_eq!(restored, input);
        let error = undo_reflow_from_replay(&input, &input, &req).expect_err("stale output");
        assert!(error.to_string().contains("stale_snapshot_conflict"));
    }

    #[test]
    fn existing_empty_next_page_region_receives_source_linked_continuation_and_undo() {
        let input = two_page_fixture(
            b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n",
            b"BT /F1 12 Tf 10 150 Td (TARGET) Tj ET\n",
        );
        let replacement = "ONE TWO THREE FOUR FIVE SIX SEVEN";
        let (width, expected_lines) = (45..140)
            .map(f64::from)
            .find_map(|width| {
                let layout =
                    line_break_text(replacement, width, 100.0, 14.0, Some("en"), None, false)
                        .ok()?;
                (layout.lines.len() == 2 && layout.overflow_status == OverflowStatus::FitInRegion)
                    .then_some((width, layout.lines.len()))
            })
            .expect("two-line continuation layout");
        assert_eq!(expected_lines, 2);
        let mut req = request("HELLO", replacement);
        req.requested_mode = TrueEditingMode::SemanticDocument;
        req.approve_low_confidence_structure = true;
        req.region = Some([10.0, 10.0, 10.0 + width, 24.0]);
        let mut session = ReflowMutationSession::new(input.clone()).expect("session");
        let report = session
            .apply_semantic(&req)
            .expect("existing next-page continuation");
        assert_eq!(report.overflow_status, OverflowStatus::FitAfterPageFlow);
        assert_eq!(
            report.scope_of_movement,
            "semantic_single_paragraph_existing_next_page_flow"
        );
        let reopened = ContentEngine::open_bytes(session.bytes().to_vec()).expect("reopen");
        assert_eq!(reopened.page_count().expect("page count"), 2);
        assert!(!reopened
            .get_page_text(1)
            .expect("first extract")
            .contains("HELLO"));
        assert!(reopened
            .get_page_text(2)
            .expect("second extract")
            .contains("TARGET"));
        let undo = session.undo_reflow().expect("undo");
        assert!(undo.byte_exact_restoration);
        assert_eq!(session.bytes(), input.as_slice());
    }

    #[test]
    fn explicit_empty_next_region_receives_source_linked_continuation_and_undo() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
        let replacement = "ONE TWO THREE FOUR FIVE SIX SEVEN";
        let (width, expected_lines) = (45..140)
            .map(f64::from)
            .find_map(|width| {
                let layout =
                    line_break_text(replacement, width, 100.0, 14.0, Some("en"), None, false)
                        .ok()?;
                (layout.lines.len() == 2 && layout.overflow_status == OverflowStatus::FitInRegion)
                    .then_some((width, layout.lines.len()))
            })
            .expect("two-line downstream layout");
        assert_eq!(expected_lines, 2);
        let mut req = request("HELLO", replacement);
        req.requested_mode = TrueEditingMode::SemanticDocument;
        req.approve_low_confidence_structure = true;
        req.region = Some([10.0, 60.0, 10.0 + width, 74.0]);
        req.next_region = Some([10.0, 40.0, 10.0 + width, 54.0]);
        let mut session = ReflowMutationSession::new(input.clone()).expect("session");
        let report = session
            .apply_semantic(&req)
            .expect("existing next-region continuation");
        assert_eq!(
            report.overflow_status,
            OverflowStatus::FitAfterDownstreamFlow
        );
        assert_eq!(
            report.scope_of_movement,
            "semantic_single_paragraph_existing_next_region_flow"
        );
        assert_eq!(report.flow_graph_changes[0]["kind"], "next_region");
        let extracted = ContentEngine::open_bytes(session.bytes().to_vec())
            .expect("reopen")
            .get_page_text(1)
            .expect("extract");
        assert!(!extracted.contains("HELLO"));
        assert!(
            layout_extraction_equivalent(&extracted, replacement),
            "same-page downstream extraction differs: extracted={extracted:?}; expected={replacement:?}"
        );
        let undo = session.undo_reflow().expect("undo");
        assert!(undo.byte_exact_restoration);
        assert_eq!(session.bytes(), input.as_slice());
    }

    #[test]
    fn preserved_style_policy_refuses_downstream_flow_without_downgrading() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
        let mut req = request("HELLO", "ONE TWO THREE FOUR FIVE SIX SEVEN");
        req.requested_mode = TrueEditingMode::SemanticDocument;
        req.approve_low_confidence_structure = true;
        req.font_policy = "preserve_original_per_run".into();
        req.region = Some([10.0, 60.0, 60.0, 74.0]);
        req.next_region = Some([10.0, 40.0, 60.0, 54.0]);
        let error = apply_reflow_document(&input, &req).expect_err("must not downgrade");
        assert!(error
            .to_string()
            .contains("preserve_original_per_run is supported only for a single local region"));
    }

    #[test]
    fn explicit_empty_next_column_receives_source_linked_continuation_and_undo() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
        let replacement = "ONE TWO THREE FOUR FIVE SIX SEVEN";
        let (width, expected_lines) = (45..140)
            .map(f64::from)
            .find_map(|width| {
                let layout =
                    line_break_text(replacement, width, 100.0, 14.0, Some("en"), None, false)
                        .ok()?;
                (layout.lines.len() == 2 && layout.overflow_status == OverflowStatus::FitInRegion)
                    .then_some((width, layout.lines.len()))
            })
            .expect("two-line next-column layout");
        assert_eq!(expected_lines, 2);
        let mut req = request("HELLO", replacement);
        req.requested_mode = TrueEditingMode::SemanticDocument;
        req.approve_low_confidence_structure = true;
        req.region = Some([10.0, 60.0, 10.0 + width, 74.0]);
        req.next_column = Some([150.0, 60.0, 150.0 + width, 74.0]);
        let mut session = ReflowMutationSession::new(input.clone()).expect("session");
        let report = session
            .apply_semantic(&req)
            .expect("existing next-column continuation");
        assert_eq!(report.overflow_status, OverflowStatus::FitAfterColumnFlow);
        assert_eq!(
            report.scope_of_movement,
            "semantic_single_paragraph_existing_next_column_flow"
        );
        assert_eq!(report.flow_graph_changes[0]["kind"], "next_column");
        let extracted = ContentEngine::open_bytes(session.bytes().to_vec())
            .expect("reopen")
            .get_page_text(1)
            .expect("extract");
        assert!(!extracted.contains("HELLO"));
        assert!(layout_extraction_equivalent(&extracted, replacement));
        let undo = session.undo_reflow().expect("undo");
        assert!(undo.byte_exact_restoration);
        assert_eq!(session.bytes(), input.as_slice());
    }

    #[test]
    fn explicit_empty_rtl_next_column_receives_source_linked_continuation_and_undo() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
        let replacement = concat!(
            "\u{0641}\u{0627}\u{062A}\u{0648}\u{0631}\u{0629} 123 ",
            "\u{0641}\u{0627}\u{062A}\u{0648}\u{0631}\u{0629} 456 ",
            "\u{0641}\u{0627}\u{062A}\u{0648}\u{0631}\u{0629} 789"
        );
        let (width, expected_lines) = (45..130)
            .map(f64::from)
            .find_map(|width| {
                let layout = line_break_text(
                    replacement,
                    width,
                    100.0,
                    14.0,
                    Some("ar"),
                    Some("rtl"),
                    false,
                )
                .ok()?;
                (layout.lines.len() == 2 && layout.overflow_status == OverflowStatus::FitInRegion)
                    .then_some((width, layout.lines.len()))
            })
            .expect("two-line RTL next-column layout");
        assert_eq!(expected_lines, 2);
        let mut req = request("HELLO", replacement);
        req.requested_mode = TrueEditingMode::SemanticDocument;
        req.approve_low_confidence_structure = true;
        req.language = Some("ar".into());
        req.direction = Some("rtl".into());
        req.region = Some([150.0, 60.0, 150.0 + width, 74.0]);
        req.next_column = Some([10.0, 60.0, 10.0 + width, 74.0]);
        let mut session = ReflowMutationSession::new(input.clone()).expect("session");
        let report = session
            .apply_semantic(&req)
            .expect("existing RTL next-column continuation");
        assert_eq!(report.overflow_status, OverflowStatus::FitAfterColumnFlow);
        assert_eq!(
            report.flow_graph_changes[0]["base_direction"],
            "right_to_left"
        );
        let extracted = ContentEngine::open_bytes(session.bytes().to_vec())
            .expect("reopen")
            .get_page_text(1)
            .expect("extract");
        assert!(!extracted.contains("HELLO"));
        assert!(layout_extraction_equivalent(&extracted, replacement));
        let undo = session.undo_reflow().expect("undo");
        assert!(undo.byte_exact_restoration);
        assert_eq!(session.bytes(), input.as_slice());
    }

    #[test]
    fn low_confidence_semantic_plan_requires_review_before_apply() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
        let mut req = request("HELLO", "WORLD");
        req.requested_mode = TrueEditingMode::SemanticDocument;
        let preview = preview_reflow(&input, &req).expect("semantic plan");
        assert_eq!(preview.confidence["decision"], "refuse");
        assert_eq!(preview.refusal.as_ref().unwrap()["code"], "refuse");
        assert_eq!(preview.applied_mode, None);
    }

    #[test]
    fn line_breaker_preserves_bidi_and_grapheme_boundaries() {
        let layout = line_break_text(
            "a\u{0301} שלום 123",
            80.0,
            80.0,
            14.0,
            Some("he"),
            Some("rtl"),
            false,
        )
        .expect("layout");
        assert!(layout.grapheme_safe);
        assert!(layout.bidi_source_visual_separated);
        assert!(!layout.lines.is_empty());
    }

    #[test]
    fn uax14_arabic_and_combining_marks_keep_grapheme_candidates() {
        let text = "\u{0633}\u{064e}\u{0644}\u{0627}\u{0645} \u{0639}\u{0627}\u{0644}\u{0645}";
        let layout = line_break_text(text, 48.0, 80.0, 14.0, Some("ar"), Some("rtl"), false)
            .expect("layout");
        let boundaries = grapheme_boundaries(text);
        assert!(layout.grapheme_safe);
        assert!(layout.break_records.iter().all(|record| record
            .grapheme_index
            .is_none_or(|index| index > 0 && index < boundaries.len())));
        assert_eq!(
            layout
                .lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<String>(),
            text
        );
    }

    #[test]
    fn uax14_nonbreaking_text_is_refused_instead_of_clipped() {
        let layout = line_break_text("A\u{00a0}B", 12.0, 80.0, 14.0, Some("en"), None, false)
            .expect("layout report");
        assert_eq!(layout.overflow_status, OverflowStatus::UnresolvedOverflow);
        assert!(layout.overflow_amount > 0.0);
        assert!(layout
            .exact_limits
            .iter()
            .any(|limit| limit.contains("never asks the writer to clip")));
    }

    #[test]
    fn mandatory_line_separator_controls_geometry_without_becoming_a_missing_glyph() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
        let replacement = "ONE\nTWO";
        let layout = line_break_text(replacement, 120.0, 80.0, 14.0, Some("en"), None, false)
            .expect("mandatory break layout");
        assert_eq!(
            layout
                .lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<String>(),
            replacement
        );
        assert_eq!(layout.lines[0].visual_text, "ONE");
        assert!(layout
            .break_records
            .iter()
            .any(|record| record.disposition == "mandatory"));
        let mut req = request("HELLO", replacement);
        req.region = Some([10.0, 10.0, 130.0, 80.0]);
        let (output, _) = apply_reflow_region(&input, &req).expect("mandatory source reflow");
        let extracted = ContentEngine::open_bytes(output)
            .expect("reopen")
            .get_page_text(1)
            .expect("extract");
        assert!(layout_extraction_equivalent(&extracted, replacement));
    }
}
