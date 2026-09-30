//! source editing canonical provenance and operator-preserving editing adapters.
//!
//! This module deliberately composes the existing advanced editing parser-backed
//! text/vector mutation path.  It does not create a second object graph,
//! renderer, or writer.  The public reports make the source identity,
//! eligibility, refusal, and validation contracts explicit for callers.

use crate::advanced_editing::{
    analyze_multi_run_text_range, analyze_same_width_patch, apply_same_width_patch_with_analysis,
    edit_vector_object, list_vector_objects, MultiRunRangeModel, SameWidthPatchEligibilityReport,
    SameWidthPatchOptions, VectorEditOperation, VectorEditOptions,
};
use crate::universal_editing::universal_image_occurrences_v2;
use crate::{Result, WellfriendError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, Instant};

pub const SOURCE_EDITING_SCHEMA_VERSION: &str = "source_editing.provenance-operator-editing.v1";

const SOURCE_ANALYSIS_CACHE_ENTRIES: usize = 32;
const SOURCE_ANALYSIS_CACHE_BYTES: usize = 64 * 1024 * 1024;
const MAX_SOURCE_ANALYSIS_BYTES: usize = 16 * 1024 * 1024;
const SOURCE_ANALYSIS_CACHE_TTL: Duration = Duration::from_secs(10 * 60);

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct SourceAnalysisKey {
    revision_id: String,
    page: usize,
    source_text: String,
    replacement_text: String,
}

#[derive(Clone)]
struct SourceAnalysisArtifact {
    same_width: SameWidthPatchEligibilityReport,
    multi_run: Option<MultiRunRangeModel>,
    estimated_bytes: usize,
    created_at: Instant,
}

#[derive(Default)]
struct SourceAnalysisCache {
    entries: HashMap<SourceAnalysisKey, SourceAnalysisArtifact>,
    order: VecDeque<SourceAnalysisKey>,
    total_bytes: usize,
}

impl SourceAnalysisCache {
    fn get(&self, key: &SourceAnalysisKey) -> Option<SourceAnalysisArtifact> {
        self.entries
            .get(key)
            .filter(|artifact| artifact.created_at.elapsed() <= SOURCE_ANALYSIS_CACHE_TTL)
            .cloned()
    }

    fn insert(&mut self, key: SourceAnalysisKey, mut artifact: SourceAnalysisArtifact) {
        artifact.estimated_bytes = serde_json::to_vec(&artifact.same_width)
            .map_or(0, |bytes| bytes.len())
            .saturating_add(
                artifact
                    .multi_run
                    .as_ref()
                    .and_then(|model| serde_json::to_vec(model).ok())
                    .map_or(0, |bytes| bytes.len()),
            );
        if artifact.estimated_bytes > MAX_SOURCE_ANALYSIS_BYTES {
            return;
        }
        if let Some(previous) = self.entries.remove(&key) {
            self.total_bytes = self.total_bytes.saturating_sub(previous.estimated_bytes);
            self.order.retain(|candidate| candidate != &key);
        }
        while self.entries.len() >= SOURCE_ANALYSIS_CACHE_ENTRIES
            || self.total_bytes.saturating_add(artifact.estimated_bytes)
                > SOURCE_ANALYSIS_CACHE_BYTES
        {
            let Some(victim) = self.order.pop_front() else {
                break;
            };
            if let Some(removed) = self.entries.remove(&victim) {
                self.total_bytes = self.total_bytes.saturating_sub(removed.estimated_bytes);
            }
        }
        self.total_bytes = self.total_bytes.saturating_add(artifact.estimated_bytes);
        self.order.push_back(key.clone());
        self.entries.insert(key, artifact);
    }
}

fn source_analysis_cache() -> &'static RwLock<SourceAnalysisCache> {
    static CACHE: OnceLock<RwLock<SourceAnalysisCache>> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(SourceAnalysisCache::default()))
}

fn source_analysis(
    input: &[u8],
    page: usize,
    source_text: &str,
    replacement_text: &str,
) -> Result<SourceAnalysisArtifact> {
    let key = SourceAnalysisKey {
        revision_id: revision_id(input),
        page,
        source_text: source_text.to_string(),
        replacement_text: replacement_text.to_string(),
    };
    let cached = {
        source_analysis_cache()
            .read()
            .expect("source analysis cache lock poisoned")
            .get(&key)
    };
    if let Some(artifact) = cached {
        return Ok(artifact);
    }
    let same_width = analyze_same_width_patch(
        input,
        page,
        source_text,
        replacement_text,
        &SameWidthPatchOptions::default(),
    )?;
    let multi_run = analyze_multi_run_text_range(input, page).ok();
    let artifact = SourceAnalysisArtifact {
        same_width,
        multi_run,
        estimated_bytes: 0,
        created_at: Instant::now(),
    };
    source_analysis_cache()
        .write()
        .expect("source analysis cache lock poisoned")
        .insert(key, artifact.clone());
    Ok(artifact)
}

pub(crate) fn prepared_multi_run_text_range(
    input: &[u8],
    page: usize,
    source_text: &str,
    replacement_text: &str,
) -> Result<MultiRunRangeModel> {
    if let Some(model) = source_analysis(input, page, source_text, replacement_text)?.multi_run {
        return Ok(model);
    }
    // Provenance reports deliberately tolerate an unavailable semantic model,
    // while callers that require the page-logical range need the exact error.
    analyze_multi_run_text_range(input, page)
}

pub(crate) fn prepared_page_text_model(input: &[u8], page: usize) -> Result<MultiRunRangeModel> {
    let revision = revision_id(input);
    let cached = {
        let cache = source_analysis_cache()
            .read()
            .expect("source analysis cache lock poisoned");
        cache
            .entries
            .iter()
            .find(|(key, artifact)| {
                key.revision_id == revision
                    && key.page == page
                    && artifact.created_at.elapsed() <= SOURCE_ANALYSIS_CACHE_TTL
                    && artifact.multi_run.is_some()
            })
            .and_then(|(_, artifact)| artifact.multi_run.clone())
    };
    cached.map_or_else(|| analyze_multi_run_text_range(input, page), Ok)
}

/// The requested editing contract.  Only [`OperatorPreserving`] is executable
/// in source editing; callers receive an explicit route for later modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrueEditingMode {
    OperatorPreserving,
    GeometricBlock,
    SemanticDocument,
}

impl TrueEditingMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "operator_preserving" | "operator-preserving" => Some(Self::OperatorPreserving),
            "geometric_block" | "geometric-block" => Some(Self::GeometricBlock),
            "semantic_document" | "semantic-document" => Some(Self::SemanticDocument),
            _ => None,
        }
    }
}

/// Strength of a provenance edge.  It prevents semantic or layout inference
/// from being presented as a byte-level parser fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceStrength {
    NormativeExact,
    ParserExact,
    RendererExact,
    DeterministicDerived,
    HeuristicInferred,
    ModelInferred,
    Ambiguous,
    Unavailable,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceInstructionIdentity {
    pub instruction_id: String,
    pub stream_identity: String,
    pub object_identity: String,
    pub revision_id: String,
    pub stream_object: u32,
    pub stream_generation: u16,
    pub opcode: String,
    pub decoded_byte_range: [usize; 2],
    pub raw_object_range: Option<[usize; 2]>,
    pub tj_element: Option<usize>,
    pub font_resource: String,
    pub marked_content_depth: usize,
    pub text_render_mode: i32,
    pub strength: ProvenanceStrength,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProvenanceSelectionReport {
    pub schema_version: String,
    pub document_id: String,
    pub revision_id: String,
    pub page: usize,
    pub source_instructions: Vec<SourceInstructionIdentity>,
    pub semantic_source_spans: Vec<serde_json::Value>,
    pub display_item_mapping: ProvenanceStrength,
    pub display_item_note: String,
    pub resource_occurrence_mapping: ProvenanceStrength,
    pub exact_limits: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorTextEditRequest {
    pub page: usize,
    pub source_text: String,
    pub replacement_text: String,
    #[serde(default)]
    pub source_instruction_id: Option<String>,
    #[serde(default)]
    pub signature_policy_override: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct OperatorEditRefusal {
    pub code: String,
    pub message: String,
    pub recommended_mode: TrueEditingMode,
    pub no_change_proof: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct OperatorTextEligibilityReport {
    pub schema_version: String,
    pub requested_mode: TrueEditingMode,
    pub eligible_mode: Option<TrueEditingMode>,
    pub document_id: String,
    pub revision_id: String,
    pub page: usize,
    pub candidates: Vec<SourceInstructionIdentity>,
    pub signature_impact: serde_json::Value,
    pub refusal: Option<OperatorEditRefusal>,
    pub exact_limits: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OperatorEditOperationReport {
    pub schema_version: String,
    pub operation_id: String,
    pub requested_mode: TrueEditingMode,
    pub applied_mode: TrueEditingMode,
    pub source_selection: ProvenanceSelectionReport,
    pub changed_instructions: Vec<String>,
    pub changed_objects: Vec<String>,
    pub changed_pages: Vec<usize>,
    pub cloned_resources: Vec<String>,
    pub unaffected_content_proof: serde_json::Value,
    pub visual_impact: String,
    pub semantic_impact: String,
    pub signature_impact: serde_json::Value,
    pub conformance_impact: String,
    pub warnings: Vec<String>,
    pub validation: serde_json::Value,
    pub output_revision: String,
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

fn document_id(input: &[u8]) -> String {
    crate::input_identity::document_id(input)
}

fn revision_id(input: &[u8]) -> String {
    crate::input_identity::revision_id(input)
}

fn identity_from_candidate(
    candidate: &crate::advanced_editing::SameWidthPatchEligibility,
    revision: &str,
    reader: &crate::PdfReader,
) -> SourceInstructionIdentity {
    let object = format!(
        "object-{}-{}-{}",
        candidate.stream_object, candidate.stream_generation, revision
    );
    let stream = format!(
        "stream-{}-{}-{}",
        candidate.stream_object, candidate.stream_generation, revision
    );
    let range = [candidate.decoded_byte_start, candidate.decoded_byte_end];
    let instruction = stable_id(
        "instruction",
        &[
            stream.as_bytes(),
            candidate.operator.as_bytes(),
            &candidate.decoded_byte_start.to_le_bytes(),
            &candidate.decoded_byte_end.to_le_bytes(),
        ],
    );
    SourceInstructionIdentity {
        instruction_id: instruction,
        stream_identity: stream,
        object_identity: object,
        revision_id: revision.to_string(),
        stream_object: candidate.stream_object,
        stream_generation: candidate.stream_generation,
        opcode: candidate.operator.clone(),
        decoded_byte_range: range,
        raw_object_range: reader
            .uncompressed_object_range(candidate.stream_object, candidate.stream_generation)
            .map(|range| [range.start, range.end]),
        tj_element: candidate.tj_element,
        font_resource: candidate.font_resource.clone(),
        marked_content_depth: candidate.marked_content_depth,
        text_render_mode: candidate.text_render_mode,
        strength: ProvenanceStrength::ParserExact,
    }
}

/// Resolve parser-backed source instructions for a text selection.  This is a
/// query only: it never paints, mutates, or creates an editable text tree.
pub fn operator_text_provenance(
    input: &[u8],
    page: usize,
    source_text: &str,
    replacement_text: &str,
) -> Result<ProvenanceSelectionReport> {
    crate::input_identity::with_input_identity(input, || {
        operator_text_provenance_inner(input, page, source_text, replacement_text)
    })
}

fn operator_text_provenance_inner(
    input: &[u8],
    page: usize,
    source_text: &str,
    replacement_text: &str,
) -> Result<ProvenanceSelectionReport> {
    let analysis = source_analysis(input, page, source_text, replacement_text)?;
    let semantic_source_spans = analysis
        .multi_run
        .map(|model| {
            model
                .source_spans
                .into_iter()
                .map(|span| serde_json::to_value(span).unwrap_or(serde_json::Value::Null))
                .collect()
        })
        .unwrap_or_default();
    let engine = crate::ContentEngine::open_bytes(input.to_vec())?;
    let revision = revision_id(input);
    Ok(ProvenanceSelectionReport {
        schema_version: SOURCE_EDITING_SCHEMA_VERSION.to_string(),
        document_id: document_id(input),
        revision_id: revision_id(input),
        page,
        source_instructions: analysis
            .same_width
            .candidates
            .iter()
            .map(|candidate| {
                identity_from_candidate(candidate, &revision, engine.document().reader())
            })
            .collect(),
        semantic_source_spans,
        // Existing display lists are canonical rendering operations, but they
        // do not yet carry stable source instruction IDs.  editing transactions owns that
        // renderer-to-scene closure, so this remains an explicit unavailable
        // edge rather than an invented correspondence.
        display_item_mapping: ProvenanceStrength::Unavailable,
        display_item_note: "The canonical display list is reused for rendering; stable display-item-to-instruction IDs are deferred to editing transactions.".to_string(),
        resource_occurrence_mapping: ProvenanceStrength::ParserExact,
        exact_limits: vec![
            "Text provenance is exact for parser-resolved page content string operands only.".to_string(),
            "Compressed/object-stream source objects retain object identity but may not expose a raw lexical object range.".to_string(),
            "Semantic spans are deterministic parser-derived links; paragraph grouping remains a higher-layer inference.".to_string(),
        ],
    })
}

/// Plan an operator-preserving text edit without changing bytes.  Refusal is
/// structured and proves that this planner has not modified the document.
pub fn operator_text_eligibility(
    input: &[u8],
    request: &OperatorTextEditRequest,
) -> Result<OperatorTextEligibilityReport> {
    crate::input_identity::with_input_identity(input, || {
        operator_text_eligibility_inner(input, request)
    })
}

fn operator_text_eligibility_inner(
    input: &[u8],
    request: &OperatorTextEditRequest,
) -> Result<OperatorTextEligibilityReport> {
    let provenance = operator_text_provenance(
        input,
        request.page,
        &request.source_text,
        &request.replacement_text,
    )?;
    let selected_identity = requested_source_instruction(&provenance, request)?;
    let analysis = source_analysis(
        input,
        request.page,
        &request.source_text,
        &request.replacement_text,
    )?;
    let selected = analysis.same_width.candidates.iter().find(|candidate| {
        candidate.eligible
            && selected_identity.is_none_or(|identity| {
                candidate.stream_object == identity.stream_object
                    && candidate.stream_generation == identity.stream_generation
                    && candidate.decoded_byte_start == identity.decoded_byte_range[0]
                    && candidate.decoded_byte_end == identity.decoded_byte_range[1]
            })
    });
    let refusal = selected.is_none().then(|| OperatorEditRefusal {
        code: analysis
            .same_width
            .candidates
            .first()
            .map(|candidate| refusal_code(candidate))
            .unwrap_or("source_not_resolved")
            .to_string(),
        message: analysis
            .same_width
            .candidates
            .first()
            .map(|candidate| candidate.exact_reason.clone())
            .unwrap_or_else(|| {
                "no source text operator resolved for the requested selection".to_string()
            }),
        recommended_mode: TrueEditingMode::GeometricBlock,
        no_change_proof: true,
    });
    Ok(OperatorTextEligibilityReport {
        schema_version: SOURCE_EDITING_SCHEMA_VERSION.to_string(),
        requested_mode: TrueEditingMode::OperatorPreserving,
        eligible_mode: selected.map(|_| TrueEditingMode::OperatorPreserving),
        document_id: document_id(input),
        revision_id: revision_id(input),
        page: request.page,
        candidates: provenance.source_instructions,
        signature_impact: serde_json::to_value(analysis.same_width.signature_policy)
            .unwrap_or(serde_json::Value::Null),
        refusal,
        exact_limits: analysis.same_width.exact_limits,
    })
}

fn refusal_code(candidate: &crate::advanced_editing::SameWidthPatchEligibility) -> &'static str {
    let reason = candidate.exact_reason.as_str();
    if reason.contains("font/CMap") {
        "replacement_not_encodable"
    } else if reason.contains("clipping") {
        "clipping_semantics_unsafe"
    } else if reason.contains("signature") {
        "signature_permission_violation"
    } else if reason.contains("glyph") || reason.contains("advance") {
        "geometric_reflow_required"
    } else if reason.contains("vertical") || reason.contains("bidi") {
        "shaping_reconstruction_required"
    } else {
        "unsupported_operator"
    }
}

/// Apply a minimal source-operator text mutation.  The underlying implementation
/// edits the resolved string operand, writes an incremental revision, reopens
/// it, and verifies text extraction.  It never uses overlay drawing.
pub fn edit_text_operator(
    input: &[u8],
    request: &OperatorTextEditRequest,
) -> Result<(Vec<u8>, OperatorEditOperationReport)> {
    crate::input_identity::with_input_identity(input, || edit_text_operator_inner(input, request))
}

fn edit_text_operator_inner(
    input: &[u8],
    request: &OperatorTextEditRequest,
) -> Result<(Vec<u8>, OperatorEditOperationReport)> {
    let provenance = operator_text_provenance(
        input,
        request.page,
        &request.source_text,
        &request.replacement_text,
    )?;
    let selected_identity = requested_source_instruction(&provenance, request)?;
    let analysis = source_analysis(
        input,
        request.page,
        &request.source_text,
        &request.replacement_text,
    )?;
    let eligible = analysis.same_width.candidates.iter().any(|candidate| {
        candidate.eligible
            && selected_identity.is_none_or(|identity| {
                candidate.stream_object == identity.stream_object
                    && candidate.stream_generation == identity.stream_generation
                    && candidate.decoded_byte_start == identity.decoded_byte_range[0]
                    && candidate.decoded_byte_end == identity.decoded_byte_range[1]
            })
    });
    if !eligible {
        let candidate = analysis.same_width.candidates.first();
        return Err(WellfriendError::UnsupportedFeature(format!(
            "source_editing {}: {}",
            candidate.map(refusal_code).unwrap_or("source_not_resolved"),
            candidate
                .map(|candidate| candidate.exact_reason.as_str())
                .unwrap_or("no source text operator resolved for the requested selection")
        )));
    }
    let (output, applied) = apply_same_width_patch_with_analysis(
        input,
        request.page,
        &request.source_text,
        &request.replacement_text,
        &SameWidthPatchOptions {
            signature_policy_override: request.signature_policy_override,
            target_stream_object: selected_identity.map(|identity| identity.stream_object),
            target_stream_generation: selected_identity.map(|identity| identity.stream_generation),
            target_decoded_byte_range: selected_identity
                .map(|identity| identity.decoded_byte_range),
            ..SameWidthPatchOptions::default()
        },
        analysis.same_width,
    )?;
    if !applied.output_reopened || !applied.replacement_extracts || !applied.old_text_absent {
        return Err(WellfriendError::MalformedPdf(
            "source_editing validation_failed: source operator mutation did not reopen/extract cleanly"
                .to_string(),
        ));
    }
    let changed_instruction = provenance
        .source_instructions
        .iter()
        .find(|item| {
            item.stream_object == applied.selected.stream_object
                && item.stream_generation == applied.selected.stream_generation
                && item.decoded_byte_range
                    == [
                        applied.selected.decoded_byte_start,
                        applied.selected.decoded_byte_end,
                    ]
        })
        .map(|item| item.instruction_id.clone())
        .into_iter()
        .collect::<Vec<_>>();
    let output_revision = revision_id(&output);
    Ok((
        output,
        OperatorEditOperationReport {
            schema_version: SOURCE_EDITING_SCHEMA_VERSION.to_string(),
            operation_id: stable_id(
                "operation",
                &[document_id(input).as_bytes(), output_revision.as_bytes()],
            ),
            requested_mode: TrueEditingMode::OperatorPreserving,
            applied_mode: TrueEditingMode::OperatorPreserving,
            source_selection: provenance,
            changed_instructions: changed_instruction,
            changed_objects: vec![format!(
                "object-{}-{}",
                applied.selected.stream_object, applied.selected.stream_generation
            )],
            changed_pages: vec![request.page],
            cloned_resources: Vec::new(),
            unaffected_content_proof: serde_json::json!({
                "original_pdf_prefix_preserved": applied.original_prefix_preserved,
                "old_source_reachable_in_current_revision": false,
                "replacement_extracts": applied.replacement_extracts,
                "old_text_absent": applied.old_text_absent,
                "overlay_used": false,
            }),
            visual_impact: "local_text_operator".to_string(),
            semantic_impact: "local_text".to_string(),
            signature_impact: serde_json::to_value(applied.signature_policy)
                .unwrap_or(serde_json::Value::Null),
            conformance_impact: "not_revalidated; callers must run the canonical standards validator for claimed profiles".to_string(),
            warnings: vec![
                "Incremental byte-prefix preservation is not a claim that an existing signature remains cryptographically valid.".to_string(),
            ],
            validation: serde_json::json!({
                "output_reopened": applied.output_reopened,
                "replacement_extracts": applied.replacement_extracts,
                "old_text_absent": applied.old_text_absent,
                "canonical_writer": "advanced_editing_incremental_writer",
            }),
            output_revision,
        },
    ))
}

fn requested_source_instruction<'a>(
    provenance: &'a ProvenanceSelectionReport,
    request: &OperatorTextEditRequest,
) -> Result<Option<&'a SourceInstructionIdentity>> {
    let Some(instruction_id) = request.source_instruction_id.as_deref() else {
        return Ok(None);
    };
    provenance
        .source_instructions
        .iter()
        .find(|identity| identity.instruction_id == instruction_id)
        .map(Some)
        .ok_or_else(|| {
            WellfriendError::invalid_input(
                "source_editing selected instruction is stale or outside the current source selection",
            )
        })
}

/// Return the canonical vector/path inventory.  Every object reports source
/// stream range, occurrence path, resource owner, clipping role, and safety.
pub fn operator_path_provenance(input: &[u8], page: usize) -> Result<serde_json::Value> {
    Ok(serde_json::json!({
        "schema_version": SOURCE_EDITING_SCHEMA_VERSION,
        "requested_mode": TrueEditingMode::OperatorPreserving,
        "inventory": list_vector_objects(input, page)?,
    }))
}

/// Apply an existing parser-backed vector/path/graphics-state edit without
/// converting it to an overlay.  Form occurrence policies are explicit.
pub fn edit_path_operator(
    input: &[u8],
    page: usize,
    stable_id: &str,
    operation: VectorEditOperation,
    options: &VectorEditOptions,
) -> Result<(Vec<u8>, serde_json::Value)> {
    let (output, report) = edit_vector_object(input, page, stable_id, operation, options)?;
    Ok((
        output,
        serde_json::json!({
            "schema_version": SOURCE_EDITING_SCHEMA_VERSION,
            "requested_mode": TrueEditingMode::OperatorPreserving,
            "applied_mode": TrueEditingMode::OperatorPreserving,
            "operation_report": report,
            "overlay_used": false,
            "canonical_writer": "advanced_editing_incremental_writer",
        }),
    ))
}

/// Resolve image definitions that can participate in universal-v2 source
/// transactions. Exact occurrence selection and clone-one approval are owned
/// by EditingTransactions/UniversalEditing; this report never substitutes an
/// overlay when source ownership is unavailable.
pub fn operator_image_eligibility(input: &[u8], page: usize) -> serde_json::Value {
    let candidates = universal_image_occurrences_v2(input, &[page])
        .map(|images| {
            images
                .into_iter()
                .map(|image| {
                    serde_json::json!({
                        "occurrence_id": image.occurrence_id,
                        "resource_name": image.resource_name,
                        "object_number": image.object_number,
                        "generation": image.generation,
                        "owner_stream_object": image.owner_stream_object,
                        "owner_stream_generation": image.owner_stream_generation,
                        "source_range": [image.operation_byte_start, image.operation_byte_end],
                        "nested_occurrence_path": image.invocation_path,
                        "bbox": image.bbox,
                        "width": image.width,
                        "height": image.height,
                        "bits_per_component": image.bits_per_component,
                        "color_space": image.color_space,
                        "filters": image.filters,
                        "inline": image.inline,
                        "shared_definition_uses": image.shared_definition_uses,
                        "source_strength": ProvenanceStrength::ParserExact,
                        "eligible_operations": if image.inline {
                            vec!["promote_inline_then_replace"]
                        } else {
                            vec!["replace_definition_edit_all", "clone_occurrence_then_replace"]
                        },
                        "approval_required": image.inline || image.shared_definition_uses > 1,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let refusal = candidates.is_empty().then(|| {
        serde_json::json!({
            "code": "source_not_resolved",
            "message": "No image source definition was resolved on the requested page.",
            "recommended_mode": TrueEditingMode::GeometricBlock,
            "no_change_proof": true,
        })
    });
    serde_json::json!({
        "schema_version": SOURCE_EDITING_SCHEMA_VERSION,
        "document_id": document_id(input),
        "revision_id": revision_id(input),
        "page": page,
        "requested_mode": TrueEditingMode::OperatorPreserving,
        "eligible_mode": (!candidates.is_empty()).then_some(TrueEditingMode::OperatorPreserving),
        "candidates": candidates,
        "refusal": refusal,
        "editing_transactions_owner": "exact occurrence selection, shared-resource policy, and revision-bound approval",
    })
}

pub fn source_editing_report() -> serde_json::Value {
    serde_json::json!({
        "schema_version": SOURCE_EDITING_SCHEMA_VERSION,
        "status": "implemented_with_limits",
        "canonical_paths": {
            "text": "advanced_editing same-width parser-backed stream operand patch",
            "path_and_graphics": "advanced_editing vector source range mutation",
            "forms": "advanced_editing explicit shared Form/appearance clone policy",
            "images": "universal_editing exact recursive image occurrence mutation",
            "writer": "canonical incremental writer",
            "semantic": "advanced_editing multi-run parser source spans",
        },
        "edit_modes": ["operator_preserving", "geometric_block", "semantic_document"],
        "editing_transactions_deferrals": [
            "stable display-list-to-instruction IDs",
            "broader font subset and shaping reconstruction",
        ],
        "text_reflow_deferrals": [
            "geometric block reflow",
            "semantic document reflow",
        ],
        "overlay_policy": "rejected_for_operator_preserving_edits",
    })
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
        font.insert("BaseFont", PdfObject::Name("Helvetica".into()));
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
    fn operator_text_edit_changes_source_instruction_without_overlay() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (ABC) Tj ET\n");
        let request = OperatorTextEditRequest {
            page: 1,
            source_text: "ABC".into(),
            replacement_text: "DEF".into(),
            source_instruction_id: None,
            signature_policy_override: false,
        };
        let plan = operator_text_eligibility(&input, &request).expect("plan");
        assert!(plan.refusal.is_none());
        assert_eq!(plan.candidates[0].opcode, "Tj");
        let (output, report) = edit_text_operator(&input, &request).expect("apply");
        assert!(output.starts_with(&input));
        assert_eq!(report.unaffected_content_proof["overlay_used"], false);
        assert_eq!(
            crate::ContentEngine::open_bytes(output)
                .unwrap()
                .get_page_text(1)
                .unwrap()
                .trim_end(),
            "DEF"
        );
    }

    #[test]
    fn immutable_revision_reuses_prepared_source_analysis() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (ABC) Tj ET\n");
        crate::input_identity::with_input_identity(&input, || {
            let first = source_analysis(&input, 1, "ABC", "DEF").expect("first analysis");
            let second = source_analysis(&input, 1, "ABC", "DEF").expect("cached analysis");
            assert_eq!(first.created_at, second.created_at);
            assert_eq!(
                first.same_width.candidates.len(),
                second.same_width.candidates.len()
            );
        });
    }

    #[test]
    fn revision_scope_reuses_the_immutable_parsed_engine() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (ABC) Tj ET\n");
        crate::input_identity::with_input_identity(&input, || {
            let first = crate::ContentEngine::open_bytes(input.clone()).expect("first open");
            let second = crate::ContentEngine::open_bytes(input.clone()).expect("cached open");
            assert!(first.shares_document_with(&second));
        });
        let outside = crate::ContentEngine::open_bytes(input.clone()).expect("outside open");
        let another = crate::ContentEngine::open_bytes(input).expect("another outside open");
        assert!(!outside.shares_document_with(&another));
    }

    #[test]
    fn quote_and_double_quote_are_resolved_as_source_operators() {
        let input = fixture(b"BT /F1 12 Tf 10 150 Td (ABC) ' 0 0 (DEF) \" ET\n");
        let quote = operator_text_provenance(&input, 1, "ABC", "GHI").expect("quote provenance");
        assert_eq!(quote.source_instructions[0].opcode, "'");
        let double =
            operator_text_provenance(&input, 1, "DEF", "GHI").expect("double quote provenance");
        assert_eq!(double.source_instructions[0].opcode, "\"");
    }

    #[test]
    fn unsupported_image_is_a_typed_no_change_refusal() {
        let value = operator_image_eligibility(b"%PDF-test", 1);
        assert_eq!(value["refusal"]["code"], "source_not_resolved");
        assert_eq!(value["refusal"]["no_change_proof"], true);
    }
}
