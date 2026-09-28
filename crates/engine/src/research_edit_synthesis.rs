//! Evidence-Constrained Bidirectional Edit Synthesis (ECBES).
//!
//! This module is the deterministic research kernel for selecting between
//! competing PDF edit constructions. It does not infer edits or mutate PDF
//! bytes. Existing source writers, layout engines, renderers, extractors and
//! validators produce candidates and evidence; ECBES computes the bounded
//! influence cone, rejects candidates with missing proof obligations, retains
//! the Pareto frontier and publishes one reproducible decision certificate.
//!
//! The fidelity class is part of the result. Appearance reconstruction can make
//! the planner total for a visible edit, but can never be reported as a native
//! source edit.

use crate::{Result, WellfriendError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const MAX_GRAPH_NODES: usize = 1_000_000;
const MAX_GRAPH_EDGES: usize = 4_000_000;
const MAX_INFLUENCE_NODES: usize = 250_000;
const MAX_CANDIDATES: usize = 4_096;
const MAX_EVIDENCE_DETAIL_BYTES: usize = 16 * 1024;
const MAX_ID_BYTES: usize = 256;

fn invalid(message: impl Into<String>) -> WellfriendError {
    WellfriendError::invalid_input(message.into())
}

fn resource(message: impl Into<String>) -> WellfriendError {
    WellfriendError::ResourceLimit(message.into())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditFidelityClass {
    /// Existing PDF source operators/resources are edited through exact owners.
    SourceNative,
    /// Missing logical structure is reconstructed, but the result remains real
    /// searchable/selectable PDF content with disclosed substitutions.
    SemanticReconstruction,
    /// Pixels or outlines are reconstructed and paired with newly authored PDF
    /// semantics. This is never described as recovery of missing source.
    AppearanceReconstruction,
}

impl EditFidelityClass {
    fn rank(self) -> u8 {
        match self {
            Self::SourceNative => 0,
            Self::SemanticReconstruction => 1,
            Self::AppearanceReconstruction => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InfluenceEdgeKind {
    PaintOrder,
    GraphicsState,
    Clip,
    SoftMask,
    Resource,
    FormInvocation,
    Layout,
    SemanticOwner,
    Annotation,
    PageTree,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InfluenceEdge {
    pub from: String,
    pub to: String,
    pub kind: InfluenceEdgeKind,
}

/// Directed provenance graph. An edge `from -> to` means changing `from` can
/// affect the rendered or semantic interpretation of `to`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditInfluenceGraph {
    pub nodes: BTreeSet<String>,
    pub edges: Vec<InfluenceEdge>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InfluenceCone {
    pub seeds: Vec<String>,
    pub nodes: Vec<String>,
    pub edges: Vec<InfluenceEdge>,
    /// Nodes immediately outside the cone that consume a cone node. With the
    /// current transitive closure this is normally empty; it becomes meaningful
    /// when `max_nodes` deliberately truncates a diagnostic query.
    pub boundary: Vec<String>,
}

impl EditInfluenceGraph {
    pub fn validate(&self) -> Result<()> {
        if self.nodes.len() > MAX_GRAPH_NODES {
            return Err(resource("edit influence graph exceeds 1,000,000 nodes"));
        }
        if self.edges.len() > MAX_GRAPH_EDGES {
            return Err(resource("edit influence graph exceeds 4,000,000 edges"));
        }
        for node in &self.nodes {
            validate_id(node, "influence node")?;
        }
        let mut unique = BTreeSet::new();
        for edge in &self.edges {
            if !self.nodes.contains(&edge.from) || !self.nodes.contains(&edge.to) {
                return Err(invalid("influence edge references an unknown node"));
            }
            if !unique.insert((&edge.from, &edge.to, edge.kind)) {
                return Err(invalid("influence graph contains a duplicate edge"));
            }
        }
        Ok(())
    }

    /// Compute the deterministic transitive impact closure. The operation
    /// fails rather than returning a silently incomplete cone.
    pub fn influence_cone(&self, seeds: &[String], max_nodes: usize) -> Result<InfluenceCone> {
        self.validate()?;
        if seeds.is_empty() {
            return Err(invalid("edit influence cone requires at least one seed"));
        }
        if max_nodes == 0 || max_nodes > MAX_INFLUENCE_NODES {
            return Err(invalid("edit influence cone max_nodes must be 1..=250,000"));
        }
        let mut canonical_seeds = BTreeSet::new();
        for seed in seeds {
            if !self.nodes.contains(seed) {
                return Err(invalid(format!(
                    "edit influence seed '{seed}' is not in the graph"
                )));
            }
            canonical_seeds.insert(seed.clone());
        }
        let mut outgoing: BTreeMap<&str, Vec<&InfluenceEdge>> = BTreeMap::new();
        for edge in &self.edges {
            outgoing.entry(&edge.from).or_default().push(edge);
        }
        for edges in outgoing.values_mut() {
            edges.sort_by(|left, right| (&left.to, left.kind).cmp(&(&right.to, right.kind)));
        }

        let mut visited = canonical_seeds.clone();
        let mut queue: VecDeque<String> = canonical_seeds.iter().cloned().collect();
        while let Some(node) = queue.pop_front() {
            crate::cancel::check_current_cancel("edit synthesis influence closure")?;
            for edge in outgoing.get(node.as_str()).into_iter().flatten() {
                if visited.insert(edge.to.clone()) {
                    if visited.len() > max_nodes {
                        return Err(resource(format!(
                            "edit influence cone exceeds caller limit of {max_nodes} nodes"
                        )));
                    }
                    queue.push_back(edge.to.clone());
                }
            }
        }
        let mut edges = self
            .edges
            .iter()
            .filter(|edge| visited.contains(&edge.from) && visited.contains(&edge.to))
            .cloned()
            .collect::<Vec<_>>();
        edges.sort_by(|left, right| {
            (&left.from, &left.to, left.kind).cmp(&(&right.from, &right.to, right.kind))
        });
        let boundary = self
            .edges
            .iter()
            .filter(|edge| visited.contains(&edge.from) && !visited.contains(&edge.to))
            .map(|edge| edge.to.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        Ok(InfluenceCone {
            seeds: canonical_seeds.into_iter().collect(),
            nodes: visited.into_iter().collect(),
            edges,
            boundary,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProofObligationKind {
    SecurityAuthority,
    SourceRevision,
    InfluenceClosure,
    LensRoundTrip,
    Reopen,
    LogicalPostcondition,
    OutsideInfluencePixels,
    InsideEditIntent,
    StructureIntegrity,
    IndependentRendererAgreement,
    ResourceBudget,
    ReconstructionDisclosure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProofStatus {
    Pass,
    Fail,
    Unverified,
}

/// Identity and artifact binding for the component that produced one item of
/// evidence. The decision certificate binds this metadata but does not, by
/// itself, establish that an external producer is trustworthy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceProducerIdentity {
    pub name: String,
    pub version: String,
    /// Exact subject evaluated by the producer (normally input, candidate
    /// output, or a render-contract digest).
    pub subject_sha256: String,
    /// Optional report/raster/transcript digest retained outside this compact
    /// decision record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_sha256: Option<String>,
    #[serde(default)]
    pub independent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProofEvidence {
    pub obligation: ProofObligationKind,
    pub status: ProofStatus,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub producer: Option<EvidenceProducerIdentity>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditCostVector {
    pub changed_indirect_objects: u32,
    pub rewritten_opaque_bytes: u64,
    pub outside_mask_changed_pixels: u64,
    pub semantic_distance_microunits: u64,
    pub layout_displacement_micropoints: u64,
    pub font_substitution_penalty: u32,
    /// Calibrated or heuristic uncertainty in parts per million. The field is
    /// evidence, not a universal probability guarantee.
    pub reconstruction_uncertainty_ppm: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceOwnerId {
    pub object_number: u32,
    pub generation: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditCandidate {
    pub id: String,
    pub fidelity: EditFidelityClass,
    pub owners: Vec<SourceOwnerId>,
    pub affected_pages: Vec<usize>,
    pub influence_nodes: Vec<String>,
    pub evidence: Vec<ProofEvidence>,
    pub cost: EditCostVector,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditSynthesisPolicy {
    /// Additional operation-specific obligations beyond the fidelity defaults.
    #[serde(default)]
    pub required_obligations: BTreeSet<ProofObligationKind>,
    #[serde(default)]
    pub allow_semantic_reconstruction: bool,
    #[serde(default)]
    pub allow_appearance_reconstruction: bool,
    #[serde(default = "default_max_candidates")]
    pub max_candidates: usize,
    #[serde(default = "default_max_uncertainty_ppm")]
    pub max_reconstruction_uncertainty_ppm: u32,
}

fn default_max_candidates() -> usize {
    256
}

fn default_max_uncertainty_ppm() -> u32 {
    50_000
}

impl Default for EditSynthesisPolicy {
    fn default() -> Self {
        Self {
            required_obligations: BTreeSet::new(),
            allow_semantic_reconstruction: false,
            allow_appearance_reconstruction: false,
            max_candidates: default_max_candidates(),
            max_reconstruction_uncertainty_ppm: default_max_uncertainty_ppm(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RejectedCandidate {
    pub id: String,
    pub candidate_sha256: String,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditSynthesisDecision {
    pub algorithm: String,
    pub algorithm_version: u32,
    pub selected: Option<EditCandidate>,
    pub qualified: Vec<String>,
    pub pareto_frontier: Vec<String>,
    pub rejected: Vec<RejectedCandidate>,
    pub decision_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceConstrainedEditRequest {
    pub graph: EditInfluenceGraph,
    pub seeds: Vec<String>,
    #[serde(default = "default_max_influence_nodes")]
    pub max_influence_nodes: usize,
    #[serde(default)]
    pub policy: EditSynthesisPolicy,
    pub candidates: Vec<EditCandidate>,
}

fn default_max_influence_nodes() -> usize {
    16_384
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceConstrainedEditDecision {
    pub influence_cone: InfluenceCone,
    pub synthesis: EditSynthesisDecision,
}

fn validate_id(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > MAX_ID_BYTES
        || value.chars().any(|character| character.is_control())
    {
        return Err(invalid(format!(
            "{label} must be 1..={MAX_ID_BYTES} non-control UTF-8 bytes"
        )));
    }
    Ok(())
}

fn validate_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn default_obligations(fidelity: EditFidelityClass) -> BTreeSet<ProofObligationKind> {
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

fn candidate_rejection_reasons(
    candidate: &EditCandidate,
    policy: &EditSynthesisPolicy,
) -> Result<Vec<String>> {
    validate_id(&candidate.id, "edit candidate id")?;
    if candidate.cost.reconstruction_uncertainty_ppm > 1_000_000 {
        return Err(invalid(
            "candidate reconstruction uncertainty exceeds 1,000,000 ppm",
        ));
    }
    let mut owners = BTreeSet::new();
    if candidate.owners.iter().any(|owner| !owners.insert(owner)) {
        return Err(invalid(format!(
            "candidate '{}' contains duplicate source owners",
            candidate.id
        )));
    }
    let mut pages = BTreeSet::new();
    if candidate
        .affected_pages
        .iter()
        .any(|page| *page == 0 || !pages.insert(*page))
    {
        return Err(invalid(format!(
            "candidate '{}' pages must be unique and one-based",
            candidate.id
        )));
    }
    let mut nodes = BTreeSet::new();
    for node in &candidate.influence_nodes {
        validate_id(node, "candidate influence node")?;
        if !nodes.insert(node) {
            return Err(invalid(format!(
                "candidate '{}' contains duplicate influence nodes",
                candidate.id
            )));
        }
    }
    let mut evidence = BTreeMap::new();
    for item in &candidate.evidence {
        if item.detail.len() > MAX_EVIDENCE_DETAIL_BYTES {
            return Err(resource(format!(
                "candidate '{}' evidence detail exceeds 16 KiB",
                candidate.id
            )));
        }
        if item
            .evidence_sha256
            .as_ref()
            .is_some_and(|digest| !validate_digest(digest))
        {
            return Err(invalid(format!(
                "candidate '{}' contains an invalid evidence digest",
                candidate.id
            )));
        }
        if let Some(producer) = &item.producer {
            validate_id(&producer.name, "evidence producer name")?;
            validate_id(&producer.version, "evidence producer version")?;
            if !validate_digest(&producer.subject_sha256)
                || producer
                    .artifact_sha256
                    .as_ref()
                    .is_some_and(|digest| !validate_digest(digest))
            {
                return Err(invalid(format!(
                    "candidate '{}' contains invalid producer evidence digests",
                    candidate.id
                )));
            }
        }
        if evidence.insert(item.obligation, item.status).is_some() {
            return Err(invalid(format!(
                "candidate '{}' repeats a proof obligation",
                candidate.id
            )));
        }
    }

    let mut reasons = Vec::new();
    if candidate.fidelity == EditFidelityClass::SemanticReconstruction
        && !policy.allow_semantic_reconstruction
    {
        reasons.push("semantic reconstruction is not authorized by policy".into());
    }
    if candidate.fidelity == EditFidelityClass::AppearanceReconstruction
        && !policy.allow_appearance_reconstruction
    {
        reasons.push("appearance reconstruction is not authorized by policy".into());
    }
    if candidate.fidelity != EditFidelityClass::SourceNative
        && candidate.cost.reconstruction_uncertainty_ppm > policy.max_reconstruction_uncertainty_ppm
    {
        reasons.push(format!(
            "reconstruction uncertainty {} ppm exceeds policy limit {} ppm",
            candidate.cost.reconstruction_uncertainty_ppm,
            policy.max_reconstruction_uncertainty_ppm
        ));
    }
    let required = default_obligations(candidate.fidelity)
        .into_iter()
        .chain(policy.required_obligations.iter().copied())
        .collect::<BTreeSet<_>>();
    for obligation in required {
        match evidence.get(&obligation) {
            Some(ProofStatus::Pass) => {}
            Some(ProofStatus::Fail) => {
                reasons.push(format!("required proof obligation {obligation:?} failed"))
            }
            Some(ProofStatus::Unverified) => reasons.push(format!(
                "required proof obligation {obligation:?} is unverified"
            )),
            None => reasons.push(format!(
                "required proof obligation {obligation:?} is missing"
            )),
        }
    }
    Ok(reasons)
}

fn dominates(left: &EditCandidate, right: &EditCandidate) -> bool {
    let l = &left.cost;
    let r = &right.cost;
    let no_worse = left.fidelity.rank() <= right.fidelity.rank()
        && l.changed_indirect_objects <= r.changed_indirect_objects
        && l.rewritten_opaque_bytes <= r.rewritten_opaque_bytes
        && l.outside_mask_changed_pixels <= r.outside_mask_changed_pixels
        && l.semantic_distance_microunits <= r.semantic_distance_microunits
        && l.layout_displacement_micropoints <= r.layout_displacement_micropoints
        && l.font_substitution_penalty <= r.font_substitution_penalty
        && l.reconstruction_uncertainty_ppm <= r.reconstruction_uncertainty_ppm;
    let strictly_better = left.fidelity.rank() < right.fidelity.rank()
        || l.changed_indirect_objects < r.changed_indirect_objects
        || l.rewritten_opaque_bytes < r.rewritten_opaque_bytes
        || l.outside_mask_changed_pixels < r.outside_mask_changed_pixels
        || l.semantic_distance_microunits < r.semantic_distance_microunits
        || l.layout_displacement_micropoints < r.layout_displacement_micropoints
        || l.font_substitution_penalty < r.font_substitution_penalty
        || l.reconstruction_uncertainty_ppm < r.reconstruction_uncertainty_ppm;
    no_worse && strictly_better
}

fn selection_key(candidate: &EditCandidate) -> (u8, &EditCostVector, &str) {
    (candidate.fidelity.rank(), &candidate.cost, &candidate.id)
}

/// Select a qualified edit construction. Validation failures in the request are
/// errors; candidate-specific failed evidence is returned as rejection data.
pub fn synthesize_edit_plan(
    policy: &EditSynthesisPolicy,
    candidates: Vec<EditCandidate>,
) -> Result<EditSynthesisDecision> {
    if policy.max_candidates == 0 || policy.max_candidates > MAX_CANDIDATES {
        return Err(invalid("edit synthesis max_candidates must be 1..=4,096"));
    }
    if candidates.len() > policy.max_candidates {
        return Err(resource(format!(
            "edit synthesis received {} candidates above policy limit {}",
            candidates.len(),
            policy.max_candidates
        )));
    }
    if policy.max_reconstruction_uncertainty_ppm > 1_000_000 {
        return Err(invalid(
            "edit synthesis uncertainty policy exceeds 1,000,000 ppm",
        ));
    }
    let mut ids = BTreeSet::new();
    let mut qualified = Vec::new();
    let mut rejected = Vec::new();
    for candidate in candidates {
        if !ids.insert(candidate.id.clone()) {
            return Err(invalid("edit synthesis candidate ids must be unique"));
        }
        let candidate_sha256 =
            format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&candidate).map_err(|error| invalid(
                    format!("edit synthesis candidate serialization failed: {error}")
                ))?)
            );
        let reasons = candidate_rejection_reasons(&candidate, policy)?;
        if reasons.is_empty() {
            qualified.push(candidate);
        } else {
            rejected.push(RejectedCandidate {
                id: candidate.id,
                candidate_sha256,
                reasons,
            });
        }
    }
    qualified.sort_by(|left, right| selection_key(left).cmp(&selection_key(right)));
    rejected.sort_by(|left, right| left.id.cmp(&right.id));
    let frontier = qualified
        .iter()
        .filter(|candidate| {
            !qualified
                .iter()
                .any(|other| other.id != candidate.id && dominates(other, candidate))
        })
        .cloned()
        .collect::<Vec<_>>();
    let selected = frontier
        .iter()
        .min_by(|left, right| selection_key(left).cmp(&selection_key(right)))
        .cloned();
    let qualified_ids = qualified
        .iter()
        .map(|candidate| candidate.id.clone())
        .collect::<Vec<_>>();
    let pareto_frontier = frontier
        .iter()
        .map(|candidate| candidate.id.clone())
        .collect::<Vec<_>>();
    #[derive(Serialize)]
    struct CertificateInput<'a> {
        algorithm: &'static str,
        version: u32,
        policy: &'a EditSynthesisPolicy,
        qualified: &'a [EditCandidate],
        selected: &'a Option<EditCandidate>,
        pareto_frontier: &'a [String],
        rejected: &'a [RejectedCandidate],
    }
    let certificate = CertificateInput {
        algorithm: "evidence_constrained_bidirectional_edit_synthesis",
        version: 1,
        policy,
        qualified: &qualified,
        selected: &selected,
        pareto_frontier: &pareto_frontier,
        rejected: &rejected,
    };
    let canonical = serde_json::to_vec(&certificate)
        .map_err(|error| invalid(format!("edit synthesis certificate failed: {error}")))?;
    let decision_sha256 = format!("{:x}", Sha256::digest(canonical));
    Ok(EditSynthesisDecision {
        algorithm: certificate.algorithm.into(),
        algorithm_version: certificate.version,
        selected,
        qualified: qualified_ids,
        pareto_frontier,
        rejected,
        decision_sha256,
    })
}

/// Full research entry point. Influence-closure evidence is computed here and
/// replaces any caller assertion of the same obligation. A candidate may cover
/// more graph nodes conservatively, but it may not omit any node in the exact
/// transitive cone or name a node outside the graph.
pub fn synthesize_evidence_constrained_edit(
    mut request: EvidenceConstrainedEditRequest,
) -> Result<EvidenceConstrainedEditDecision> {
    let cone = request
        .graph
        .influence_cone(&request.seeds, request.max_influence_nodes)?;
    let cone_nodes = cone.nodes.iter().cloned().collect::<BTreeSet<_>>();
    let cone_bytes = serde_json::to_vec(&cone)
        .map_err(|error| invalid(format!("influence cone serialization failed: {error}")))?;
    let cone_sha256 = format!("{:x}", Sha256::digest(cone_bytes));
    for candidate in &mut request.candidates {
        candidate
            .evidence
            .retain(|item| item.obligation != ProofObligationKind::InfluenceClosure);
        let covered = candidate
            .influence_nodes
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let missing = cone_nodes.difference(&covered).cloned().collect::<Vec<_>>();
        let unknown = covered
            .difference(&request.graph.nodes)
            .cloned()
            .collect::<Vec<_>>();
        let status = if missing.is_empty() && unknown.is_empty() {
            ProofStatus::Pass
        } else {
            ProofStatus::Fail
        };
        let detail = if status == ProofStatus::Pass {
            format!(
                "candidate covers all {} nodes in the exact influence cone",
                cone.nodes.len()
            )
        } else {
            format!(
                "candidate misses {} influence nodes and names {} unknown nodes",
                missing.len(),
                unknown.len()
            )
        };
        candidate.evidence.push(ProofEvidence {
            obligation: ProofObligationKind::InfluenceClosure,
            status,
            detail,
            evidence_sha256: Some(cone_sha256.clone()),
            producer: Some(EvidenceProducerIdentity {
                name: "wellfriendpdf-ecbes-kernel".into(),
                version: "1".into(),
                subject_sha256: cone_sha256.clone(),
                artifact_sha256: Some(cone_sha256.clone()),
                independent: false,
            }),
        });
    }
    let synthesis = synthesize_edit_plan(&request.policy, request.candidates)?;
    Ok(EvidenceConstrainedEditDecision {
        influence_cone: cone,
        synthesis,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passing_evidence(fidelity: EditFidelityClass) -> Vec<ProofEvidence> {
        default_obligations(fidelity)
            .into_iter()
            .map(|obligation| ProofEvidence {
                obligation,
                status: ProofStatus::Pass,
                detail: "fixture evidence".into(),
                evidence_sha256: None,
                producer: None,
            })
            .collect()
    }

    fn candidate(id: &str, fidelity: EditFidelityClass, cost: EditCostVector) -> EditCandidate {
        EditCandidate {
            id: id.into(),
            fidelity,
            owners: vec![SourceOwnerId {
                object_number: 7,
                generation: 0,
            }],
            affected_pages: vec![1],
            influence_nodes: vec!["page:1/content:0".into()],
            evidence: passing_evidence(fidelity),
            cost,
        }
    }

    #[test]
    fn influence_cone_is_transitive_cycle_safe_and_deterministic() {
        let graph = EditInfluenceGraph {
            nodes: BTreeSet::from([
                "source".into(),
                "clip".into(),
                "form".into(),
                "page".into(),
                "unrelated".into(),
            ]),
            edges: vec![
                InfluenceEdge {
                    from: "form".into(),
                    to: "page".into(),
                    kind: InfluenceEdgeKind::FormInvocation,
                },
                InfluenceEdge {
                    from: "clip".into(),
                    to: "form".into(),
                    kind: InfluenceEdgeKind::Clip,
                },
                InfluenceEdge {
                    from: "source".into(),
                    to: "clip".into(),
                    kind: InfluenceEdgeKind::PaintOrder,
                },
                InfluenceEdge {
                    from: "page".into(),
                    to: "source".into(),
                    kind: InfluenceEdgeKind::Resource,
                },
            ],
        };
        let cone = graph.influence_cone(&["source".into()], 16).unwrap();
        assert_eq!(cone.nodes, vec!["clip", "form", "page", "source"]);
        assert!(cone.boundary.is_empty());
        assert_eq!(cone.edges.len(), 4);
    }

    #[test]
    fn influence_cone_canonicalizes_equivalent_edge_orderings() {
        let nodes = BTreeSet::from(["a".into(), "b".into(), "c".into()]);
        let first = EditInfluenceGraph {
            nodes: nodes.clone(),
            edges: vec![
                InfluenceEdge {
                    from: "b".into(),
                    to: "c".into(),
                    kind: InfluenceEdgeKind::Layout,
                },
                InfluenceEdge {
                    from: "a".into(),
                    to: "b".into(),
                    kind: InfluenceEdgeKind::Resource,
                },
            ],
        };
        let second = EditInfluenceGraph {
            nodes,
            edges: first.edges.iter().cloned().rev().collect(),
        };
        assert_eq!(
            first.influence_cone(&["a".into()], 8).unwrap(),
            second.influence_cone(&["a".into()], 8).unwrap()
        );
    }

    #[test]
    fn missing_hard_evidence_rejects_without_publishing_a_candidate() {
        let mut candidate = candidate(
            "native",
            EditFidelityClass::SourceNative,
            EditCostVector::default(),
        );
        candidate
            .evidence
            .retain(|item| item.obligation != ProofObligationKind::Reopen);
        let decision =
            synthesize_edit_plan(&EditSynthesisPolicy::default(), vec![candidate]).unwrap();
        assert!(decision.selected.is_none());
        assert!(decision.rejected[0].reasons[0].contains("Reopen"));
    }

    #[test]
    fn source_native_candidate_wins_and_dominates_reconstruction() {
        let native = candidate(
            "native",
            EditFidelityClass::SourceNative,
            EditCostVector {
                changed_indirect_objects: 1,
                ..Default::default()
            },
        );
        let reconstructed = candidate(
            "reconstructed",
            EditFidelityClass::AppearanceReconstruction,
            EditCostVector {
                changed_indirect_objects: 2,
                reconstruction_uncertainty_ppm: 10_000,
                ..Default::default()
            },
        );
        let policy = EditSynthesisPolicy {
            allow_appearance_reconstruction: true,
            ..Default::default()
        };
        let first =
            synthesize_edit_plan(&policy, vec![reconstructed.clone(), native.clone()]).unwrap();
        let second = synthesize_edit_plan(&policy, vec![native, reconstructed]).unwrap();
        assert_eq!(first.selected.as_ref().unwrap().id, "native");
        assert_eq!(first.pareto_frontier, vec!["native"]);
        assert_eq!(first.decision_sha256, second.decision_sha256);
    }

    #[test]
    fn reconstruction_requires_explicit_authority_and_remains_disclosed() {
        let reconstructed = candidate(
            "scan",
            EditFidelityClass::AppearanceReconstruction,
            EditCostVector {
                reconstruction_uncertainty_ppm: 20_000,
                ..Default::default()
            },
        );
        let refused =
            synthesize_edit_plan(&EditSynthesisPolicy::default(), vec![reconstructed.clone()])
                .unwrap();
        assert!(refused.selected.is_none());
        let accepted = synthesize_edit_plan(
            &EditSynthesisPolicy {
                allow_appearance_reconstruction: true,
                ..Default::default()
            },
            vec![reconstructed],
        )
        .unwrap();
        assert_eq!(
            accepted.selected.unwrap().fidelity,
            EditFidelityClass::AppearanceReconstruction
        );
    }

    #[test]
    fn full_synthesis_computes_and_enforces_the_influence_cone() {
        let graph = EditInfluenceGraph {
            nodes: BTreeSet::from(["operand".into(), "clip".into(), "page".into()]),
            edges: vec![
                InfluenceEdge {
                    from: "operand".into(),
                    to: "clip".into(),
                    kind: InfluenceEdgeKind::Clip,
                },
                InfluenceEdge {
                    from: "clip".into(),
                    to: "page".into(),
                    kind: InfluenceEdgeKind::PaintOrder,
                },
            ],
        };
        let mut incomplete = candidate(
            "incomplete",
            EditFidelityClass::SourceNative,
            EditCostVector::default(),
        );
        incomplete.influence_nodes = vec!["operand".into()];
        let mut complete = candidate(
            "complete",
            EditFidelityClass::SourceNative,
            EditCostVector {
                changed_indirect_objects: 2,
                ..Default::default()
            },
        );
        complete.influence_nodes = vec!["clip".into(), "operand".into(), "page".into()];
        let decision = synthesize_evidence_constrained_edit(EvidenceConstrainedEditRequest {
            graph,
            seeds: vec!["operand".into()],
            max_influence_nodes: 8,
            policy: EditSynthesisPolicy::default(),
            candidates: vec![incomplete, complete],
        })
        .unwrap();
        assert_eq!(
            decision.influence_cone.nodes,
            vec!["clip", "operand", "page"]
        );
        assert_eq!(decision.synthesis.selected.unwrap().id, "complete");
        assert_eq!(decision.synthesis.rejected[0].id, "incomplete");
    }
}
