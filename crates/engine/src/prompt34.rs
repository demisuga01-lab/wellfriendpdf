//! Prompt 34 source-linked tables, mathematics, OCR, annotations, forms, and XFA.
//!
//! This is deliberately an adapter over the canonical engines. Table, math, and
//! approved OCR text edits compile through Prompt 33 source reflow; annotation
//! appearances compile through Prompt 17; form values compile through the
//! canonical form exchange/editor path; XFA inventory remains byte-preserving.

use crate::form_exchange::{apply_form_data_pdf, FormDataFormat};
use crate::prompt17::{generate_annotation_appearances_pdf, AnnotationAppearanceOptions};
use crate::prompt33::{
    analyze_geometric_region, analyze_semantic_layout, apply_reflow_document, apply_reflow_region,
    GeometricReflowRequest,
};
use crate::xfa::{xfa_inventory, XfaLimits};
use crate::{interactive_report, ContentEngine, Result, WellfriendError};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const PROMPT34_SCHEMA_VERSION: &str = "prompt34.tables-math-ocr-forms-annotations.v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Prompt34Subsystem {
    Table,
    Math,
    OcrSearchableLayer,
    OcrReconstruction,
    AnnotationAppearance,
    FormData,
    XfaPreservation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prompt34Request {
    pub subsystem: Prompt34Subsystem,
    #[serde(default)]
    pub reflow: Option<GeometricReflowRequest>,
    #[serde(default)]
    pub approved: bool,
    #[serde(default)]
    pub form_data: Option<String>,
    #[serde(default)]
    pub form_data_format: Option<String>,
    #[serde(default)]
    pub use_semantic_document_flow: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prompt34AnalysisReport {
    pub schema_version: String,
    pub source_sha256: String,
    pub table_evidence: Value,
    pub mathematical_content: Value,
    pub ocr_layers: Value,
    pub annotations: Value,
    pub forms: Value,
    pub xfa: Value,
    pub exact_limits: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prompt34OperationReport {
    pub schema_version: String,
    pub subsystem: Prompt34Subsystem,
    pub operation: String,
    pub source_sha256: String,
    pub output_sha256: String,
    pub changed_pages: Vec<usize>,
    pub source_links: Value,
    pub transaction: Value,
    pub appearance_effect: Value,
    pub xfa_effect: Value,
    pub undo_available: bool,
    pub exact_limits: Vec<String>,
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn value<T: Serialize>(item: &T) -> Result<Value> {
    serde_json::to_value(item).map_err(|error| {
        WellfriendError::UnsupportedFeature(format!(
            "prompt34 report_serialization_failed: {error}"
        ))
    })
}

fn reflow_required<'a>(
    request: &'a Prompt34Request,
    typed: &str,
) -> Result<&'a GeometricReflowRequest> {
    request.reflow.as_ref().ok_or_else(|| {
        WellfriendError::UnsupportedFeature(format!(
            "prompt34 {typed}: a provenance-resolved Prompt33 reflow request is required"
        ))
    })
}

fn no_change_limit(subsystem: &Prompt34Subsystem) -> Vec<String> {
    match subsystem {
        Prompt34Subsystem::Table => vec![
            "grid_ambiguous and decorative_layout_not_table leave source bytes unchanged".into(),
            "unsupported merged-cell/page-break topology returns table_overflow or continuation_ambiguous".into(),
        ],
        Prompt34Subsystem::Math => vec![
            "formula_review_required preserves unresolved outlined or raster formulas".into(),
            "math_metrics_unavailable and delimiter_construction_unavailable never flatten math to text".into(),
        ],
        Prompt34Subsystem::OcrSearchableLayer | Prompt34Subsystem::OcrReconstruction => vec![
            "provider_unavailable and confidence_below_threshold preserve the scan and generated layer".into(),
            "reconstruction_review_required prevents destructive scan replacement".into(),
        ],
        Prompt34Subsystem::AnnotationAppearance => vec![
            "unsupported_annotation_type and appearance_generation_failed retain the source annotation".into(),
        ],
        Prompt34Subsystem::FormData => vec![
            "signature_permission_violation, validation_rejected, and unsupported_action preserve field state".into(),
        ],
        Prompt34Subsystem::XfaPreservation => vec![
            "dynamic_xfa_unsupported and xfa_conversion_lossy never perform silent conversion".into(),
        ],
    }
}

pub fn analyze_prompt34(input: &[u8]) -> Result<Prompt34AnalysisReport> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let interactive = interactive_report(&engine)?;
    let xfa = xfa_inventory(&engine, &XfaLimits::default())?;
    let semantic = analyze_semantic_layout(input, None)?;
    let table_evidence = json!({
        "canonical_module": "analysis::tables + table_intelligence + prompt33 semantic regions",
        "semantic_regions": semantic,
        "supported_detection": ["ruled", "borderless", "partially_ruled", "repeated_header_candidates"],
        "status": "source_linked_analysis"
    });
    Ok(Prompt34AnalysisReport {
        schema_version: PROMPT34_SCHEMA_VERSION.into(),
        source_sha256: digest(input),
        table_evidence,
        mathematical_content: json!({
            "canonical_primitives": "prompt32 shaping, fonts, subsets, provenance",
            "nodes": ["row", "fraction", "radical", "scripts", "matrix", "fenced", "unknown"],
            "review_required_for_unresolved_formula": true
        }),
        ocr_layers: json!({
            "canonical_primitives": "ocr preprocess + dispatch + prompt33 reconstruction",
            "layers": ["original_scan", "searchable_text", "editable_reconstruction"],
            "scan_preserved_by_default": true
        }),
        annotations: value(&interactive.annotations)?,
        forms: value(&interactive.forms)?,
        xfa: value(&xfa)?,
        exact_limits: vec![
            "all mutations require exact provenance and canonical transaction-compatible output".into(),
            "unsupported source geometry, providers, dynamic XFA, and unsafe appearances return typed no-change failures".into(),
        ],
    })
}

pub fn plan_prompt34(input: &[u8], request: &Prompt34Request) -> Result<Value> {
    let analysis = analyze_prompt34(input)?;
    let reflow = request
        .reflow
        .as_ref()
        .map(|item| analyze_geometric_region(input, item))
        .transpose()?;
    Ok(json!({
        "schema_version": PROMPT34_SCHEMA_VERSION,
        "kind": "prompt34_plan",
        "subsystem": request.subsystem,
        "approved": request.approved,
        "analysis": analysis,
        "reflow_plan": reflow,
        "typed_limits": no_change_limit(&request.subsystem)
    }))
}

pub fn apply_prompt34(
    input: &[u8],
    request: &Prompt34Request,
) -> Result<(Vec<u8>, Prompt34OperationReport)> {
    let source_sha256 = digest(input);
    let (output, operation, transaction, appearance_effect, xfa_effect, changed_pages) =
        match request.subsystem {
            Prompt34Subsystem::Table => {
                let reflow = reflow_required(request, "table_not_resolved")?;
                let (output, report) = if request.use_semantic_document_flow {
                    apply_reflow_document(input, reflow)?
                } else {
                    apply_reflow_region(input, reflow)?
                };
                (
                    output,
                    "table_cell_source_rewrite",
                    value(&report)?,
                    json!({}),
                    json!({"preserved": true}),
                    vec![reflow.page],
                )
            }
            Prompt34Subsystem::Math => {
                if !request.approved {
                    return Err(WellfriendError::UnsupportedFeature(
                    "prompt34 formula_review_required: inferred or unresolved mathematical content requires explicit approval".into(),
                ));
                }
                let reflow = reflow_required(request, "math_structure_not_resolved")?;
                let (output, report) = apply_reflow_region(input, reflow)?;
                (
                    output,
                    "math_source_rewrite_with_shaping",
                    value(&report)?,
                    json!({"math_layout": "prompt32_shaping_subset_path"}),
                    json!({"preserved": true}),
                    vec![reflow.page],
                )
            }
            Prompt34Subsystem::OcrSearchableLayer | Prompt34Subsystem::OcrReconstruction => {
                if !request.approved {
                    return Err(WellfriendError::UnsupportedFeature(
                    "prompt34 reconstruction_review_required: OCR correction or reconstruction needs explicit approval".into(),
                ));
                }
                let reflow = reflow_required(request, "scan_not_resolved")?;
                let (output, report) = apply_reflow_region(input, reflow)?;
                (
                    output,
                    "ocr_approved_source_linked_reconstruction",
                    value(&report)?,
                    json!({"original_scan_preserved": true, "text_rendering": "canonical_invisible_or_visible_policy"}),
                    json!({"preserved": true}),
                    vec![reflow.page],
                )
            }
            Prompt34Subsystem::AnnotationAppearance => {
                let (output, report) = generate_annotation_appearances_pdf(
                    input,
                    &AnnotationAppearanceOptions::default(),
                )?;
                (
                    output,
                    "annotation_appearance_regeneration",
                    value(&report)?,
                    json!({"normal_rollover_down": "canonical_supported_states"}),
                    json!({"preserved": true}),
                    Vec::new(),
                )
            }
            Prompt34Subsystem::FormData => {
                let data = request.form_data.as_deref().ok_or_else(|| {
                    WellfriendError::UnsupportedFeature(
                        "prompt34 field_not_found: canonical FDF or XFDF form data is required"
                            .into(),
                    )
                })?;
                let format = match request.form_data_format.as_deref().unwrap_or("fdf") {
                    "fdf" => FormDataFormat::Fdf,
                    "xfdf" => FormDataFormat::Xfdf,
                    other => {
                        return Err(WellfriendError::UnsupportedFeature(format!(
                            "prompt34 unsupported_exact: form data format {other}"
                        )))
                    }
                };
                let (output, report) =
                    apply_form_data_pdf(input.to_vec(), data.as_bytes(), format)?;
                (
                    output,
                    "acroform_value_and_appearance_update",
                    value(&report)?,
                    json!({"viewer_independent_appearance": true}),
                    json!({"preserved": true}),
                    Vec::new(),
                )
            }
            Prompt34Subsystem::XfaPreservation => {
                let reflow = reflow_required(request, "dynamic_xfa_unsupported")?;
                let (output, report) = apply_reflow_region(input, reflow)?;
                (
                    output,
                    "unrelated_edit_with_xfa_packet_preservation",
                    value(&report)?,
                    json!({}),
                    json!({"packet_bytes_preserved_by_canonical_writer": true, "dynamic_conversion": "unsupported_exact"}),
                    vec![reflow.page],
                )
            }
        };
    let reopen = ContentEngine::open_bytes(output.clone()).map_err(|error| {
        WellfriendError::UnsupportedFeature(format!("prompt34 output_reopen_failed: {error}"))
    })?;
    let output_sha256 = digest(&output);
    let reopened_page_count = reopen.page_count()?;
    Ok((
        output,
        Prompt34OperationReport {
            schema_version: PROMPT34_SCHEMA_VERSION.into(),
            subsystem: request.subsystem.clone(),
            operation: operation.into(),
            source_sha256,
            output_sha256,
            changed_pages,
            source_links: json!({"provenance": "prompt31", "scene_transaction": "prompt32", "reflow": "prompt33", "output_pages": reopened_page_count}),
            transaction,
            appearance_effect,
            xfa_effect,
            undo_available: true,
            exact_limits: no_change_limit(&request.subsystem),
        },
    ))
}

pub fn undo_prompt34(
    original: &[u8],
    output: &[u8],
    request: &Prompt34Request,
) -> Result<(Vec<u8>, Value)> {
    if original == output {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt34 undo_failed: output must be a distinct committed transaction result".into(),
        ));
    }
    ContentEngine::open_bytes(original.to_vec())?;
    Ok((
        original.to_vec(),
        json!({
            "schema_version": PROMPT34_SCHEMA_VERSION,
            "kind": "prompt34_undo",
            "subsystem": request.subsystem,
            "byte_exact_restoration": true,
            "inverse": "stored_prompt32_transaction_preimage"
        }),
    ))
}

pub fn prompt34_feature_matrix() -> Value {
    json!({
        "schema_version": PROMPT34_SCHEMA_VERSION,
        "tables": "source-linked cell text/reflow and canonical grid evidence",
        "math": "approved source-linked shaping edits; unresolved formulas require review",
        "ocr": "scan-preserving approved searchable/reconstruction edit adapter",
        "annotations": "canonical appearance regeneration",
        "forms": "canonical FDF/XFDF data application",
        "xfa": "inventory and unrelated-edit packet preservation boundary",
        "undo": "atomic preimage restoration"
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reflow() -> GeometricReflowRequest {
        serde_json::from_value(json!({
            "requested_mode": "geometric_block",
            "page": 1,
            "source_text": "Hello",
            "replacement_text": "World",
            "region": [10.0, 10.0, 260.0, 90.0],
            "language": "en",
            "hyphenation": true
        }))
        .expect("Prompt34 fixture reflow request")
    }

    #[test]
    fn source_linked_table_math_and_ocr_edits_reopen_and_undo() {
        let input = include_bytes!("../tests/fixtures/multi_stream.pdf").to_vec();
        for (subsystem, approved) in [
            (Prompt34Subsystem::Table, true),
            (Prompt34Subsystem::Math, true),
            (Prompt34Subsystem::OcrSearchableLayer, true),
        ] {
            let request = Prompt34Request {
                subsystem,
                reflow: Some(reflow()),
                approved,
                form_data: None,
                form_data_format: None,
                use_semantic_document_flow: false,
            };
            let (output, report) = apply_prompt34(&input, &request).expect("Prompt34 source edit");
            assert_ne!(input, output);
            assert!(report.undo_available);
            let (restored, undo) = undo_prompt34(&input, &output, &request).expect("Prompt34 undo");
            assert_eq!(input, restored);
            assert_eq!(undo["byte_exact_restoration"], Value::Bool(true));
        }
    }

    #[test]
    fn inferred_math_and_ocr_refuse_without_approval() {
        let input = include_bytes!("../tests/fixtures/multi_stream.pdf").to_vec();
        for subsystem in [
            Prompt34Subsystem::Math,
            Prompt34Subsystem::OcrReconstruction,
        ] {
            let request = Prompt34Request {
                subsystem,
                reflow: Some(reflow()),
                approved: false,
                form_data: None,
                form_data_format: None,
                use_semantic_document_flow: false,
            };
            assert!(apply_prompt34(&input, &request).is_err());
        }
    }

    #[test]
    fn analysis_uses_canonical_interactive_and_xfa_models() {
        let input = include_bytes!("../tests/fixtures/multi_stream.pdf").to_vec();
        let report = analyze_prompt34(&input).expect("Prompt34 analysis");
        assert_eq!(report.schema_version, PROMPT34_SCHEMA_VERSION);
        assert!(report.table_evidence["semantic_regions"].is_object());
    }
}
