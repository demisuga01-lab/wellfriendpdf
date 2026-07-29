//! Standards-validation profile reports.
//!
//! These are veraPDF-style rule containers for Wellfriend's supported validation
//! subsets. They are not legal certification claims; each rule states exactly
//! what Wellfriend evaluated.

use serde::{Deserialize, Serialize};

use crate::compliance::{validate_pdfa, validate_pdfua, ComplianceSeverity, PdfAProfile};
use crate::object::{PdfDictionary, PdfObject};
use crate::parser_report::arlington_status;
use crate::security::scan_risky_content;
use crate::{ContentEngine, Result};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StandardsProfile {
    PdfA,
    PdfUa,
    PdfX,
    Security,
    All,
}

impl StandardsProfile {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "pdfa" | "pdf/a" | "pdf-a" => Some(Self::PdfA),
            "pdfua" | "pdf/ua" | "pdf-ua" => Some(Self::PdfUa),
            "pdfx" | "pdf/x" | "pdf-x" => Some(Self::PdfX),
            "security" => Some(Self::Security),
            "all" => Some(Self::All),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationStatus {
    Pass,
    Fail,
    Warn,
    NotApplicable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ValidationRuleResult {
    pub profile: String,
    pub rule_id: String,
    pub severity: ValidationSeverity,
    pub status: ValidationStatus,
    pub location: String,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StandardsValidationReport {
    pub schema_version: u32,
    pub profile: StandardsProfile,
    pub passed: bool,
    pub rules: Vec<ValidationRuleResult>,
    pub arlington_status: String,
    pub arlington_source: String,
    pub arlington_commit: String,
    pub arlington_rule_count: usize,
    pub certification_claimed: bool,
}

pub fn validate_standards_profile(
    engine: &ContentEngine,
    profile: StandardsProfile,
) -> Result<StandardsValidationReport> {
    let mut rules = Vec::new();
    match profile {
        StandardsProfile::PdfA => add_pdfa_rules(engine, &mut rules)?,
        StandardsProfile::PdfUa => add_pdfua_rules(engine, &mut rules)?,
        StandardsProfile::PdfX => add_pdfx_rules(engine, &mut rules)?,
        StandardsProfile::Security => add_security_rules(engine, &mut rules)?,
        StandardsProfile::All => {
            add_pdfa_rules(engine, &mut rules)?;
            add_pdfua_rules(engine, &mut rules)?;
            add_pdfx_rules(engine, &mut rules)?;
            add_security_rules(engine, &mut rules)?;
        }
    }
    add_arlington_rule(&mut rules);
    let arlington = arlington_status();
    let passed = rules
        .iter()
        .all(|rule| !matches!(rule.status, ValidationStatus::Fail));
    Ok(StandardsValidationReport {
        schema_version: 1,
        profile,
        passed,
        rules,
        arlington_status: arlington.status,
        arlington_source: arlington.source,
        arlington_commit: arlington.commit,
        arlington_rule_count: arlington.keys,
        certification_claimed: false,
    })
}

fn add_pdfa_rules(engine: &ContentEngine, rules: &mut Vec<ValidationRuleResult>) -> Result<()> {
    let report = validate_pdfa(engine.document(), PdfAProfile::PdfA2B)?;
    if report.violations.is_empty() {
        rules.push(pass(
            "pdfa",
            "pdfa.supported_subset",
            "document",
            "Wellfriend supported PDF/A-2B color/font/action subset passed.",
        ));
    }
    for violation in report.violations {
        rules.push(from_compliance("pdfa", violation));
    }
    rules.push(warn(
        "pdfa",
        "pdfa.certification_scope",
        "document",
        "Wellfriend reports a supported PDF/A subset only; external validator certification is not claimed.",
    ));
    Ok(())
}

fn add_pdfua_rules(engine: &ContentEngine, rules: &mut Vec<ValidationRuleResult>) -> Result<()> {
    let report = validate_pdfua(engine.document())?;
    if report.violations.is_empty() {
        rules.push(pass(
            "pdfua",
            "pdfua.supported_subset",
            "document",
            "Wellfriend supported PDF/UA tag/MCID/alt-text subset passed.",
        ));
    }
    for violation in report.violations {
        rules.push(from_compliance("pdfua", violation));
    }
    rules.push(warn(
        "pdfua",
        "pdfua.certification_scope",
        "document",
        "Wellfriend reports a supported PDF/UA subset only; complete accessibility certification is not claimed.",
    ));
    Ok(())
}

fn add_pdfx_rules(engine: &ContentEngine, rules: &mut Vec<ValidationRuleResult>) -> Result<()> {
    let doc = engine.document();
    let catalog = doc.get_catalog()?;
    let has_pdfx_output_intent = has_pdfx_output_intent(&catalog, engine);
    if has_pdfx_output_intent {
        rules.push(pass(
            "pdfx",
            "pdfx.output_intent",
            "/Root/OutputIntents",
            "PDF/X output intent is present.",
        ));
    } else {
        rules.push(fail(
            "pdfx",
            "pdfx.output_intent",
            "/Root/OutputIntents",
            "PDF/X validation subset requires a GTS_PDFX output intent.",
        ));
    }

    let pages = doc.get_pages()?;
    let mut trim_missing = 0usize;
    for page in &pages {
        let page_obj = doc
            .reader()
            .get_and_resolve(page.object_number, page.generation_number)?;
        let Some(dict) = page_obj.as_dict() else {
            continue;
        };
        if !dict.contains_key("TrimBox") {
            trim_missing += 1;
        }
    }
    if trim_missing == 0 {
        rules.push(pass(
            "pdfx",
            "pdfx.trim_box",
            "pages",
            "All pages expose explicit TrimBox entries in the supported subset.",
        ));
    } else {
        rules.push(warn(
            "pdfx",
            "pdfx.trim_box",
            "pages",
            &format!("{trim_missing} page(s) have no explicit TrimBox; inherited/prepress box validation remains bounded."),
        ));
    }

    let risky = scan_risky_content(doc)?;
    if risky.risky_total() == 0 {
        rules.push(pass(
            "pdfx",
            "pdfx.active_content",
            "document",
            "No active content detected by the Annotation Ocg Rendering scanner.",
        ));
    } else {
        rules.push(fail(
            "pdfx",
            "pdfx.active_content",
            "document",
            &format!(
                "{} risky active-content item(s) detected; PDF/X/prepress workflows require sanitization.",
                risky.risky_total()
            ),
        ));
    }
    rules.push(warn(
        "pdfx",
        "pdfx.certification_scope",
        "document",
        "Wellfriend reports a supported PDF/X color/prepress subset only; complete PDF/X certification is not claimed.",
    ));
    Ok(())
}

fn add_security_rules(engine: &ContentEngine, rules: &mut Vec<ValidationRuleResult>) -> Result<()> {
    let report = crate::security::security_report(engine)?;
    if report.risky_content.risky_total() == 0 {
        rules.push(pass(
            "security",
            "security.active_content",
            "document",
            "No risky active content was detected.",
        ));
    } else {
        rules.push(fail(
            "security",
            "security.active_content",
            "document",
            &format!(
                "{} risky active-content item(s) detected.",
                report.risky_content.risky_total()
            ),
        ));
    }
    if report.public_key_security_handler_detected {
        rules.push(warn(
            "security",
            "security.public_key_handler",
            "/Encrypt",
            "Public-key security handler detected; explicit-provider PubSec decrypt is supported for scoped KeyTrans profiles, with no certificate-trust claim.",
        ));
    }
    if report.aes_gcm_detected && !report.aes_gcm_supported {
        rules.push(warn(
            "security",
            "security.aes_gcm",
            "/Encrypt",
            "AES-GCM crypt filter detected outside the supported AESV4 Standard-handler shape or without full ISO/TS 32004 PDF-MAC validation.",
        ));
    }
    Ok(())
}

fn add_arlington_rule(rules: &mut Vec<ValidationRuleResult>) {
    let arlington = arlington_status();
    rules.push(ValidationRuleResult {
        profile: "arlington".to_string(),
        rule_id: "arlington.generated_tables".to_string(),
        severity: ValidationSeverity::Info,
        status: ValidationStatus::Pass,
        location: "parser".to_string(),
        message: format!(
            "Generated Arlington model active: {} keys from {} at {}. Unsupported predicates are reported, not silently evaluated.",
            arlington.keys, arlington.source, arlington.commit
        ),
    });
}

fn has_pdfx_output_intent(catalog: &PdfDictionary, engine: &ContentEngine) -> bool {
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

fn from_compliance(
    profile: &str,
    violation: crate::compliance::ComplianceViolation,
) -> ValidationRuleResult {
    let severity = match violation.severity {
        ComplianceSeverity::Error => ValidationSeverity::Error,
        ComplianceSeverity::Warning => ValidationSeverity::Warning,
    };
    let status = match violation.severity {
        ComplianceSeverity::Error => ValidationStatus::Fail,
        ComplianceSeverity::Warning => ValidationStatus::Warn,
    };
    ValidationRuleResult {
        profile: profile.to_string(),
        rule_id: violation.rule,
        severity,
        status,
        location: violation.location,
        message: violation.message,
    }
}

fn pass(profile: &str, rule_id: &str, location: &str, message: &str) -> ValidationRuleResult {
    ValidationRuleResult {
        profile: profile.to_string(),
        rule_id: rule_id.to_string(),
        severity: ValidationSeverity::Info,
        status: ValidationStatus::Pass,
        location: location.to_string(),
        message: message.to_string(),
    }
}

fn warn(profile: &str, rule_id: &str, location: &str, message: &str) -> ValidationRuleResult {
    ValidationRuleResult {
        profile: profile.to_string(),
        rule_id: rule_id.to_string(),
        severity: ValidationSeverity::Warning,
        status: ValidationStatus::Warn,
        location: location.to_string(),
        message: message.to_string(),
    }
}

fn fail(profile: &str, rule_id: &str, location: &str, message: &str) -> ValidationRuleResult {
    ValidationRuleResult {
        profile: profile.to_string(),
        rule_id: rule_id.to_string(),
        severity: ValidationSeverity::Error,
        status: ValidationStatus::Fail,
        location: location.to_string(),
        message: message.to_string(),
    }
}
