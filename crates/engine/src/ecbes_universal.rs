//! End-to-end ECBES orchestration over the canonical universal-editing engine.
//!
//! Each candidate is planned and materialized from the same immutable input.
//! Candidate bytes remain private until the ECBES kernel has computed a
//! decision. Only the selected bytes are returned. The integrated provenance
//! graph is derived from the revision-bound intent, canonical request digests,
//! affected indirect owners, and affected pages. Its completeness is explicitly
//! bounded by the canonical planner/apply reports; external pixel evidence is
//! still required outside that known cone.

use crate::document_subsystems::{DocumentSubsystemsAction, DocumentSubsystemsSubsystem};
use crate::research_edit_synthesis::{
    EditCandidate, EditCostVector, EditFidelityClass, EditInfluenceGraph, EditSynthesisPolicy,
    EvidenceConstrainedEditDecision, EvidenceConstrainedEditRequest, EvidenceProducerIdentity,
    InfluenceEdge, InfluenceEdgeKind, ProofEvidence, ProofObligationKind, ProofStatus,
    SourceOwnerId,
};
use crate::source_editing::TrueEditingMode;
use crate::universal_editing::{
    apply_universal_edit_v2, apply_universal_edit_v2_with_output_security,
    create_universal_approval_token_v2, digest_hex, plan_universal_edit_v2,
    UniversalApprovalDecisionV2, UniversalEditOperationV2, UniversalEditOutcomeV2,
    UniversalEditPolicyV2, UniversalEditRequestV2, UniversalOutputSecurityCredentialsV2,
    UniversalOutputSecurityPolicyV2, UniversalPlanStateV2,
};
use crate::{ContentEngine, ErrorKind, Result, WellfriendError};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

pub const ECBES_UNIVERSAL_SCHEMA_VERSION: &str = "ecbes.universal-edit-transaction.v1";
const MAX_MATERIALIZED_CANDIDATES: usize = 64;
const HARD_MAX_CANDIDATE_OUTPUT_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const HARD_MAX_TOTAL_OUTPUT_BYTES: u64 = 4 * 1024 * 1024 * 1024;

fn default_max_candidate_output_bytes() -> u64 {
    512 * 1024 * 1024
}

fn default_max_total_output_bytes() -> u64 {
    1024 * 1024 * 1024
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EcbesMeasuredCostHints {
    /// Caller-defined semantic distance in a versioned fixed-point scale.
    #[serde(default)]
    pub semantic_distance_microunits: u64,
    /// Caller-measured displacement in millionths of a PDF point.
    #[serde(default)]
    pub layout_displacement_micropoints: u64,
    /// Versioned, caller-defined font substitution penalty.
    #[serde(default)]
    pub font_substitution_penalty: u32,
    /// Heuristic or calibrated reconstruction uncertainty. This is not treated
    /// as a probability unless the supplied evidence establishes calibration.
    #[serde(default)]
    pub reconstruction_uncertainty_ppm: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EcbesUniversalCandidateRequest {
    pub id: String,
    pub fidelity: EditFidelityClass,
    pub request: UniversalEditRequestV2,
    /// Explicit caller approval for a canonical plan that reports
    /// `approval_required`. The engine creates and validates the bound token;
    /// callers never supply an arbitrary token.
    #[serde(default)]
    pub approval: Option<UniversalApprovalDecisionV2>,
    #[serde(default)]
    pub cost_hints: EcbesMeasuredCostHints,
    /// Evidence that the core cannot generate itself, such as a formal lens
    /// check or an independently produced renderer artifact. Core-produced
    /// obligations always replace caller assertions for the same kind.
    #[serde(default)]
    pub external_evidence: Vec<ProofEvidence>,
}

/// Ask ECBES to deterministically enumerate the canonical fidelity routes that
/// genuinely exist for one universal-edit intent. Text requests expand to
/// operator-preserving, geometric-block, and semantic-document constructions;
/// every other operation has one canonical route. Generated constructions are
/// subject to exactly the same evidence gates as explicit candidates.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EcbesAutomaticCandidateRequest {
    pub id_prefix: String,
    pub request: UniversalEditRequestV2,
    /// Default approval used when a route-specific decision is absent.
    #[serde(default)]
    pub approval: Option<UniversalApprovalDecisionV2>,
    /// Optional decisions keyed by `operator`, `geometric`, `semantic`, or
    /// `canonical`. This keeps exact plan candidate selections route-local.
    #[serde(default)]
    pub route_approvals: BTreeMap<String, UniversalApprovalDecisionV2>,
    #[serde(default)]
    pub cost_hints: EcbesMeasuredCostHints,
    /// Precomputed output-bound evidence keyed by the same route suffix. This
    /// is useful for deterministic replay; normally renderer evidence is
    /// generated from producer-bound reference rasters in the edit contract.
    #[serde(default)]
    pub route_external_evidence: BTreeMap<String, Vec<ProofEvidence>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EcbesUniversalEditPolicy {
    #[serde(default)]
    pub synthesis: EditSynthesisPolicy,
    #[serde(default = "default_max_candidate_output_bytes")]
    pub max_candidate_output_bytes: u64,
    #[serde(default = "default_max_total_output_bytes")]
    pub max_total_output_bytes: u64,
}

impl Default for EcbesUniversalEditPolicy {
    fn default() -> Self {
        Self {
            synthesis: EditSynthesisPolicy::default(),
            max_candidate_output_bytes: default_max_candidate_output_bytes(),
            max_total_output_bytes: default_max_total_output_bytes(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EcbesUniversalEditRequest {
    #[serde(default)]
    pub policy: EcbesUniversalEditPolicy,
    #[serde(default)]
    pub candidates: Vec<EcbesUniversalCandidateRequest>,
    #[serde(default)]
    pub automatic_candidates: Vec<EcbesAutomaticCandidateRequest>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EcbesUniversalCandidateReport {
    pub id: String,
    pub origin: String,
    pub requested_fidelity: EditFidelityClass,
    pub minimum_fidelity: EditFidelityClass,
    pub request_sha256: String,
    pub plan_id: Option<String>,
    pub plan_sha256: Option<String>,
    pub plan_state: Option<UniversalPlanStateV2>,
    pub approval_binding_digest: Option<String>,
    pub universal_outcome: Option<UniversalEditOutcomeV2>,
    pub output_sha256: Option<String>,
    pub output_bytes: Option<u64>,
    pub affected_pages: Vec<usize>,
    pub affected_objects: Vec<String>,
    pub evidence: Vec<ProofEvidence>,
    pub cost: EditCostVector,
    pub elapsed_millis: u128,
    pub error_code: Option<String>,
    pub error: Option<String>,
    pub universal_report: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EcbesPublicationReceipt {
    pub schema_version: String,
    pub input_sha256: String,
    pub selected_candidate_id: Option<String>,
    pub selected_fidelity: Option<EditFidelityClass>,
    pub selected_output_sha256: String,
    pub decision_sha256: String,
    pub published: bool,
    pub transport: Value,
    pub receipt_sha256: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct EcbesUniversalEditReport {
    pub schema_version: String,
    pub input_sha256: String,
    pub input_revision_id: String,
    pub conservative_influence_scope: String,
    pub decision: EvidenceConstrainedEditDecision,
    pub candidates: Vec<EcbesUniversalCandidateReport>,
    pub publication: EcbesPublicationReceipt,
}

struct MaterializedCandidate {
    report: EcbesUniversalCandidateReport,
    edit_candidate: EditCandidate,
    output: Option<Vec<u8>>,
}

struct PreparedCandidate {
    request: EcbesUniversalCandidateRequest,
    origin: &'static str,
}

fn automatic_candidates(requests: &[EcbesAutomaticCandidateRequest]) -> Vec<PreparedCandidate> {
    let mut output = Vec::new();
    for automatic in requests {
        match &automatic.request.operation {
            UniversalEditOperationV2::Text { .. } => {
                for (suffix, mode, fidelity) in [
                    (
                        "operator",
                        TrueEditingMode::OperatorPreserving,
                        EditFidelityClass::SourceNative,
                    ),
                    (
                        "geometric",
                        TrueEditingMode::GeometricBlock,
                        EditFidelityClass::SemanticReconstruction,
                    ),
                    (
                        "semantic",
                        TrueEditingMode::SemanticDocument,
                        EditFidelityClass::SemanticReconstruction,
                    ),
                ] {
                    let mut universal = automatic.request.clone();
                    if let UniversalEditOperationV2::Text { request } = &mut universal.operation {
                        request.requested_mode = mode;
                    }
                    output.push(PreparedCandidate {
                        request: EcbesUniversalCandidateRequest {
                            id: format!("{}-{suffix}", automatic.id_prefix),
                            fidelity,
                            request: universal,
                            approval: automatic
                                .route_approvals
                                .get(suffix)
                                .cloned()
                                .or_else(|| automatic.approval.clone()),
                            cost_hints: automatic.cost_hints.clone(),
                            external_evidence: automatic
                                .route_external_evidence
                                .get(suffix)
                                .cloned()
                                .unwrap_or_default(),
                        },
                        origin: "automatic_text_route",
                    });
                }
            }
            operation => output.push(PreparedCandidate {
                request: EcbesUniversalCandidateRequest {
                    id: format!("{}-canonical", automatic.id_prefix),
                    fidelity: minimum_fidelity(operation),
                    request: automatic.request.clone(),
                    approval: automatic
                        .route_approvals
                        .get("canonical")
                        .cloned()
                        .or_else(|| automatic.approval.clone()),
                    cost_hints: automatic.cost_hints.clone(),
                    external_evidence: automatic
                        .route_external_evidence
                        .get("canonical")
                        .cloned()
                        .unwrap_or_default(),
                },
                origin: "automatic_canonical_route",
            }),
        }
    }
    output
}

fn prepare_candidates(request: &EcbesUniversalEditRequest) -> Vec<PreparedCandidate> {
    let mut output = request
        .candidates
        .iter()
        .cloned()
        .map(|request| PreparedCandidate {
            request,
            origin: "explicit",
        })
        .collect::<Vec<_>>();
    output.extend(automatic_candidates(&request.automatic_candidates));
    output
}

fn invalid(message: impl Into<String>) -> WellfriendError {
    WellfriendError::invalid_input(message.into())
}

fn json_bytes<T: Serialize>(value: &T, label: &str) -> Result<Vec<u8>> {
    serde_json::to_vec(value)
        .map_err(|error| invalid(format!("ECBES {label} serialization failed: {error}")))
}

fn json_sha256<T: Serialize>(value: &T, label: &str) -> Result<String> {
    Ok(digest_hex(&json_bytes(value, label)?))
}

fn fidelity_rank(fidelity: EditFidelityClass) -> u8 {
    match fidelity {
        EditFidelityClass::SourceNative => 0,
        EditFidelityClass::SemanticReconstruction => 1,
        EditFidelityClass::AppearanceReconstruction => 2,
    }
}

fn minimum_fidelity(operation: &UniversalEditOperationV2) -> EditFidelityClass {
    match operation {
        UniversalEditOperationV2::Text { request } => match request.requested_mode {
            TrueEditingMode::OperatorPreserving => EditFidelityClass::SourceNative,
            TrueEditingMode::GeometricBlock | TrueEditingMode::SemanticDocument => {
                EditFidelityClass::SemanticReconstruction
            }
        },
        UniversalEditOperationV2::LinkedStory { .. }
        | UniversalEditOperationV2::StoryFigureTransfer { .. }
        | UniversalEditOperationV2::StructureCorrection { .. }
        | UniversalEditOperationV2::DocumentSecurity { .. } => {
            EditFidelityClass::SemanticReconstruction
        }
        UniversalEditOperationV2::DocumentSubsystem { request }
            if request.subsystem == DocumentSubsystemsSubsystem::OcrReconstruction
                || matches!(
                    &request.action,
                    Some(DocumentSubsystemsAction::OcrReconstructVisibleWords { .. })
                ) =>
        {
            EditFidelityClass::AppearanceReconstruction
        }
        UniversalEditOperationV2::DocumentSubsystem { .. } => {
            EditFidelityClass::SemanticReconstruction
        }
        UniversalEditOperationV2::ScopedText { .. }
        | UniversalEditOperationV2::Image { .. }
        | UniversalEditOperationV2::ImageFragment { .. }
        | UniversalEditOperationV2::Vector { .. }
        | UniversalEditOperationV2::ObjectGraph { .. } => EditFidelityClass::SourceNative,
    }
}

fn required_obligations(fidelity: EditFidelityClass) -> BTreeSet<ProofObligationKind> {
    use ProofObligationKind::*;
    let mut required = BTreeSet::from([
        SecurityAuthority,
        SourceRevision,
        InfluenceClosure,
        Reopen,
        LogicalPostcondition,
        OutsideInfluencePixels,
        StructureIntegrity,
        ResourceBudget,
    ]);
    match fidelity {
        EditFidelityClass::SourceNative => {
            required.insert(LensRoundTrip);
        }
        EditFidelityClass::SemanticReconstruction => {
            required.insert(IndependentRendererAgreement);
            required.insert(ReconstructionDisclosure);
        }
        EditFidelityClass::AppearanceReconstruction => {
            required.insert(InsideEditIntent);
            required.insert(IndependentRendererAgreement);
            required.insert(ReconstructionDisclosure);
        }
    }
    required
}

fn internal_evidence(
    obligation: ProofObligationKind,
    status: ProofStatus,
    detail: impl Into<String>,
    subject_sha256: &str,
    artifact_sha256: Option<String>,
) -> ProofEvidence {
    let detail = detail.into();
    let mut digest = Sha256::new();
    digest.update(format!(
        "{obligation:?}\0{status:?}\0{detail}\0{subject_sha256}"
    ));
    if let Some(artifact) = &artifact_sha256 {
        digest.update(artifact.as_bytes());
    }
    ProofEvidence {
        obligation,
        status,
        detail,
        evidence_sha256: Some(format!("{:x}", digest.finalize())),
        producer: Some(EvidenceProducerIdentity {
            name: "wellfriendpdf-ecbes-universal".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            subject_sha256: subject_sha256.into(),
            artifact_sha256,
            independent: false,
        }),
    }
}

fn merge_evidence(
    external: &[ProofEvidence],
    generated: Vec<ProofEvidence>,
    output_sha256: &str,
) -> Result<Vec<ProofEvidence>> {
    let generated_kinds = generated
        .iter()
        .map(|item| item.obligation)
        .collect::<BTreeSet<_>>();
    let mut map = BTreeMap::new();
    for item in external {
        if generated_kinds.contains(&item.obligation) {
            continue;
        }
        if item.status == ProofStatus::Pass {
            let producer = item.producer.as_ref().ok_or_else(|| {
                invalid(format!(
                    "passing ECBES external evidence for {:?} requires producer identity",
                    item.obligation
                ))
            })?;
            if producer.subject_sha256 != output_sha256 {
                return Err(invalid(format!(
                    "passing ECBES external evidence for {:?} is bound to {}, not candidate output {}",
                    item.obligation, producer.subject_sha256, output_sha256
                )));
            }
            if producer.artifact_sha256.is_none() || item.evidence_sha256.is_none() {
                return Err(invalid(format!(
                    "passing ECBES external evidence for {:?} requires evidence and artifact SHA-256 digests",
                    item.obligation
                )));
            }
            if item.obligation == ProofObligationKind::IndependentRendererAgreement
                && !producer.independent
            {
                return Err(invalid(
                    "passing independent-renderer evidence must identify an independent producer",
                ));
            }
        }
        if map.insert(item.obligation, item.clone()).is_some() {
            return Err(invalid(format!(
                "ECBES external evidence repeats obligation {:?}",
                item.obligation
            )));
        }
    }
    for item in generated {
        map.insert(item.obligation, item);
    }
    Ok(map.into_values().collect())
}

fn ensure_required_evidence(
    evidence: &mut Vec<ProofEvidence>,
    fidelity: EditFidelityClass,
    policy: &EditSynthesisPolicy,
    subject_sha256: &str,
) {
    let mut present = evidence
        .iter()
        .map(|item| item.obligation)
        .collect::<BTreeSet<_>>();
    for obligation in required_obligations(fidelity)
        .into_iter()
        .chain(policy.required_obligations.iter().copied())
    {
        if obligation != ProofObligationKind::InfluenceClosure && present.insert(obligation) {
            evidence.push(internal_evidence(
                obligation,
                ProofStatus::Unverified,
                "no bound producer supplied this required evidence",
                subject_sha256,
                None,
            ));
        }
    }
    evidence.sort_by_key(|item| item.obligation);
}

fn rewritten_byte_count(input: &[u8], output: &[u8]) -> u64 {
    let prefix = input
        .iter()
        .zip(output)
        .take_while(|(left, right)| left == right)
        .count();
    let input_tail = &input[prefix..];
    let output_tail = &output[prefix..];
    let suffix = input_tail
        .iter()
        .rev()
        .zip(output_tail.iter().rev())
        .take_while(|(left, right)| left == right)
        .count();
    let input_changed = input_tail.len().saturating_sub(suffix);
    let output_changed = output_tail.len().saturating_sub(suffix);
    u64::try_from(input_changed.saturating_add(output_changed)).unwrap_or(u64::MAX)
}

fn parse_owner(value: &str) -> Option<SourceOwnerId> {
    let value = value.strip_prefix("object-")?;
    let mut parts = value.split('-');
    let object_number = parts.next()?.parse().ok()?;
    let generation = parts.next()?.parse().ok()?;
    parts.next().is_none().then_some(SourceOwnerId {
        object_number,
        generation,
    })
}

fn validate_reopened_structure(
    output: &[u8],
    credentials: Option<&UniversalOutputSecurityCredentialsV2>,
) -> Result<Value> {
    crate::cancel::check_current_cancel("ECBES structural reopen")?;
    let engine = match credentials {
        Some(credentials) => ContentEngine::open_bytes_with_password(
            output.to_vec(),
            credentials.user_password.as_slice(),
        )?,
        None => ContentEngine::open_bytes(output.to_vec())?,
    };
    let page_count = engine.page_count()?;
    for page in 1..=page_count {
        crate::cancel::check_current_cancel("ECBES page-tree validation")?;
        engine.get_page(page)?;
    }
    let object_ids = engine.document().reader().object_ids();
    for (number, generation) in &object_ids {
        crate::cancel::check_current_cancel("ECBES indirect-object validation")?;
        engine
            .document()
            .reader()
            .get_object(*number, *generation)?;
    }
    Ok(json!({
        "strict_reopen": true,
        "page_tree_pages_resolved": page_count,
        "in_use_indirect_objects_resolved": object_ids.len(),
        "semantic_meaning_certified": false,
    }))
}

fn recursively_find_positive_number(value: &Value, key: &str) -> bool {
    match value {
        Value::Object(map) => map.iter().any(|(name, value)| {
            (name == key && value.as_u64().is_some_and(|count| count > 0))
                || recursively_find_positive_number(value, key)
        }),
        Value::Array(values) => values
            .iter()
            .any(|value| recursively_find_positive_number(value, key)),
        _ => false,
    }
}

fn has_bound_render_oracle(policy: &UniversalEditPolicyV2) -> bool {
    policy
        .edit_contract
        .as_ref()
        .and_then(|contract| contract.render_oracle.as_ref())
        .is_some()
}

fn independent_render_evidence(
    policy: &UniversalEditPolicyV2,
    output_sha256: &str,
    operation_report: &Value,
) -> Result<Option<ProofEvidence>> {
    let Some(oracle) = policy
        .edit_contract
        .as_ref()
        .and_then(|contract| contract.render_oracle.as_ref())
    else {
        return Ok(None);
    };
    let Some(first) = oracle
        .reference_rasters
        .first()
        .and_then(|reference| reference.producer.as_ref())
    else {
        return Ok(None);
    };
    if !first.independent
        || oracle.reference_rasters.iter().any(|reference| {
            reference.producer.as_ref().is_none_or(|producer| {
                !producer.independent
                    || producer.name != first.name
                    || producer.version != first.version
                    || !producer
                        .artifact_sha256
                        .eq_ignore_ascii_case(&digest_hex(&reference.rgba))
            })
        })
    {
        return Ok(None);
    }
    let mut manifest = oracle
        .reference_rasters
        .iter()
        .map(|reference| {
            let producer = reference.producer.as_ref().ok_or_else(|| {
                invalid("independent-render producer disappeared after completeness check")
            })?;
            Ok(format!(
                "{}:{}",
                reference.page,
                producer.artifact_sha256.to_ascii_lowercase()
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    manifest.sort_unstable();
    Ok(Some(ProofEvidence {
        obligation: ProofObligationKind::IndependentRendererAgreement,
        status: ProofStatus::Pass,
        detail: "the canonical edit contract compared every selected output page with hash-verified rasters from one independently declared renderer producer".into(),
        evidence_sha256: Some(digest_hex(&json_bytes(
            operation_report,
            "independent-render comparison report",
        )?)),
        producer: Some(EvidenceProducerIdentity {
            name: first.name.clone(),
            version: first.version.clone(),
            subject_sha256: output_sha256.into(),
            artifact_sha256: Some(digest_hex(manifest.join("\0").as_bytes())),
            independent: true,
        }),
    }))
}

fn raster_contract_covers_document(
    policy: &UniversalEditPolicyV2,
    input_page_count: usize,
    output_page_count: usize,
) -> bool {
    let Some(raster) = policy
        .edit_contract
        .as_ref()
        .and_then(|contract| contract.raster_preservation.as_ref())
    else {
        return false;
    };
    let input_pages = raster
        .pages
        .iter()
        .map(|mapping| mapping.input_page)
        .collect::<BTreeSet<_>>();
    let output_pages = raster
        .pages
        .iter()
        .map(|mapping| mapping.output_page)
        .collect::<BTreeSet<_>>();
    input_pages.len() == raster.pages.len()
        && output_pages.len() == raster.pages.len()
        && input_pages == (1..=input_page_count).collect::<BTreeSet<_>>()
        && output_pages == (1..=output_page_count).collect::<BTreeSet<_>>()
}

fn make_failure_evidence(
    fidelity: EditFidelityClass,
    policy: &EditSynthesisPolicy,
    input_sha256: &str,
    detail: &str,
) -> Vec<ProofEvidence> {
    let obligations = required_obligations(fidelity)
        .into_iter()
        .chain(policy.required_obligations.iter().copied())
        .collect::<BTreeSet<_>>();
    let mut evidence = obligations
        .into_iter()
        .filter(|kind| *kind != ProofObligationKind::InfluenceClosure)
        .map(|obligation| {
            internal_evidence(
                obligation,
                if obligation == ProofObligationKind::SecurityAuthority {
                    ProofStatus::Fail
                } else {
                    ProofStatus::Unverified
                },
                detail,
                input_sha256,
                None,
            )
        })
        .collect::<Vec<_>>();
    evidence.sort_by_key(|item| item.obligation);
    evidence
}

fn validate_policy(
    request: &EcbesUniversalEditRequest,
    candidates: &[PreparedCandidate],
) -> Result<()> {
    if candidates.is_empty() {
        return Err(invalid(
            "ECBES universal edit requires at least one explicit or automatic candidate",
        ));
    }
    if candidates.len() > MAX_MATERIALIZED_CANDIDATES
        || candidates.len() > request.policy.synthesis.max_candidates
    {
        return Err(WellfriendError::ResourceLimit(format!(
            "ECBES universal edit candidate count {} exceeds materialization/policy limits",
            candidates.len()
        )));
    }
    if request.policy.max_candidate_output_bytes == 0
        || request.policy.max_candidate_output_bytes > HARD_MAX_CANDIDATE_OUTPUT_BYTES
        || request.policy.max_total_output_bytes == 0
        || request.policy.max_total_output_bytes > HARD_MAX_TOTAL_OUTPUT_BYTES
        || request.policy.max_total_output_bytes < request.policy.max_candidate_output_bytes
    {
        return Err(invalid(
            "ECBES output budgets must be positive, within hard limits, and total must cover one candidate",
        ));
    }
    for automatic in &request.automatic_candidates {
        let allowed_route_keys = if matches!(
            &automatic.request.operation,
            UniversalEditOperationV2::Text { .. }
        ) {
            BTreeSet::from(["operator", "geometric", "semantic"])
        } else {
            BTreeSet::from(["canonical"])
        };
        for key in automatic
            .route_approvals
            .keys()
            .chain(automatic.route_external_evidence.keys())
        {
            if !allowed_route_keys.contains(key.as_str()) {
                return Err(invalid(format!(
                    "ECBES automatic route key '{key}' is not valid for this operation family"
                )));
            }
        }
    }
    let mut ids = BTreeSet::new();
    for candidate in candidates {
        let candidate = &candidate.request;
        if candidate.id.is_empty()
            || candidate.id.len() > 256
            || candidate.id.chars().any(char::is_control)
            || !ids.insert(candidate.id.as_str())
        {
            return Err(invalid(
                "ECBES candidate IDs must be unique 1..=256-byte non-control strings",
            ));
        }
    }
    Ok(())
}

fn materialize_candidate(
    input: &[u8],
    input_sha256: &str,
    input_page_count: usize,
    candidate: &EcbesUniversalCandidateRequest,
    origin: &str,
    policy: &EcbesUniversalEditPolicy,
    output_credentials: Option<&UniversalOutputSecurityCredentialsV2>,
) -> Result<MaterializedCandidate> {
    let started = Instant::now();
    let request_sha256 = json_sha256(&candidate.request, "candidate request")?;
    let minimum = minimum_fidelity(&candidate.request.operation);
    let mut report = EcbesUniversalCandidateReport {
        id: candidate.id.clone(),
        origin: origin.into(),
        requested_fidelity: candidate.fidelity,
        minimum_fidelity: minimum,
        request_sha256,
        plan_id: None,
        plan_sha256: None,
        plan_state: None,
        approval_binding_digest: None,
        universal_outcome: None,
        output_sha256: None,
        output_bytes: None,
        affected_pages: Vec::new(),
        affected_objects: Vec::new(),
        evidence: Vec::new(),
        cost: EditCostVector {
            semantic_distance_microunits: candidate.cost_hints.semantic_distance_microunits,
            layout_displacement_micropoints: candidate.cost_hints.layout_displacement_micropoints,
            font_substitution_penalty: candidate.cost_hints.font_substitution_penalty,
            reconstruction_uncertainty_ppm: candidate.cost_hints.reconstruction_uncertainty_ppm,
            ..EditCostVector::default()
        },
        elapsed_millis: 0,
        error_code: None,
        error: None,
        universal_report: None,
    };

    if fidelity_rank(candidate.fidelity) < fidelity_rank(minimum) {
        let message = format!(
            "candidate labels {:?} as {:?}; a lower-fidelity disclosure is required",
            minimum, candidate.fidelity
        );
        report.error_code = Some("fidelity_understatement".into());
        report.error = Some(message.clone());
        report.evidence = make_failure_evidence(
            candidate.fidelity,
            &policy.synthesis,
            input_sha256,
            &message,
        );
        report.elapsed_millis = started.elapsed().as_millis();
        return Ok(MaterializedCandidate {
            edit_candidate: EditCandidate {
                id: candidate.id.clone(),
                fidelity: candidate.fidelity,
                owners: Vec::new(),
                affected_pages: Vec::new(),
                influence_nodes: Vec::new(),
                evidence: report.evidence.clone(),
                cost: report.cost.clone(),
            },
            report,
            output: None,
        });
    }

    let plan = match plan_universal_edit_v2(input, &candidate.request) {
        Ok(plan) => plan,
        Err(error) if error.kind() != ErrorKind::Cancelled => {
            let message = error.to_string();
            report.error_code = Some(error.code().into());
            report.error = Some(message.clone());
            report.evidence = make_failure_evidence(
                candidate.fidelity,
                &policy.synthesis,
                input_sha256,
                &message,
            );
            report.elapsed_millis = started.elapsed().as_millis();
            return Ok(MaterializedCandidate {
                edit_candidate: EditCandidate {
                    id: candidate.id.clone(),
                    fidelity: candidate.fidelity,
                    owners: Vec::new(),
                    affected_pages: Vec::new(),
                    influence_nodes: Vec::new(),
                    evidence: report.evidence.clone(),
                    cost: report.cost.clone(),
                },
                report,
                output: None,
            });
        }
        Err(error) => return Err(error),
    };
    let plan_sha256 = json_sha256(&plan, "universal plan")?;
    report.plan_id = Some(plan.plan_id.clone());
    report.plan_sha256 = Some(plan_sha256.clone());
    report.plan_state = Some(plan.state);
    let execution_minimum = minimum_fidelity(&plan.execution_operation);
    report.minimum_fidelity = execution_minimum;
    if fidelity_rank(candidate.fidelity) < fidelity_rank(execution_minimum) {
        let message = format!(
            "canonical planning escalated the route to {:?}, but candidate '{}' is labeled {:?}",
            execution_minimum, candidate.id, candidate.fidelity
        );
        report.error_code = Some("planned_fidelity_understatement".into());
        report.error = Some(message.clone());
        report.evidence = make_failure_evidence(
            candidate.fidelity,
            &policy.synthesis,
            input_sha256,
            &message,
        );
        report.elapsed_millis = started.elapsed().as_millis();
        return Ok(MaterializedCandidate {
            edit_candidate: EditCandidate {
                id: candidate.id.clone(),
                fidelity: candidate.fidelity,
                owners: Vec::new(),
                affected_pages: Vec::new(),
                influence_nodes: Vec::new(),
                evidence: report.evidence.clone(),
                cost: report.cost.clone(),
            },
            report,
            output: None,
        });
    }

    let approval = match plan.state {
        UniversalPlanStateV2::Ready => None,
        UniversalPlanStateV2::ApprovalRequired => {
            let Some(decision) = candidate.approval.clone() else {
                let message =
                    "canonical universal candidate requires an explicit approval decision";
                report.error_code = Some("approval_required".into());
                report.error = Some(message.into());
                report.evidence = make_failure_evidence(
                    candidate.fidelity,
                    &policy.synthesis,
                    input_sha256,
                    message,
                );
                report.elapsed_millis = started.elapsed().as_millis();
                return Ok(MaterializedCandidate {
                    edit_candidate: EditCandidate {
                        id: candidate.id.clone(),
                        fidelity: candidate.fidelity,
                        owners: Vec::new(),
                        affected_pages: Vec::new(),
                        influence_nodes: Vec::new(),
                        evidence: report.evidence.clone(),
                        cost: report.cost.clone(),
                    },
                    report,
                    output: None,
                });
            };
            let token = create_universal_approval_token_v2(&plan, decision)?;
            report.approval_binding_digest = Some(token.binding_digest.clone());
            Some(token)
        }
        state => {
            let message = format!("canonical universal plan is not executable: {state:?}");
            report.error_code = Some("universal_plan_not_executable".into());
            report.error = Some(message.clone());
            report.evidence = make_failure_evidence(
                candidate.fidelity,
                &policy.synthesis,
                input_sha256,
                &message,
            );
            report.elapsed_millis = started.elapsed().as_millis();
            return Ok(MaterializedCandidate {
                edit_candidate: EditCandidate {
                    id: candidate.id.clone(),
                    fidelity: candidate.fidelity,
                    owners: Vec::new(),
                    affected_pages: Vec::new(),
                    influence_nodes: Vec::new(),
                    evidence: report.evidence.clone(),
                    cost: report.cost.clone(),
                },
                report,
                output: None,
            });
        }
    };

    crate::cancel::check_current_cancel("ECBES candidate apply")?;
    let applied = match &plan.policy.output_security {
        UniversalOutputSecurityPolicyV2::Unencrypted => {
            apply_universal_edit_v2(input, &plan, approval.as_ref())
        }
        UniversalOutputSecurityPolicyV2::Standard { .. } => {
            let Some(credentials) = output_credentials else {
                let message = "secured ECBES candidate requires apply-only output credentials";
                report.error_code = Some("output_credentials_required".into());
                report.error = Some(message.into());
                report.evidence = make_failure_evidence(
                    candidate.fidelity,
                    &policy.synthesis,
                    input_sha256,
                    message,
                );
                report.elapsed_millis = started.elapsed().as_millis();
                return Ok(MaterializedCandidate {
                    edit_candidate: EditCandidate {
                        id: candidate.id.clone(),
                        fidelity: candidate.fidelity,
                        owners: Vec::new(),
                        affected_pages: Vec::new(),
                        influence_nodes: Vec::new(),
                        evidence: report.evidence.clone(),
                        cost: report.cost.clone(),
                    },
                    report,
                    output: None,
                });
            };
            apply_universal_edit_v2_with_output_security(
                input,
                &plan,
                approval.as_ref(),
                credentials,
            )
        }
    };
    let (output, universal_result) = match applied {
        Ok(value) => value,
        Err(error) if error.kind() != ErrorKind::Cancelled => {
            let message = error.to_string();
            report.error_code = Some(error.code().into());
            report.error = Some(message.clone());
            report.evidence = make_failure_evidence(
                candidate.fidelity,
                &policy.synthesis,
                input_sha256,
                &message,
            );
            report.elapsed_millis = started.elapsed().as_millis();
            return Ok(MaterializedCandidate {
                edit_candidate: EditCandidate {
                    id: candidate.id.clone(),
                    fidelity: candidate.fidelity,
                    owners: Vec::new(),
                    affected_pages: Vec::new(),
                    influence_nodes: Vec::new(),
                    evidence: report.evidence.clone(),
                    cost: report.cost.clone(),
                },
                report,
                output: None,
            });
        }
        Err(error) => return Err(error),
    };
    report.universal_outcome = Some(universal_result.outcome);
    report.affected_pages = universal_result.affected_pages.clone();
    report.affected_objects = universal_result.affected_objects.clone();
    report.universal_report = Some(serde_json::to_value(&universal_result).map_err(|error| {
        invalid(format!(
            "ECBES universal result serialization failed: {error}"
        ))
    })?);

    if !universal_result.changed || universal_result.outcome != UniversalEditOutcomeV2::Applied {
        let message = "universal candidate returned a governed no-change outcome";
        report.error_code = Some("candidate_not_applied".into());
        report.error = Some(message.into());
        report.evidence =
            make_failure_evidence(candidate.fidelity, &policy.synthesis, input_sha256, message);
        report.elapsed_millis = started.elapsed().as_millis();
        return Ok(MaterializedCandidate {
            edit_candidate: EditCandidate {
                id: candidate.id.clone(),
                fidelity: candidate.fidelity,
                owners: Vec::new(),
                affected_pages: report.affected_pages.clone(),
                influence_nodes: Vec::new(),
                evidence: report.evidence.clone(),
                cost: report.cost.clone(),
            },
            report,
            output: None,
        });
    }

    let output_len = u64::try_from(output.len()).unwrap_or(u64::MAX);
    if output_len > policy.max_candidate_output_bytes {
        let message = format!(
            "candidate output {output_len} bytes exceeds per-candidate limit {}",
            policy.max_candidate_output_bytes
        );
        report.error_code = Some("candidate_output_limit".into());
        report.error = Some(message.clone());
        report.evidence = make_failure_evidence(
            candidate.fidelity,
            &policy.synthesis,
            input_sha256,
            &message,
        );
        report.elapsed_millis = started.elapsed().as_millis();
        return Ok(MaterializedCandidate {
            edit_candidate: EditCandidate {
                id: candidate.id.clone(),
                fidelity: candidate.fidelity,
                owners: Vec::new(),
                affected_pages: report.affected_pages.clone(),
                influence_nodes: Vec::new(),
                evidence: report.evidence.clone(),
                cost: report.cost.clone(),
            },
            report,
            output: None,
        });
    }

    let output_sha256 = digest_hex(&output);
    let structure_report = validate_reopened_structure(
        &output,
        matches!(
            plan.policy.output_security,
            UniversalOutputSecurityPolicyV2::Standard { .. }
        )
        .then_some(output_credentials)
        .flatten(),
    )?;
    let structure_sha256 = json_sha256(&structure_report, "structure report")?;
    let output_page_count = structure_report["page_tree_pages_resolved"]
        .as_u64()
        .and_then(|count| usize::try_from(count).ok())
        .ok_or_else(|| invalid("ECBES strict-reopen report omitted output page count"))?;
    let plan_evidence_sha256 = digest_hex(
        format!(
            "{}\0{}\0{}",
            plan.plan_id,
            plan_sha256,
            report.approval_binding_digest.as_deref().unwrap_or("ready")
        )
        .as_bytes(),
    );
    let mut generated = vec![
        internal_evidence(
            ProofObligationKind::SecurityAuthority,
            ProofStatus::Pass,
            "canonical universal signature policy and any required revision-bound approval passed",
            &plan_sha256,
            Some(plan_evidence_sha256),
        ),
        internal_evidence(
            ProofObligationKind::SourceRevision,
            ProofStatus::Pass,
            format!("candidate plan is bound to input revision {}", plan.revision_id),
            input_sha256,
            Some(plan_sha256.clone()),
        ),
        internal_evidence(
            ProofObligationKind::Reopen,
            ProofStatus::Pass,
            "candidate output reopened with the declared output-security policy",
            &output_sha256,
            Some(structure_sha256.clone()),
        ),
        internal_evidence(
            ProofObligationKind::LogicalPostcondition,
            ProofStatus::Pass,
            "canonical universal route returned Applied after its route-specific postconditions",
            &output_sha256,
            Some(digest_hex(&json_bytes(&universal_result, "universal result")?)),
        ),
        internal_evidence(
            ProofObligationKind::StructureIntegrity,
            ProofStatus::Pass,
            "strict reopen resolved every page and every in-use indirect object; semantic meaning is not certified by this check",
            &output_sha256,
            Some(structure_sha256),
        ),
        internal_evidence(
            ProofObligationKind::ResourceBudget,
            ProofStatus::Pass,
            format!(
                "candidate output {output_len} bytes is within the configured byte and candidate-count budgets"
            ),
            &output_sha256,
            None,
        ),
    ];

    if raster_contract_covers_document(
        &candidate.request.policy,
        input_page_count,
        output_page_count,
    ) {
        generated.push(internal_evidence(
            ProofObligationKind::OutsideInfluencePixels,
            ProofStatus::Pass,
            "the bound edit contract rendered input/output and rejected every pixel change outside the declared device-space edit regions",
            &output_sha256,
            Some(digest_hex(&json_bytes(
                &universal_result.operation_report,
                "raster-preservation report",
            )?)),
        ));
    }

    if candidate.fidelity != EditFidelityClass::SourceNative {
        generated.push(internal_evidence(
            ProofObligationKind::ReconstructionDisclosure,
            ProofStatus::Pass,
            format!(
                "candidate is permanently labeled {:?}; native source recovery is not claimed",
                candidate.fidelity
            ),
            &output_sha256,
            Some(request_sha256_for(&candidate.request)?),
        ));
    }
    if candidate.fidelity == EditFidelityClass::SourceNative
        && recursively_find_positive_number(
            &universal_result.operation_report,
            "exact_parsed_model_lens_law_checks",
        )
    {
        generated.push(internal_evidence(
            ProofObligationKind::LensRoundTrip,
            ProofStatus::Pass,
            "universal object lens executed exact parsed-model put/get and inverse-preservation checks",
            &output_sha256,
            Some(digest_hex(&json_bytes(
                &universal_result.operation_report,
                "lens report",
            )?)),
        ));
    }
    if let Some(evidence) = independent_render_evidence(
        &candidate.request.policy,
        &output_sha256,
        &universal_result.operation_report,
    )? {
        generated.push(evidence);
    }
    if has_bound_render_oracle(&candidate.request.policy)
        && candidate.fidelity == EditFidelityClass::AppearanceReconstruction
    {
        generated.push(internal_evidence(
            ProofObligationKind::InsideEditIntent,
            ProofStatus::Pass,
            "bound output raster oracle and route-specific logical postconditions passed",
            &output_sha256,
            candidate
                .request
                .policy
                .edit_contract
                .as_ref()
                .map(|contract| json_sha256(contract, "edit contract"))
                .transpose()?,
        ));
    }

    let mut evidence = merge_evidence(&candidate.external_evidence, generated, &output_sha256)?;
    ensure_required_evidence(
        &mut evidence,
        candidate.fidelity,
        &policy.synthesis,
        &output_sha256,
    );
    let owners = universal_result
        .affected_objects
        .iter()
        .filter_map(|value| parse_owner(value))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    report.cost.changed_indirect_objects = u32::try_from(owners.len()).unwrap_or(u32::MAX);
    report.cost.rewritten_opaque_bytes = rewritten_byte_count(input, &output);
    report.output_sha256 = Some(output_sha256);
    report.output_bytes = Some(output_len);
    report.evidence = evidence.clone();
    report.elapsed_millis = started.elapsed().as_millis();
    Ok(MaterializedCandidate {
        edit_candidate: EditCandidate {
            id: candidate.id.clone(),
            fidelity: candidate.fidelity,
            owners,
            affected_pages: universal_result.affected_pages,
            influence_nodes: Vec::new(),
            evidence,
            cost: report.cost.clone(),
        },
        report,
        output: Some(output),
    })
}

fn request_sha256_for(request: &UniversalEditRequestV2) -> Result<String> {
    json_sha256(request, "candidate request")
}

fn plan_derived_graph(
    revision_id: &str,
    materialized: &[MaterializedCandidate],
) -> (EditInfluenceGraph, String) {
    let mut intent_material = materialized
        .iter()
        .map(|candidate| candidate.report.request_sha256.as_str())
        .collect::<Vec<_>>();
    intent_material.sort_unstable();
    let root = format!(
        "intent:{}",
        digest_hex(format!("{revision_id}\0{}", intent_material.join("\0")).as_bytes())
    );
    let revision = format!("revision:{revision_id}");
    let mut nodes = BTreeSet::from([root.clone(), revision.clone()]);
    let mut edge_set = BTreeSet::from([(root.clone(), revision, InfluenceEdgeKind::SemanticOwner)]);
    for candidate in materialized {
        let operation = format!("operation:{}", candidate.report.request_sha256);
        nodes.insert(operation.clone());
        edge_set.insert((
            root.clone(),
            operation.clone(),
            InfluenceEdgeKind::SemanticOwner,
        ));
        let pages = candidate
            .report
            .affected_pages
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        for page in &pages {
            let page_node = format!("page:{page}");
            nodes.insert(page_node.clone());
            edge_set.insert((operation.clone(), page_node, InfluenceEdgeKind::Layout));
        }
        for owner in &candidate.edit_candidate.owners {
            let object = format!("object:{}:{}", owner.object_number, owner.generation);
            nodes.insert(object.clone());
            edge_set.insert((
                operation.clone(),
                object.clone(),
                InfluenceEdgeKind::Resource,
            ));
            for page in &pages {
                edge_set.insert((
                    object.clone(),
                    format!("page:{page}"),
                    InfluenceEdgeKind::PaintOrder,
                ));
            }
        }
    }
    let edges = edge_set
        .into_iter()
        .map(|(from, to, kind)| InfluenceEdge { from, to, kind })
        .collect();
    (EditInfluenceGraph { nodes, edges }, root)
}

fn receipt_digest(receipt: &EcbesPublicationReceipt) -> Result<String> {
    #[derive(Serialize)]
    struct Material<'a> {
        schema_version: &'a str,
        input_sha256: &'a str,
        selected_candidate_id: &'a Option<String>,
        selected_fidelity: &'a Option<EditFidelityClass>,
        selected_output_sha256: &'a str,
        decision_sha256: &'a str,
        published: bool,
        transport: &'a Value,
    }
    json_sha256(
        &Material {
            schema_version: &receipt.schema_version,
            input_sha256: &receipt.input_sha256,
            selected_candidate_id: &receipt.selected_candidate_id,
            selected_fidelity: &receipt.selected_fidelity,
            selected_output_sha256: &receipt.selected_output_sha256,
            decision_sha256: &receipt.decision_sha256,
            published: receipt.published,
            transport: &receipt.transport,
        },
        "publication receipt",
    )
}

/// Materialize, validate, select, and publish one ECBES universal-edit
/// candidate. All candidates execute from the same immutable input revision.
pub fn execute_ecbes_universal_edit(
    input: &[u8],
    request: &EcbesUniversalEditRequest,
    output_credentials: Option<&UniversalOutputSecurityCredentialsV2>,
) -> Result<(Vec<u8>, EcbesUniversalEditReport)> {
    crate::cancel::check_current_cancel("ECBES universal edit entry")?;
    let prepared = prepare_candidates(request);
    validate_policy(request, &prepared)?;
    let input_sha256 = digest_hex(input);
    let snapshot = crate::editing_transactions::build_document_snapshot(input, None)?;
    let input_page_count = ContentEngine::open_bytes(input.to_vec())?.page_count()?;
    let mut materialized = Vec::with_capacity(prepared.len());
    let mut total_output_bytes = 0u64;
    for candidate in &prepared {
        crate::cancel::check_current_cancel("ECBES candidate materialization")?;
        let item = materialize_candidate(
            input,
            &input_sha256,
            input_page_count,
            &candidate.request,
            candidate.origin,
            &request.policy,
            output_credentials,
        )?;
        if let Some(output) = &item.output {
            total_output_bytes = total_output_bytes
                .checked_add(u64::try_from(output.len()).unwrap_or(u64::MAX))
                .ok_or_else(|| {
                    WellfriendError::ResourceLimit(
                        "ECBES total candidate output byte count overflow".into(),
                    )
                })?;
            if total_output_bytes > request.policy.max_total_output_bytes {
                return Err(WellfriendError::ResourceLimit(format!(
                    "ECBES materialized outputs exceed total limit {}",
                    request.policy.max_total_output_bytes
                )));
            }
        }
        materialized.push(item);
    }

    let (graph, root) = plan_derived_graph(&snapshot.revision_id, &materialized);
    let cone_nodes = graph.nodes.iter().cloned().collect::<Vec<_>>();
    for item in &mut materialized {
        item.edit_candidate.influence_nodes = cone_nodes.clone();
    }
    let decision = crate::research_edit_synthesis::synthesize_evidence_constrained_edit(
        EvidenceConstrainedEditRequest {
            max_influence_nodes: graph.nodes.len().max(1),
            graph,
            seeds: vec![root],
            policy: request.policy.synthesis.clone(),
            candidates: materialized
                .iter()
                .map(|item| item.edit_candidate.clone())
                .collect(),
        },
    )?;
    let selected_id = decision
        .synthesis
        .selected
        .as_ref()
        .map(|candidate| candidate.id.clone());
    let selected_fidelity = decision
        .synthesis
        .selected
        .as_ref()
        .map(|candidate| candidate.fidelity);
    let output = selected_id
        .as_ref()
        .and_then(|id| {
            materialized
                .iter_mut()
                .find(|item| item.report.id == *id)
                .and_then(|item| item.output.take())
        })
        .unwrap_or_else(|| input.to_vec());
    let output_sha256 = digest_hex(&output);
    let published = selected_id.is_some();
    let mut publication = EcbesPublicationReceipt {
        schema_version: ECBES_UNIVERSAL_SCHEMA_VERSION.into(),
        input_sha256: input_sha256.clone(),
        selected_candidate_id: selected_id,
        selected_fidelity,
        selected_output_sha256: output_sha256,
        decision_sha256: decision.synthesis.decision_sha256.clone(),
        published,
        transport: json!({
            "returned_exact_original_container": !published,
            "planning_input_was_transport_normalized": false,
        }),
        receipt_sha256: String::new(),
    };
    publication.receipt_sha256 = receipt_digest(&publication)?;
    let report = EcbesUniversalEditReport {
        schema_version: ECBES_UNIVERSAL_SCHEMA_VERSION.into(),
        input_sha256,
        input_revision_id: snapshot.revision_id,
        conservative_influence_scope: "plan-derived intent, affected-owner and affected-page provenance; completeness is bounded by canonical universal plan/apply reports and every outside pixel still requires whole-document raster preservation or bound external evidence".into(),
        decision,
        candidates: materialized.into_iter().map(|item| item.report).collect(),
        publication,
    };
    Ok((output, report))
}

/// Preserve the caller's exact encrypted or otherwise normalized transport when
/// ECBES selects no candidate, and rebind the publication receipt accordingly.
pub fn preserve_ecbes_no_change_transport(
    original: &[u8],
    planning_input: &[u8],
    output: Vec<u8>,
    report: &mut EcbesUniversalEditReport,
) -> Result<Vec<u8>> {
    if report.publication.published || original == planning_input {
        return Ok(output);
    }
    report.publication.selected_output_sha256 = digest_hex(original);
    report.publication.transport = json!({
        "returned_exact_original_container": true,
        "planning_input_was_transport_normalized": true,
        "planning_input_sha256": digest_hex(planning_input),
        "returned_transport_sha256": digest_hex(original),
    });
    report.publication.receipt_sha256 = receipt_digest(&report.publication)?;
    Ok(original.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fidelity_classifier_never_labels_visible_ocr_as_native() {
        let request = crate::document_subsystems::DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::OcrReconstruction,
            action: None,
            reflow: None,
            approved: false,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        assert_eq!(
            minimum_fidelity(&UniversalEditOperationV2::DocumentSubsystem { request }),
            EditFidelityClass::AppearanceReconstruction
        );
    }

    #[test]
    fn rewritten_byte_count_ignores_equal_prefix_and_suffix() {
        assert_eq!(rewritten_byte_count(b"abcOLDxyz", b"abcNEWxyz"), 6);
        assert_eq!(rewritten_byte_count(b"same", b"same"), 0);
    }

    #[test]
    fn publication_receipt_digest_changes_with_selection() {
        let mut receipt = EcbesPublicationReceipt {
            schema_version: ECBES_UNIVERSAL_SCHEMA_VERSION.into(),
            input_sha256: "0".repeat(64),
            selected_candidate_id: None,
            selected_fidelity: None,
            selected_output_sha256: "0".repeat(64),
            decision_sha256: "1".repeat(64),
            published: false,
            transport: json!({"returned_exact_original_container": true}),
            receipt_sha256: String::new(),
        };
        let before = receipt_digest(&receipt).unwrap();
        receipt.selected_candidate_id = Some("native".into());
        receipt.selected_fidelity = Some(EditFidelityClass::SourceNative);
        receipt.published = true;
        assert_ne!(before, receipt_digest(&receipt).unwrap());
    }

    #[test]
    fn independent_renderer_pass_requires_independent_output_binding() {
        let output_sha256 = "a".repeat(64);
        let mut item = ProofEvidence {
            obligation: ProofObligationKind::IndependentRendererAgreement,
            status: ProofStatus::Pass,
            detail: "external renderer comparison passed".into(),
            evidence_sha256: Some("b".repeat(64)),
            producer: Some(EvidenceProducerIdentity {
                name: "reference-renderer".into(),
                version: "1".into(),
                subject_sha256: output_sha256.clone(),
                artifact_sha256: Some("c".repeat(64)),
                independent: false,
            }),
        };
        assert!(merge_evidence(&[item.clone()], Vec::new(), &output_sha256).is_err());
        item.producer.as_mut().unwrap().independent = true;
        assert!(merge_evidence(&[item], Vec::new(), &output_sha256).is_ok());
    }

    #[test]
    fn passing_external_evidence_is_bound_to_exact_candidate_output() {
        let item = ProofEvidence {
            obligation: ProofObligationKind::LensRoundTrip,
            status: ProofStatus::Pass,
            detail: "external lens check passed".into(),
            evidence_sha256: Some("b".repeat(64)),
            producer: Some(EvidenceProducerIdentity {
                name: "lens-checker".into(),
                version: "1".into(),
                subject_sha256: "d".repeat(64),
                artifact_sha256: Some("c".repeat(64)),
                independent: false,
            }),
        };
        assert!(merge_evidence(&[item], Vec::new(), &"a".repeat(64)).is_err());
    }

    #[test]
    fn automatic_text_intent_expands_all_canonical_fidelity_routes() {
        let automatic = EcbesAutomaticCandidateRequest {
            id_prefix: "replace-heading".into(),
            request: UniversalEditRequestV2 {
                operation: UniversalEditOperationV2::Text {
                    request: crate::editing_transactions::SceneTextEditRequest::default(),
                },
                policy: UniversalEditPolicyV2::default(),
            },
            approval: None,
            route_approvals: BTreeMap::new(),
            cost_hints: EcbesMeasuredCostHints::default(),
            route_external_evidence: BTreeMap::new(),
        };
        let generated = automatic_candidates(&[automatic]);
        assert_eq!(generated.len(), 3);
        assert_eq!(generated[0].request.id, "replace-heading-operator");
        assert_eq!(generated[1].request.id, "replace-heading-geometric");
        assert_eq!(generated[2].request.id, "replace-heading-semantic");
        assert_eq!(
            generated[0].request.fidelity,
            EditFidelityClass::SourceNative
        );
        assert_eq!(
            generated[1].request.fidelity,
            EditFidelityClass::SemanticReconstruction
        );
    }

    #[test]
    fn independent_reference_evidence_binds_exact_rgba_artifact() {
        let rgba = vec![255, 255, 255, 255];
        let producer = crate::universal_editing::UniversalReferenceRasterProducerV2 {
            name: "reference-renderer".into(),
            version: "1.0".into(),
            artifact_sha256: digest_hex(&rgba),
            independent: true,
        };
        let oracle = crate::universal_editing::UniversalRenderQualificationOptionsV2 {
            pages: vec![1],
            reference_rasters: vec![crate::universal_editing::UniversalReferenceRasterV2 {
                page: 1,
                width: 1,
                height: 1,
                rgba,
                max_mean_absolute_error: 0.0,
                min_ssim: 1.0,
                max_channel_error: 0,
                producer: Some(producer),
            }],
            ..Default::default()
        };
        let mut contract = crate::edit_contracts::EditContract::default();
        contract.render_oracle = Some(oracle);
        let policy = UniversalEditPolicyV2 {
            edit_contract: Some(contract),
            ..Default::default()
        };
        let evidence =
            independent_render_evidence(&policy, &"a".repeat(64), &json!({"status": "passed"}))
                .unwrap()
                .unwrap();
        assert_eq!(
            evidence.obligation,
            ProofObligationKind::IndependentRendererAgreement
        );
        assert!(evidence.producer.unwrap().independent);
    }
}
