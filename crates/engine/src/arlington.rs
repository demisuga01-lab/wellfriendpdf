use serde::{Deserialize, Serialize};

use crate::object::{PdfDictionary, PdfObject};
use crate::parser_report::{ParserCategory, ParserDiagnostic, ParserSeverity};

mod generated {
    include!("generated/arlington_tables.rs");
}

/// Validation mode for Arlington-style dictionary checks.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArlingtonValidationMode {
    Strict,
    Permissive,
    Audit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ArlingtonValueType {
    Any,
    Name,
    Integer,
    Number,
    Boolean,
    String,
    Array,
    Dictionary,
    Stream,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ArlingtonIndirectPolicy {
    Any,
    AllowsIndirect,
    MustBeDirect,
    MustBeIndirect,
}

#[derive(Clone, Copy, Debug)]
struct ArlingtonRule {
    object_type: &'static str,
    key: &'static str,
    required: bool,
    value_types: &'static [ArlingtonValueType],
    allowed_names: &'static [&'static str],
    since_version: Option<&'static str>,
    deprecated_in: Option<&'static str>,
    link: Option<&'static str>,
    indirect_policy: ArlingtonIndirectPolicy,
    unsupported_predicates: &'static [&'static str],
}

const RULES: &[ArlingtonRule] = generated::ARLINGTON_RULES;

/// Coverage metadata for the generated Arlington tables in this build.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArlingtonCoverage {
    pub source: String,
    pub commit: String,
    pub tsv_files: usize,
    pub object_models: usize,
    pub keys: usize,
    pub required_key_rules: usize,
    pub type_rules: usize,
    pub version_rules: usize,
    pub indirect_reference_rules: usize,
    pub link_rules: usize,
    pub unsupported_predicates: usize,
    pub parse_warnings: usize,
}

pub fn arlington_coverage() -> ArlingtonCoverage {
    ArlingtonCoverage {
        source: generated::ARLINGTON_SOURCE.to_string(),
        commit: generated::ARLINGTON_COMMIT.to_string(),
        tsv_files: generated::ARLINGTON_TSV_FILES,
        object_models: generated::ARLINGTON_OBJECT_MODELS,
        keys: generated::ARLINGTON_KEYS,
        required_key_rules: generated::ARLINGTON_REQUIRED_KEY_RULES,
        type_rules: generated::ARLINGTON_TYPE_RULES,
        version_rules: generated::ARLINGTON_VERSION_RULES,
        indirect_reference_rules: generated::ARLINGTON_INDIRECT_REFERENCE_RULES,
        link_rules: generated::ARLINGTON_LINK_RULES,
        unsupported_predicates: generated::ARLINGTON_UNSUPPORTED_PREDICATES,
        parse_warnings: generated::ARLINGTON_PARSE_WARNINGS,
    }
}

/// Validate one dictionary against the generated Arlington rule tables.
pub fn validate_arlington_dictionary(
    object_type: &str,
    dict: &PdfDictionary,
) -> Vec<ParserDiagnostic> {
    validate_arlington_dictionary_at_path(object_type, dict, ArlingtonValidationMode::Audit, "/")
}

/// Validate one dictionary and include a caller-supplied object path in diagnostics.
pub fn validate_arlington_dictionary_at_path(
    object_type: &str,
    dict: &PdfDictionary,
    _mode: ArlingtonValidationMode,
    path: &str,
) -> Vec<ParserDiagnostic> {
    let mut diagnostics = Vec::new();
    for rule in RULES.iter().filter(|rule| rule.object_type == object_type) {
        let Some(value) = dict.get(rule.key) else {
            if rule.required {
                diagnostics.push(arlington_diagnostic(
                    ParserSeverity::RecoverableError,
                    "arlington_required_key_missing",
                    format!(
                        "/{object_type} dictionary is missing required /{}",
                        rule.key
                    ),
                    path,
                    rule,
                    None,
                ));
            }
            continue;
        };

        let type_ok = value_matches_any_type(value, rule.value_types, rule.indirect_policy);
        if !type_ok {
            diagnostics.push(arlington_diagnostic(
                ParserSeverity::RecoverableError,
                "arlington_wrong_object_type",
                format!(
                    "/{object_type} /{} expected {}, got {}",
                    rule.key,
                    expected_types(rule),
                    value.variant_name()
                ),
                path,
                rule,
                Some(value),
            ));
        }

        if let Some(indirect) = indirect_policy_diagnostic(value, rule) {
            diagnostics.push(arlington_diagnostic(
                ParserSeverity::RecoverableError,
                "arlington_indirect_reference_policy_failed",
                indirect,
                path,
                rule,
                Some(value),
            ));
        }

        if type_ok && !rule.allowed_names.is_empty() {
            let Some(name) = value.as_name() else {
                continue;
            };
            if !rule.allowed_names.contains(&name) {
                diagnostics.push(arlington_diagnostic(
                    ParserSeverity::RecoverableError,
                    "arlington_invalid_name_value",
                    format!("/{object_type} /{} has invalid name /{name}", rule.key),
                    path,
                    rule,
                    Some(value),
                ));
            }
        }

        if let Some(deprecated) = rule.deprecated_in {
            diagnostics.push(arlington_diagnostic(
                ParserSeverity::Info,
                "arlington_deprecated_key",
                format!(
                    "/{object_type} /{} is deprecated in PDF {deprecated}",
                    rule.key
                ),
                path,
                rule,
                Some(value),
            ));
        }

        for predicate in rule.unsupported_predicates {
            diagnostics.push(arlington_diagnostic(
                ParserSeverity::Info,
                "arlington_unsupported_predicate",
                format!(
                    "Arlington predicate {predicate} for /{object_type} /{} is not evaluated yet",
                    rule.key
                ),
                path,
                rule,
                Some(value),
            ));
        }
    }
    diagnostics
}

fn arlington_diagnostic(
    severity: ParserSeverity,
    code: impl Into<String>,
    message: impl Into<String>,
    path: &str,
    rule: &ArlingtonRule,
    actual: Option<&PdfObject>,
) -> ParserDiagnostic {
    ParserDiagnostic::new(severity, ParserCategory::Validation, code, message)
        .with_source("arlington")
        .with_path(format!("{}/{}", path.trim_end_matches('/'), rule.key))
        .with_key(rule.key)
        .with_expected(expected_types(rule))
        .with_actual(actual.map_or("missing", PdfObject::variant_name))
}

fn indirect_policy_diagnostic(value: &PdfObject, rule: &ArlingtonRule) -> Option<String> {
    match rule.indirect_policy {
        ArlingtonIndirectPolicy::MustBeDirect if value.as_reference().is_some() => Some(format!(
            "/{} must be direct according to Arlington",
            rule.key
        )),
        ArlingtonIndirectPolicy::MustBeIndirect if value.as_reference().is_none() => Some(format!(
            "/{} must be indirect according to Arlington",
            rule.key
        )),
        _ => None,
    }
}

fn value_matches_any_type(
    value: &PdfObject,
    expected: &[ArlingtonValueType],
    indirect_policy: ArlingtonIndirectPolicy,
) -> bool {
    if matches!(
        indirect_policy,
        ArlingtonIndirectPolicy::AllowsIndirect | ArlingtonIndirectPolicy::MustBeIndirect
    ) && value.as_reference().is_some()
    {
        return true;
    }
    expected
        .iter()
        .any(|expected| value_matches_type(value, *expected))
}

fn value_matches_type(value: &PdfObject, expected: ArlingtonValueType) -> bool {
    match expected {
        ArlingtonValueType::Any => true,
        ArlingtonValueType::Name => value.as_name().is_some(),
        ArlingtonValueType::Integer => value.as_integer().is_some(),
        ArlingtonValueType::Number => value.as_number().is_some(),
        ArlingtonValueType::Boolean => value.as_boolean().is_some(),
        ArlingtonValueType::String => value.as_string().is_some(),
        ArlingtonValueType::Array => value.as_array().is_some(),
        ArlingtonValueType::Dictionary => value.as_dict().is_some(),
        ArlingtonValueType::Stream => value.as_stream().is_some(),
    }
}

fn expected_types(rule: &ArlingtonRule) -> String {
    let mut types = rule
        .value_types
        .iter()
        .map(|value_type| format!("{value_type:?}"))
        .collect::<Vec<_>>();
    if matches!(
        rule.indirect_policy,
        ArlingtonIndirectPolicy::AllowsIndirect | ArlingtonIndirectPolicy::MustBeIndirect
    ) {
        types.push("Reference".to_string());
    }
    if let Some(since) = rule.since_version {
        types.push(format!("since:{since}"));
    }
    if let Some(link) = rule.link {
        types.push(format!("link:{link}"));
    }
    types.join("|")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn dict(entries: &[(&str, PdfObject)]) -> PdfDictionary {
        PdfDictionary::new(
            entries
                .iter()
                .map(|(key, value)| ((*key).to_string(), value.clone()))
                .collect::<BTreeMap<_, _>>(),
        )
    }

    #[test]
    fn validates_required_catalog_keys() {
        let dict = dict(&[("Type", PdfObject::Name("Catalog".to_string()))]);

        let diagnostics = validate_arlington_dictionary_at_path(
            "Catalog",
            &dict,
            ArlingtonValidationMode::Audit,
            "/Root",
        );

        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "arlington_required_key_missing"));
    }

    #[test]
    fn reports_wrong_type_and_unsupported_predicate() {
        let dict = dict(&[
            ("Type", PdfObject::Name("Catalog".to_string())),
            ("Pages", PdfObject::Integer(2)),
            ("Dests", PdfObject::Dictionary(PdfDictionary::empty())),
        ]);

        let diagnostics = validate_arlington_dictionary_at_path(
            "Catalog",
            &dict,
            ArlingtonValidationMode::Audit,
            "/Root",
        );

        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "arlington_wrong_object_type"));
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "arlington_unsupported_predicate"));
    }

    #[test]
    fn generated_tables_include_real_upstream_rules() {
        let coverage = arlington_coverage();
        assert!(coverage.tsv_files > 100);
        assert!(coverage.keys > 1000);
        assert_ne!(coverage.commit, "mock");
        assert!(RULES.iter().any(|rule| rule.object_type == "XRefStream"));
        assert!(RULES.iter().any(|rule| rule.object_type == "ObjectStream"));
        assert!(RULES.iter().any(|rule| rule.object_type == "AnnotWidget"));
        assert!(RULES
            .iter()
            .any(|rule| rule.object_type == "InteractiveForm"));
        assert!(RULES.iter().any(|rule| rule.object_type == "FileTrailer"));
    }
}
