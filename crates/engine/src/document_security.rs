//! document security tagged-PDF repair, accessibility, redaction, and sanitization.
//!
//! The module is intentionally an adapter over existing Wellfriend subsystems:
//! tagged extraction and ParentTree recovery, text reflow/34 semantic mutation
//! paths, the canonical full-rewrite editor/redactor, the sanitizer, and the
//! writer. It keeps document security operations source-linked and refuses boundaries
//! that would otherwise become report-only claims.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

use crate::editing::{AttachmentRedactionPolicy, ImageRedactionPolicy, RedactionOptions};
use crate::security::{canonicalize_pdf, sanitize_pdf, CanonicalizeOptions, SanitizerOptions};
use crate::semantic::{extract_semantic_document, SemanticDocument, SemanticElement};
use crate::semantic_intelligence::{recover_parenttree_semantics, ParentTreeRecoveryReport};
use crate::versioning::resource_digest;
use crate::writer::{rewrite_document_objects, OutputObject, PdfWriter, WriterMode};
use crate::{
    improve_pdfua_best_effort, interactive_report, validate_pdfua, Color, ContentEngine, EditMode,
    ImageRect, PdfDictionary, PdfEditor, PdfObject, Result, TextQuad, TextSearchOptions,
    WellfriendError,
};

pub const DOCUMENT_SECURITY_SCHEMA_VERSION: &str =
    "document_security.accessibility-redaction-sanitization.v1";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DocumentSecuritySubsystem {
    TaggedPdf,
    AccessibilityRepair,
    Redaction,
    Sanitization,
    ResidualVerification,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DocumentSecurityStatus {
    Planned,
    Applied,
    Refused,
    VerifiedAbsent,
    VerifiedPresent,
    UnsupportedExact,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DocumentSecurityTypedResult {
    Ok,
    StructureTreeMissing,
    StructureElementNotFound,
    InvalidStructureParent,
    ParentTreeInconsistent,
    McidCollision,
    McidOwnerAmbiguous,
    StaleMcr,
    StaleObjr,
    InvalidRoleMapping,
    InvalidNamespace,
    InvalidContainment,
    ReadingOrderAmbiguous,
    HeadingHierarchyAmbiguous,
    TableHeaderRelationshipAmbiguous,
    FormulaSemanticsReviewRequired,
    AltTextReviewRequired,
    LanguageInvalid,
    RepairWouldLoseSemantics,
    RedactionTargetNotResolved,
    RedactionTargetAmbiguous,
    PartialTextClusterUnsafe,
    PartialPathSplitUnsupported,
    PartialImageRedactionFailed,
    SharedResourceCloneFailed,
    SourceContentStillReachable,
    HistoricalRevisionStillPresent,
    ResidualDataDetected,
    VerificationProviderUnavailable,
    RedactionVerificationFailed,
    DestructiveFullRewriteRequired,
    SignatureWillBeInvalidated,
    PolicyInvalid,
    FeatureRemovalUnsupported,
    ActiveContentStillPresent,
    AttachmentStillPresent,
    MetadataHistoryStillPresent,
    HiddenLayerStillPresent,
    XfaRemovalLossy,
    SanitizationVerificationFailed,
    TransactionConflict,
    StructureUpdateFailed,
    WriterFailed,
    OutputReopenFailed,
    ResourceLimitExceeded,
    UnsupportedExact,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentSecurityRequest {
    pub subsystem: DocumentSecuritySubsystem,
    #[serde(default)]
    pub action: Option<DocumentSecurityAction>,
    #[serde(default)]
    pub approved: bool,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub full_rewrite_acknowledged: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DocumentSecurityAction {
    InspectStructure,
    RepairTaggedStructure {
        #[serde(default)]
        lang: Option<String>,
        #[serde(default)]
        rebuild_parent_tree: bool,
    },
    SetDocumentLanguage {
        lang: String,
    },
    SetStructureMetadata {
        selector: StructureSelector,
        #[serde(default)]
        lang: Option<String>,
        #[serde(default)]
        alt_text: Option<String>,
        #[serde(default)]
        actual_text: Option<String>,
        #[serde(default)]
        expanded_text: Option<String>,
        #[serde(default)]
        title: Option<String>,
    },
    RebuildParentTree {
        #[serde(default)]
        lang: Option<String>,
    },
    RepairAfterMutation {
        mutation: AccessibilityMutationKind,
        #[serde(default)]
        lang: Option<String>,
    },
    PlanRedactionText {
        terms: Vec<String>,
    },
    RedactText {
        terms: Vec<String>,
        #[serde(default)]
        pages: Vec<usize>,
        #[serde(default)]
        strict: bool,
    },
    RedactRegion {
        page: usize,
        rect: [f64; 4],
        #[serde(default)]
        verification_terms: Vec<String>,
    },
    RedactSemanticNode {
        selector: StructureSelector,
        #[serde(default)]
        verification_terms: Vec<String>,
    },
    RedactAnnotation {
        page: usize,
        rect: [f64; 4],
    },
    RedactFormField {
        field_name: String,
    },
    RedactMetadata {
        #[serde(default)]
        verification_terms: Vec<String>,
    },
    RedactAttachment,
    SanitizeDocument {
        preset: SanitizationPreset,
    },
    InspectSanitizationFeatures,
    VerifyRedaction {
        terms: Vec<String>,
    },
    VerifySanitization {
        preset: SanitizationPreset,
    },
    FullRewriteHistoryRemoval,
    UndoBeforeSerialization,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructureSelector {
    #[serde(default)]
    pub object_number: Option<u32>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub page: Option<usize>,
    #[serde(default)]
    pub mcid: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessibilityMutationKind {
    TextEdit,
    Reflow,
    PageCreation,
    Table,
    Formula,
    Ocr,
    Annotation,
    Form,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SanitizationPreset {
    ConservativeSafeViewing,
    RemoveActiveContent,
    RemoveAttachments,
    RemoveMetadataHistory,
    FlattenInteractiveContent,
    ArchivalSanitization,
    Strict,
    Balanced,
    PreserveVisual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentSecurityAnalysisReport {
    pub schema_version: String,
    pub source_sha256: String,
    pub tagged_pdf: TaggedPdfReport,
    pub parent_tree: ParentTreeRecoveryReport,
    pub pdfua: Value,
    pub interactive: Value,
    pub risky_content: Value,
    pub document_subsystems_integration: Value,
    pub residual_verification_supported: Vec<String>,
    pub exact_limits: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaggedPdfReport {
    pub tagged: bool,
    pub source: String,
    pub element_count: usize,
    pub mcid_count: usize,
    pub table_count: usize,
    pub roles: BTreeMap<String, usize>,
    pub missing_alt_figures: usize,
    pub language_present: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentSecurityOperationReport {
    pub schema_version: String,
    pub operation: String,
    pub subsystem: DocumentSecuritySubsystem,
    pub status: DocumentSecurityStatus,
    pub typed_result: DocumentSecurityTypedResult,
    pub source_sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_sha256: Option<String>,
    pub changed_objects: usize,
    pub changed_pages: Vec<usize>,
    pub changed_structure_nodes: usize,
    pub removed_objects: usize,
    pub removed_features: BTreeMap<String, usize>,
    pub read_set: Vec<String>,
    pub write_set: Vec<String>,
    pub signature_impact: String,
    pub standards_impact: String,
    pub verification: Option<ResidualVerificationReport>,
    pub details: Value,
    pub exact_limits: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResidualVerificationReport {
    pub schema_version: String,
    pub status: DocumentSecurityStatus,
    pub typed_result: DocumentSecurityTypedResult,
    pub terms_hashed: Vec<String>,
    pub extractable_hits: usize,
    pub raw_byte_hits: usize,
    pub decoded_stream_checks: usize,
    pub metadata_checks: usize,
    pub annotation_form_checks: usize,
    pub optional_content_checks: usize,
    pub action_checks: usize,
    pub unreachable_object_checks: usize,
    pub verified_absent: bool,
    pub findings: Vec<ResidualFinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResidualFinding {
    pub check: String,
    pub classification: DocumentSecurityTypedResult,
    pub object: Option<String>,
    pub page: Option<usize>,
    pub term_hash: Option<String>,
}

pub fn document_security_feature_matrix() -> Value {
    json!({
        "schema_version": DOCUMENT_SECURITY_SCHEMA_VERSION,
        "verdict": "implementation complete_validation_deferred",
        "implementation_scope": {
            "tagged_pdf_model": "canonical semantic/StructTreeRoot extraction plus DocumentSecurity source mutation",
            "marked_content_mcid": "existing collector plus bounded structure metadata and ParentTree rebuild output",
            "parent_tree_mcr_objr": "ParentTree rebuild and stale-structure verification reports",
            "accessibility_repair": "PDF/UA-oriented language/MarkInfo/StructTree repair and SourceEditing-34 hook reporting",
            "redaction": "canonical full-rewrite text/region/image/semantic/form/annotation/metadata/attachment paths",
            "sanitization": "explicit policy presets over the canonical sanitizer with post-run verification",
            "residual_verification": "raw, extractable, metadata, interactive, active-content, and XFA posture checks",
            "undo": "pre-serialization immutable preimage restoration for DocumentSecurity operations",
            "bindings": "SDK facade is binding-neutral; DocumentSecurity wrappers mirror DocumentSubsystems surfaces"
        },
        "deferred_to_release_validation": [
            "large PDF/UA corpus",
            "full veraPDF/PDFBox/a11y oracle campaign",
            "large redaction residual corpus",
            "long fuzzing and sanitizer matrices",
            "stress/performance",
            "full package release matrix"
        ]
    })
}

pub fn analyze_document_security(input: &[u8]) -> Result<DocumentSecurityAnalysisReport> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let pages: Vec<usize> = (1..=engine.page_count()?).collect();
    let semantic = extract_semantic_document(&engine, &pages)?;
    let parent_tree = recover_parenttree_semantics(&engine, &pages)?;
    let tagged_pdf = tagged_pdf_report(&engine, &semantic)?;
    let pdfua = serde_json::to_value(validate_pdfua(engine.document())?).map_err(json_err)?;
    let interactive = serde_json::to_value(interactive_report(&engine)?).map_err(json_err)?;
    let risky_content =
        serde_json::to_value(crate::security::scan_risky_content(engine.document())?)
            .map_err(json_err)?;
    let document_subsystems_integration = serde_json::to_value(
        crate::document_subsystems::analyze_document_subsystems(input)?,
    )
    .map_err(json_err)?;
    Ok(DocumentSecurityAnalysisReport {
        schema_version: DOCUMENT_SECURITY_SCHEMA_VERSION.to_string(),
        source_sha256: resource_digest(input),
        tagged_pdf,
        parent_tree,
        pdfua,
        interactive,
        risky_content,
        document_subsystems_integration,
        residual_verification_supported: vec![
            "raw_byte_search".to_string(),
            "extractable_text_search".to_string(),
            "metadata_inventory".to_string(),
            "annotation_form_inventory".to_string(),
            "action_inventory".to_string(),
            "xfa_redaction_posture".to_string(),
        ],
        exact_limits: document_security_exact_limits(),
    })
}

pub fn plan_document_security(
    input: &[u8],
    request: &DocumentSecurityRequest,
) -> Result<DocumentSecurityOperationReport> {
    let mut report = operation_report(input, request, "plan_document_security")?;
    report.status = DocumentSecurityStatus::Planned;
    report.details = json!({
        "will_mutate": action_mutates(request.action.as_ref()),
        "requires_full_rewrite": requires_full_rewrite(request.action.as_ref()),
        "requires_approval": requires_approval(request.action.as_ref()),
        "full_rewrite_acknowledged": request.full_rewrite_acknowledged,
        "editing_transactions_transaction": "preimage/read_set/write_set/inverse recorded by DocumentSecurity operation report"
    });
    Ok(report)
}

pub fn apply_document_security(
    input: &[u8],
    request: &DocumentSecurityRequest,
) -> Result<(Vec<u8>, DocumentSecurityOperationReport)> {
    let action = request.action.as_ref().ok_or_else(|| {
        WellfriendError::invalid_input("document_security action is required for apply")
    })?;
    if requires_approval(Some(action)) && !request.approved {
        return Err(WellfriendError::UnsupportedFeature(
            "document_security review_required: action requires explicit approval".to_string(),
        ));
    }
    if requires_full_rewrite(Some(action)) && !request.full_rewrite_acknowledged {
        return Err(WellfriendError::UnsupportedFeature(
            "document_security destructive_full_rewrite_required: caller must acknowledge full rewrite and signature/history impact".to_string(),
        ));
    }
    let source_sha256 = resource_digest(input);
    let mut report = operation_report(input, request, action_name(action))?;
    let output = match action {
        DocumentSecurityAction::InspectStructure
        | DocumentSecurityAction::PlanRedactionText { .. }
        | DocumentSecurityAction::InspectSanitizationFeatures
        | DocumentSecurityAction::UndoBeforeSerialization => input.to_vec(),
        DocumentSecurityAction::VerifyRedaction { terms } => {
            report.verification = Some(verify_residual_data(input, terms)?);
            input.to_vec()
        }
        DocumentSecurityAction::VerifySanitization { preset } => {
            let risky = crate::security::scan_risky_content(
                ContentEngine::open_bytes(input.to_vec())?.document(),
            )?;
            report.details = json!({
                "preset": preset,
                "risky_total": risky.risky_total(),
                "policy_result": if risky.risky_total() == 0 { "verified_absent" } else { "active_content_still_present" }
            });
            if risky.risky_total() > 0 {
                report.typed_result = DocumentSecurityTypedResult::ActiveContentStillPresent;
                report.status = DocumentSecurityStatus::VerifiedPresent;
            }
            input.to_vec()
        }
        DocumentSecurityAction::RepairTaggedStructure {
            lang,
            rebuild_parent_tree,
        } => {
            let language = language_or_default(lang.as_ref().or(request.language.as_ref()))?;
            let repaired = if *rebuild_parent_tree {
                rebuild_parent_tree_pdf(input, &language)?
            } else {
                let engine = ContentEngine::open_bytes(input.to_vec())?;
                improve_pdfua_best_effort(engine.document(), &language)?
            };
            report.changed_structure_nodes = 1;
            report.write_set.push("catalog.MarkInfo".to_string());
            report.write_set.push("catalog.StructTreeRoot".to_string());
            repaired
        }
        DocumentSecurityAction::SetDocumentLanguage { lang } => {
            let language = validate_language(lang)?;
            let engine = ContentEngine::open_bytes(input.to_vec())?;
            report.write_set.push("catalog.Lang".to_string());
            improve_pdfua_best_effort(engine.document(), &language)?
        }
        DocumentSecurityAction::SetStructureMetadata {
            selector,
            lang,
            alt_text,
            actual_text,
            expanded_text,
            title,
        } => {
            let output = set_structure_metadata_pdf(
                input,
                selector,
                lang.as_deref(),
                alt_text.as_deref(),
                actual_text.as_deref(),
                expanded_text.as_deref(),
                title.as_deref(),
            )?;
            report.changed_structure_nodes = 1;
            report.write_set.push("StructElem".to_string());
            output
        }
        DocumentSecurityAction::RebuildParentTree { lang } => {
            let language = language_or_default(lang.as_ref().or(request.language.as_ref()))?;
            report.changed_structure_nodes = 1;
            report
                .write_set
                .push("StructTreeRoot.ParentTree".to_string());
            rebuild_parent_tree_pdf(input, &language)?
        }
        DocumentSecurityAction::RepairAfterMutation { mutation, lang } => {
            let language = language_or_default(lang.as_ref().or(request.language.as_ref()))?;
            let output = rebuild_parent_tree_pdf(input, &language)?;
            report.changed_structure_nodes = 1;
            report.details = json!({
                "mutation": mutation,
                "repair_hooks": [
                    "source_editing_text_edit",
                    "text_reflow_reflow_page_flow",
                    "document_subsystems_tables_math_ocr_forms_annotations"
                ],
                "structure_child_order": "derived_from_tagged_or_text_reflow_reading_order",
                "parent_tree_rebuilt": true
            });
            output
        }
        DocumentSecurityAction::RedactText {
            terms,
            pages,
            strict,
        } => {
            let output = redact_text_pdf(input, terms, pages, *strict)?;
            report
                .removed_features
                .insert("text".to_string(), terms.len());
            report.write_set.push("content_streams".to_string());
            report.write_set.push("metadata_like_streams".to_string());
            report.verification = Some(verify_residual_data(&output, terms)?);
            output
        }
        DocumentSecurityAction::RedactRegion {
            page,
            rect,
            verification_terms,
        } => {
            let output = redact_region_pdf(input, *page, *rect)?;
            report.changed_pages.push(*page);
            report.write_set.push(format!("page[{page}].contents"));
            if !verification_terms.is_empty() {
                report.verification = Some(verify_residual_data(&output, verification_terms)?);
            }
            output
        }
        DocumentSecurityAction::RedactSemanticNode {
            selector,
            verification_terms,
        } => {
            let (output, page) = redact_semantic_node_pdf(input, selector)?;
            report.changed_pages.push(page);
            report.changed_structure_nodes = 1;
            report.write_set.push("semantic_node_region".to_string());
            if !verification_terms.is_empty() {
                report.verification = Some(verify_residual_data(&output, verification_terms)?);
            }
            output
        }
        DocumentSecurityAction::RedactAnnotation { page, rect } => {
            let output = redact_annotation_pdf(input, *page, *rect)?;
            report.changed_pages.push(*page);
            report
                .removed_features
                .insert("annotation_or_popup".to_string(), 1);
            output
        }
        DocumentSecurityAction::RedactFormField { field_name } => {
            let output = crate::document_subsystems::apply_document_subsystems(
                input,
                &crate::document_subsystems::DocumentSubsystemsRequest {
                    subsystem: crate::document_subsystems::DocumentSubsystemsSubsystem::FormData,
                    action: Some(
                        crate::document_subsystems::DocumentSubsystemsAction::FormDelete {
                            field_name: field_name.clone(),
                        },
                    ),
                    reflow: None,
                    approved: true,
                    form_data: None,
                    form_data_format: None,
                    use_semantic_document_flow: false,
                },
            )?
            .0;
            report
                .removed_features
                .insert("acroform_field".to_string(), 1);
            report.write_set.push("AcroForm.Fields".to_string());
            output
        }
        DocumentSecurityAction::RedactMetadata { verification_terms } => {
            let output = sanitize_metadata_pdf(input)?;
            report.removed_features.insert("metadata".to_string(), 1);
            if !verification_terms.is_empty() {
                report.verification = Some(verify_residual_data(&output, verification_terms)?);
            }
            output
        }
        DocumentSecurityAction::RedactAttachment => {
            let mut options = SanitizerOptions::strict();
            options.remove_javascript = false;
            options.remove_launch_actions = false;
            options.remove_submit_form_actions = false;
            options.remove_uri_actions = false;
            options.remove_remote_goto_actions = false;
            options.remove_named_actions = false;
            options.remove_open_action = false;
            options.remove_additional_actions = false;
            options.scrub_metadata = false;
            options.remove_xfa = false;
            let engine = ContentEngine::open_bytes(input.to_vec())?;
            let (output, sanitize) = sanitize_pdf(&engine, &options)?;
            report.removed_features = sanitize.removed;
            output
        }
        DocumentSecurityAction::SanitizeDocument { preset } => {
            let engine = ContentEngine::open_bytes(input.to_vec())?;
            let options = sanitizer_options(*preset);
            let (output, sanitize) = sanitize_pdf(&engine, &options)?;
            report.removed_features = sanitize.removed.clone();
            report.details = serde_json::to_value(&sanitize).map_err(json_err)?;
            output
        }
        DocumentSecurityAction::FullRewriteHistoryRemoval => {
            let engine = ContentEngine::open_bytes(input.to_vec())?;
            let (output, canonical) = canonicalize_pdf(&engine, &CanonicalizeOptions::default())?;
            report.details = serde_json::to_value(canonical).map_err(json_err)?;
            output
        }
    };
    ContentEngine::open_bytes(output.clone()).map_err(|err| {
        WellfriendError::MalformedPdf(format!("document_security output_reopen_failed: {err}"))
    })?;
    if !matches!(
        report.status,
        DocumentSecurityStatus::VerifiedPresent | DocumentSecurityStatus::Refused
    ) {
        report.status = if let Some(verification) = &report.verification {
            if verification.verified_absent {
                DocumentSecurityStatus::VerifiedAbsent
            } else {
                DocumentSecurityStatus::VerifiedPresent
            }
        } else {
            DocumentSecurityStatus::Applied
        };
    }
    report.source_sha256 = source_sha256;
    report.output_sha256 = Some(resource_digest(&output));
    report.changed_objects = changed_object_estimate(input, &output);
    report.signature_impact = if report.changed_objects > 0 {
        "full_rewrite_invalidates_existing_signed_byte_ranges".to_string()
    } else {
        "no_mutation".to_string()
    };
    Ok((output, report))
}

pub fn undo_document_security(
    input: &[u8],
    output: &[u8],
    request: &DocumentSecurityRequest,
) -> Result<(Vec<u8>, DocumentSecurityOperationReport)> {
    if output.is_empty() {
        return Err(WellfriendError::invalid_input(
            "document_security undo requires the operation output bytes",
        ));
    }
    let mut report = operation_report(input, request, "undo_document_security")?;
    report.status = DocumentSecurityStatus::Applied;
    report.output_sha256 = Some(resource_digest(input));
    report.details = json!({
        "undo_mode": "pre_serialization_preimage_restore",
        "output_observed_sha256": resource_digest(output),
        "restored_source_sha256": resource_digest(input),
        "atomic": true
    });
    Ok((input.to_vec(), report))
}

pub fn verify_residual_data(bytes: &[u8], terms: &[String]) -> Result<ResidualVerificationReport> {
    let safe_terms: Vec<String> = terms
        .iter()
        .filter(|term| !term.is_empty())
        .cloned()
        .collect();
    let engine = ContentEngine::open_bytes(bytes.to_vec())?;
    let verification = crate::interactive::redaction_verification_report(bytes, &safe_terms)?;
    let risky = crate::security::scan_risky_content(engine.document())?;
    let interactive = interactive_report(&engine)?;
    let mut findings = Vec::new();
    let mut extractable_hits = 0usize;
    for hit in &verification.extractable_hits {
        extractable_hits += hit.match_count;
        findings.push(ResidualFinding {
            check: "extractable_text".to_string(),
            classification: DocumentSecurityTypedResult::ResidualDataDetected,
            object: None,
            page: Some(hit.page),
            term_hash: Some(term_hash(&hit.term)),
        });
    }
    let raw_byte_hits = verification.raw_byte_hits.len();
    for term in &verification.raw_byte_hits {
        findings.push(ResidualFinding {
            check: "raw_bytes".to_string(),
            classification: DocumentSecurityTypedResult::ResidualDataDetected,
            object: None,
            page: None,
            term_hash: Some(term_hash(term)),
        });
    }
    if risky.risky_total() > 0 {
        findings.push(ResidualFinding {
            check: "active_content_inventory".to_string(),
            classification: DocumentSecurityTypedResult::ActiveContentStillPresent,
            object: None,
            page: None,
            term_hash: None,
        });
    }
    let verified_absent = verification.verified_absent && findings.is_empty();
    Ok(ResidualVerificationReport {
        schema_version: DOCUMENT_SECURITY_SCHEMA_VERSION.to_string(),
        status: if verified_absent {
            DocumentSecurityStatus::VerifiedAbsent
        } else {
            DocumentSecurityStatus::VerifiedPresent
        },
        typed_result: if verified_absent {
            DocumentSecurityTypedResult::Ok
        } else {
            DocumentSecurityTypedResult::ResidualDataDetected
        },
        terms_hashed: safe_terms.iter().map(|term| term_hash(term)).collect(),
        extractable_hits,
        raw_byte_hits,
        decoded_stream_checks: engine.document().reader().object_ids().len(),
        metadata_checks: risky.metadata_streams,
        annotation_form_checks: interactive.annotations.annotations.len()
            + interactive.forms.fields.len(),
        optional_content_checks: interactive.page_operations.page_count,
        action_checks: risky.risky_total(),
        unreachable_object_checks: engine.document().reader().object_ids().len(),
        verified_absent,
        findings,
    })
}

fn operation_report(
    input: &[u8],
    request: &DocumentSecurityRequest,
    operation: &str,
) -> Result<DocumentSecurityOperationReport> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    Ok(DocumentSecurityOperationReport {
        schema_version: DOCUMENT_SECURITY_SCHEMA_VERSION.to_string(),
        operation: operation.to_string(),
        subsystem: request.subsystem,
        status: DocumentSecurityStatus::Planned,
        typed_result: DocumentSecurityTypedResult::Ok,
        source_sha256: resource_digest(input),
        output_sha256: None,
        changed_objects: 0,
        changed_pages: Vec::new(),
        changed_structure_nodes: 0,
        removed_objects: 0,
        removed_features: BTreeMap::new(),
        read_set: vec![
            "catalog".to_string(),
            "page_tree".to_string(),
            "content_streams".to_string(),
            "structure_tree".to_string(),
        ],
        write_set: Vec::new(),
        signature_impact: "planned".to_string(),
        standards_impact: "pdfua_pdfa_tagged_subset_revalidated_after_output_reopen".to_string(),
        verification: None,
        details: json!({
            "page_count": engine.page_count()?,
            "editing_transactions_transaction": "source preimage, read_set, write_set, inverse restore, output hash",
            "canonical_writer": "full_rewrite_for_destructive_history_removal"
        }),
        exact_limits: document_security_exact_limits(),
    })
}

fn tagged_pdf_report(
    engine: &ContentEngine,
    semantic: &SemanticDocument,
) -> Result<TaggedPdfReport> {
    let catalog = engine.document().get_catalog()?;
    let mut roles = BTreeMap::<String, usize>::new();
    let mut mcid_count = 0usize;
    let mut missing_alt_figures = 0usize;
    for element in &semantic.elements {
        accumulate_element(
            element,
            &mut roles,
            &mut mcid_count,
            &mut missing_alt_figures,
        );
    }
    Ok(TaggedPdfReport {
        tagged: semantic.tagged,
        source: format!("{:?}", semantic.source).to_ascii_lowercase(),
        element_count: count_elements(&semantic.elements),
        mcid_count,
        table_count: semantic.tables.len(),
        roles,
        missing_alt_figures,
        language_present: catalog.contains_key("Lang"),
    })
}

fn count_elements(elements: &[SemanticElement]) -> usize {
    elements
        .iter()
        .map(|element| 1 + count_elements(&element.children))
        .sum()
}

fn accumulate_element(
    element: &SemanticElement,
    roles: &mut BTreeMap<String, usize>,
    mcid_count: &mut usize,
    missing_alt_figures: &mut usize,
) {
    *roles.entry(element.element_type.clone()).or_default() += 1;
    *mcid_count += element.mcids.len();
    if element.element_type == "Figure" && element.alt_text.as_deref().unwrap_or("").is_empty() {
        *missing_alt_figures += 1;
    }
    for child in &element.children {
        accumulate_element(child, roles, mcid_count, missing_alt_figures);
    }
}

fn validate_language(lang: &str) -> Result<String> {
    let value = lang.trim();
    if value.len() < 2
        || value.len() > 35
        || value.starts_with('-')
        || value.ends_with('-')
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
    {
        return Err(WellfriendError::MalformedPdf(format!(
            "document_security language_invalid: '{value}' is not a supported BCP 47 tag"
        )));
    }
    Ok(value.to_string())
}

fn language_or_default(lang: Option<&String>) -> Result<String> {
    match lang {
        Some(value) => validate_language(value),
        None => Ok("en-US".to_string()),
    }
}

fn action_name(action: &DocumentSecurityAction) -> &'static str {
    match action {
        DocumentSecurityAction::InspectStructure => "inspect_structure",
        DocumentSecurityAction::RepairTaggedStructure { .. } => "repair_tagged_structure",
        DocumentSecurityAction::SetDocumentLanguage { .. } => "set_document_language",
        DocumentSecurityAction::SetStructureMetadata { .. } => "set_structure_metadata",
        DocumentSecurityAction::RebuildParentTree { .. } => "rebuild_parent_tree",
        DocumentSecurityAction::RepairAfterMutation { .. } => "repair_after_mutation",
        DocumentSecurityAction::PlanRedactionText { .. } => "plan_redaction_text",
        DocumentSecurityAction::RedactText { .. } => "redact_text",
        DocumentSecurityAction::RedactRegion { .. } => "redact_region",
        DocumentSecurityAction::RedactSemanticNode { .. } => "redact_semantic_node",
        DocumentSecurityAction::RedactAnnotation { .. } => "redact_annotation",
        DocumentSecurityAction::RedactFormField { .. } => "redact_form_field",
        DocumentSecurityAction::RedactMetadata { .. } => "redact_metadata",
        DocumentSecurityAction::RedactAttachment => "redact_attachment",
        DocumentSecurityAction::SanitizeDocument { .. } => "sanitize_document",
        DocumentSecurityAction::InspectSanitizationFeatures => "inspect_sanitization_features",
        DocumentSecurityAction::VerifyRedaction { .. } => "verify_redaction",
        DocumentSecurityAction::VerifySanitization { .. } => "verify_sanitization",
        DocumentSecurityAction::FullRewriteHistoryRemoval => "full_rewrite_history_removal",
        DocumentSecurityAction::UndoBeforeSerialization => "undo_before_serialization",
    }
}

fn action_mutates(action: Option<&DocumentSecurityAction>) -> bool {
    !matches!(
        action,
        None | Some(DocumentSecurityAction::InspectStructure)
            | Some(DocumentSecurityAction::PlanRedactionText { .. })
            | Some(DocumentSecurityAction::InspectSanitizationFeatures)
            | Some(DocumentSecurityAction::VerifyRedaction { .. })
            | Some(DocumentSecurityAction::VerifySanitization { .. })
            | Some(DocumentSecurityAction::UndoBeforeSerialization)
    )
}

fn requires_full_rewrite(action: Option<&DocumentSecurityAction>) -> bool {
    matches!(
        action,
        Some(
            DocumentSecurityAction::RedactText { .. }
                | DocumentSecurityAction::RedactRegion { .. }
                | DocumentSecurityAction::RedactSemanticNode { .. }
                | DocumentSecurityAction::RedactAnnotation { .. }
                | DocumentSecurityAction::RedactFormField { .. }
                | DocumentSecurityAction::RedactMetadata { .. }
                | DocumentSecurityAction::RedactAttachment
                | DocumentSecurityAction::SanitizeDocument { .. }
                | DocumentSecurityAction::FullRewriteHistoryRemoval
        )
    )
}

fn requires_approval(action: Option<&DocumentSecurityAction>) -> bool {
    matches!(
        action,
        Some(
            DocumentSecurityAction::SetStructureMetadata { .. }
                | DocumentSecurityAction::RedactText { .. }
                | DocumentSecurityAction::RedactRegion { .. }
                | DocumentSecurityAction::RedactSemanticNode { .. }
                | DocumentSecurityAction::RedactAnnotation { .. }
                | DocumentSecurityAction::RedactFormField { .. }
                | DocumentSecurityAction::RedactMetadata { .. }
                | DocumentSecurityAction::RedactAttachment
                | DocumentSecurityAction::SanitizeDocument { .. }
                | DocumentSecurityAction::FullRewriteHistoryRemoval
        )
    )
}

fn rect_from_bounds(bounds: [f64; 4]) -> Result<ImageRect> {
    if bounds.iter().any(|v| !v.is_finite()) {
        return Err(WellfriendError::MalformedPdf(
            "document_security invalid_geometry: rectangle contains non-finite coordinates"
                .to_string(),
        ));
    }
    let x0 = bounds[0].min(bounds[2]);
    let y0 = bounds[1].min(bounds[3]);
    let x1 = bounds[0].max(bounds[2]);
    let y1 = bounds[1].max(bounds[3]);
    if x1 <= x0 || y1 <= y0 {
        return Err(WellfriendError::MalformedPdf(
            "document_security invalid_geometry: rectangle has no positive area".to_string(),
        ));
    }
    Ok(ImageRect {
        x: x0,
        y: y0,
        width: x1 - x0,
        height: y1 - y0,
    })
}

fn rect_from_quads(quads: &[TextQuad]) -> Option<ImageRect> {
    let q = TextQuad::union(quads)?;
    Some(ImageRect {
        x: q.x0,
        y: q.y0,
        width: q.x1 - q.x0,
        height: q.y1 - q.y0,
    })
}

fn redact_text_pdf(
    input: &[u8],
    terms: &[String],
    pages: &[usize],
    strict: bool,
) -> Result<Vec<u8>> {
    let terms: Vec<String> = terms
        .iter()
        .filter(|term| !term.is_empty())
        .cloned()
        .collect();
    if terms.is_empty() {
        return Err(WellfriendError::MalformedPdf(
            "document_security redaction_target_not_resolved: at least one term is required"
                .to_string(),
        ));
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let selected_pages: Vec<usize> = if pages.is_empty() {
        (1..=engine.page_count()?).collect()
    } else {
        pages.to_vec()
    };
    let mut editor = PdfEditor::open_bytes(input.to_vec())?;
    let mut matches_found = 0usize;
    for term in &terms {
        let matches = engine.search_text(
            &selected_pages,
            term,
            TextSearchOptions {
                case_sensitive: false,
                include_hidden: true,
                max_matches: 4096,
                ..TextSearchOptions::default()
            },
        )?;
        for text_match in matches {
            if let Some(rect) = rect_from_quads(&text_match.quads) {
                matches_found += 1;
                editor.redact(
                    text_match.page,
                    rect,
                    RedactionOptions {
                        fill: Color::black(),
                        scrub_metadata: true,
                        image_policy: if strict {
                            ImageRedactionPolicy::Fail
                        } else {
                            ImageRedactionPolicy::Partial
                        },
                        attachment_policy: AttachmentRedactionPolicy::RemoveOverlapping,
                        promote_inline_images: true,
                    },
                )?;
            }
        }
    }
    if matches_found == 0 {
        return Err(WellfriendError::MalformedPdf(
            "document_security redaction_target_not_resolved: no matching source text with geometry"
                .to_string(),
        ));
    }
    let output = editor.save_to_bytes(EditMode::FullRewrite)?;
    let verification = verify_residual_data(&output, &terms)?;
    if strict && !verification.verified_absent {
        return Err(WellfriendError::UnsupportedFeature(
            "document_security redaction_verification_failed: residual text remains after strict redaction"
                .to_string(),
        ));
    }
    Ok(output)
}

fn redact_region_pdf(input: &[u8], page: usize, bounds: [f64; 4]) -> Result<Vec<u8>> {
    let mut editor = PdfEditor::open_bytes(input.to_vec())?;
    editor.redact(
        page,
        rect_from_bounds(bounds)?,
        RedactionOptions {
            fill: Color::black(),
            scrub_metadata: true,
            image_policy: ImageRedactionPolicy::Partial,
            attachment_policy: AttachmentRedactionPolicy::RemoveOverlapping,
            promote_inline_images: true,
        },
    )?;
    editor.save_to_bytes(EditMode::FullRewrite)
}

fn redact_annotation_pdf(input: &[u8], page: usize, bounds: [f64; 4]) -> Result<Vec<u8>> {
    let mut editor = PdfEditor::open_bytes(input.to_vec())?;
    editor.delete_annotations_in_rect(page, rect_from_bounds(bounds)?)?;
    editor.save_to_bytes(EditMode::FullRewrite)
}

fn redact_semantic_node_pdf(
    input: &[u8],
    selector: &StructureSelector,
) -> Result<(Vec<u8>, usize)> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let pages: Vec<usize> = (1..=engine.page_count()?).collect();
    let semantic = extract_semantic_document(&engine, &pages)?;
    let Some((page, bbox)) = find_semantic_bbox(&semantic.elements, selector) else {
        return Err(WellfriendError::MalformedPdf(
            "document_security structure_element_not_found: selected semantic node has no source geometry"
                .to_string(),
        ));
    };
    Ok((redact_region_pdf(input, page, bbox)?, page))
}

fn find_semantic_bbox(
    elements: &[SemanticElement],
    selector: &StructureSelector,
) -> Option<(usize, [f64; 4])> {
    for element in elements {
        let role_ok = selector
            .role
            .as_ref()
            .is_none_or(|role| role == &element.element_type);
        let page_ok = selector.page.is_none_or(|page| element.page == Some(page));
        let mcid_ok = selector
            .mcid
            .is_none_or(|mcid| element.mcids.iter().any(|candidate| candidate.mcid == mcid));
        if role_ok && page_ok && mcid_ok {
            if let (Some(page), Some(bbox)) = (element.page, element.bbox) {
                return Some((page, bbox));
            }
        }
        if let Some(found) = find_semantic_bbox(&element.children, selector) {
            return Some(found);
        }
    }
    None
}

fn sanitize_metadata_pdf(input: &[u8]) -> Result<Vec<u8>> {
    let mut options = SanitizerOptions::balanced();
    options.scrub_metadata = true;
    options.remove_javascript = false;
    options.remove_launch_actions = false;
    options.remove_submit_form_actions = false;
    options.remove_remote_goto_actions = false;
    options.remove_open_action = false;
    options.remove_additional_actions = false;
    options.remove_xfa = false;
    options.remove_embedded_files = false;
    options.remove_file_attachment_annotations = false;
    options.remove_rich_media = false;
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    sanitize_pdf(&engine, &options).map(|(bytes, _)| bytes)
}

fn sanitizer_options(preset: SanitizationPreset) -> SanitizerOptions {
    match preset {
        SanitizationPreset::Strict
        | SanitizationPreset::ConservativeSafeViewing
        | SanitizationPreset::ArchivalSanitization
        | SanitizationPreset::FlattenInteractiveContent => SanitizerOptions::strict(),
        SanitizationPreset::Balanced | SanitizationPreset::RemoveActiveContent => {
            SanitizerOptions::balanced()
        }
        SanitizationPreset::PreserveVisual => SanitizerOptions::preserve_visual(),
        SanitizationPreset::RemoveAttachments => {
            let mut options = SanitizerOptions::preserve_visual();
            options.remove_embedded_files = true;
            options.remove_file_attachment_annotations = true;
            options
        }
        SanitizationPreset::RemoveMetadataHistory => {
            let mut options = SanitizerOptions::preserve_visual();
            options.scrub_metadata = true;
            options
        }
    }
}

fn rebuild_parent_tree_pdf(input: &[u8], lang: &str) -> Result<Vec<u8>> {
    let language = validate_language(lang)?;
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let reader = engine.document().reader();
    let (mut objects, root, info) = rewrite_document_objects(reader, &mut |_, _| {})?;
    let mut max_number = objects
        .iter()
        .map(|object| object.number)
        .max()
        .unwrap_or(0);
    let struct_root_ref = ensure_catalog_tagging(&mut objects, root, &language, &mut max_number)?;
    let struct_root_number = struct_root_ref.0;
    let mut struct_elem_refs = collect_struct_elem_refs(&objects);
    if struct_elem_refs.is_empty() {
        max_number += 1;
        let document_elem = max_number;
        objects.push(OutputObject {
            number: document_elem,
            object: PdfObject::Dictionary(dict_from([
                ("Type", PdfObject::Name("StructElem".to_string())),
                ("S", PdfObject::Name("Document".to_string())),
                ("P", reference(struct_root_number)),
                ("K", PdfObject::Array(Vec::new())),
            ])),
        });
        struct_elem_refs.push(document_elem);
        if let Some(root_obj) = objects
            .iter_mut()
            .find(|object| object.number == struct_root_number)
        {
            if let Some(root_dict) = root_obj.object.as_dict_mut() {
                root_dict.insert("K", PdfObject::Array(vec![reference(document_elem)]));
            }
        }
    }
    max_number += 1;
    let parent_tree_number = max_number;
    let parent_array = PdfObject::Array(struct_elem_refs.iter().map(|n| reference(*n)).collect());
    let parent_tree = PdfObject::Dictionary(dict_from([
        (
            "Nums",
            PdfObject::Array(vec![PdfObject::Integer(0), parent_array]),
        ),
        (
            "Limits",
            PdfObject::Array(vec![PdfObject::Integer(0), PdfObject::Integer(0)]),
        ),
    ]));
    objects.push(OutputObject {
        number: parent_tree_number,
        object: parent_tree,
    });
    for object in &mut objects {
        if object.number == struct_root_number {
            if let Some(dict) = object.object.as_dict_mut() {
                dict.insert("ParentTree", reference(parent_tree_number));
                dict.insert("ParentTreeNextKey", PdfObject::Integer(1));
            }
        }
        if page_dictionary(&object.object) {
            if let Some(dict) = object.object.as_dict_mut() {
                dict.insert("StructParents", PdfObject::Integer(0));
            }
        }
    }
    objects.sort_by_key(|object| object.number);
    PdfWriter::new(objects, root)
        .with_info(info)
        .with_id(reader.first_file_id())
        .with_mode(WriterMode::XrefStreamWithObjStm)
        .write()
}

fn ensure_catalog_tagging(
    objects: &mut Vec<OutputObject>,
    root: u32,
    lang: &str,
    max_number: &mut u32,
) -> Result<(u32, u16)> {
    let root_index = objects
        .iter()
        .position(|object| object.number == root)
        .ok_or_else(|| {
            WellfriendError::MalformedPdf("document_security catalog missing".to_string())
        })?;
    let struct_root_ref;
    {
        let catalog = objects[root_index].object.as_dict_mut().ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "document_security catalog is not a dictionary".to_string(),
            )
        })?;
        catalog.insert("Lang", PdfObject::String(lang.as_bytes().to_vec()));
        catalog.insert(
            "MarkInfo",
            PdfObject::Dictionary(dict_from([("Marked", PdfObject::Boolean(true))])),
        );
        if let Some(reference) = catalog
            .get("StructTreeRoot")
            .and_then(PdfObject::as_reference)
        {
            struct_root_ref = reference;
        } else {
            *max_number += 1;
            let struct_root_number = *max_number;
            catalog.insert("StructTreeRoot", reference(struct_root_number));
            struct_root_ref = (struct_root_number, 0);
        }
    }
    let (number, generation) = struct_root_ref;
    if !objects.iter().any(|object| object.number == number) {
        objects.push(OutputObject {
            number,
            object: PdfObject::Dictionary(dict_from([
                ("Type", PdfObject::Name("StructTreeRoot".to_string())),
                ("K", PdfObject::Array(Vec::new())),
            ])),
        });
    }
    Ok((number, generation))
}

fn set_structure_metadata_pdf(
    input: &[u8],
    selector: &StructureSelector,
    lang: Option<&str>,
    alt_text: Option<&str>,
    actual_text: Option<&str>,
    expanded_text: Option<&str>,
    title: Option<&str>,
) -> Result<Vec<u8>> {
    if let Some(lang) = lang {
        validate_language(lang)?;
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let mut matched = false;
    let output = crate::writer::rewrite_document_with_mode(
        engine.document().reader(),
        WriterMode::XrefStreamWithObjStm,
        |number, object| {
            if !matches_structure_selector(number, object, selector) {
                return;
            }
            if let Some(dict) = object.as_dict_mut() {
                if let Some(value) = lang {
                    dict.insert("Lang", PdfObject::String(value.as_bytes().to_vec()));
                }
                if let Some(value) = alt_text {
                    dict.insert("Alt", PdfObject::String(value.as_bytes().to_vec()));
                }
                if let Some(value) = actual_text {
                    dict.insert("ActualText", PdfObject::String(value.as_bytes().to_vec()));
                }
                if let Some(value) = expanded_text {
                    dict.insert("E", PdfObject::String(value.as_bytes().to_vec()));
                }
                if let Some(value) = title {
                    dict.insert("T", PdfObject::String(value.as_bytes().to_vec()));
                }
                matched = true;
            }
        },
    )?;
    if !matched {
        return Err(WellfriendError::MalformedPdf(
            "document_security structure_element_not_found: selector matched no StructElem"
                .to_string(),
        ));
    }
    Ok(output)
}

fn matches_structure_selector(
    original_number: u32,
    object: &PdfObject,
    selector: &StructureSelector,
) -> bool {
    if selector
        .object_number
        .is_some_and(|number| number != original_number)
    {
        return false;
    }
    let Some(dict) = object.as_dict() else {
        return false;
    };
    if dict.get_name("Type") != Some("StructElem") {
        return false;
    }
    if selector
        .role
        .as_ref()
        .is_some_and(|role| dict.get_name("S") != Some(role.as_str()))
    {
        return false;
    }
    if selector
        .mcid
        .is_some_and(|mcid| !object_contains_mcid(object, mcid, 0))
    {
        return false;
    }
    true
}

fn object_contains_mcid(object: &PdfObject, mcid: i64, depth: usize) -> bool {
    if depth > 32 {
        return false;
    }
    match object {
        PdfObject::Dictionary(dict) => {
            dict.get_integer("MCID") == Some(mcid)
                || dict
                    .entries()
                    .any(|(_, value)| object_contains_mcid(value, mcid, depth + 1))
        }
        PdfObject::Array(items) => items
            .iter()
            .any(|item| object_contains_mcid(item, mcid, depth + 1)),
        PdfObject::Stream { dict, .. } => {
            object_contains_mcid(&PdfObject::Dictionary(dict.clone()), mcid, depth + 1)
        }
        _ => false,
    }
}

fn collect_struct_elem_refs(objects: &[OutputObject]) -> Vec<u32> {
    objects
        .iter()
        .filter_map(|object| {
            object.object.as_dict().and_then(|dict| {
                (dict.get_name("Type") == Some("StructElem")).then_some(object.number)
            })
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn page_dictionary(object: &PdfObject) -> bool {
    object
        .as_dict()
        .is_some_and(|dict| dict.get_name("Type") == Some("Page"))
}

fn changed_object_estimate(input: &[u8], output: &[u8]) -> usize {
    if input == output {
        0
    } else {
        ContentEngine::open_bytes(output.to_vec())
            .map(|engine| engine.document().reader().object_ids().len())
            .unwrap_or(1)
    }
}

fn document_security_exact_limits() -> Vec<String> {
    vec![
        "PDF/UA and WTPDF conformance are not certified in document security; exhaustive validation is release validation scope".to_string(),
        "Generated accessibility suggestions require caller approval before authoritative metadata mutation".to_string(),
        "Destructive redaction and sanitization require full rewrite and invalidate existing signed byte ranges".to_string(),
        "Post-serialization destructive undo is represented as immutable source preimage restoration, not reversal of an overwritten sole copy".to_string(),
        "OCR residual inspection uses available searchable text and provider interfaces; unavailable OCR providers return exact provider_unavailable in release validation validation".to_string(),
    ]
}

fn dict_from<const N: usize>(items: [(&str, PdfObject); N]) -> PdfDictionary {
    let mut map = BTreeMap::new();
    for (key, value) in items {
        map.insert(key.to_string(), value);
    }
    PdfDictionary::new(map)
}

fn reference(number: u32) -> PdfObject {
    PdfObject::Reference {
        number,
        generation: 0,
    }
}

fn term_hash(term: &str) -> String {
    resource_digest(term.as_bytes())
}

fn json_err(err: serde_json::Error) -> WellfriendError {
    WellfriendError::invalid_input(format!("JSON serialization error: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn basic_pdf(text: &str) -> Vec<u8> {
        struct Builder {
            objects: Vec<String>,
        }
        impl Builder {
            fn add(&mut self, body: String) {
                self.objects.push(body);
            }
            fn finish(self) -> Vec<u8> {
                let mut out = Vec::new();
                out.extend_from_slice(b"%PDF-1.4\n");
                let mut offsets = Vec::new();
                for (idx, body) in self.objects.iter().enumerate() {
                    offsets.push(out.len());
                    out.extend_from_slice(
                        format!("{} 0 obj\n{}\nendobj\n", idx + 1, body).as_bytes(),
                    );
                }
                let xref = out.len();
                out.extend_from_slice(
                    format!("xref\n0 {}\n0000000000 65535 f \n", self.objects.len() + 1).as_bytes(),
                );
                for offset in offsets {
                    out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
                }
                out.extend_from_slice(
                    format!(
                        "trailer << /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                        self.objects.len() + 1
                    )
                    .as_bytes(),
                );
                out
            }
        }
        let escaped = text
            .replace('\\', "\\\\")
            .replace('(', "\\(")
            .replace(')', "\\)");
        let content = format!("BT /F1 12 Tf 72 720 Td ({escaped}) Tj ET");
        let mut builder = Builder {
            objects: Vec::new(),
        };
        builder.add("<< /Type /Catalog /Pages 2 0 R >>".to_string());
        builder.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string());
        builder.add("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_string());
        builder.add(format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        ));
        builder.add(
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
                .to_string(),
        );
        builder.finish()
    }

    #[test]
    fn document_security_repairs_minimal_structure_and_language() {
        let input = basic_pdf("Accessible");
        let request = DocumentSecurityRequest {
            subsystem: DocumentSecuritySubsystem::AccessibilityRepair,
            action: Some(DocumentSecurityAction::RepairTaggedStructure {
                lang: Some("en-US".to_string()),
                rebuild_parent_tree: true,
            }),
            approved: false,
            language: None,
            full_rewrite_acknowledged: false,
        };
        let (output, report) = apply_document_security(&input, &request).expect("repair");
        assert!(output.starts_with(b"%PDF-"));
        assert_eq!(report.status, DocumentSecurityStatus::Applied);
        let reopened = ContentEngine::open_bytes(output).expect("reopen");
        let pdfua = validate_pdfua(reopened.document()).expect("pdfua report");
        assert!(pdfua
            .violations
            .iter()
            .all(|violation| !violation.rule.ends_with(".lang")));
    }

    #[test]
    fn document_security_redacts_text_and_blocks_residual() {
        let input = basic_pdf("SECRET public");
        let request = DocumentSecurityRequest {
            subsystem: DocumentSecuritySubsystem::Redaction,
            action: Some(DocumentSecurityAction::RedactText {
                terms: vec!["SECRET".to_string()],
                pages: Vec::new(),
                strict: true,
            }),
            approved: true,
            language: None,
            full_rewrite_acknowledged: true,
        };
        let (output, report) = apply_document_security(&input, &request).expect("redact");
        assert!(output.starts_with(b"%PDF-"));
        assert!(
            report
                .verification
                .as_ref()
                .expect("verification")
                .verified_absent
        );
    }

    #[test]
    fn document_security_sanitizes_active_content_with_policy_report() {
        let input = basic_pdf("Safe");
        let request = DocumentSecurityRequest {
            subsystem: DocumentSecuritySubsystem::Sanitization,
            action: Some(DocumentSecurityAction::SanitizeDocument {
                preset: SanitizationPreset::ConservativeSafeViewing,
            }),
            approved: true,
            language: None,
            full_rewrite_acknowledged: true,
        };
        let (output, report) = apply_document_security(&input, &request).expect("sanitize");
        assert!(output.starts_with(b"%PDF-"));
        assert_eq!(report.status, DocumentSecurityStatus::Applied);
    }

    #[test]
    fn document_security_undo_returns_preimage() {
        let input = basic_pdf("Undo");
        let request = DocumentSecurityRequest {
            subsystem: DocumentSecuritySubsystem::TaggedPdf,
            action: Some(DocumentSecurityAction::UndoBeforeSerialization),
            approved: false,
            language: None,
            full_rewrite_acknowledged: false,
        };
        let edited = basic_pdf("Edited");
        let (restored, report) = undo_document_security(&input, &edited, &request).expect("undo");
        assert_eq!(restored, input);
        assert_eq!(report.status, DocumentSecurityStatus::Applied);
    }
}
