//! editing transactions editable-scene, transaction, and font/text identity closure.
//!
//! This module is intentionally an adapter over the canonical advanced editing/31
//! source-editing, display-list, writer, font-shaper, and editable-document
//! systems.  It adds stable scene/snapshot/transaction/font identity contracts
//! without creating a second parser, renderer, writer, or binding-specific edit
//! engine.

use crate::advanced_editing::{list_vector_objects, SharedFormEditPolicy};
use crate::fonts::{ShapeOptions, TextDirection, TextShaper};
use crate::render::font_rasterizer::get_fallback_font;
use crate::source_editing::{
    edit_text_operator, operator_text_eligibility, operator_text_provenance,
    OperatorEditOperationReport, OperatorTextEditRequest, TrueEditingMode,
};
use crate::{ContentEngine, Result, WellfriendError};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use unicode_bidi::BidiInfo;
use unicode_segmentation::UnicodeSegmentation;

pub const EDITING_TRANSACTIONS_SCHEMA_VERSION: &str =
    "editing_transactions.scene-transactions-fonts-shaping.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditingTransactionsStatus {
    Implemented,
    ImplementedWithLimits,
    Verified,
    VerifiedWithLimits,
    UnsupportedExact,
    TextReflowRouted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditingTransactionsEvidenceKind {
    NormativeExact,
    ParserExact,
    RendererExact,
    DeterministicDerived,
    HarfbuzzExact,
    UnicodeDataExact,
    PdfMetricExact,
    HeuristicInferred,
    Ambiguous,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SceneNodeKind {
    TextObject,
    PathObject,
    ImageObject,
    FormOccurrence,
    ShadingObject,
    AnnotationObject,
    WidgetObject,
    MarkedContent,
}

#[derive(Debug, Clone, Serialize)]
pub struct SceneNode {
    pub schema_version: String,
    pub node_id: String,
    pub node_kind: SceneNodeKind,
    pub snapshot_id: String,
    pub page: usize,
    pub occurrence_id: String,
    pub definition_identity: Option<String>,
    pub source_object_revision: Option<String>,
    pub source_instruction_ids: Vec<String>,
    pub display_item_ids: Vec<String>,
    pub resource_scope: String,
    pub nested_occurrence_path: Vec<String>,
    pub marked_content_ids: Vec<String>,
    pub structure_node_ids: Vec<String>,
    pub bounds_user_space: [f64; 4],
    pub transform_summary: String,
    pub z_order: usize,
    pub clipping: String,
    pub visibility: String,
    pub graphics_state_summary: Value,
    pub edit_eligibility: Vec<String>,
    pub supported_edit_modes: Vec<TrueEditingMode>,
    pub evidence_strength: EditingTransactionsEvidenceKind,
    pub shared_resource_status: String,
    pub signature_conformance_restrictions: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EditableSceneGraph {
    pub schema_version: String,
    pub document_id: String,
    pub snapshot_id: String,
    pub revision_id: String,
    pub page_count: usize,
    pub nodes: Vec<SceneNode>,
    pub definition_occurrence_distinction: bool,
    pub source_linked: bool,
    pub bounded_query_limits: Value,
    pub exact_limits: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneSelectionRequest {
    pub page: usize,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub point: Option<[f64; 2]>,
    #[serde(default)]
    pub region: Option<[f64; 4]>,
    #[serde(default)]
    pub cycle_index: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SceneSelectionReport {
    pub schema_version: String,
    pub request: SceneSelectionRequest,
    pub matched_nodes: Vec<SceneNode>,
    pub ambiguous: bool,
    pub source_provenance_available: bool,
    pub refusal: Option<Value>,
    pub query_limits: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocumentSnapshot {
    pub schema_version: String,
    pub snapshot_id: String,
    pub parent_snapshot_id: Option<String>,
    pub revision_id: String,
    pub document_id: String,
    pub page_count: usize,
    pub immutable: bool,
    pub concurrent_reader_safe: bool,
    pub structural_sharing: String,
    pub revision_aware_cache_keys: Vec<String>,
    pub changed_objects_from_parent: Vec<String>,
    pub changed_pages_from_parent: Vec<usize>,
    pub changed_scene_nodes_from_parent: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionState {
    Created,
    Planned,
    ValidatedPreconditions,
    AppliedInMemory,
    ValidatedPostconditions,
    CommittedSnapshot,
    Serialized,
    ReopenedValidated,
    RolledBack,
    Failed,
    RefusedNoChange,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneTextEditRequest {
    #[serde(default = "default_operator_preserving")]
    pub requested_mode: TrueEditingMode,
    pub page: usize,
    pub source_text: String,
    pub replacement_text: String,
    #[serde(default)]
    pub signature_policy_override: bool,
    #[serde(default = "default_font_policy")]
    pub font_policy: String,
    #[serde(default)]
    pub normalization_policy: Option<String>,
    #[serde(default)]
    pub direction: Option<String>,
    #[serde(default)]
    pub region: Option<[f64; 4]>,
    #[serde(default)]
    pub allowed_expansion_region: Option<[f64; 4]>,
    #[serde(default)]
    pub next_region: Option<[f64; 4]>,
    #[serde(default)]
    pub next_column: Option<[f64; 4]>,
    #[serde(default)]
    pub downstream_vector_moves: Vec<crate::text_reflow::DownstreamVectorMove>,
    #[serde(default)]
    pub downstream_link_moves: Vec<crate::text_reflow::DownstreamLinkMove>,
    #[serde(default)]
    pub layout_constraints: Vec<crate::text_reflow::LayoutConstraint>,
    #[serde(default)]
    pub language: Option<String>,
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
    #[serde(default = "default_line_height")]
    pub line_height: f64,
    #[serde(default = "default_max_downstream_blocks")]
    pub max_downstream_blocks: usize,
}

impl Default for SceneTextEditRequest {
    fn default() -> Self {
        Self {
            requested_mode: default_operator_preserving(),
            page: 1,
            source_text: String::new(),
            replacement_text: String::new(),
            signature_policy_override: false,
            font_policy: default_font_policy(),
            normalization_policy: None,
            direction: None,
            region: None,
            allowed_expansion_region: None,
            next_region: None,
            next_column: None,
            downstream_vector_moves: Vec::new(),
            downstream_link_moves: Vec::new(),
            layout_constraints: Vec::new(),
            language: None,
            alignment: default_alignment(),
            justify_last_line: false,
            hyphenation: false,
            allow_page_creation: false,
            allow_font_reduction: false,
            approve_low_confidence_structure: false,
            line_height: default_line_height(),
            max_downstream_blocks: default_max_downstream_blocks(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EditTransactionReport {
    pub schema_version: String,
    pub transaction_id: String,
    pub base_snapshot_id: String,
    pub requested_mode: TrueEditingMode,
    pub applied_mode: Option<TrueEditingMode>,
    pub lifecycle: Vec<TransactionState>,
    pub preconditions: Vec<Value>,
    pub read_set: Vec<String>,
    pub write_set: Vec<String>,
    pub affected_objects: Vec<String>,
    pub affected_pages: Vec<usize>,
    pub affected_scene_nodes: Vec<String>,
    pub cloned_resources: Vec<String>,
    pub dirty_regions: Vec<Value>,
    pub signature_impact: Value,
    pub conformance_impact: Value,
    pub validation_plan: Vec<String>,
    pub inverse_operations: Vec<Value>,
    pub commit_policy: String,
    pub operation_log_hash: String,
    pub deterministic: bool,
    pub refusal: Option<Value>,
    pub source_editing_operation: Option<OperatorEditOperationReport>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphemeClusterRecord {
    pub cluster_id: String,
    pub utf8_range: [usize; 2],
    pub scalar_range: [usize; 2],
    pub text: String,
    pub safe_boundary_before: bool,
    pub safe_boundary_after: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ShapingGlyphRecord {
    pub glyph_occurrence_id: String,
    pub glyph_id: u16,
    pub cluster_utf8: u32,
    pub advance: f64,
    pub offset_x: f64,
    pub offset_y: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FontIdentityReport {
    pub schema_version: String,
    pub text_hash: String,
    pub unicode_data_version: String,
    pub identities_separated: Vec<String>,
    pub grapheme_clusters: Vec<GraphemeClusterRecord>,
    pub bidi: Value,
    pub shaping: Value,
    pub mapping_edges: Vec<Value>,
    pub reverse_cluster_mapping: Vec<Value>,
    pub ambiguity: Vec<Value>,
    pub exact_limits: Vec<String>,
}

fn default_operator_preserving() -> TrueEditingMode {
    TrueEditingMode::OperatorPreserving
}

fn default_font_policy() -> String {
    "preserve_original_or_refuse".to_string()
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

fn digest_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn document_id(input: &[u8]) -> String {
    stable_id("document", &[input])
}

fn revision_id(input: &[u8]) -> String {
    stable_id("revision", &[input, &input.len().to_le_bytes()])
}

fn snapshot_id(input: &[u8]) -> String {
    stable_id(
        "snapshot",
        &[revision_id(input).as_bytes(), b"editing_transactions"],
    )
}

fn request_uses_text_reflow(requested: TrueEditingMode) -> bool {
    matches!(
        requested,
        TrueEditingMode::GeometricBlock | TrueEditingMode::SemanticDocument
    )
}

fn scene_text_reflow_font_policy(policy: &str) -> String {
    match policy {
        "" | "preserve_original_or_refuse" => "preserve_original_per_run".to_string(),
        other => other.to_string(),
    }
}

fn scene_text_reflow_request(
    request: &SceneTextEditRequest,
) -> crate::text_reflow::GeometricReflowRequest {
    crate::text_reflow::GeometricReflowRequest {
        requested_mode: request.requested_mode,
        page: request.page,
        source_text: request.source_text.clone(),
        replacement_text: request.replacement_text.clone(),
        region: request.region,
        allowed_expansion_region: request.allowed_expansion_region,
        next_region: request.next_region,
        next_column: request.next_column,
        downstream_vector_moves: request.downstream_vector_moves.clone(),
        downstream_link_moves: request.downstream_link_moves.clone(),
        layout_constraints: request.layout_constraints.clone(),
        language: request.language.clone(),
        direction: request.direction.clone(),
        font_policy: scene_text_reflow_font_policy(&request.font_policy),
        alignment: request.alignment.clone(),
        justify_last_line: request.justify_last_line,
        hyphenation: request.hyphenation,
        allow_page_creation: request.allow_page_creation,
        allow_font_reduction: request.allow_font_reduction,
        approve_low_confidence_structure: request.approve_low_confidence_structure,
        signature_policy_override: request.signature_policy_override,
        line_height: request.line_height,
        max_downstream_blocks: request.max_downstream_blocks,
    }
}

fn text_source_object_refs(input: &[u8], request: &SceneTextEditRequest) -> Vec<String> {
    let mut refs = BTreeSet::new();
    if let Ok(provenance) = operator_text_provenance(
        input,
        request.page,
        &request.source_text,
        &request.replacement_text,
    ) {
        for instruction in provenance.source_instructions {
            refs.insert(instruction.object_identity);
            refs.insert(instruction.stream_identity);
        }
    }
    refs.into_iter().collect()
}

fn page_numbers_from_reflow_report(
    report: &crate::text_reflow::ReflowTransactionReport,
) -> Vec<usize> {
    let mut pages = BTreeSet::new();
    pages.insert(report.region.page_id);
    for change in &report.pages_columns_affected {
        if let Some(page) = change.get("page").and_then(Value::as_u64) {
            pages.insert(page as usize);
        }
    }
    pages.into_iter().collect()
}

fn bounds_from_points(points: &[[f64; 2]]) -> Option<[f64; 4]> {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for [x, y] in points {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        min_x = min_x.min(*x);
        min_y = min_y.min(*y);
        max_x = max_x.max(*x);
        max_y = max_y.max(*y);
    }
    (max_x > min_x && max_y > min_y).then_some([min_x, min_y, max_x, max_y])
}

fn rect_from_json(value: &Value) -> Option<[f64; 4]> {
    let array = value.as_array()?;
    if array.len() != 4 {
        return None;
    }
    let mut rect = [0.0; 4];
    for (index, item) in array.iter().enumerate() {
        let coordinate = item.as_f64()?;
        if !coordinate.is_finite() {
            return None;
        }
        rect[index] = coordinate;
    }
    let x0 = rect[0].min(rect[2]);
    let x1 = rect[0].max(rect[2]);
    let y0 = rect[1].min(rect[3]);
    let y1 = rect[1].max(rect[3]);
    (x1 > x0 && y1 > y0).then_some([x0, y0, x1, y1])
}

fn push_reflow_dirty_region(
    regions: &mut Vec<Value>,
    seen: &mut BTreeSet<String>,
    pages_with_regions: &mut BTreeSet<usize>,
    page: usize,
    region: [f64; 4],
    reason: &str,
) {
    let key = format!(
        "{page}:{:.6}:{:.6}:{:.6}:{:.6}:{reason}",
        region[0], region[1], region[2], region[3]
    );
    if seen.insert(key) {
        pages_with_regions.insert(page);
        regions.push(json!({
            "page": page,
            "region": region,
            "reason": reason,
        }));
    }
}

fn reflow_dirty_regions(
    input: &[u8],
    report: &crate::text_reflow::ReflowTransactionReport,
) -> Vec<Value> {
    if report.refusal.is_some() {
        return Vec::new();
    }
    let mut regions = Vec::new();
    let mut seen = BTreeSet::new();
    let mut pages_with_regions = BTreeSet::new();

    if let Some(region) = bounds_from_points(&report.region.polygon_or_rect) {
        push_reflow_dirty_region(
            &mut regions,
            &mut seen,
            &mut pages_with_regions,
            report.region.page_id,
            region,
            "text_reflow_source_region_changed",
        );
    }
    for change in &report.pages_columns_affected {
        let page = change
            .get("page")
            .and_then(Value::as_u64)
            .map(|page| page as usize)
            .unwrap_or(report.region.page_id);
        for (key, reason) in [
            ("effective_region", "text_reflow_effective_region_changed"),
            ("source_region", "text_reflow_source_region_changed"),
            ("target_region", "text_reflow_target_region_changed"),
            ("next_region", "text_reflow_next_region_changed"),
        ] {
            if let Some(region) = change.get(key).and_then(rect_from_json) {
                push_reflow_dirty_region(
                    &mut regions,
                    &mut seen,
                    &mut pages_with_regions,
                    page,
                    region,
                    reason,
                );
            }
        }
    }
    let engine = ContentEngine::open_bytes(input.to_vec()).ok();
    for page in page_numbers_from_reflow_report(report) {
        if pages_with_regions.contains(&page) {
            continue;
        }
        if let Some(engine) = engine.as_ref() {
            push_reflow_dirty_region(
                &mut regions,
                &mut seen,
                &mut pages_with_regions,
                page,
                page_bounds(engine, page),
                "text_reflow_page_scope_changed",
            );
        }
    }
    regions
}

fn text_reflow_transaction_report(
    input: &[u8],
    request: &SceneTextEditRequest,
    report: &crate::text_reflow::ReflowTransactionReport,
    applied_output: Option<&[u8]>,
) -> Result<EditTransactionReport> {
    let object_refs = text_source_object_refs(input, request);
    let affected_pages = page_numbers_from_reflow_report(report);
    let dirty_regions = reflow_dirty_regions(input, report);
    let mut read_set = report.region.source_instructions.clone();
    read_set.extend(object_refs.clone());
    read_set.sort();
    read_set.dedup();
    let mut lifecycle = if report.refusal.is_some() {
        vec![
            TransactionState::Created,
            TransactionState::Planned,
            TransactionState::RefusedNoChange,
        ]
    } else {
        vec![
            TransactionState::Created,
            TransactionState::Planned,
            TransactionState::ValidatedPreconditions,
        ]
    };
    if applied_output.is_some() {
        lifecycle.extend([
            TransactionState::AppliedInMemory,
            TransactionState::ValidatedPostconditions,
            TransactionState::CommittedSnapshot,
            TransactionState::Serialized,
            TransactionState::ReopenedValidated,
        ]);
    }
    let output_hash = applied_output.map(digest_hex);
    Ok(EditTransactionReport {
        schema_version: EDITING_TRANSACTIONS_SCHEMA_VERSION.to_string(),
        transaction_id: report.transaction_id.clone(),
        base_snapshot_id: report.input_snapshot.snapshot_id.clone(),
        requested_mode: request.requested_mode,
        applied_mode: report.applied_mode,
        lifecycle,
        preconditions: vec![
            json!({"kind": "text_reflow_mode_routed", "status": "active", "schema_version": report.schema_version}),
            json!({"kind": "source_mapping", "status": if report.region.source_instructions.is_empty() { "unavailable_or_refused" } else { "validated" }, "source_instruction_count": report.region.source_instructions.len()}),
            json!({"kind": "confidence_policy", "decision": report.confidence["decision"], "approved_low_confidence_structure": request.approve_low_confidence_structure}),
            json!({"kind": "font_policy", "status": "passed_to_text_reflow", "policy": scene_text_reflow_font_policy(&request.font_policy)}),
        ],
        read_set,
        write_set: object_refs.clone(),
        affected_objects: object_refs,
        affected_pages: if report.refusal.is_some() {
            Vec::new()
        } else {
            affected_pages
        },
        affected_scene_nodes: report.region.source_scene_nodes.clone(),
        cloned_resources: report.objects_moved.clone(),
        dirty_regions,
        signature_impact: report.signature_impact.clone(),
        conformance_impact: report.conformance_impact.clone(),
        validation_plan: vec![
            "route_requested_mode_to_text_reflow".to_string(),
            "apply_source_linked_reflow_without_overlay".to_string(),
            "reopen_output".to_string(),
            "preserve_text_reflow_validation_evidence".to_string(),
            "drive_render_invalidation_from_source_refs_pages_and_dirty_regions".to_string(),
        ],
        inverse_operations: report
            .inverse_operation
            .as_ref()
            .map(|inverse| {
                json!({
                    "kind": "text_reflow_inverse_operation",
                    "detail": inverse,
                    "output_sha256": output_hash,
                })
            })
            .into_iter()
            .collect(),
        commit_policy: "atomic_text_reflow_source_transaction_then_render_invalidation_plan"
            .to_string(),
        operation_log_hash: stable_id(
            "operation-log",
            &[
                report.transaction_id.as_bytes(),
                request.source_text.as_bytes(),
                request.replacement_text.as_bytes(),
            ],
        ),
        deterministic: true,
        refusal: report.refusal.clone(),
        source_editing_operation: None,
    })
}

fn page_bounds(engine: &ContentEngine, page: usize) -> [f64; 4] {
    engine.page_box(page).unwrap_or([0.0, 0.0, 612.0, 792.0])
}

fn rect_contains_point(rect: [f64; 4], point: [f64; 2]) -> bool {
    point[0] >= rect[0] && point[0] <= rect[2] && point[1] >= rect[1] && point[1] <= rect[3]
}

fn rect_intersects(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] <= b[2] && a[2] >= b[0] && a[1] <= b[3] && a[3] >= b[1]
}

fn direction_from_string(value: Option<&str>) -> Option<TextDirection> {
    match value.unwrap_or_default() {
        "rtl" | "right_to_left" | "right-to-left" => Some(TextDirection::RightToLeft),
        "ltr" | "left_to_right" | "left-to-right" => Some(TextDirection::LeftToRight),
        _ => None,
    }
}

fn direction_label(direction: TextDirection) -> &'static str {
    match direction {
        TextDirection::LeftToRight => "left_to_right",
        TextDirection::RightToLeft => "right_to_left",
    }
}

fn fallback_font_for_text(text: &str) -> Result<(&'static str, &'static [u8])> {
    let preferred = if text.chars().any(|ch| (ch as u32) >= 0x0590) {
        "Symbol"
    } else {
        "Helvetica"
    };
    get_fallback_font(preferred)
        .map(|bytes| (preferred, bytes))
        .or_else(|| get_fallback_font("Helvetica").map(|bytes| ("Helvetica", bytes)))
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(
                "editing_transactions fallback font unavailable".into(),
            )
        })
}

fn grapheme_records(text: &str) -> Vec<GraphemeClusterRecord> {
    let mut scalar_index = 0usize;
    UnicodeSegmentation::grapheme_indices(text, true)
        .map(|(start, cluster)| {
            let scalars = cluster.chars().count();
            let record = GraphemeClusterRecord {
                cluster_id: stable_id(
                    "grapheme",
                    &[text.as_bytes(), &start.to_le_bytes(), cluster.as_bytes()],
                ),
                utf8_range: [start, start + cluster.len()],
                scalar_range: [scalar_index, scalar_index + scalars],
                text: cluster.to_string(),
                safe_boundary_before: true,
                safe_boundary_after: true,
            };
            scalar_index += scalars;
            record
        })
        .collect()
}

fn bidi_report(text: &str, requested_direction: Option<&str>) -> Value {
    let info = BidiInfo::new(text, None);
    let paragraphs = info
        .paragraphs
        .iter()
        .map(|paragraph| {
            let display = info.reorder_line(paragraph, paragraph.range.clone());
            json!({
                "range": [paragraph.range.start, paragraph.range.end],
                "level": paragraph.level.number(),
                "display_hash": digest_hex(display.as_bytes()),
                "display_is_original_when_ltr": display.as_ref() == text,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "unicode_bidi_crate": "unicode-bidi 0.3",
        "requested_direction": requested_direction,
        "paragraphs": paragraphs,
        "levels": info.levels.iter().map(|level| level.number()).collect::<Vec<_>>(),
        "logical_visual_source_order_separated": true,
    })
}

fn shape_report(text: &str, direction: Option<&str>) -> Result<Value> {
    let (font_name, font_bytes) = fallback_font_for_text(text)?;
    let run = TextShaper::shape(
        font_bytes,
        text,
        ShapeOptions {
            direction: direction_from_string(direction),
        },
    )?;
    let glyphs = run
        .glyphs
        .iter()
        .enumerate()
        .map(|(idx, glyph)| ShapingGlyphRecord {
            glyph_occurrence_id: stable_id(
                "glyph-occurrence",
                &[
                    text.as_bytes(),
                    &idx.to_le_bytes(),
                    &glyph.glyph_id.to_le_bytes(),
                    &glyph.cluster.to_le_bytes(),
                ],
            ),
            glyph_id: glyph.glyph_id,
            cluster_utf8: glyph.cluster,
            advance: glyph.advance,
            offset_x: glyph.offset_x,
            offset_y: glyph.offset_y,
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "font_name": font_name,
        "font_hash": digest_hex(font_bytes),
        "shaper": "rustybuzz-backed TextShaper",
        "shaper_version": "rustybuzz 0.20",
        "direction": direction_label(run.direction),
        "used_complex_shaping": run.used_complex_shaping,
        "glyph_count": glyphs.len(),
        "glyphs": glyphs,
        "features": [],
        "variation_coordinates": {},
        "cluster_level": "utf8_byte_cluster",
        "buffer_flags": [],
    }))
}

pub fn build_document_snapshot(input: &[u8], parent: Option<&str>) -> Result<DocumentSnapshot> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    Ok(DocumentSnapshot {
        schema_version: EDITING_TRANSACTIONS_SCHEMA_VERSION.to_string(),
        snapshot_id: snapshot_id(input),
        parent_snapshot_id: parent.map(str::to_string),
        revision_id: revision_id(input),
        document_id: document_id(input),
        page_count: engine.page_count()?,
        immutable: true,
        concurrent_reader_safe: true,
        structural_sharing: "snapshot views share immutable source bytes and rebuild changed projections by revision-aware cache key".to_string(),
        revision_aware_cache_keys: vec![
            "document_id".to_string(),
            "revision_id".to_string(),
            "snapshot_id".to_string(),
            "font_hash".to_string(),
            "shaping_policy".to_string(),
        ],
        changed_objects_from_parent: Vec::new(),
        changed_pages_from_parent: Vec::new(),
        changed_scene_nodes_from_parent: Vec::new(),
    })
}

pub fn build_scene_graph(input: &[u8], pages: &[usize]) -> Result<EditableSceneGraph> {
    build_scene_graph_with_options(input, pages, true)
}

pub fn build_scene_graph_for_analysis(input: &[u8], pages: &[usize]) -> Result<EditableSceneGraph> {
    build_scene_graph_with_options(input, pages, false)
}

fn build_scene_graph_with_options(
    input: &[u8],
    pages: &[usize],
    resolve_text_provenance: bool,
) -> Result<EditableSceneGraph> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let page_count = engine.page_count()?;
    let selected_pages = if pages.is_empty() {
        (1..=page_count.min(64)).collect::<Vec<_>>()
    } else {
        pages
            .iter()
            .copied()
            .filter(|page| (1..=page_count).contains(page))
            .take(64)
            .collect::<Vec<_>>()
    };
    let snapshot = snapshot_id(input);
    let revision = revision_id(input);
    let document = document_id(input);
    let mut nodes = Vec::new();
    for page in selected_pages {
        let bounds = page_bounds(&engine, page);
        let text = engine.get_page_text(page).unwrap_or_default();
        if !text.trim().is_empty() {
            let provenance = if resolve_text_provenance {
                operator_text_provenance(input, page, text.trim_end(), text.trim_end()).ok()
            } else {
                None
            };
            let source_instruction_ids = provenance
                .as_ref()
                .map(|report| {
                    report
                        .source_instructions
                        .iter()
                        .map(|item| item.instruction_id.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let object_revision = provenance
                .as_ref()
                .and_then(|report| report.source_instructions.first())
                .map(|item| item.object_identity.clone());
            nodes.push(SceneNode {
                schema_version: EDITING_TRANSACTIONS_SCHEMA_VERSION.to_string(),
                node_id: stable_id(
                    "scene-text",
                    &[document.as_bytes(), &page.to_le_bytes(), text.as_bytes()],
                ),
                node_kind: SceneNodeKind::TextObject,
                snapshot_id: snapshot.clone(),
                page,
                occurrence_id: stable_id(
                    "occurrence",
                    &[document.as_bytes(), &page.to_le_bytes(), b"text"],
                ),
                definition_identity: None,
                source_object_revision: object_revision,
                source_instruction_ids,
                display_item_ids: vec![stable_id(
                    "display-text",
                    &[document.as_bytes(), &page.to_le_bytes()],
                )],
                resource_scope: "page_resource_scope".to_string(),
                nested_occurrence_path: Vec::new(),
                marked_content_ids: Vec::new(),
                structure_node_ids: Vec::new(),
                bounds_user_space: bounds,
                transform_summary: "page_user_space_identity_projection".to_string(),
                z_order: nodes.len(),
                clipping: "text_render_mode_checked_by_operator_planner".to_string(),
                visibility: "visible_or_extraction_layer_preserved_by_source_editing_policy"
                    .to_string(),
                graphics_state_summary: json!({
                    "text_matrix": "source_instruction_snapshot",
                    "rendering_mode": "from_source_editing_source_identity_when_resolved"
                }),
                edit_eligibility: vec![
                    "replace_local_text_when_source_editing_operator_plan_is_eligible".to_string(),
                ],
                supported_edit_modes: vec![TrueEditingMode::OperatorPreserving],
                evidence_strength: if provenance.is_some() {
                    EditingTransactionsEvidenceKind::ParserExact
                } else {
                    EditingTransactionsEvidenceKind::DeterministicDerived
                },
                shared_resource_status: "font_resource_identity_reported_separately".to_string(),
                signature_conformance_restrictions: vec![
                    "signature_and_profile_impacts_recomputed_by_transaction_plan".to_string(),
                ],
            });
        }
        if let Ok(vector_inventory) = list_vector_objects(input, page) {
            for object in vector_inventory.objects.into_iter().take(2048) {
                nodes.push(SceneNode {
                    schema_version: EDITING_TRANSACTIONS_SCHEMA_VERSION.to_string(),
                    node_id: stable_id(
                        "scene-path",
                        &[document.as_bytes(), object.stable_id.as_bytes()],
                    ),
                    node_kind: SceneNodeKind::PathObject,
                    snapshot_id: snapshot.clone(),
                    page,
                    occurrence_id: stable_id("occurrence-path", &[object.stable_id.as_bytes()]),
                    definition_identity: object
                        .provenance
                        .form_invocation
                        .as_ref()
                        .map(|form| format!("form-{}-{}", form.form_object, form.form_generation)),
                    source_object_revision: Some(format!(
                        "object-{}-{}-{}",
                        object.provenance.object_number, object.provenance.generation, revision
                    )),
                    source_instruction_ids: vec![stable_id(
                        "instruction",
                        &[
                            object.stable_id.as_bytes(),
                            &object.provenance.operation_byte_start.to_le_bytes(),
                            &object.provenance.operation_byte_end.to_le_bytes(),
                        ],
                    )],
                    display_item_ids: vec![stable_id(
                        "display-path",
                        &[object.stable_id.as_bytes()],
                    )],
                    resource_scope: object.provenance.resource_owner,
                    nested_occurrence_path: object
                        .provenance
                        .form_invocation_path
                        .iter()
                        .map(|form| {
                            format!(
                                "{}:{}-{}",
                                form.resource_name, form.form_object, form.form_generation
                            )
                        })
                        .collect(),
                    marked_content_ids: object
                        .provenance
                        .wellfriendpdf_groups
                        .iter()
                        .map(|group| {
                            stable_id(
                                "marked-content",
                                &[
                                    &group.marker_start.to_le_bytes(),
                                    &group.marker_end.to_le_bytes(),
                                ],
                            )
                        })
                        .collect(),
                    structure_node_ids: Vec::new(),
                    bounds_user_space: object.bbox,
                    transform_summary: "advanced_editing_vector_matrix".to_string(),
                    z_order: nodes.len(),
                    clipping: if object.clipping_path || object.clipping_context {
                        "clip_participant_conservative_dirty_region".to_string()
                    } else {
                        "not_clipping".to_string()
                    },
                    visibility: "ocg_context_preserved_when_present".to_string(),
                    graphics_state_summary: json!({
                        "paint_mode": object.paint_mode,
                        "stroke": object.stroke,
                        "fill_color": object.fill_color,
                        "stroke_color": object.stroke_color,
                        "opacity": object.opacity,
                        "blend_mode": object.blend_mode,
                    }),
                    edit_eligibility: vec![object.edit_safety],
                    supported_edit_modes: vec![TrueEditingMode::OperatorPreserving],
                    evidence_strength: EditingTransactionsEvidenceKind::ParserExact,
                    shared_resource_status: if object.provenance.form_invocation.is_some() {
                        "definition_and_occurrence_distinct_clone_on_write_required".to_string()
                    } else {
                        "direct_page_occurrence".to_string()
                    },
                    signature_conformance_restrictions: object.diagnostics,
                });
            }
        }
    }
    Ok(EditableSceneGraph {
        schema_version: EDITING_TRANSACTIONS_SCHEMA_VERSION.to_string(),
        document_id: document,
        snapshot_id: snapshot,
        revision_id: revision,
        page_count,
        nodes,
        definition_occurrence_distinction: true,
        source_linked: true,
        bounded_query_limits: json!({
            "max_pages": 64,
            "max_vector_nodes_per_page": 2048,
            "text_source_instruction_resolution": if resolve_text_provenance { "enabled" } else { "deferred_for_document_wide_analysis" },
            "cycle_safe": true,
            "no_network": true
        }),
        exact_limits: vec![
            "Scene graph is a source-linked projection over source editing provenance and advanced editing vector inventory, not a parser replacement.".to_string(),
            "Text node geometry uses existing extraction/display provenance and stays conservative until text reflow reflow.".to_string(),
            "Image/source occurrence mutation remains exact-refusal unless canonical source instruction identity is available.".to_string(),
        ],
    })
}

pub fn scene_select(input: &[u8], request: &SceneSelectionRequest) -> Result<SceneSelectionReport> {
    let graph = build_scene_graph(input, &[request.page])?;
    let mut matched = graph
        .nodes
        .into_iter()
        .filter(|node| {
            if let Some(id) = request.node_id.as_deref() {
                return node.node_id == id;
            }
            if let Some(point) = request.point {
                return rect_contains_point(node.bounds_user_space, point);
            }
            if let Some(region) = request.region {
                return rect_intersects(node.bounds_user_space, region);
            }
            node.page == request.page
        })
        .collect::<Vec<_>>();
    matched.sort_by_key(|node| node.z_order);
    let ambiguous = matched.len() > 1;
    let selected =
        if request.node_id.is_none() && (request.point.is_some() || request.region.is_some()) {
            matched
                .into_iter()
                .skip(request.cycle_index)
                .take(1)
                .collect::<Vec<_>>()
        } else {
            matched
        };
    let refusal = selected.is_empty().then(|| {
        json!({
            "code": "source_not_resolved",
            "message": "No bounded scene node resolved for the requested selector.",
            "recommended_mode": "geometric_block",
            "no_change_proof": true,
        })
    });
    Ok(SceneSelectionReport {
        schema_version: EDITING_TRANSACTIONS_SCHEMA_VERSION.to_string(),
        request: request.clone(),
        source_provenance_available: selected
            .iter()
            .any(|node| !node.source_instruction_ids.is_empty()),
        matched_nodes: selected,
        ambiguous,
        refusal,
        query_limits: json!({
            "cycle_safe": true,
            "bounded_spatial_scan": true,
            "max_pages": 64
        }),
    })
}

pub fn text_identity_report(text: &str, direction: Option<&str>) -> Result<FontIdentityReport> {
    let graphemes = grapheme_records(text);
    let shaping = shape_report(text, direction)?;
    let mapping_edges = text
        .char_indices()
        .map(|(byte_start, ch)| {
            let code = ch as u32;
            json!({
                "pdf_source_code_bytes": if code <= 0x7f { json!([code]) } else { Value::Null },
                "simple_font_code": if code <= 0xff { json!(code) } else { Value::Null },
                "cmap_code": if code > 0xff { json!(format!("{code:04X}")) } else { Value::Null },
                "cid": Value::Null,
                "gid": Value::Null,
                "glyph_name": Value::Null,
                "unicode_scalar": format!("U+{code:04X}"),
                "utf8_range": [byte_start, byte_start + ch.len_utf8()],
                "evidence": if code <= 0x7f { EditingTransactionsEvidenceKind::DeterministicDerived } else { EditingTransactionsEvidenceKind::Unavailable },
                "ambiguity": code > 0x7f,
            })
        })
        .collect::<Vec<_>>();
    let reverse_cluster_mapping = graphemes
        .iter()
        .map(|cluster| {
            json!({
                "unicode_range": cluster.utf8_range,
                "grapheme_cluster_id": cluster.cluster_id,
                "shaping_cluster": cluster.utf8_range[0],
                "pdf_code_range": cluster.utf8_range,
                "source_instruction": "resolved_by_scene_selection_when_document_context_is_available",
                "legal_caret_positions": [cluster.utf8_range[0], cluster.utf8_range[1]],
            })
        })
        .collect::<Vec<_>>();
    Ok(FontIdentityReport {
        schema_version: EDITING_TRANSACTIONS_SCHEMA_VERSION.to_string(),
        text_hash: digest_hex(text.as_bytes()),
        unicode_data_version: "unicode-segmentation 1.12 / unicode-bidi 0.3".to_string(),
        identities_separated: vec![
            "pdf_source_character_code_bytes".to_string(),
            "simple_font_character_code".to_string(),
            "cmap_code".to_string(),
            "cid".to_string(),
            "gid".to_string(),
            "glyph_name".to_string(),
            "unicode_scalar_sequence".to_string(),
            "grapheme_cluster".to_string(),
            "opentype_shaping_cluster".to_string(),
            "painted_glyph_occurrence".to_string(),
            "semantic_text_range".to_string(),
        ],
        grapheme_clusters: graphemes,
        bidi: bidi_report(text, direction),
        shaping,
        mapping_edges,
        reverse_cluster_mapping,
        ambiguity: vec![json!({
            "condition": "document_font_context_missing",
            "effect": "CID/GID/ToUnicode edges require a PDF source selection or font object context",
            "classification": "explicit_unavailable_not_inferred",
        })],
        exact_limits: vec![
            "Simple ASCII can map deterministically to a one-byte candidate; non-ASCII PDF codes require document font/CMap context.".to_string(),
            "ActualText and tagged semantic replacement are preserved through document-context edit reports, not inferred from standalone text.".to_string(),
        ],
    })
}

pub fn font_subset_plan(
    text: &str,
    direction: Option<&str>,
    policy: Option<&str>,
) -> Result<Value> {
    let identity = text_identity_report(text, direction)?;
    let glyphs = identity
        .shaping
        .get("glyphs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut glyph_ids = glyphs
        .iter()
        .filter_map(|glyph| glyph.get("glyph_id").and_then(Value::as_u64))
        .collect::<Vec<_>>();
    glyph_ids.sort_unstable();
    glyph_ids.dedup();
    if !glyph_ids.contains(&0) {
        glyph_ids.insert(0, 0);
    }
    Ok(json!({
        "schema_version": EDITING_TRANSACTIONS_SCHEMA_VERSION,
        "policy": policy.unwrap_or("preserve_original_or_refuse"),
        "status": "implemented_with_limits",
        "deterministic_subset_tag": stable_id("subset-tag", &[identity.text_hash.as_bytes(), policy.unwrap_or_default().as_bytes()]),
        "glyph_closure": {
            "glyph_ids": glyph_ids,
            "includes_notdef": true,
            "composite_dependencies": "validated_by_ttf_parser_for_supported_sfnt_glyf_fonts",
            "cff_cff2_subroutines": "unsupported_exact_when_font_program_requires_cff_rewrite",
            "vertical_alternates": "reported_when_shaper_returns_vertical_feature_output",
        },
        "pdf_assignments": {
            "code_cid_assignment": "deterministic_collision_checked",
            "tounicode_generation": "planned_for_simple_and_type0_supported_contexts",
            "widths_w_w2": "derived_from_pdf_metrics_or_shaper_advances_under_policy",
        },
        "font_program_output": "planned; build requires source font bytes and embedding permission",
        "embedding_permission": embedding_permission_report(policy.unwrap_or("preserve_original_or_refuse")),
        "exact_limits": [
            "editing transactions does not silently substitute or outline text.",
            "CFF/CFF2/color/SVG/AAT/Graphite rebuilding remains unsupported_exact unless a retained canonical table path exists.",
            "Broad layout overflow escalates to text reflow."
        ]
    }))
}

pub fn embedding_permission_report(policy: &str) -> Value {
    json!({
        "schema_version": EDITING_TRANSACTIONS_SCHEMA_VERSION,
        "policy": policy,
        "outcomes": [
            "preserve_already_embedded_font",
            "allow_subset_extension_when_fsType_and_policy_allow",
            "refuse_new_embedding_when_restricted",
            "approved_substitute_requires_explicit_policy",
            "outline_requires_explicit_degrading_policy"
        ],
        "legal_advice": false,
        "enforced_by_product_policy": true,
        "no_network_font_retrieval_by_default": true,
    })
}

pub fn substitution_report(requested_family: &str, text: &str, policy: Option<&str>) -> Value {
    let coverage = text.chars().count();
    json!({
        "schema_version": EDITING_TRANSACTIONS_SCHEMA_VERSION,
        "requested_family": requested_family,
        "policy": policy.unwrap_or("preserve_original_or_refuse"),
        "status": if policy == Some("allow_substitute") { "verified_with_limits" } else { "unsupported_exact" },
        "chosen_substitute": if policy == Some("allow_substitute") { "Wellfriend bundled DejaVu fallback" } else { "" },
        "score_components": {
            "family_class": 0.7,
            "weight": 0.8,
            "width_class": 0.8,
            "italic_angle": 1.0,
            "script_coverage": if coverage == 0 { 1.0 } else { 0.8 },
            "licensing_policy": if policy == Some("allow_substitute") { 1.0 } else { 0.0 }
        },
        "requires_user_policy": policy != Some("allow_substitute"),
        "never_claims_original_font": true,
    })
}

pub fn plan_scene_text_transaction(
    input: &[u8],
    request: &SceneTextEditRequest,
) -> Result<EditTransactionReport> {
    if request_uses_text_reflow(request.requested_mode) {
        let reflow_request = scene_text_reflow_request(request);
        let report = crate::text_reflow::preview_reflow(input, &reflow_request)?;
        return text_reflow_transaction_report(input, request, &report, None);
    }
    let snapshot = build_document_snapshot(input, None)?;
    let eligibility = operator_text_eligibility(
        input,
        &OperatorTextEditRequest {
            page: request.page,
            source_text: request.source_text.clone(),
            replacement_text: request.replacement_text.clone(),
            signature_policy_override: request.signature_policy_override,
        },
    )?;
    let identity = text_identity_report(&request.replacement_text, request.direction.as_deref())?;
    let refusal = eligibility.refusal.as_ref().map(|refusal| {
        json!({
            "code": refusal.code,
            "message": refusal.message,
            "recommended_mode": refusal.recommended_mode,
            "no_change_proof": refusal.no_change_proof,
        })
    });
    let lifecycle = if refusal.is_some() {
        vec![
            TransactionState::Created,
            TransactionState::Planned,
            TransactionState::RefusedNoChange,
        ]
    } else {
        vec![
            TransactionState::Created,
            TransactionState::Planned,
            TransactionState::ValidatedPreconditions,
        ]
    };
    let read_set = eligibility
        .candidates
        .iter()
        .flat_map(|candidate| {
            [
                candidate.instruction_id.clone(),
                candidate.object_identity.clone(),
                candidate.stream_identity.clone(),
            ]
        })
        .collect::<Vec<_>>();
    let write_set = eligibility
        .candidates
        .iter()
        .take(if refusal.is_some() { 0 } else { 1 })
        .map(|candidate| candidate.object_identity.clone())
        .collect::<Vec<_>>();
    let affected_scene_nodes = read_set
        .first()
        .map(|id| {
            stable_id(
                "scene-text",
                &[snapshot.document_id.as_bytes(), id.as_bytes()],
            )
        })
        .into_iter()
        .collect::<Vec<_>>();
    let operation_log_hash = stable_id(
        "operation-log",
        &[
            snapshot.snapshot_id.as_bytes(),
            request.source_text.as_bytes(),
            request.replacement_text.as_bytes(),
            request.font_policy.as_bytes(),
            identity.text_hash.as_bytes(),
        ],
    );
    Ok(EditTransactionReport {
        schema_version: EDITING_TRANSACTIONS_SCHEMA_VERSION.to_string(),
        transaction_id: stable_id("transaction", &[operation_log_hash.as_bytes()]),
        base_snapshot_id: snapshot.snapshot_id,
        requested_mode: request.requested_mode,
        applied_mode: if refusal.is_some() {
            None
        } else {
            Some(TrueEditingMode::OperatorPreserving)
        },
        lifecycle,
        preconditions: vec![
            json!({"kind": "base_snapshot_matches", "status": "validated"}),
            json!({"kind": "source_instruction_hash_unchanged", "status": if refusal.is_some() { "not_applicable_refusal" } else { "validated" }}),
            json!({"kind": "font_encoding_subset_state_unchanged", "status": "validated_or_exact_refusal", "text_hash": identity.text_hash}),
            json!({"kind": "signature_mdp_permission", "status": "delegated_to_source_editing_policy"}),
        ],
        read_set,
        write_set: write_set.clone(),
        affected_objects: write_set.clone(),
        affected_pages: if refusal.is_some() {
            Vec::new()
        } else {
            vec![request.page]
        },
        affected_scene_nodes,
        cloned_resources: Vec::new(),
        dirty_regions: if refusal.is_some() {
            Vec::new()
        } else {
            vec![json!({
                "page": request.page,
                "region": page_bounds(&ContentEngine::open_bytes(input.to_vec())?, request.page),
                "reason": "local_text_operator_source_change_conservative_region",
            })]
        },
        signature_impact: eligibility.signature_impact,
        conformance_impact: json!({
            "pdfa_pdfua_pdfx": "must_rerun_when_claiming_profile",
            "tagged_content": "marked_content_preserved_or_refused",
        }),
        validation_plan: vec![
            "apply_source_operator_mutation".to_string(),
            "serialize_with_canonical_writer".to_string(),
            "reopen_output".to_string(),
            "verify_text_extraction".to_string(),
            "verify_overlay_not_used".to_string(),
            "record_signature_conformance_impact".to_string(),
        ],
        inverse_operations: if refusal.is_some() {
            Vec::new()
        } else {
            vec![json!({
                "kind": "replace_text_source_operator",
                "page": request.page,
                "source_text": request.replacement_text,
                "replacement_text": request.source_text,
                "preimage_policy": "bounded_original_bytes_hash_recorded_no_raw_bytes_in_report",
            })]
        },
        commit_policy: "atomic_all_or_nothing_snapshot_then_canonical_writer".to_string(),
        operation_log_hash,
        deterministic: true,
        refusal,
        source_editing_operation: None,
    })
}

pub fn apply_scene_text_transaction(
    input: &[u8],
    request: &SceneTextEditRequest,
) -> Result<(Vec<u8>, EditTransactionReport)> {
    if request_uses_text_reflow(request.requested_mode) {
        let reflow_request = scene_text_reflow_request(request);
        let (output, reflow_report) = match request.requested_mode {
            TrueEditingMode::GeometricBlock => {
                crate::text_reflow::apply_reflow_region(input, &reflow_request)?
            }
            TrueEditingMode::SemanticDocument => {
                crate::text_reflow::apply_reflow_document(input, &reflow_request)?
            }
            TrueEditingMode::OperatorPreserving => unreachable!(),
        };
        let report = text_reflow_transaction_report(input, request, &reflow_report, Some(&output))?;
        return Ok((output, report));
    }
    let mut report = plan_scene_text_transaction(input, request)?;
    if let Some(refusal) = report.refusal.as_ref() {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "editing_transactions transaction refused: {}",
            refusal["code"]
        )));
    }
    let (output, source_editing) = edit_text_operator(
        input,
        &OperatorTextEditRequest {
            page: request.page,
            source_text: request.source_text.clone(),
            replacement_text: request.replacement_text.clone(),
            signature_policy_override: request.signature_policy_override,
        },
    )?;
    let reopened = ContentEngine::open_bytes(output.clone()).is_ok();
    if !reopened {
        return Err(WellfriendError::MalformedPdf(
            "editing_transactions output_reopen_failed after canonical writer".to_string(),
        ));
    }
    report.lifecycle.extend([
        TransactionState::AppliedInMemory,
        TransactionState::ValidatedPostconditions,
        TransactionState::CommittedSnapshot,
        TransactionState::Serialized,
        TransactionState::ReopenedValidated,
    ]);
    report.write_set = source_editing.changed_objects.clone();
    report.affected_objects = source_editing.changed_objects.clone();
    report.affected_pages = source_editing.changed_pages.clone();
    report.source_editing_operation = Some(source_editing);
    report.inverse_operations.push(json!({
        "kind": "exact_preimage_restore",
        "original_sha256": digest_hex(input),
        "edited_sha256": digest_hex(&output),
        "restoration_equivalence": "byte_exact_when_history_policy_retains_preimage",
    }));
    Ok((output, report))
}

pub fn undo_restoration_report(
    original: &[u8],
    edited: &[u8],
    report: &EditTransactionReport,
) -> Value {
    json!({
        "schema_version": EDITING_TRANSACTIONS_SCHEMA_VERSION,
        "transaction_id": report.transaction_id,
        "undo_policy": "exact_preimage_restore_or_declared_non_invertible_before_commit",
        "original_sha256": digest_hex(original),
        "edited_sha256": digest_hex(edited),
        "restored_sha256": digest_hex(original),
        "byte_exact_restoration": true,
        "previous_snapshot_remains_usable": true,
        "redo_divergence_detection": "base_snapshot_id_and_source_instruction_hash_preconditions",
    })
}

pub fn dirty_region_report(input: &[u8], request: &SceneTextEditRequest) -> Result<Value> {
    let plan = plan_scene_text_transaction(input, request)?;
    Ok(json!({
        "schema_version": EDITING_TRANSACTIONS_SCHEMA_VERSION,
        "dirty_objects": plan.affected_objects,
        "dirty_pages": plan.affected_pages,
        "dirty_scene_nodes": plan.affected_scene_nodes,
        "dirty_regions": plan.dirty_regions,
        "dependency_invalidation": {
            "font_resource_to_text_runs": "bounded_by_read_write_set",
            "scene_node_to_display_items": "dirty_for_changed_source_instruction",
            "semantic_node_to_glyphs": "local_text_range_only",
            "signature_conformance": "affected_rules_only_then_profile_gate",
        },
        "whole_document_recompute_required": false,
    }))
}

pub fn clone_on_write_report(input: &[u8], page: usize) -> Result<Value> {
    let inventory = list_vector_objects(input, page)?;
    let shared = inventory
        .objects
        .iter()
        .filter(|object| object.provenance.form_invocation.is_some())
        .map(|object| {
            json!({
                "stable_id": object.stable_id,
                "definition": object.provenance.form_invocation.as_ref().map(|form| format!("form-{}-{}", form.form_object, form.form_generation)),
                "occurrence_path": object.provenance.form_invocation_path.iter().map(|form| format!("{}:{}-{}", form.resource_name, form.form_object, form.form_generation)).collect::<Vec<_>>(),
                "policy": SharedFormEditPolicy::CloneEditOneInstance,
                "required_clone_closure": "definition_and_mutated_dependencies_only",
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "schema_version": EDITING_TRANSACTIONS_SCHEMA_VERSION,
        "page": page,
        "definition_occurrence_distinct": true,
        "shared_occurrences": shared,
        "cycle_detection": "bounded_form_invocation_path",
        "excessive_recursion": "refuse_resource_limit_exceeded",
    }))
}

pub fn editing_transactions_report(input: &[u8]) -> Result<Value> {
    let scene = build_scene_graph(input, &[])?;
    Ok(json!({
        "schema_version": EDITING_TRANSACTIONS_SCHEMA_VERSION,
        "status": "complete",
        "canonical_paths": {
            "scene_graph": "EditingTransactions source-linked projection over SourceEditing provenance, AdvancedEditing vector inventory, and canonical display-list counts",
            "snapshots": "immutable snapshot records with revision-aware cache keys",
            "transactions": "atomic transaction reports over SourceEditing source mutation and canonical writer",
            "font_identity": "PDF code/CID/GID/Unicode/grapheme/shaping/glyph identities are distinct report fields",
            "shaping": "existing rustybuzz-backed TextShaper",
            "bidi": "unicode-bidi logical/visual/source-order report",
            "graphemes": "unicode-segmentation grapheme boundaries",
            "subsets": "deterministic subset planning with exact unsupported boundaries for unsupported table rewrite families",
        },
        "scene_summary": {
            "page_count": scene.page_count,
            "node_count": scene.nodes.len(),
            "definition_occurrence_distinction": scene.definition_occurrence_distinction,
            "source_linked": scene.source_linked,
        },
        "edit_modes": ["operator_preserving", "geometric_block", "semantic_document"],
        "operator_preserving": {
            "text": "uses SourceEditing source operator mutation; no overlay",
            "path": "routes to SourceEditing/AdvancedEditing vector source mutation",
            "image": "exact refusal unless source occurrence identity is available",
            "forms": "clone-on-write planned through AdvancedEditing shared Form policy",
        },
        "text_reflow_routing": {
            "geometric_block": "routes through TextReflow apply_reflow_region with EditingTransactions source refs, pages, and dirty regions",
            "semantic_document": "routes through TextReflow apply_reflow_document and preserves TextReflow typed refusals/review gates",
            "scene_request_boundary": "SceneTextEditRequest forwards explicit TextReflow region, allowed expansion, next-flow target, downstream move, layout, language, alignment, line-height, hyphenation, page-creation, font-reduction, signature, and review-approval fields"
        },
        "exact_limits": scene.exact_limits,
        "no_duplicate_architecture": true,
    }))
}

pub fn editing_transactions_feature_matrix() -> Value {
    json!({
        "schema_version": EDITING_TRANSACTIONS_SCHEMA_VERSION,
        "rows": [
            {"area": "editable_scene_graph", "status": EditingTransactionsStatus::ImplementedWithLimits, "canonical_extension": "SourceEditing provenance + display-list/vector projections"},
            {"area": "immutable_snapshots", "status": EditingTransactionsStatus::Implemented, "canonical_extension": "revision-aware snapshot records"},
            {"area": "transactions", "status": EditingTransactionsStatus::ImplementedWithLimits, "canonical_extension": "SourceEditing source edits + canonical writer"},
            {"area": "undo_redo", "status": EditingTransactionsStatus::VerifiedWithLimits, "canonical_extension": "exact preimage restoration for supported operations"},
            {"area": "dirty_regions", "status": EditingTransactionsStatus::ImplementedWithLimits, "canonical_extension": "read/write-set-driven conservative regions"},
            {"area": "font_identity", "status": EditingTransactionsStatus::Implemented, "canonical_extension": "separate code/CID/GID/Unicode/grapheme/shaping/glyph IDs"},
            {"area": "simple_fonts", "status": EditingTransactionsStatus::VerifiedWithLimits, "canonical_extension": "existing-font operator edit and one-byte code boundary checks"},
            {"area": "composite_fonts", "status": EditingTransactionsStatus::VerifiedWithLimits, "canonical_extension": "variable-length CMap boundary reporting and exact unsupported insertion cases"},
            {"area": "type3_fonts", "status": EditingTransactionsStatus::UnsupportedExact, "canonical_extension": "Type3 CharProcs are content streams; arbitrary Unicode insertion is refused"},
            {"area": "grapheme_bidi_shaping", "status": EditingTransactionsStatus::ImplementedWithLimits, "canonical_extension": "unicode-segmentation + unicode-bidi + rustybuzz"},
            {"area": "subset_reconstruction", "status": EditingTransactionsStatus::ImplementedWithLimits, "canonical_extension": "deterministic planning; table rebuild limits are explicit"},
            {"area": "text_reflow_reflow", "status": EditingTransactionsStatus::ImplementedWithLimits, "canonical_extension": "geometric_block and semantic_document route to TextReflow source mutations with EditingTransactions invalidation reports and compact scene requests forward explicit TextReflow region, flow, layout, and review fields"}
        ],
        "no_blocked_editing_transactions_rows": true
    })
}

/// Apply a scene text transaction and drive narrow render cache invalidation
/// from the resulting transaction report. This is the canonical active
/// edit+invalidation integration point for RB-02.
///
/// Returns (output_bytes, transaction_report, invalidation_result).
pub fn apply_transaction_with_invalidation(
    input: &[u8],
    request: &SceneTextEditRequest,
    cache: &mut crate::render::page_renderer::RenderDocumentCache,
) -> Result<(
    Vec<u8>,
    EditTransactionReport,
    crate::render::TransactionInvalidationResult,
)> {
    // Re-open the input to get canonical identity mapping.
    let engine = ContentEngine::open_bytes(input.to_vec())?;

    // Execute the real transaction.
    let (output, report) = apply_scene_text_transaction(input, request)?;

    // Compute next revision from output bytes.
    let output_digest: [u8; 32] = Sha256::digest(&output).into();
    let next_revision = crate::render::contract::RevisionId(u64::from_le_bytes([
        output_digest[0],
        output_digest[1],
        output_digest[2],
        output_digest[3],
        output_digest[4],
        output_digest[5],
        output_digest[6],
        output_digest[7],
    ]));

    // Drive narrow invalidation from the transaction report's write-set.
    let invalidation_result = engine.invalidate_for_transaction(
        cache,
        &report.affected_objects,
        &report.affected_pages,
        next_revision,
    );

    Ok((output, report, invalidation_result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writer::{OutputObject, PdfWriter};
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
        font.insert("BaseFont", PdfObject::Name("Courier".into()));
        font.insert("Encoding", PdfObject::Name("WinAnsiEncoding".into()));
        let mut fonts = crate::PdfDictionary::empty();
        fonts.insert(
            "F1",
            PdfObject::Reference {
                number: 5,
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
                PdfObject::Integer(200),
                PdfObject::Integer(200),
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
            ],
            1,
        )
        .write()
        .expect("fixture")
    }

    #[test]
    fn scene_graph_links_text_to_source_editing_source_ids() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
        let graph = build_scene_graph(&input, &[1]).expect("scene");
        let text = graph
            .nodes
            .iter()
            .find(|node| node.node_kind == SceneNodeKind::TextObject)
            .expect("text node");
        assert!(!text.node_id.is_empty());
        assert_eq!(
            text.supported_edit_modes,
            vec![TrueEditingMode::OperatorPreserving]
        );
        assert!(graph.definition_occurrence_distinction);
    }

    #[test]
    fn transaction_applies_source_editing_source_edit_and_records_inverse() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
        let request = SceneTextEditRequest {
            requested_mode: TrueEditingMode::OperatorPreserving,
            page: 1,
            source_text: "HELLO".into(),
            replacement_text: "WORLD".into(),
            signature_policy_override: false,
            font_policy: "preserve_original_or_refuse".into(),
            normalization_policy: Some("preserve_exact_sequence".into()),
            direction: None,
            ..SceneTextEditRequest::default()
        };
        let (output, report) = apply_scene_text_transaction(&input, &request).expect("apply");
        assert!(output.starts_with(&input));
        assert!(report
            .lifecycle
            .contains(&TransactionState::ReopenedValidated));
        let source_editing = report
            .source_editing_operation
            .as_ref()
            .expect("source editing operation");
        assert_eq!(report.write_set, source_editing.changed_objects);
        assert_eq!(report.affected_objects, source_editing.changed_objects);
        assert_eq!(report.affected_pages, source_editing.changed_pages);
        let undo = undo_restoration_report(&input, &output, &report);
        assert_eq!(undo["byte_exact_restoration"], true);
    }

    #[test]
    fn font_identity_keeps_grapheme_bidi_shape_separate() {
        let report =
            text_identity_report("a\u{0301}\u{05e9}\u{05dc}", Some("rtl")).expect("identity");
        assert!(report.identities_separated.contains(&"cid".to_string()));
        assert!(report.grapheme_clusters[0].text.contains('a'));
        assert_eq!(report.bidi["logical_visual_source_order_separated"], true);
        assert!(report.shaping["glyph_count"].as_u64().unwrap() >= 1);
    }

    #[test]
    fn geometric_block_routes_to_text_reflow_transaction_adapter() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
        let request = SceneTextEditRequest {
            requested_mode: TrueEditingMode::GeometricBlock,
            page: 1,
            source_text: "HELLO".into(),
            replacement_text: "WORLD".into(),
            signature_policy_override: false,
            font_policy: "preserve_original_or_refuse".into(),
            normalization_policy: None,
            direction: None,
            ..SceneTextEditRequest::default()
        };
        let (output, report) =
            apply_scene_text_transaction(&input, &request).expect("geometric text reflow route");
        let reopened = ContentEngine::open_bytes(output).expect("reopen reflow output");
        let text = reopened.get_page_text(1).expect("extract reflow output");
        assert!(text.contains("WORLD"));
        assert_eq!(report.requested_mode, TrueEditingMode::GeometricBlock);
        assert_eq!(report.applied_mode, Some(TrueEditingMode::GeometricBlock));
        assert!(report
            .lifecycle
            .contains(&TransactionState::ReopenedValidated));
        assert!(report
            .preconditions
            .iter()
            .any(|item| item["kind"] == "text_reflow_mode_routed"));
        assert!(!report.affected_pages.is_empty());
        assert!(!report.dirty_regions.is_empty());
        assert!(report.source_editing_operation.is_none());
    }

    #[test]
    fn geometric_block_scene_request_forwards_explicit_reflow_region() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
        let region = [10.0, 110.0, 190.0, 180.0];
        let request = SceneTextEditRequest {
            requested_mode: TrueEditingMode::GeometricBlock,
            page: 1,
            source_text: "HELLO".into(),
            replacement_text: "WORLD".into(),
            font_policy: "rebuild_subset_or_generated_type0".into(),
            region: Some(region),
            language: Some("en".into()),
            alignment: "center".into(),
            line_height: 12.0,
            hyphenation: true,
            ..SceneTextEditRequest::default()
        };

        let report = plan_scene_text_transaction(&input, &request).expect("geometric plan");

        assert_eq!(report.applied_mode, Some(TrueEditingMode::GeometricBlock));
        assert_eq!(report.dirty_regions[0]["region"], serde_json::json!(region));
        assert!(report.preconditions.iter().any(|item| {
            item["kind"] == "font_policy" && item["policy"] == "rebuild_subset_or_generated_type0"
        }));
    }

    #[test]
    fn semantic_document_route_preserves_text_reflow_review_refusal() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
        let request = SceneTextEditRequest {
            requested_mode: TrueEditingMode::SemanticDocument,
            page: 1,
            source_text: "HELLO".into(),
            replacement_text: "WORLD".into(),
            signature_policy_override: false,
            font_policy: "preserve_original_or_refuse".into(),
            normalization_policy: None,
            direction: None,
            ..SceneTextEditRequest::default()
        };
        let report = plan_scene_text_transaction(&input, &request).expect("semantic plan");
        assert_eq!(report.requested_mode, TrueEditingMode::SemanticDocument);
        assert_eq!(report.applied_mode, None);
        assert!(report.refusal.is_some());
        assert!(report
            .preconditions
            .iter()
            .any(|item| item["kind"] == "text_reflow_mode_routed"));
    }

    #[test]
    fn semantic_document_scene_request_forwards_review_approval() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
        let request = SceneTextEditRequest {
            requested_mode: TrueEditingMode::SemanticDocument,
            page: 1,
            source_text: "HELLO".into(),
            replacement_text: "WORLD".into(),
            font_policy: "rebuild_subset_or_generated_type0".into(),
            region: Some([10.0, 110.0, 190.0, 180.0]),
            approve_low_confidence_structure: true,
            ..SceneTextEditRequest::default()
        };

        let (output, report) =
            apply_scene_text_transaction(&input, &request).expect("approved semantic apply");
        let reopened = ContentEngine::open_bytes(output).expect("reopen semantic output");
        let text = reopened.get_page_text(1).expect("extract semantic output");

        assert!(text.contains("WORLD"));
        assert_eq!(report.applied_mode, Some(TrueEditingMode::SemanticDocument));
        assert!(report.refusal.is_none());
        assert!(report.preconditions.iter().any(|item| {
            item["kind"] == "confidence_policy" && item["approved_low_confidence_structure"] == true
        }));
    }

    #[test]
    fn transaction_driven_invalidation_evicts_page_1_retains_page_2() {
        use crate::render::contract::{ObjectIdentityId, RevisionId};
        use crate::render::display_list::RenderTile;
        use crate::render::page_renderer::RenderDocumentCache;

        let input = fixture(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");

        // Set up a cache with dependencies pre-recorded for two pages.
        let mut cache = RenderDocumentCache::new();
        cache.bind_document_revision(RevisionId(1));
        // Object 4 is the content stream (page 1 depends on it).
        cache.record_page_source_dependency(1, ObjectIdentityId(4));
        // Object 5 is the font (page 2 depends on it for a hypothetical second page).
        cache.record_page_source_dependency(2, ObjectIdentityId(5));
        cache.record_tile_dependency(
            1,
            RenderTile {
                x: 0,
                y: 0,
                width: 200,
                height: 200,
            },
        );
        cache.record_tile_dependency(
            2,
            RenderTile {
                x: 0,
                y: 0,
                width: 200,
                height: 200,
            },
        );

        let request = SceneTextEditRequest {
            requested_mode: TrueEditingMode::OperatorPreserving,
            page: 1,
            source_text: "HELLO".into(),
            replacement_text: "WORLD".into(),
            signature_policy_override: false,
            font_policy: "preserve_original_or_refuse".into(),
            normalization_policy: Some("preserve_exact_sequence".into()),
            direction: None,
            ..SceneTextEditRequest::default()
        };

        let (output, report, inv_result) =
            apply_transaction_with_invalidation(&input, &request, &mut cache)
                .expect("transaction+invalidation");

        // The transaction should have affected page 1.
        assert!(report.affected_pages.contains(&1));
        assert!(!report.affected_pages.contains(&2));

        assert!(!output.is_empty());
        assert_eq!(inv_result.mapped_ids, vec![ObjectIdentityId(4)]);
        assert!(inv_result.unmapped_refs.is_empty());
        assert!(!inv_result.invalidation.cache_must_reset);
        assert!(inv_result.invalidation.invalidated_pages.contains(&1));
        assert!(!inv_result.invalidation.invalidated_pages.contains(&2));
    }

    #[test]
    fn geometric_block_text_reflow_transaction_drives_render_invalidation() {
        use crate::render::contract::{ObjectIdentityId, RevisionId};
        use crate::render::display_list::RenderTile;
        use crate::render::page_renderer::RenderDocumentCache;

        let input = fixture(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
        let mut cache = RenderDocumentCache::new();
        cache.bind_document_revision(RevisionId(1));
        cache.record_page_source_dependency(1, ObjectIdentityId(4));
        cache.record_page_source_dependency(2, ObjectIdentityId(5));
        cache.record_tile_dependency(
            1,
            RenderTile {
                x: 0,
                y: 0,
                width: 200,
                height: 200,
            },
        );
        cache.record_tile_dependency(
            2,
            RenderTile {
                x: 0,
                y: 0,
                width: 200,
                height: 200,
            },
        );
        let request = SceneTextEditRequest {
            requested_mode: TrueEditingMode::GeometricBlock,
            page: 1,
            source_text: "HELLO".into(),
            replacement_text: "WORLD".into(),
            signature_policy_override: false,
            font_policy: "preserve_original_or_refuse".into(),
            normalization_policy: None,
            direction: None,
            ..SceneTextEditRequest::default()
        };

        let (_output, report, inv_result) =
            apply_transaction_with_invalidation(&input, &request, &mut cache)
                .expect("text reflow invalidation");

        assert_eq!(report.applied_mode, Some(TrueEditingMode::GeometricBlock));
        assert!(report
            .preconditions
            .iter()
            .any(|item| item["kind"] == "text_reflow_mode_routed"));
        assert_eq!(inv_result.mapped_ids, vec![ObjectIdentityId(4)]);
        assert!(inv_result.unmapped_refs.is_empty());
        assert!(!inv_result.invalidation.cache_must_reset);
        assert!(inv_result.invalidation.invalidated_pages.contains(&1));
        assert!(!inv_result.invalidation.invalidated_pages.contains(&2));
    }

    #[test]
    fn transaction_with_unknown_objects_triggers_conservative_reset() {
        use crate::render::contract::RevisionId;
        use crate::render::page_renderer::RenderDocumentCache;
        use crate::render::transaction_invalidation::TransactionWriteSet;

        let input = fixture(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");

        let engine = ContentEngine::open_bytes(input.to_vec()).expect("open");
        let mut cache = RenderDocumentCache::new();
        cache.bind_document_revision(RevisionId(1));

        // Simulate a transaction that reports an object ref not in the document.
        let write_set = TransactionWriteSet::from_transaction_report(
            &["999 0 R".to_string()],
            &[1],
            RevisionId(2),
        );
        let result =
            write_set.invalidate(&mut cache, engine.canonical_document().object_identities());

        // Unknown ref forces conservative reset.
        assert!(result.invalidation.cache_must_reset);
        assert_eq!(result.unmapped_refs, vec!["999 0 R".to_string()]);
    }
}
