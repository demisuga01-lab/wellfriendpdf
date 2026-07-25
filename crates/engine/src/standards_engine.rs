//! Prompt 26 shared clause-mapped standards-validation architecture.
//!
//! This is the single report envelope used by the PDF/A, PDF/UA, and PDF/X
//! rule engines and by every binding surface. It is deliberately independent of
//! the first-generation [`crate::standards`] summary reports (which the earlier
//! `standards_profile_json` envelope still uses) so existing callers keep
//! working while the certification-grade engines are layered on top.
//!
//! Certification honesty: a `Conformant` verdict here means "the rules Wellfriend
//! implemented for this profile subset passed and were clause-mapped and
//! evidenced". It is not an accredited archival/accessibility/print
//! certification. Unknown rules are reported exactly (never silently passed).

use serde::{Deserialize, Serialize};

use crate::object::{PdfDictionary, PdfObject};
use crate::{ContentEngine, Result};

/// Schema version for every Prompt 26 standards report.
pub const STANDARDS_ENGINE_SCHEMA_VERSION: u32 = 1;

/// Standards family a run targets.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StandardsFamily {
    PdfA,
    PdfUa,
    PdfX,
}

impl StandardsFamily {
    pub fn as_str(self) -> &'static str {
        match self {
            StandardsFamily::PdfA => "pdfa",
            StandardsFamily::PdfUa => "pdfua",
            StandardsFamily::PdfX => "pdfx",
        }
    }
}

/// Per-rule status. Exactly these values are permitted; there is no vague
/// "maybe"/"partial"/"todo" state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleStatus {
    Pass,
    Fail,
    Warning,
    Indeterminate,
    NotApplicable,
    UnsupportedReportedExact,
    DeferredPrompt27CorpusParity,
    BlockedNormativeDependency,
}

impl RuleStatus {
    /// A status that must sink the overall conformance verdict to fail.
    pub fn is_failing(self) -> bool {
        matches!(self, RuleStatus::Fail)
    }

    /// A status that prevents a clean `Conformant` verdict but is not a hard
    /// fail (the engine reports it honestly instead of guessing).
    pub fn is_inconclusive(self) -> bool {
        matches!(
            self,
            RuleStatus::Indeterminate
                | RuleStatus::UnsupportedReportedExact
                | RuleStatus::DeferredPrompt27CorpusParity
                | RuleStatus::BlockedNormativeDependency
        )
    }
}

/// How completely Wellfriend implements a given rule.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleImplementation {
    FullyImplemented,
    ImplementedWithLimits,
    Unsupported,
    DeferredPrompt27,
    Blocked,
}

/// Diagnostic severity independent of status (an implemented rule can `pass`
/// with `info`, or `fail` with `error`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationSeverity {
    Info,
    Warning,
    Error,
}

/// Overall conformance verdict for one profile report.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConformanceStatus {
    /// All implemented rules passed and nothing inconclusive remained.
    Conformant,
    /// At least one implemented rule failed.
    NonConformant,
    /// No hard failure, but inconclusive/unsupported/deferred rows remain, so a
    /// clean conformance claim cannot be made.
    Indeterminate,
    /// The requested profile is not supported by Wellfriend at all.
    Unsupported,
}

/// Normative clause reference for a rule (identifiers only; no restricted
/// normative text is copied into the repository).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StandardsClauseRef {
    /// e.g. `ISO 19005-2:2011`, `ISO 14289-1:2014`, `ISO 15930-4:2003`.
    pub standard: String,
    /// e.g. `6.2.4.3` or `7.1`.
    pub clause: String,
    /// Short derived description of the requirement (Wellfriend's own words).
    pub title: String,
}

impl StandardsClauseRef {
    pub fn new(standard: &str, clause: &str, title: &str) -> Self {
        Self {
            standard: standard.to_string(),
            clause: clause.to_string(),
            title: title.to_string(),
        }
    }
}

/// Object/page/resource context plus an optional evidence pointer.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ValidationEvidence {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_path: Option<String>,
}

impl ValidationEvidence {
    pub fn at(location: &str) -> Self {
        Self {
            object: Some(location.to_string()),
            ..Self::default()
        }
    }

    pub fn document() -> Self {
        Self::at("document")
    }

    pub fn with_page(mut self, page: usize) -> Self {
        self.page = Some(page);
        self
    }

    pub fn with_resource(mut self, resource: &str) -> Self {
        self.resource = Some(resource.to_string());
        self
    }

    pub fn with_detail(mut self, detail: &str) -> Self {
        self.detail = Some(detail.to_string());
        self
    }
}

/// A single clause-mapped rule result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StandardsRuleResult {
    pub rule_id: String,
    pub profile: String,
    pub clause: StandardsClauseRef,
    pub context: ValidationEvidence,
    pub status: RuleStatus,
    pub severity: ValidationSeverity,
    pub diagnostic: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unsupported_reason: Option<String>,
    pub implementation: RuleImplementation,
}

impl StandardsRuleResult {
    /// Construct a fully-implemented rule result with an explicit status.
    pub fn implemented(
        profile: &str,
        rule_id: &str,
        clause: StandardsClauseRef,
        context: ValidationEvidence,
        status: RuleStatus,
        severity: ValidationSeverity,
        diagnostic: impl Into<String>,
    ) -> Self {
        Self {
            rule_id: rule_id.to_string(),
            profile: profile.to_string(),
            clause,
            context,
            status,
            severity,
            diagnostic: diagnostic.into(),
            unsupported_reason: None,
            implementation: RuleImplementation::FullyImplemented,
        }
    }

    /// A rule that Wellfriend does not implement, reported exactly.
    pub fn unsupported(
        profile: &str,
        rule_id: &str,
        clause: StandardsClauseRef,
        reason: impl Into<String>,
    ) -> Self {
        let reason = reason.into();
        Self {
            rule_id: rule_id.to_string(),
            profile: profile.to_string(),
            clause,
            context: ValidationEvidence::document(),
            status: RuleStatus::UnsupportedReportedExact,
            severity: ValidationSeverity::Warning,
            diagnostic: format!("Rule not evaluated by Wellfriend: {reason}"),
            unsupported_reason: Some(reason),
            implementation: RuleImplementation::Unsupported,
        }
    }

    /// A rule deferred to Prompt 27 full-corpus parity.
    pub fn deferred_prompt27(
        profile: &str,
        rule_id: &str,
        clause: StandardsClauseRef,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            rule_id: rule_id.to_string(),
            profile: profile.to_string(),
            clause,
            context: ValidationEvidence::document(),
            status: RuleStatus::DeferredPrompt27CorpusParity,
            severity: ValidationSeverity::Info,
            diagnostic: reason.into(),
            unsupported_reason: None,
            implementation: RuleImplementation::DeferredPrompt27,
        }
    }

    /// A rule blocked because the normative source is not available locally.
    pub fn blocked_normative(
        profile: &str,
        rule_id: &str,
        clause: StandardsClauseRef,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            rule_id: rule_id.to_string(),
            profile: profile.to_string(),
            clause,
            context: ValidationEvidence::document(),
            status: RuleStatus::BlockedNormativeDependency,
            severity: ValidationSeverity::Warning,
            diagnostic: reason.into(),
            unsupported_reason: None,
            implementation: RuleImplementation::Blocked,
        }
    }

    pub fn with_context(mut self, context: ValidationEvidence) -> Self {
        self.context = context;
        self
    }

    pub fn with_implementation_limits(mut self) -> Self {
        self.implementation = RuleImplementation::ImplementedWithLimits;
        self
    }
}

/// Aggregate counts across a rule set.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct StandardsRuleCounts {
    pub total: usize,
    pub pass: usize,
    pub fail: usize,
    pub warning: usize,
    pub indeterminate: usize,
    pub not_applicable: usize,
    pub unsupported_reported_exact: usize,
    pub deferred_prompt27_corpus_parity: usize,
    pub blocked_normative_dependency: usize,
}

impl StandardsRuleCounts {
    pub fn tally(rules: &[StandardsRuleResult]) -> Self {
        let mut counts = StandardsRuleCounts {
            total: rules.len(),
            ..Self::default()
        };
        for rule in rules {
            match rule.status {
                RuleStatus::Pass => counts.pass += 1,
                RuleStatus::Fail => counts.fail += 1,
                RuleStatus::Warning => counts.warning += 1,
                RuleStatus::Indeterminate => counts.indeterminate += 1,
                RuleStatus::NotApplicable => counts.not_applicable += 1,
                RuleStatus::UnsupportedReportedExact => counts.unsupported_reported_exact += 1,
                RuleStatus::DeferredPrompt27CorpusParity => {
                    counts.deferred_prompt27_corpus_parity += 1
                }
                RuleStatus::BlockedNormativeDependency => counts.blocked_normative_dependency += 1,
            }
        }
        counts
    }
}

/// Profile detection outcome for a family.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProfileDetection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claimed_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claimed_part: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claimed_conformance: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier_source: Option<String>,
    pub malformed_identifier: bool,
    pub conflicting_identifiers: bool,
}

/// A single-profile clause-mapped validation report.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StandardsValidationReport {
    pub schema_version: u32,
    pub family: StandardsFamily,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claimed_profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_profile: Option<String>,
    pub detection: ProfileDetection,
    pub conformance: ConformanceStatus,
    pub counts: StandardsRuleCounts,
    pub rules: Vec<StandardsRuleResult>,
    pub certification_claimed: bool,
    pub certification_disclaimer: String,
}

impl StandardsValidationReport {
    /// Assemble a report, deriving conformance from the rule statuses.
    pub fn assemble(
        family: StandardsFamily,
        claimed_profile: Option<String>,
        target_profile: Option<String>,
        detection: ProfileDetection,
        rules: Vec<StandardsRuleResult>,
    ) -> Self {
        let counts = StandardsRuleCounts::tally(&rules);
        let conformance = derive_conformance(&rules);
        Self {
            schema_version: STANDARDS_ENGINE_SCHEMA_VERSION,
            family,
            claimed_profile,
            target_profile,
            detection,
            conformance,
            counts,
            rules,
            certification_claimed: false,
            certification_disclaimer:
                "Clause-mapped Wellfriend validation subset; not an accredited certification claim."
                    .to_string(),
        }
    }

    pub fn is_conformant(&self) -> bool {
        matches!(self.conformance, ConformanceStatus::Conformant)
    }
}

fn derive_conformance(rules: &[StandardsRuleResult]) -> ConformanceStatus {
    if rules.iter().any(|rule| rule.status.is_failing()) {
        return ConformanceStatus::NonConformant;
    }
    if rules.iter().any(|rule| rule.status.is_inconclusive()) {
        return ConformanceStatus::Indeterminate;
    }
    ConformanceStatus::Conformant
}

/// Options controlling a standards validation run.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct StandardsValidationOptions {
    /// Force a specific target profile label (e.g. `PDF/A-2B`, `PDF/UA-1`,
    /// `PDF/X-4`). When absent, the detected/claimed profile is used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_profile: Option<String>,
    /// Validate every profile claimed in metadata.
    pub validate_all_claimed: bool,
    /// Validate every profile family Wellfriend supports.
    pub validate_all_supported: bool,
    /// Include passing rows in the report (default true).
    #[serde(default = "default_true")]
    pub include_pass_rules: bool,
}

fn default_true() -> bool {
    true
}

impl StandardsValidationOptions {
    pub fn with_target(target: &str) -> Self {
        Self {
            target_profile: Some(target.to_string()),
            include_pass_rules: true,
            ..Self::default()
        }
    }
}

/// One cross-profile conflict between two claimed/target profiles.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CrossProfileConflict {
    pub left_profile: String,
    pub right_profile: String,
    pub severity: ValidationSeverity,
    pub description: String,
}

/// Combined multi-profile report plus detected conflicts.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CrossProfileConflictReport {
    pub schema_version: u32,
    pub reports: Vec<StandardsValidationReport>,
    pub conflicts: Vec<CrossProfileConflict>,
    /// True only when every included report is conformant and no conflict is an
    /// error. A single profile passing never hides another failing.
    pub overall_pass: bool,
}

// ---------------------------------------------------------------------------
// Profile detection (XMP + output-intent based)
// ---------------------------------------------------------------------------

/// Decoded XMP metadata packet bytes from `/Root/Metadata`, if present.
pub fn read_xmp_metadata(engine: &ContentEngine) -> Option<Vec<u8>> {
    let doc = engine.document();
    let catalog = doc.get_catalog().ok()?;
    let meta = catalog.get("Metadata")?;
    let resolved = match meta {
        PdfObject::Reference { number, generation } => {
            doc.reader().get_and_resolve(*number, *generation).ok()?
        }
        other => other.clone(),
    };
    let (dict, raw) = resolved.as_stream()?;
    crate::filters::decode_stream_from_dict(dict, raw)
        .ok()
        .or_else(|| Some(raw.to_vec()))
}

/// Extract the value of an XMP field `prefix:local` in either attribute
/// (`prefix:local="V"`) or element (`<prefix:local>V</prefix:local>`) form.
/// Bounded, allocation-light, and namespace-prefix tolerant.
pub fn xmp_lookup(text: &str, prefix: &str, local: &str) -> Option<String> {
    let token = format!("{prefix}:{local}");
    let mut search_from = 0usize;
    while let Some(rel) = text[search_from..].find(&token) {
        let idx = search_from + rel;
        let after = idx + token.len();
        let rest = text.get(after..)?;
        let mut chars = rest.char_indices().peekable();
        // Skip whitespace.
        while let Some(&(_, c)) = chars.peek() {
            if c.is_whitespace() {
                chars.next();
            } else {
                break;
            }
        }
        match chars.peek().copied() {
            Some((eq_i, '=')) => {
                // Attribute form: skip '=' and whitespace, then read quoted value.
                let mut q = rest[eq_i + 1..].char_indices().peekable();
                while let Some(&(_, c)) = q.peek() {
                    if c.is_whitespace() {
                        q.next();
                    } else {
                        break;
                    }
                }
                if let Some((quote_i, quote @ ('"' | '\''))) = q.peek().copied() {
                    let value_start = eq_i + 1 + quote_i + quote.len_utf8();
                    if let Some(end_rel) = rest[value_start..].find(quote) {
                        let value = rest[value_start..value_start + end_rel].trim();
                        if !value.is_empty() {
                            return Some(value.to_string());
                        }
                    }
                }
            }
            Some((gt_i, '>')) => {
                // Element form: text up to the next '<'.
                let value_start = gt_i + 1;
                if let Some(end_rel) = rest[value_start..].find('<') {
                    let value = rest[value_start..value_start + end_rel].trim();
                    if !value.is_empty() {
                        return Some(value.to_string());
                    }
                }
            }
            _ => {}
        }
        search_from = after;
    }
    None
}

/// Detect a claimed PDF/A profile from XMP `pdfaid:part` + `pdfaid:conformance`.
pub fn detect_pdfa(engine: &ContentEngine) -> ProfileDetection {
    let mut detection = ProfileDetection::default();
    let Some(bytes) = read_xmp_metadata(engine) else {
        return detection;
    };
    let text = String::from_utf8_lossy(&bytes);
    let part = xmp_lookup(&text, "pdfaid", "part");
    let conformance = xmp_lookup(&text, "pdfaid", "conformance");
    if part.is_none() && conformance.is_none() {
        return detection;
    }
    detection.identifier_source = Some("xmp:pdfaid".to_string());
    detection.claimed_part = part.clone();
    detection.claimed_conformance = conformance.clone();
    match (&part, &conformance) {
        (Some(p), Some(c)) if is_valid_pdfa_part(p) && is_valid_pdfa_conformance(c) => {
            detection.claimed_label = Some(format!("PDF/A-{p}{}", c.to_uppercase()));
        }
        (Some(p), None) if is_valid_pdfa_part(p) => {
            detection.claimed_label = Some(format!("PDF/A-{p}"));
        }
        _ => {
            detection.malformed_identifier = true;
        }
    }
    detection
}

/// Detect a claimed PDF/UA profile from XMP `pdfuaid:part`.
pub fn detect_pdfua(engine: &ContentEngine) -> ProfileDetection {
    let mut detection = ProfileDetection::default();
    let Some(bytes) = read_xmp_metadata(engine) else {
        return detection;
    };
    let text = String::from_utf8_lossy(&bytes);
    if let Some(part) = xmp_lookup(&text, "pdfuaid", "part") {
        detection.identifier_source = Some("xmp:pdfuaid".to_string());
        detection.claimed_part = Some(part.clone());
        if part == "1" {
            detection.claimed_label = Some("PDF/UA-1".to_string());
        } else {
            detection.malformed_identifier = true;
        }
    }
    detection
}

/// Detect a claimed PDF/X profile from XMP `pdfxid:GTS_PDFXVersion` or the
/// `GTS_PDFX` output intent + `Info` version key.
pub fn detect_pdfx(engine: &ContentEngine) -> ProfileDetection {
    let mut detection = ProfileDetection::default();
    let mut labels: Vec<String> = Vec::new();

    if let Some(bytes) = read_xmp_metadata(engine) {
        let text = String::from_utf8_lossy(&bytes);
        if let Some(version) = xmp_lookup(&text, "pdfxid", "GTS_PDFXVersion") {
            detection.identifier_source = Some("xmp:pdfxid".to_string());
            if let Some(label) = normalize_pdfx_version(&version) {
                labels.push(label);
            } else {
                detection.malformed_identifier = true;
            }
        }
    }

    if let Ok(catalog) = engine.document().get_catalog() {
        if has_gts_pdfx_output_intent(&catalog, engine) {
            if detection.identifier_source.is_none() {
                detection.identifier_source = Some("output_intent:GTS_PDFX".to_string());
            }
            if labels.is_empty() {
                labels.push("PDF/X".to_string());
            }
        }
    }

    labels.sort();
    labels.dedup();
    if labels.len() > 1 {
        detection.conflicting_identifiers = true;
    }
    detection.claimed_label = labels.into_iter().next();
    detection
}

fn is_valid_pdfa_part(part: &str) -> bool {
    matches!(part, "1" | "2" | "3" | "4")
}

fn is_valid_pdfa_conformance(conformance: &str) -> bool {
    matches!(
        conformance.to_uppercase().as_str(),
        "A" | "B" | "U" | "F" | "E"
    )
}

fn normalize_pdfx_version(version: &str) -> Option<String> {
    let upper = version.to_uppercase();
    for token in [
        "X-1A", "X-1", "X-3", "X-4P", "X-4", "X-5PG", "X-5G", "X-5N", "X-6",
    ] {
        if upper.contains(token) {
            return Some(format!("PDF/{token}"));
        }
    }
    if upper.contains("PDF/X") || upper.contains("PDFX") {
        return Some("PDF/X".to_string());
    }
    None
}

fn has_gts_pdfx_output_intent(catalog: &PdfDictionary, engine: &ContentEngine) -> bool {
    let Some(PdfObject::Array(items)) = catalog.get("OutputIntents") else {
        return false;
    };
    items.iter().any(|item| {
        let resolved = match item {
            PdfObject::Reference { number, generation } => engine
                .document()
                .reader()
                .get_and_resolve(*number, *generation)
                .ok(),
            other => Some(other.clone()),
        };
        matches!(
            resolved
                .as_ref()
                .and_then(PdfObject::as_dict)
                .and_then(|dict| dict.get_name("S")),
            Some("GTS_PDFX")
        )
    })
}

/// Build a cross-profile report from individual profile reports, computing
/// conflicts so that one profile passing cannot hide another failing.
pub fn assemble_cross_profile(
    reports: Vec<StandardsValidationReport>,
    conflicts: Vec<CrossProfileConflict>,
) -> CrossProfileConflictReport {
    let all_conformant = reports.iter().all(StandardsValidationReport::is_conformant);
    let no_error_conflict = !conflicts
        .iter()
        .any(|c| matches!(c.severity, ValidationSeverity::Error));
    CrossProfileConflictReport {
        schema_version: STANDARDS_ENGINE_SCHEMA_VERSION,
        reports,
        conflicts,
        overall_pass: all_conformant && no_error_conflict,
    }
}

/// Result alias to keep engine entry points uniform.
pub type StandardsResult<T> = Result<T>;

// ---------------------------------------------------------------------------
// PDF/A clause-mapped validator (ISO 19005 family)
// ---------------------------------------------------------------------------

use crate::compliance::{
    validate_pdfa, validate_pdfua, ComplianceSeverity, ComplianceViolation, PdfAProfile,
};

fn pdfa_profile_from_label(label: &str) -> Option<PdfAProfile> {
    match label.to_uppercase().replace(' ', "").as_str() {
        "PDF/A-1B" => Some(PdfAProfile::PdfA1B),
        "PDF/A-2B" => Some(PdfAProfile::PdfA2B),
        "PDF/A-2A" => Some(PdfAProfile::PdfA2A),
        "PDF/A-3B" => Some(PdfAProfile::PdfA3B),
        "PDF/A-3A" => Some(PdfAProfile::PdfA3A),
        _ => None,
    }
}

fn pdfa_part_from_label(label: &str) -> i32 {
    label
        .chars()
        .find(|c| c.is_ascii_digit())
        .and_then(|c| c.to_digit(10))
        .map(|d| d as i32)
        .unwrap_or(0)
}

fn pdfa_standard_for(part: i32) -> &'static str {
    match part {
        1 => "ISO 19005-1:2005",
        2 => "ISO 19005-2:2011",
        3 => "ISO 19005-3:2012",
        4 => "ISO 19005-4:2020",
        _ => "ISO 19005",
    }
}

fn pdfa_clause_for(rule_id: &str) -> (&'static str, &'static str) {
    let r = rule_id;
    if r.contains("font") {
        ("6.3.4", "All fonts and font programs are embedded")
    } else if r.contains("output_intent") {
        (
            "6.2.2",
            "PDF/A OutputIntent with a valid ICC destination profile",
        )
    } else if r.contains("info") {
        (
            "6.7.3",
            "Document information dictionary synchronized with XMP",
        )
    } else if r.contains("xmp") || r.contains("metadata") {
        ("6.7", "XMP metadata packet presence and validity")
    } else if r.contains("embedded_file") {
        ("6.8", "Embedded/associated file requirements")
    } else if r.contains("action") || r.contains("javascript") || r.contains("js") {
        ("6.6.1", "Prohibited actions and JavaScript")
    } else if r.contains("encrypt") {
        ("6.1.3", "Encryption is prohibited")
    } else if r.contains("transparency") {
        ("6.2.8", "Transparency restrictions")
    } else if r.contains("color") || r.contains("device") {
        ("6.2.3", "Device and ICC colour space rules")
    } else if r.contains("structure") || r.contains("tag") {
        (
            "6.8",
            "Logical structure requirements (Level A conformance)",
        )
    } else {
        ("6", "PDF/A conformance requirement")
    }
}

fn compliance_rule_status(severity: ComplianceSeverity) -> (RuleStatus, ValidationSeverity) {
    match severity {
        ComplianceSeverity::Error => (RuleStatus::Fail, ValidationSeverity::Error),
        ComplianceSeverity::Warning => (RuleStatus::Warning, ValidationSeverity::Warning),
    }
}

fn category_hit(violations: &[ComplianceViolation], keyword: &str) -> bool {
    violations.iter().any(|v| v.rule.contains(keyword))
}

fn add_pass_if_clean(
    rules: &mut Vec<StandardsRuleResult>,
    violations: &[ComplianceViolation],
    keyword: &str,
    profile: &str,
    rule_id: &str,
    standard: &str,
    diagnostic: &str,
) {
    if !category_hit(violations, keyword) {
        let (clause, title) = pdfa_clause_for(rule_id);
        rules.push(StandardsRuleResult::implemented(
            profile,
            rule_id,
            StandardsClauseRef::new(standard, clause, title),
            ValidationEvidence::document(),
            RuleStatus::Pass,
            ValidationSeverity::Info,
            diagnostic,
        ));
    }
}

/// Clause-mapped PDF/A validation for a target profile.
pub fn validate_pdfa_profile(
    engine: &ContentEngine,
    options: &StandardsValidationOptions,
) -> Result<StandardsValidationReport> {
    let detection = detect_pdfa(engine);
    let target_label = options
        .target_profile
        .clone()
        .or_else(|| detection.claimed_label.clone())
        .unwrap_or_else(|| "PDF/A-2B".to_string());
    let profile_label = "pdfa";
    let mut rules: Vec<StandardsRuleResult> = Vec::new();

    // Metadata identifier sanity.
    if detection.malformed_identifier {
        rules.push(StandardsRuleResult::implemented(
            profile_label,
            "pdfa.metadata.identifier_wellformed",
            StandardsClauseRef::new("ISO 19005-1:2005", "6.7", "PDF/A identifier well-formed"),
            ValidationEvidence::at("/Root/Metadata"),
            RuleStatus::Fail,
            ValidationSeverity::Error,
            "XMP pdfaid:part/conformance is present but malformed.",
        ));
    }

    let part = pdfa_part_from_label(&target_label);
    let standard = pdfa_standard_for(part);
    let Some(profile) = pdfa_profile_from_label(&target_label) else {
        // Part 4 or conformance level not implemented by Wellfriend's rule engine.
        let (status_rule, reason) = if part == 4 {
            (
                "pdfa4.profile_support",
                "PDF/A-4 (ISO 19005-4:2020) rule execution is not implemented by Wellfriend's current rule engine; Prompt 27 veraPDF parity treats PDF/A-4 as an exact unsupported profile rather than a conformance pass.",
            )
        } else {
            (
                "pdfa.profile_support",
                "Requested PDF/A profile/conformance level is not implemented by Wellfriend's rule engine.",
            )
        };
        rules.push(StandardsRuleResult::unsupported(
            profile_label,
            status_rule,
            StandardsClauseRef::new(
                standard,
                if part == 4 { "4" } else { "6" },
                if part == 4 {
                    "PDF/A-4 profile support"
                } else {
                    "PDF/A profile support"
                },
            ),
            reason,
        ));
        return Ok(StandardsValidationReport::assemble(
            StandardsFamily::PdfA,
            detection.claimed_label.clone(),
            Some(target_label),
            detection,
            rules,
        ));
    };

    // Encryption is prohibited by PDF/A (checked directly).
    let encrypted = engine.document().reader().is_encrypted();
    rules.push(StandardsRuleResult::implemented(
        profile_label,
        "pdfa.encrypt.prohibited",
        StandardsClauseRef::new(standard, "6.1.3", "Encryption is prohibited"),
        ValidationEvidence::at("/Encrypt"),
        if encrypted {
            RuleStatus::Fail
        } else {
            RuleStatus::Pass
        },
        if encrypted {
            ValidationSeverity::Error
        } else {
            ValidationSeverity::Info
        },
        if encrypted {
            "Document is encrypted; PDF/A prohibits encryption."
        } else {
            "Document is not encrypted."
        },
    ));

    // Run the implemented compliance rule engine and map violations to clauses.
    let report = validate_pdfa(engine.document(), profile)?;
    for v in &report.violations {
        let (clause, title) = pdfa_clause_for(&v.rule);
        let (status, severity) = compliance_rule_status(v.severity);
        rules.push(StandardsRuleResult::implemented(
            profile_label,
            &format!("pdfa.{}", v.rule),
            StandardsClauseRef::new(standard, clause, title),
            ValidationEvidence::at(&v.location),
            status,
            severity,
            v.message.clone(),
        ));
    }

    // Positive rows for evaluated categories with no violation.
    add_pass_if_clean(
        &mut rules,
        &report.violations,
        "font",
        profile_label,
        "pdfa.fonts.embedded",
        standard,
        "All fonts are embedded.",
    );
    add_pass_if_clean(
        &mut rules,
        &report.violations,
        "output_intent",
        profile_label,
        "pdfa.output_intent.present",
        standard,
        "A PDF/A OutputIntent with an ICC profile is present.",
    );
    add_pass_if_clean(
        &mut rules,
        &report.violations,
        "xmp",
        profile_label,
        "pdfa.xmp.valid",
        standard,
        "XMP metadata packet is present and consistent.",
    );
    add_pass_if_clean(
        &mut rules,
        &report.violations,
        "action",
        profile_label,
        "pdfa.actions.allowed",
        standard,
        "No prohibited actions/JavaScript detected.",
    );
    if matches!(profile, PdfAProfile::PdfA3B | PdfAProfile::PdfA3A) {
        add_pass_if_clean(
            &mut rules,
            &report.violations,
            "embedded_file",
            profile_label,
            "pdfa3.embedded_file.afrelationship",
            standard,
            "Embedded files declare a valid /AFRelationship.",
        );
    }
    if matches!(profile, PdfAProfile::PdfA2A | PdfAProfile::PdfA3A) {
        add_pass_if_clean(
            &mut rules,
            &report.violations,
            "structure",
            profile_label,
            "pdfa.structure.tagged",
            standard,
            "Tagged logical structure basics are present (Level A).",
        );
    }

    // Prompt 27 closes the self-referential "deferred to Prompt 27" status.
    // Keep a deterministic implemented-with-limits row so callers can see that
    // veraPDF parity is evidence-backed for the selected corpus, while the
    // library still does not claim accredited certification or every ISO rule.
    rules.push(StandardsRuleResult::implemented(
        profile_label,
        "pdfa.corpus.verapdf_parity_prompt27",
        StandardsClauseRef::new(standard, "6", "Full ISO 19005 rule coverage"),
        ValidationEvidence::document()
            .with_detail("See target/prompt27-verapdf-crypto-fuzz/verapdf-parity-results.json"),
        RuleStatus::Warning,
        ValidationSeverity::Warning,
        "Prompt 27 runs veraPDF corpus parity for the supported profile scope; this row is not an accredited certification claim and records exact limits outside the selected corpus.",
    )
    .with_implementation_limits());

    Ok(StandardsValidationReport::assemble(
        StandardsFamily::PdfA,
        detection.claimed_label.clone(),
        Some(target_label),
        detection,
        rules,
    ))
}

// ---------------------------------------------------------------------------
// PDF/UA clause-mapped validator (ISO 14289-1)
// ---------------------------------------------------------------------------

fn pdfua_clause_for(rule_id: &str) -> (&'static str, &'static str) {
    let r = rule_id;
    if r.contains("tag") || r.contains("marked") {
        ("7.1", "Document is tagged; content is marked or artifacted")
    } else if r.contains("structtree") || r.contains("struct_tree") || r.contains("structure") {
        ("7.1", "StructTreeRoot present and reachable")
    } else if r.contains("parenttree") || r.contains("mcid") {
        ("7.1", "MCID/ParentTree consistency")
    } else if r.contains("rolemap") || r.contains("role") {
        ("7.1", "Role map validity for non-standard roles")
    } else if r.contains("alt") || r.contains("figure") {
        ("7.3", "Figures have alternate text or are artifacts")
    } else if r.contains("lang") {
        ("7.2", "Natural language is declared")
    } else if r.contains("table") {
        ("7.5", "Table structure and headers")
    } else if r.contains("heading") {
        ("7.4", "Heading nesting")
    } else if r.contains("link") {
        ("7.18", "Links carry meaningful structure")
    } else if r.contains("form") || r.contains("field") {
        ("7.18.1", "Form fields have accessible names")
    } else {
        ("7", "PDF/UA-1 accessibility requirement")
    }
}

/// Clause-mapped PDF/UA validation.
pub fn validate_pdfua_profile(
    engine: &ContentEngine,
    options: &StandardsValidationOptions,
) -> Result<StandardsValidationReport> {
    let detection = detect_pdfua(engine);
    let target_label = options
        .target_profile
        .clone()
        .or_else(|| detection.claimed_label.clone())
        .unwrap_or_else(|| "PDF/UA-1".to_string());
    let profile_label = "pdfua";
    let standard = "ISO 14289-1:2014";
    let mut rules: Vec<StandardsRuleResult> = Vec::new();

    if detection.malformed_identifier {
        rules.push(StandardsRuleResult::implemented(
            profile_label,
            "pdfua.metadata.identifier_wellformed",
            StandardsClauseRef::new(standard, "5", "PDF/UA identifier well-formed"),
            ValidationEvidence::at("/Root/Metadata"),
            RuleStatus::Fail,
            ValidationSeverity::Error,
            "XMP pdfuaid:part is present but not a supported value.",
        ));
    }

    let report = validate_pdfua(engine.document())?;
    for v in &report.violations {
        let (clause, title) = pdfua_clause_for(&v.rule);
        let (status, severity) = compliance_rule_status(v.severity);
        rules.push(StandardsRuleResult::implemented(
            profile_label,
            &format!("pdfua.{}", v.rule),
            StandardsClauseRef::new(standard, clause, title),
            ValidationEvidence::at(&v.location),
            status,
            severity,
            v.message.clone(),
        ));
    }

    if report.violations.is_empty() {
        rules.push(StandardsRuleResult::implemented(
            profile_label,
            "pdfua.structure.tagged_basics",
            StandardsClauseRef::new(standard, "7.1", "Tagged structure basics"),
            ValidationEvidence::document(),
            RuleStatus::Pass,
            ValidationSeverity::Info,
            "Tagging, StructTreeRoot, MCID mapping, and alt-text basics passed the implemented checks.",
        ));
    }

    // Human-judgment requirements Wellfriend cannot deterministically certify.
    rules.push(StandardsRuleResult::unsupported(
        profile_label,
        "pdfua.reading_order.human_judgment",
        StandardsClauseRef::new(standard, "7.2", "Meaningful reading order"),
        "Human-judgment reading-order correctness cannot be deterministically certified; structural reading-order diagnostics are provided instead.",
    ));
    rules.push(StandardsRuleResult::unsupported(
        profile_label,
        "pdfua.corpus.full_rule_coverage",
        StandardsClauseRef::new(standard, "7", "Full ISO 14289-1 rule coverage"),
        "Full ISO 14289-1 semantic/corpus parity remains outside Prompt 27's PDF/A-focused veraPDF unit and is reported exactly rather than counted as conformant.",
    ));

    Ok(StandardsValidationReport::assemble(
        StandardsFamily::PdfUa,
        detection.claimed_label.clone(),
        Some(target_label),
        detection,
        rules,
    ))
}

// ---------------------------------------------------------------------------
// PDF/X clause-mapped validator (ISO 15930 family)
// ---------------------------------------------------------------------------

fn pdfx_output_intent_has_icc(engine: &ContentEngine) -> (bool, bool) {
    let Ok(catalog) = engine.document().get_catalog() else {
        return (false, false);
    };
    let Some(PdfObject::Array(items)) = catalog.get("OutputIntents") else {
        return (false, false);
    };
    let mut has_gts = false;
    let mut has_icc = false;
    for item in items {
        let resolved = match item {
            PdfObject::Reference { number, generation } => engine
                .document()
                .reader()
                .get_and_resolve(*number, *generation)
                .ok(),
            other => Some(other.clone()),
        };
        if let Some(dict) = resolved.as_ref().and_then(PdfObject::as_dict) {
            if dict.get_name("S") == Some("GTS_PDFX") {
                has_gts = true;
                if dict.contains_key("DestOutputProfile") {
                    has_icc = true;
                }
            }
        }
    }
    (has_gts, has_icc)
}

/// Clause-mapped PDF/X validation.
pub fn validate_pdfx_profile(
    engine: &ContentEngine,
    options: &StandardsValidationOptions,
) -> Result<StandardsValidationReport> {
    let detection = detect_pdfx(engine);
    let target_label = options
        .target_profile
        .clone()
        .or_else(|| detection.claimed_label.clone())
        .unwrap_or_else(|| "PDF/X-4".to_string());
    let profile_label = "pdfx";
    let standard = "ISO 15930-7:2010"; // PDF/X-4 baseline; other parts noted per rule.
    let mut rules: Vec<StandardsRuleResult> = Vec::new();
    let doc = engine.document();

    // Output intent (GTS_PDFX) + ICC destination profile.
    let (has_gts, has_icc) = pdfx_output_intent_has_icc(engine);
    rules.push(StandardsRuleResult::implemented(
        profile_label,
        "pdfx.output_intent.present",
        StandardsClauseRef::new(standard, "6.2", "PDF/X output intent (GTS_PDFX) is present"),
        ValidationEvidence::at("/Root/OutputIntents"),
        if has_gts {
            RuleStatus::Pass
        } else {
            RuleStatus::Fail
        },
        if has_gts {
            ValidationSeverity::Info
        } else {
            ValidationSeverity::Error
        },
        if has_gts {
            "A GTS_PDFX output intent is present."
        } else {
            "PDF/X requires a GTS_PDFX output intent."
        },
    ));
    rules.push(
        StandardsRuleResult::implemented(
            profile_label,
            "pdfx.output_intent.icc",
            StandardsClauseRef::new(
                standard,
                "6.2.2",
                "Output intent has an ICC destination profile",
            ),
            ValidationEvidence::at("/Root/OutputIntents"),
            if has_icc {
                RuleStatus::Pass
            } else if has_gts {
                RuleStatus::Fail
            } else {
                RuleStatus::NotApplicable
            },
            if has_icc || !has_gts {
                ValidationSeverity::Info
            } else {
                ValidationSeverity::Error
            },
            if has_icc {
                "Output intent references a DestOutputProfile ICC stream."
            } else if has_gts {
                "GTS_PDFX output intent is missing a DestOutputProfile ICC stream."
            } else {
                "No GTS_PDFX output intent; ICC check not applicable."
            },
        )
        .with_implementation_limits(),
    );

    // Encryption prohibited.
    let encrypted = doc.reader().is_encrypted();
    rules.push(StandardsRuleResult::implemented(
        profile_label,
        "pdfx.encrypt.prohibited",
        StandardsClauseRef::new(standard, "6.1", "Encryption is prohibited"),
        ValidationEvidence::at("/Encrypt"),
        if encrypted {
            RuleStatus::Fail
        } else {
            RuleStatus::Pass
        },
        if encrypted {
            ValidationSeverity::Error
        } else {
            ValidationSeverity::Info
        },
        if encrypted {
            "Document is encrypted; PDF/X prohibits encryption."
        } else {
            "Document is not encrypted."
        },
    ));

    // Fonts embedded.
    let fonts = engine.list_fonts()?;
    let non_embedded: Vec<&crate::fonts_report::FontInfo> =
        fonts.iter().filter(|f| !f.embedded).collect();
    if non_embedded.is_empty() {
        rules.push(StandardsRuleResult::implemented(
            profile_label,
            "pdfx.fonts.embedded",
            StandardsClauseRef::new(standard, "6.2.4", "All fonts are embedded"),
            ValidationEvidence::document(),
            RuleStatus::Pass,
            ValidationSeverity::Info,
            "All fonts are embedded.",
        ));
    } else {
        for font in non_embedded {
            rules.push(StandardsRuleResult::implemented(
                profile_label,
                "pdfx.fonts.embedded",
                StandardsClauseRef::new(standard, "6.2.4", "All fonts are embedded"),
                ValidationEvidence::at(&format!(
                    "object {} {}",
                    font.object_number, font.generation
                )),
                RuleStatus::Fail,
                ValidationSeverity::Error,
                format!(
                    "Font '{}' is not embedded; PDF/X requires embedding.",
                    font.name
                ),
            ));
        }
    }

    // Page geometry: TrimBox (or ArtBox) required per page.
    let pages = doc.get_pages()?;
    let mut trim_missing = 0usize;
    for page in &pages {
        if let Ok(obj) = doc
            .reader()
            .get_and_resolve(page.object_number, page.generation_number)
        {
            if let Some(dict) = obj.as_dict() {
                if !dict.contains_key("TrimBox") && !dict.contains_key("ArtBox") {
                    trim_missing += 1;
                }
            }
        }
    }
    rules.push(StandardsRuleResult::implemented(
        profile_label,
        "pdfx.page.trim_or_art_box",
        StandardsClauseRef::new(standard, "6.3", "Each page defines TrimBox or ArtBox"),
        ValidationEvidence::at("pages"),
        if trim_missing == 0 {
            RuleStatus::Pass
        } else {
            RuleStatus::Fail
        },
        if trim_missing == 0 {
            ValidationSeverity::Info
        } else {
            ValidationSeverity::Error
        },
        if trim_missing == 0 {
            "All pages define a TrimBox or ArtBox.".to_string()
        } else {
            format!("{trim_missing} page(s) lack a TrimBox/ArtBox.")
        },
    ));

    // Prohibited active content / interactivity.
    let risky = crate::security::scan_risky_content(doc)?;
    rules.push(StandardsRuleResult::implemented(
        profile_label,
        "pdfx.active_content.prohibited",
        StandardsClauseRef::new(standard, "6.6", "No prohibited actions/interactivity"),
        ValidationEvidence::document(),
        if risky.risky_total() == 0 {
            RuleStatus::Pass
        } else {
            RuleStatus::Fail
        },
        if risky.risky_total() == 0 {
            ValidationSeverity::Info
        } else {
            ValidationSeverity::Error
        },
        if risky.risky_total() == 0 {
            "No active content detected.".to_string()
        } else {
            format!(
                "{} active-content item(s) detected; PDF/X prohibits them.",
                risky.risky_total()
            )
        },
    ));

    // Colour / transparency / OPI / trapping depth are exact limits rather than
    // Prompt 27 self-deferrals.
    rules.push(StandardsRuleResult::unsupported(
        profile_label,
        "pdfx.color.full_colorant_validation",
        StandardsClauseRef::new(standard, "6.2.3", "Colorant/overprint/spot validation"),
        "Full DeviceN/Separation/overprint colorant validation is outside the current PDF/X implemented scope; output-intent ICC presence is checked here.",
    ));
    rules.push(StandardsRuleResult::unsupported(
        profile_label,
        "pdfx.transparency.profile_restrictions",
        StandardsClauseRef::new(
            "ISO 15930-1:2001",
            "6",
            "Transparency restrictions (X-1a/X-3)",
        ),
        "Transparency prohibition for older PDF/X profiles is outside the current implemented scope and reported exactly.",
    ));

    Ok(StandardsValidationReport::assemble(
        StandardsFamily::PdfX,
        detection.claimed_label.clone(),
        Some(target_label),
        detection,
        rules,
    ))
}

// ---------------------------------------------------------------------------
// Combined + cross-profile drivers
// ---------------------------------------------------------------------------

/// Validate a single family with the given options.
pub fn validate_standards_family(
    engine: &ContentEngine,
    family: StandardsFamily,
    options: &StandardsValidationOptions,
) -> Result<StandardsValidationReport> {
    match family {
        StandardsFamily::PdfA => validate_pdfa_profile(engine, options),
        StandardsFamily::PdfUa => validate_pdfua_profile(engine, options),
        StandardsFamily::PdfX => validate_pdfx_profile(engine, options),
    }
}

/// Validate PDF/A, PDF/UA, and PDF/X and compute cross-profile conflicts. A
/// single profile passing never hides another failing.
pub fn validate_all_standards(
    engine: &ContentEngine,
    options: &StandardsValidationOptions,
) -> Result<CrossProfileConflictReport> {
    let pdfa = validate_pdfa_profile(engine, options)?;
    let pdfua = validate_pdfua_profile(engine, options)?;
    let pdfx = validate_pdfx_profile(engine, options)?;

    let mut conflicts = Vec::new();
    let encrypted = engine.document().reader().is_encrypted();
    if encrypted {
        conflicts.push(CrossProfileConflict {
            left_profile: "PDF/A".to_string(),
            right_profile: "PDF/X".to_string(),
            severity: ValidationSeverity::Error,
            description:
                "Document is encrypted, which is invalid for both PDF/A and PDF/X workflows."
                    .to_string(),
        });
    }
    if pdfa.is_conformant() && matches!(pdfua.conformance, ConformanceStatus::NonConformant) {
        conflicts.push(CrossProfileConflict {
            left_profile: "PDF/A".to_string(),
            right_profile: "PDF/UA".to_string(),
            severity: ValidationSeverity::Warning,
            description:
                "PDF/A (level B) validation passed but PDF/UA tagged-accessibility validation failed; a level-B archive is not necessarily accessible."
                    .to_string(),
        });
    }
    if pdfa.is_conformant() && matches!(pdfx.conformance, ConformanceStatus::NonConformant) {
        conflicts.push(CrossProfileConflict {
            left_profile: "PDF/A".to_string(),
            right_profile: "PDF/X".to_string(),
            severity: ValidationSeverity::Warning,
            description:
                "PDF/A validation passed but PDF/X prepress validation failed (e.g. output intent or page boxes); archival and print conformance are distinct."
                    .to_string(),
        });
    }

    Ok(assemble_cross_profile(vec![pdfa, pdfua, pdfx], conflicts))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xmp_lookup_reads_attribute_and_element_forms() {
        let attr = r#"<rdf:Description pdfaid:part="2" pdfaid:conformance="B"/>"#;
        assert_eq!(xmp_lookup(attr, "pdfaid", "part").as_deref(), Some("2"));
        assert_eq!(
            xmp_lookup(attr, "pdfaid", "conformance").as_deref(),
            Some("B")
        );

        let elem = "<pdfaid:part>3</pdfaid:part><pdfaid:conformance>A</pdfaid:conformance>";
        assert_eq!(xmp_lookup(elem, "pdfaid", "part").as_deref(), Some("3"));
        assert_eq!(
            xmp_lookup(elem, "pdfaid", "conformance").as_deref(),
            Some("A")
        );

        assert_eq!(xmp_lookup("no identifiers here", "pdfaid", "part"), None);
    }

    #[test]
    fn pdfx_version_normalization() {
        assert_eq!(
            normalize_pdfx_version("PDF/X-4").as_deref(),
            Some("PDF/X-4")
        );
        assert_eq!(
            normalize_pdfx_version("PDF/X-1a:2001").as_deref(),
            Some("PDF/X-1A")
        );
        assert_eq!(normalize_pdfx_version("nonsense"), None);
    }

    #[test]
    fn counts_and_conformance_derivation() {
        let clause = StandardsClauseRef::new("ISO 19005-2:2011", "6.1", "test");
        let rules = vec![
            StandardsRuleResult::implemented(
                "pdfa",
                "pdfa.test.pass",
                clause.clone(),
                ValidationEvidence::document(),
                RuleStatus::Pass,
                ValidationSeverity::Info,
                "ok",
            ),
            StandardsRuleResult::unsupported("pdfa", "pdfa.test.unsupported", clause.clone(), "x"),
        ];
        let counts = StandardsRuleCounts::tally(&rules);
        assert_eq!(counts.total, 2);
        assert_eq!(counts.pass, 1);
        assert_eq!(counts.unsupported_reported_exact, 1);
        // A pass + an unsupported row => Indeterminate (never a clean pass).
        assert_eq!(derive_conformance(&rules), ConformanceStatus::Indeterminate);

        let mut failing = rules.clone();
        failing.push(StandardsRuleResult::implemented(
            "pdfa",
            "pdfa.test.fail",
            clause,
            ValidationEvidence::document(),
            RuleStatus::Fail,
            ValidationSeverity::Error,
            "bad",
        ));
        assert_eq!(
            derive_conformance(&failing),
            ConformanceStatus::NonConformant
        );
    }

    #[test]
    fn cross_profile_pass_does_not_hide_fail() {
        let clause = StandardsClauseRef::new("ISO 19005-2:2011", "6.1", "t");
        let ok = StandardsValidationReport::assemble(
            StandardsFamily::PdfA,
            None,
            None,
            ProfileDetection::default(),
            vec![StandardsRuleResult::implemented(
                "pdfa",
                "pdfa.ok",
                clause.clone(),
                ValidationEvidence::document(),
                RuleStatus::Pass,
                ValidationSeverity::Info,
                "ok",
            )],
        );
        let bad = StandardsValidationReport::assemble(
            StandardsFamily::PdfUa,
            None,
            None,
            ProfileDetection::default(),
            vec![StandardsRuleResult::implemented(
                "pdfua",
                "pdfua.bad",
                clause,
                ValidationEvidence::document(),
                RuleStatus::Fail,
                ValidationSeverity::Error,
                "bad",
            )],
        );
        let combined = assemble_cross_profile(vec![ok, bad], Vec::new());
        assert!(!combined.overall_pass);
    }

    fn authored_engine() -> ContentEngine {
        use crate::authoring::{PageSize, PdfBuilder};
        let mut builder = PdfBuilder::new();
        builder.add_page(PageSize::custom(300.0, 300.0));
        let bytes = builder.to_bytes().expect("authored pdf");
        ContentEngine::open_bytes(bytes).expect("open authored pdf")
    }

    #[test]
    fn pdfa_missing_output_intent_is_nonconformant_and_clause_mapped() {
        let engine = authored_engine();
        let report = validate_pdfa_profile(
            &engine,
            &StandardsValidationOptions::with_target("PDF/A-2B"),
        )
        .unwrap();
        assert_eq!(report.family, StandardsFamily::PdfA);
        // Every rule is clause-mapped (non-empty standard + clause).
        assert!(report
            .rules
            .iter()
            .all(|r| !r.clause.standard.is_empty() && !r.clause.clause.is_empty()));
        assert!(report
            .rules
            .iter()
            .any(|r| r.rule_id.contains("output_intent") && matches!(r.status, RuleStatus::Fail)));
        assert_ne!(report.conformance, ConformanceStatus::Conformant);
        assert!(report.rules.iter().any(|r| {
            r.rule_id == "pdfa.corpus.verapdf_parity_prompt27"
                && matches!(r.status, RuleStatus::Warning)
                && matches!(r.implementation, RuleImplementation::ImplementedWithLimits)
        }));
    }

    #[test]
    fn pdfa4_target_is_unsupported_exact_not_falsely_passed() {
        let engine = authored_engine();
        let report =
            validate_pdfa_profile(&engine, &StandardsValidationOptions::with_target("PDF/A-4"))
                .unwrap();
        assert!(report.rules.iter().any(|r| {
            r.rule_id == "pdfa4.profile_support"
                && matches!(r.status, RuleStatus::UnsupportedReportedExact)
        }));
        assert_ne!(report.conformance, ConformanceStatus::Conformant);
    }

    #[test]
    fn pdfx_missing_output_intent_fails() {
        let engine = authored_engine();
        let report =
            validate_pdfx_profile(&engine, &StandardsValidationOptions::with_target("PDF/X-4"))
                .unwrap();
        assert!(report
            .rules
            .iter()
            .any(|r| r.rule_id == "pdfx.output_intent.present"
                && matches!(r.status, RuleStatus::Fail)));
        assert!(report
            .rules
            .iter()
            .any(|r| matches!(r.status, RuleStatus::UnsupportedReportedExact)));
    }

    #[test]
    fn pdfua_reports_human_judgment_as_unsupported_exact() {
        let engine = authored_engine();
        let report =
            validate_pdfua_profile(&engine, &StandardsValidationOptions::default()).unwrap();
        assert_eq!(report.family, StandardsFamily::PdfUa);
        assert!(report
            .rules
            .iter()
            .any(|r| matches!(r.status, RuleStatus::UnsupportedReportedExact)));
    }

    #[test]
    fn cross_profile_runs_all_three_and_does_not_falsely_pass() {
        let engine = authored_engine();
        let combined =
            validate_all_standards(&engine, &StandardsValidationOptions::default()).unwrap();
        assert_eq!(combined.reports.len(), 3);
        // An authored non-archival/print/tagged doc must not report overall pass.
        assert!(!combined.overall_pass);
    }
}
