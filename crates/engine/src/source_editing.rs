//! source editing canonical provenance and operator-preserving editing adapters.
//!
//! This module deliberately composes the existing advanced editing parser-backed
//! text/vector mutation path.  It does not create a second object graph,
//! renderer, or writer.  The public reports make the source identity,
//! eligibility, refusal, and validation contracts explicit for callers.

use crate::advanced_editing::{
    analyze_multi_run_text_range, analyze_same_width_patch, apply_same_width_patch,
    edit_vector_object, list_vector_objects, SameWidthPatchOptions, VectorEditOperation,
    VectorEditOptions,
};
use crate::{Result, WellfriendError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const SOURCE_EDITING_SCHEMA_VERSION: &str = "source_editing.provenance-operator-editing.v1";

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
    stable_id("document", &[input])
}

fn revision_id(input: &[u8]) -> String {
    stable_id("revision", &[input, &input.len().to_le_bytes()])
}

fn identity_from_candidate(
    input: &[u8],
    candidate: &crate::advanced_editing::SameWidthPatchEligibility,
) -> SourceInstructionIdentity {
    let revision = revision_id(input);
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
        revision_id: revision,
        opcode: candidate.operator.clone(),
        decoded_byte_range: range,
        raw_object_range: crate::ContentEngine::open_bytes(input.to_vec())
            .ok()
            .and_then(|engine| {
                engine
                    .document()
                    .reader()
                    .uncompressed_object_range(candidate.stream_object, candidate.stream_generation)
                    .map(|range| [range.start, range.end])
            }),
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
    let analysis = analyze_same_width_patch(
        input,
        page,
        source_text,
        replacement_text,
        &SameWidthPatchOptions::default(),
    )?;
    let semantic_source_spans = analyze_multi_run_text_range(input, page)
        .map(|model| {
            model
                .source_spans
                .into_iter()
                .map(|span| serde_json::to_value(span).unwrap_or(serde_json::Value::Null))
                .collect()
        })
        .unwrap_or_default();
    Ok(ProvenanceSelectionReport {
        schema_version: SOURCE_EDITING_SCHEMA_VERSION.to_string(),
        document_id: document_id(input),
        revision_id: revision_id(input),
        page,
        source_instructions: analysis
            .candidates
            .iter()
            .map(|candidate| identity_from_candidate(input, candidate))
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
    let analysis = analyze_same_width_patch(
        input,
        request.page,
        &request.source_text,
        &request.replacement_text,
        &SameWidthPatchOptions {
            signature_policy_override: request.signature_policy_override,
            ..SameWidthPatchOptions::default()
        },
    )?;
    let provenance = operator_text_provenance(
        input,
        request.page,
        &request.source_text,
        &request.replacement_text,
    )?;
    let selected = analysis
        .candidates
        .iter()
        .find(|candidate| candidate.eligible);
    let refusal = selected.is_none().then(|| OperatorEditRefusal {
        code: analysis
            .candidates
            .first()
            .map(|candidate| refusal_code(candidate))
            .unwrap_or("source_not_resolved")
            .to_string(),
        message: analysis
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
        signature_impact: serde_json::to_value(analysis.signature_policy)
            .unwrap_or(serde_json::Value::Null),
        refusal,
        exact_limits: analysis.exact_limits,
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
    let eligibility = operator_text_eligibility(input, request)?;
    if let Some(refusal) = eligibility.refusal {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "source_editing {}: {}",
            refusal.code, refusal.message
        )));
    }
    let provenance = operator_text_provenance(
        input,
        request.page,
        &request.source_text,
        &request.replacement_text,
    )?;
    let (output, applied) = apply_same_width_patch(
        input,
        request.page,
        &request.source_text,
        &request.replacement_text,
        &SameWidthPatchOptions {
            signature_policy_override: request.signature_policy_override,
            ..SameWidthPatchOptions::default()
        },
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
            item.object_identity
                .contains(&applied.selected.stream_object.to_string())
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

/// Images have canonical bounded decode/redaction paths, but the current
/// source model does not yet resolve a selected image occurrence to a mutable
/// placement instruction.  Fail closed instead of adding a cover-up.
pub fn operator_image_eligibility(input: &[u8], page: usize) -> serde_json::Value {
    serde_json::json!({
        "schema_version": SOURCE_EDITING_SCHEMA_VERSION,
        "document_id": document_id(input),
        "page": page,
        "requested_mode": TrueEditingMode::OperatorPreserving,
        "eligible_mode": serde_json::Value::Null,
        "refusal": {
            "code": "source_not_resolved",
            "message": "Canonical image decode/redaction is available, but occurrence-to-Do/inline-image source provenance is not yet normalized for mutation.",
            "recommended_mode": TrueEditingMode::GeometricBlock,
            "no_change_proof": true,
        },
        "editing_transactions_owner": "scene occurrence and image source-resolution closure",
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
            "writer": "canonical incremental writer",
            "semantic": "advanced_editing multi-run parser source spans",
        },
        "edit_modes": ["operator_preserving", "geometric_block", "semantic_document"],
        "editing_transactions_deferrals": [
            "stable display-list-to-instruction IDs",
            "image occurrence source mutation",
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
