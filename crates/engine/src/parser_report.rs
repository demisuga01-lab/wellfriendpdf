use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::cancel::CancelToken;
use crate::filters::{
    decode_stream_report_from_dict_scheduled, DecodeDiagnostic, DecodeLimits, DecodeReport,
    DecodeSeverity,
};
use crate::reader::PdfReader;

/// Parser mode used for parser-report and validation entry points.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParserMode {
    /// Require a syntactically valid header, startxref, xref chain, and trailer.
    Strict,
    /// Use Wellfriend's bounded repair fallbacks when strict opening fails.
    Repair,
    /// Run strict first, then repair, and return diagnostics for both outcomes.
    Audit,
}

/// Stable parser diagnostic severity.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParserSeverity {
    Info,
    Warning,
    RecoverableError,
    FatalError,
    SecurityLimit,
}

/// Stable parser diagnostic category.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParserCategory {
    Lexer,
    ObjectSyntax,
    Xref,
    Trailer,
    IncrementalUpdate,
    ObjectStream,
    StreamBoundary,
    FilterDecode,
    Encryption,
    Linearization,
    Validation,
    Repair,
    Source,
    ResourceLimit,
}

/// Machine-readable parser diagnostic for SDK audit/reporting surfaces.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ParserDiagnostic {
    pub severity: ParserSeverity,
    pub category: ParserCategory,
    pub source: Option<String>,
    pub code: String,
    pub message: String,
    pub path: Option<String>,
    pub key: Option<String>,
    pub expected: Option<String>,
    pub actual: Option<String>,
    pub offset: Option<usize>,
    pub byte_range: Option<(usize, usize)>,
    pub object: Option<(u32, u16)>,
    pub page: Option<usize>,
    pub recovery_action: Option<String>,
    pub output_may_be_incomplete: bool,
    pub hostile: bool,
}

impl ParserDiagnostic {
    pub fn new(
        severity: ParserSeverity,
        category: ParserCategory,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity,
            category,
            source: None,
            code: code.into(),
            message: message.into(),
            path: None,
            key: None,
            expected: None,
            actual: None,
            offset: None,
            byte_range: None,
            object: None,
            page: None,
            recovery_action: None,
            output_may_be_incomplete: false,
            hostile: false,
        }
    }

    pub fn at_offset(mut self, offset: usize) -> Self {
        self.offset = Some(offset);
        self
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    pub fn with_key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn with_expected(mut self, expected: impl Into<String>) -> Self {
        self.expected = Some(expected.into());
        self
    }

    pub fn with_actual(mut self, actual: impl Into<String>) -> Self {
        self.actual = Some(actual.into());
        self
    }

    pub fn with_recovery(mut self, action: impl Into<String>) -> Self {
        self.recovery_action = Some(action.into());
        self
    }

    pub fn incomplete(mut self) -> Self {
        self.output_may_be_incomplete = true;
        self
    }

    pub fn hostile(mut self) -> Self {
        self.hostile = true;
        self
    }
}

/// Source and laziness metrics available without forcing full object parsing.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ParserSourceMetrics {
    pub file_size_bytes: usize,
    pub file_backed: bool,
    pub startxref: Option<usize>,
    pub xref_entries: usize,
    pub objects_known: usize,
    pub objects_parsed_during_open: usize,
    pub object_streams_decoded_during_open: usize,
    pub bytes_read_during_open: Option<usize>,
}

/// Linearization metadata detected near the beginning of the file.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LinearizationInfo {
    pub is_linearized: bool,
    pub damaged: bool,
    pub valid: bool,
    pub first_object_number: Option<u32>,
    pub length: Option<i64>,
    pub hint_table_offset: Option<i64>,
    pub hint_table_length: Option<i64>,
    pub end_of_first_page_section: Option<i64>,
    pub declared_page_count: Option<i64>,
    pub actual_page_count: Option<usize>,
    pub main_xref_offset: Option<i64>,
    pub main_xref_status: String,
    pub first_page_fast_open_candidate: bool,
    pub diagnostics: Vec<ParserDiagnostic>,
}

/// Arlington integration state for this build.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArlingtonIntegrationStatus {
    pub status: String,
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
    pub supported_checks: Vec<String>,
    pub unsupported_predicates_reported: bool,
}

/// One xref/trailer section observed while following the incremental chain.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RevisionSection {
    pub index: usize,
    pub offset: usize,
    pub section_type: String,
    pub prev: Option<usize>,
    pub size: Option<i64>,
    pub root: Option<String>,
    pub info: Option<String>,
    pub encrypt_present: bool,
    pub id_present: bool,
    pub xref_stm: Option<usize>,
    pub trailer_keys: Vec<String>,
    pub object_numbers: Vec<u32>,
}

/// Incremental-update provenance summary.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RevisionHistory {
    pub contains_incremental_updates: bool,
    pub section_count: usize,
    pub sections: Vec<RevisionSection>,
    pub duplicate_objects: Vec<u32>,
    pub winning_revision_by_object: BTreeMap<u32, usize>,
    pub diagnostics: Vec<ParserDiagnostic>,
}

/// Best-effort repair/object-carving summary.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RepairSummary {
    pub total_objects_expected_from_xref: Option<usize>,
    pub total_objects_recovered_from_xref: usize,
    pub total_objects_recovered_from_scan: usize,
    pub missing_objects: Vec<u32>,
    pub duplicate_objects: Vec<u32>,
    pub truncated_objects: Vec<u32>,
    pub stream_length_corrected: usize,
    pub stream_end_inferred: usize,
    pub trailer_reconstructed: bool,
    pub page_tree_reconstructed: bool,
    pub recovered_page_objects: Vec<u32>,
    pub parse_failures: Vec<String>,
    pub byte_ranges_skipped: Vec<(usize, usize)>,
    pub confidence: String,
}

/// Parser report used by strict/repair/audit surfaces.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ParserReport {
    pub mode: ParserMode,
    pub opened: bool,
    pub strict_opened: Option<bool>,
    pub repair_opened: Option<bool>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub diagnostics: Vec<ParserDiagnostic>,
    pub source_metrics: ParserSourceMetrics,
    pub linearization: LinearizationInfo,
    pub arlington: ArlingtonIntegrationStatus,
    pub revision_history: RevisionHistory,
    pub repair_summary: RepairSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decode: Option<DecodeReport>,
}

impl ParserReport {
    pub fn diagnostic_counts(&self) -> DiagnosticCounts {
        DiagnosticCounts {
            info: self
                .diagnostics
                .iter()
                .filter(|d| d.severity == ParserSeverity::Info)
                .count(),
            warning: self
                .diagnostics
                .iter()
                .filter(|d| d.severity == ParserSeverity::Warning)
                .count(),
            recoverable_error: self
                .diagnostics
                .iter()
                .filter(|d| d.severity == ParserSeverity::RecoverableError)
                .count(),
            fatal_error: self
                .diagnostics
                .iter()
                .filter(|d| d.severity == ParserSeverity::FatalError)
                .count(),
            security_limit: self
                .diagnostics
                .iter()
                .filter(|d| d.severity == ParserSeverity::SecurityLimit)
                .count(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticCounts {
    pub info: usize,
    pub warning: usize,
    pub recoverable_error: usize,
    pub fatal_error: usize,
    pub security_limit: usize,
}

pub fn arlington_status() -> ArlingtonIntegrationStatus {
    let coverage = crate::arlington::arlington_coverage();
    ArlingtonIntegrationStatus {
        status: "generated_upstream".to_string(),
        source: coverage.source,
        commit: coverage.commit,
        tsv_files: coverage.tsv_files,
        object_models: coverage.object_models,
        keys: coverage.keys,
        required_key_rules: coverage.required_key_rules,
        type_rules: coverage.type_rules,
        version_rules: coverage.version_rules,
        indirect_reference_rules: coverage.indirect_reference_rules,
        link_rules: coverage.link_rules,
        unsupported_predicates: coverage.unsupported_predicates,
        parse_warnings: coverage.parse_warnings,
        supported_checks: vec![
            "dictionary type checks".to_string(),
            "required key checks".to_string(),
            "name enum checks".to_string(),
            "version metadata reporting".to_string(),
            "indirect-reference policy reporting for supported direct/indirect predicates"
                .to_string(),
            "unsupported predicate diagnostics".to_string(),
        ],
        unsupported_predicates_reported: true,
    }
}

/// Build a parser report for in-memory PDF bytes without changing normal open behavior.
pub fn parser_report_bytes(data: &[u8], mode: ParserMode) -> ParserReport {
    parser_report_bytes_with_password(data, mode, b"")
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ParserReportOptions {
    pub include_decode: bool,
    pub decode_limits: DecodeLimits,
}

/// Build a parser report for in-memory PDF bytes using a supplied user password.
pub fn parser_report_bytes_with_password(
    data: &[u8],
    mode: ParserMode,
    password: &[u8],
) -> ParserReport {
    parser_report_bytes_with_options(data, mode, password, ParserReportOptions::default())
}

pub fn parser_report_bytes_with_options(
    data: &[u8],
    mode: ParserMode,
    password: &[u8],
    options: ParserReportOptions,
) -> ParserReport {
    let mut diagnostics = diagnose_pdf_bytes(data);
    let linearization = detect_linearization(data);
    diagnostics.extend(linearization.diagnostics.iter().cloned());
    let revision_history = inspect_revision_history(data);
    diagnostics.extend(revision_history.diagnostics.iter().cloned());

    let strict = match mode {
        ParserMode::Strict | ParserMode::Audit => Some(PdfReader::from_bytes_strict_with_password(
            data.to_vec(),
            password,
        )),
        ParserMode::Repair => None,
    };
    let repair = match mode {
        ParserMode::Repair | ParserMode::Audit => {
            Some(PdfReader::from_bytes_with_password(data.to_vec(), password))
        }
        ParserMode::Strict => None,
    };

    if let Some(Err(err)) = &strict {
        diagnostics.push(
            ParserDiagnostic::new(
                ParserSeverity::FatalError,
                ParserCategory::Xref,
                "strict_open_failed",
                err.to_string(),
            )
            .incomplete(),
        );
    }
    if let Some(Ok(reader)) = &repair {
        diagnostics.extend_from_slice(reader.parser_diagnostics());
    }
    if matches!(mode, ParserMode::Audit)
        && strict.as_ref().is_some_and(Result::is_err)
        && repair.as_ref().is_some_and(Result::is_ok)
    {
        diagnostics.push(
            ParserDiagnostic::new(
                ParserSeverity::RecoverableError,
                ParserCategory::Repair,
                "strict_failed_repair_succeeded",
                "strict parsing failed but repair mode produced an object model",
            )
            .with_recovery("repair open used bounded fallback parsing")
            .incomplete(),
        );
    }

    let opened = match mode {
        ParserMode::Strict => strict.as_ref().is_some_and(Result::is_ok),
        ParserMode::Repair => repair.as_ref().is_some_and(Result::is_ok),
        ParserMode::Audit => {
            strict.as_ref().is_some_and(Result::is_ok) || repair.as_ref().is_some_and(Result::is_ok)
        }
    };

    let reader_for_metrics = repair
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .or_else(|| strict.as_ref().and_then(|r| r.as_ref().ok()));
    if let Some(reader) = reader_for_metrics {
        diagnostics.extend(crate::arlington::validate_arlington_dictionary_at_path(
            "FileTrailer",
            reader.trailer(),
            crate::arlington::ArlingtonValidationMode::Audit,
            "/Trailer",
        ));
        if let Some((number, generation)) = reader.root_reference() {
            match reader.get_and_resolve(number, generation) {
                Ok(crate::object::PdfObject::Dictionary(dict)) => {
                    diagnostics.extend(crate::arlington::validate_arlington_dictionary_at_path(
                        "Catalog",
                        &dict,
                        crate::arlington::ArlingtonValidationMode::Audit,
                        "/Root",
                    ))
                }
                Ok(other) => diagnostics.push(ParserDiagnostic::new(
                    ParserSeverity::RecoverableError,
                    ParserCategory::Validation,
                    "arlington_root_wrong_type",
                    format!(
                        "trailer /Root resolved to {}, not a dictionary",
                        other.variant_name()
                    ),
                )),
                Err(err) => diagnostics.push(
                    ParserDiagnostic::new(
                        ParserSeverity::RecoverableError,
                        ParserCategory::Validation,
                        "arlington_root_unresolved",
                        format!("trailer /Root could not be resolved: {err}"),
                    )
                    .incomplete(),
                ),
            }
        }
    }

    let decode = reader_for_metrics
        .filter(|_| options.include_decode)
        .map(|reader| summarize_decode(reader, &options.decode_limits));
    if let Some(decode) = &decode {
        diagnostics.extend(decode.diagnostics.iter().map(decode_to_parser_diagnostic));
    }

    let repair_summary = summarize_repair(data, reader_for_metrics, &diagnostics);

    let (error_code, error_message) = if opened {
        (None, None)
    } else {
        let err = strict
            .as_ref()
            .and_then(|r| r.as_ref().err())
            .or_else(|| repair.as_ref().and_then(|r| r.as_ref().err()));
        (
            err.map(|e| e.code().to_string()),
            err.map(|e| e.to_string()),
        )
    };

    ParserReport {
        mode,
        opened,
        strict_opened: strict.as_ref().map(Result::is_ok),
        repair_opened: repair.as_ref().map(Result::is_ok),
        error_code,
        error_message,
        diagnostics,
        source_metrics: reader_for_metrics.map_or_else(
            || ParserSourceMetrics {
                file_size_bytes: data.len(),
                ..ParserSourceMetrics::default()
            },
            PdfReader::source_metrics,
        ),
        linearization,
        arlington: arlington_status(),
        revision_history,
        repair_summary,
        decode,
    }
}

fn summarize_decode(reader: &PdfReader, limits: &DecodeLimits) -> DecodeReport {
    let mut aggregate = DecodeReport::empty(limits.clone());
    for (number, generation) in reader.object_ids() {
        let Ok(object) = reader.get_object(number, generation) else {
            continue;
        };
        let Some((dict, raw)) = object.as_stream() else {
            continue;
        };
        let context = format!("parser report decode object {number} {generation}");
        let report = decode_stream_report_from_dict_scheduled(
            dict,
            raw,
            Some(reader),
            limits,
            Some((number, generation)),
            &CancelToken::none(),
            &context,
        );
        aggregate.merge(report);
    }
    aggregate
}

fn decode_to_parser_diagnostic(diagnostic: &DecodeDiagnostic) -> ParserDiagnostic {
    let severity = match diagnostic.severity {
        DecodeSeverity::Info => ParserSeverity::Info,
        DecodeSeverity::Warning => ParserSeverity::Warning,
        DecodeSeverity::Error => ParserSeverity::RecoverableError,
        DecodeSeverity::Fatal => ParserSeverity::FatalError,
        DecodeSeverity::SecurityLimit => ParserSeverity::SecurityLimit,
    };
    let category = if diagnostic.limit_name.is_some() {
        ParserCategory::ResourceLimit
    } else {
        ParserCategory::FilterDecode
    };
    let mut parser = ParserDiagnostic::new(
        severity,
        category,
        diagnostic.code.clone(),
        diagnostic.message.clone(),
    )
    .with_source(format!("{:?}", diagnostic.source));
    parser.object = diagnostic.object;
    parser.page = diagnostic.page_index;
    parser.path = diagnostic.stream_context.clone();
    parser.expected = diagnostic
        .limit_name
        .as_ref()
        .zip(diagnostic.limit_value)
        .map(|(name, value)| format!("{name}<={value}"));
    parser.actual = diagnostic.observed_value.map(|value| value.to_string());
    parser.output_may_be_incomplete = diagnostic.partial_output_discarded;
    parser.hostile = matches!(diagnostic.severity, DecodeSeverity::SecurityLimit);
    parser
}

pub(crate) fn diagnose_pdf_bytes(data: &[u8]) -> Vec<ParserDiagnostic> {
    let mut diagnostics = Vec::new();
    if data
        .windows(b"%PDF-".len())
        .position(|w| w == b"%PDF-")
        .is_none()
    {
        diagnostics.push(ParserDiagnostic::new(
            ParserSeverity::FatalError,
            ParserCategory::Lexer,
            "missing_pdf_header",
            "PDF header was not found in the first parser scan window",
        ));
    }

    let eof_positions = find_marker_positions(data, b"%%EOF");
    match eof_positions.len() {
        0 => diagnostics.push(
            ParserDiagnostic::new(
                ParserSeverity::Warning,
                ParserCategory::Trailer,
                "missing_eof_marker",
                "%%EOF marker is missing",
            )
            .with_recovery("repair mode may use xref or object-scan fallback")
            .incomplete(),
        ),
        1 => {
            let end = eof_positions[0] + b"%%EOF".len();
            if data[end..]
                .iter()
                .any(|b| !matches!(b, b'\r' | b'\n' | b' ' | b'\t' | 0x0C | 0x00))
            {
                diagnostics.push(ParserDiagnostic::new(
                    ParserSeverity::Warning,
                    ParserCategory::Trailer,
                    "trailing_bytes_after_eof",
                    "non-whitespace bytes follow the last %%EOF marker",
                ));
            }
        }
        _ => diagnostics.push(ParserDiagnostic::new(
            ParserSeverity::Info,
            ParserCategory::IncrementalUpdate,
            "multiple_eof_markers",
            "multiple %%EOF markers suggest incremental updates",
        )),
    }

    match find_startxref_offset(data) {
        Some((marker, offset)) if offset >= data.len() => diagnostics.push(
            ParserDiagnostic::new(
                ParserSeverity::RecoverableError,
                ParserCategory::Xref,
                "startxref_beyond_eof",
                format!("startxref points to {offset}, beyond EOF {}", data.len()),
            )
            .at_offset(marker)
            .with_recovery("repair mode scans for xref sections or indirect objects")
            .incomplete()
            .hostile(),
        ),
        Some((marker, offset)) if !looks_like_xref_at(data, offset) => diagnostics.push(
            ParserDiagnostic::new(
                ParserSeverity::Warning,
                ParserCategory::Xref,
                "startxref_target_not_xref",
                format!("startxref points to {offset}, which is not an xref table or xref stream header"),
            )
            .at_offset(marker)
            .with_recovery("near-offset xref repair may correct small producer offsets")
            .incomplete(),
        ),
        Some(_) => {}
        None => diagnostics.push(
            ParserDiagnostic::new(
                ParserSeverity::RecoverableError,
                ParserCategory::Xref,
                "missing_startxref",
                "startxref marker is missing",
            )
            .with_recovery("repair mode falls back to a bounded indirect-object scan")
            .incomplete(),
        ),
    }

    diagnostics
}

fn find_marker_positions(data: &[u8], marker: &[u8]) -> Vec<usize> {
    data.windows(marker.len())
        .enumerate()
        .filter_map(|(idx, window)| (window == marker).then_some(idx))
        .collect()
}

fn find_startxref_offset(data: &[u8]) -> Option<(usize, usize)> {
    let marker = b"startxref";
    let marker_pos = data.windows(marker.len()).rposition(|w| w == marker)?;
    let mut pos = marker_pos + marker.len();
    while data
        .get(pos)
        .copied()
        .is_some_and(|b| matches!(b, 0x00 | b'\t' | b'\n' | 0x0C | b'\r' | b' '))
    {
        pos += 1;
    }
    let start = pos;
    while data.get(pos).copied().is_some_and(|b| b.is_ascii_digit()) {
        pos += 1;
    }
    if pos == start {
        return None;
    }
    let text = std::str::from_utf8(&data[start..pos]).ok()?;
    Some((marker_pos, text.parse().ok()?))
}

fn looks_like_xref_at(data: &[u8], offset: usize) -> bool {
    data.get(offset..offset + 4).is_some_and(|s| s == b"xref")
        || data
            .get(offset..offset.saturating_add(64).min(data.len()))
            .is_some_and(|s| s.windows(b" obj".len()).any(|w| w == b" obj"))
}

const REVISION_DEPTH_CAP: usize = 256;

fn inspect_revision_history(data: &[u8]) -> RevisionHistory {
    let mut history = RevisionHistory::default();
    let Some((_, startxref)) = find_startxref_offset(data) else {
        history.diagnostics.push(
            ParserDiagnostic::new(
                ParserSeverity::RecoverableError,
                ParserCategory::IncrementalUpdate,
                "revision_history_missing_startxref",
                "revision history could not start because startxref is missing",
            )
            .with_source("revision_inspector")
            .incomplete(),
        );
        return history;
    };

    let mut next = Some(startxref);
    let mut visited = HashSet::new();
    let mut winner_by_object = BTreeMap::new();
    let mut duplicates = HashSet::new();

    while let Some(offset) = next {
        if history.sections.len() >= REVISION_DEPTH_CAP {
            history.diagnostics.push(
                ParserDiagnostic::new(
                    ParserSeverity::SecurityLimit,
                    ParserCategory::IncrementalUpdate,
                    "revision_prev_depth_cap",
                    format!("xref /Prev chain exceeded depth cap {REVISION_DEPTH_CAP}"),
                )
                .with_source("revision_inspector")
                .at_offset(offset)
                .hostile(),
            );
            break;
        }
        if offset >= data.len() {
            history.diagnostics.push(
                ParserDiagnostic::new(
                    ParserSeverity::RecoverableError,
                    ParserCategory::IncrementalUpdate,
                    "revision_prev_offset_out_of_range",
                    format!("/Prev/startxref offset {offset} is outside the file"),
                )
                .with_source("revision_inspector")
                .at_offset(offset)
                .incomplete(),
            );
            break;
        }
        if !visited.insert(offset) {
            history.diagnostics.push(
                ParserDiagnostic::new(
                    ParserSeverity::RecoverableError,
                    ParserCategory::IncrementalUpdate,
                    "revision_prev_loop",
                    format!("/Prev chain loops at xref offset {offset}"),
                )
                .with_source("revision_inspector")
                .at_offset(offset)
                .hostile()
                .incomplete(),
            );
            break;
        }

        let section_index = history.sections.len();
        let section = inspect_xref_section(data, offset, section_index);
        match section {
            Ok(section) => {
                for object_number in &section.object_numbers {
                    if winner_by_object.contains_key(object_number) {
                        duplicates.insert(*object_number);
                    } else {
                        winner_by_object.insert(*object_number, section_index);
                    }
                }
                next = section.prev;
                history.sections.push(section);
            }
            Err(diagnostic) => {
                history.diagnostics.push(*diagnostic);
                break;
            }
        }
    }

    history.section_count = history.sections.len();
    history.contains_incremental_updates = history.section_count > 1;
    history.duplicate_objects = duplicates.into_iter().collect();
    history.duplicate_objects.sort_unstable();
    history.winning_revision_by_object = winner_by_object;
    history
}

fn inspect_xref_section(
    data: &[u8],
    offset: usize,
    index: usize,
) -> std::result::Result<RevisionSection, Box<ParserDiagnostic>> {
    if data.get(offset..offset.saturating_add(4)) == Some(b"xref") {
        inspect_classic_xref_section(data, offset, index)
    } else {
        inspect_xref_stream_section(data, offset, index)
    }
}

fn inspect_classic_xref_section(
    data: &[u8],
    offset: usize,
    index: usize,
) -> std::result::Result<RevisionSection, Box<ParserDiagnostic>> {
    let trailer_marker = find_after(data, offset, b"trailer").ok_or_else(|| {
        Box::new(
            ParserDiagnostic::new(
                ParserSeverity::RecoverableError,
                ParserCategory::IncrementalUpdate,
                "revision_classic_trailer_missing",
                "classic xref section did not contain a trailer marker",
            )
            .with_source("revision_inspector")
            .at_offset(offset)
            .incomplete(),
        )
    })?;
    let object_numbers = parse_classic_xref_object_numbers(&data[offset..trailer_marker]);
    let mut parser = crate::parser::PdfParser::new(data, trailer_marker + b"trailer".len())
        .map_err(|err| revision_parse_diag(offset, err.to_string()))?;
    let trailer = match parser.parse_object() {
        Ok(crate::object::PdfObject::Dictionary(dict)) => dict,
        Ok(other) => {
            return Err(Box::new(
                ParserDiagnostic::new(
                    ParserSeverity::RecoverableError,
                    ParserCategory::IncrementalUpdate,
                    "revision_trailer_wrong_type",
                    format!(
                        "classic xref trailer parsed as {}, not Dictionary",
                        other.variant_name()
                    ),
                )
                .with_source("revision_inspector")
                .at_offset(trailer_marker),
            ));
        }
        Err(err) => return Err(revision_parse_diag(trailer_marker, err.to_string())),
    };
    Ok(section_from_trailer(
        index,
        offset,
        "xref_table",
        trailer,
        object_numbers,
    ))
}

fn inspect_xref_stream_section(
    data: &[u8],
    offset: usize,
    index: usize,
) -> std::result::Result<RevisionSection, Box<ParserDiagnostic>> {
    let mut parser = crate::parser::PdfParser::new(data, offset)
        .map_err(|err| revision_parse_diag(offset, err.to_string()))?;
    let indirect = parser
        .parse_indirect_object()
        .map_err(|err| revision_parse_diag(offset, err.to_string()))?;
    let crate::object::PdfObject::Stream { dict, .. } = indirect.object else {
        return Err(Box::new(
            ParserDiagnostic::new(
                ParserSeverity::RecoverableError,
                ParserCategory::IncrementalUpdate,
                "revision_xref_stream_expected",
                format!("xref offset {offset} did not point to an xref stream object"),
            )
            .with_source("revision_inspector")
            .at_offset(offset)
            .incomplete(),
        ));
    };
    let object_numbers = xref_stream_object_numbers(&dict);
    Ok(section_from_trailer(
        index,
        offset,
        "xref_stream",
        dict,
        object_numbers,
    ))
}

fn section_from_trailer(
    index: usize,
    offset: usize,
    section_type: &str,
    trailer: crate::object::PdfDictionary,
    object_numbers: Vec<u32>,
) -> RevisionSection {
    RevisionSection {
        index,
        offset,
        section_type: section_type.to_string(),
        prev: trailer
            .get_integer("Prev")
            .and_then(|value| usize::try_from(value).ok()),
        size: trailer.get_integer("Size"),
        root: trailer.get("Root").map(object_summary),
        info: trailer.get("Info").map(object_summary),
        encrypt_present: trailer.contains_key("Encrypt"),
        id_present: trailer.contains_key("ID"),
        xref_stm: trailer
            .get_integer("XRefStm")
            .and_then(|value| usize::try_from(value).ok()),
        trailer_keys: trailer.entries().map(|(key, _)| key.clone()).collect(),
        object_numbers,
    }
}

fn parse_classic_xref_object_numbers(section: &[u8]) -> Vec<u32> {
    let text = String::from_utf8_lossy(section);
    let mut numbers = Vec::new();
    let mut lines = text.lines();
    let _ = lines.next();
    while let Some(line) = lines.next() {
        let mut parts = line.split_whitespace();
        let Some(start) = parts.next().and_then(|part| part.parse::<u32>().ok()) else {
            continue;
        };
        let Some(count) = parts.next().and_then(|part| part.parse::<u32>().ok()) else {
            continue;
        };
        for idx in 0..count {
            if let Some(entry) = lines.next() {
                if entry
                    .split_whitespace()
                    .nth(2)
                    .is_some_and(|status| status == "n")
                {
                    numbers.push(start.saturating_add(idx));
                }
            }
        }
    }
    numbers
}

fn xref_stream_object_numbers(dict: &crate::object::PdfDictionary) -> Vec<u32> {
    let mut numbers = Vec::new();
    if let Some(index) = dict.get_array("Index") {
        for pair in index.chunks(2) {
            if let [start, count] = pair {
                if let (Some(start), Some(count)) = (start.as_integer(), count.as_integer()) {
                    for idx in 0..count.max(0) {
                        if let Ok(number) = u32::try_from(start.saturating_add(idx)) {
                            numbers.push(number);
                        }
                    }
                }
            }
        }
    } else if let Some(size) = dict.get_integer("Size") {
        for number in 0..size.max(0) {
            if let Ok(number) = u32::try_from(number) {
                numbers.push(number);
            }
        }
    }
    numbers
}

fn revision_parse_diag(offset: usize, message: String) -> Box<ParserDiagnostic> {
    Box::new(
        ParserDiagnostic::new(
            ParserSeverity::RecoverableError,
            ParserCategory::IncrementalUpdate,
            "revision_section_parse_failed",
            message,
        )
        .with_source("revision_inspector")
        .at_offset(offset)
        .incomplete(),
    )
}

fn find_after(data: &[u8], offset: usize, marker: &[u8]) -> Option<usize> {
    data.get(offset..)?
        .windows(marker.len())
        .position(|window| window == marker)
        .map(|relative| offset + relative)
}

fn object_summary(object: &crate::object::PdfObject) -> String {
    match object {
        crate::object::PdfObject::Reference { number, generation } => {
            format!("{number} {generation} R")
        }
        crate::object::PdfObject::Name(name) => format!("/{name}"),
        other => other.variant_name().to_string(),
    }
}

#[derive(Clone, Debug)]
struct ScannedObject {
    number: u32,
    offset: usize,
    truncated: bool,
    missing_endstream: bool,
    stream_length_mismatch: bool,
    is_page: bool,
}

fn summarize_repair(
    data: &[u8],
    reader: Option<&PdfReader>,
    diagnostics: &[ParserDiagnostic],
) -> RepairSummary {
    let scanned = scan_indirect_objects(data);
    let mut duplicate_counts: HashMap<u32, usize> = HashMap::new();
    for object in &scanned {
        *duplicate_counts.entry(object.number).or_default() += 1;
    }
    let duplicate_objects = duplicate_counts
        .into_iter()
        .filter_map(|(number, count)| (count > 1).then_some(number))
        .collect::<Vec<_>>();
    let mut summary = RepairSummary {
        total_objects_expected_from_xref: reader
            .and_then(PdfReader::size)
            .and_then(|size| usize::try_from(size.max(0)).ok()),
        total_objects_recovered_from_xref: reader.map_or(0, |reader| reader.object_ids().len()),
        total_objects_recovered_from_scan: scanned.len(),
        missing_objects: Vec::new(),
        duplicate_objects,
        truncated_objects: scanned
            .iter()
            .filter_map(|object| object.truncated.then_some(object.number))
            .collect(),
        stream_length_corrected: scanned
            .iter()
            .filter(|object| object.stream_length_mismatch)
            .count(),
        stream_end_inferred: scanned
            .iter()
            .filter(|object| object.missing_endstream)
            .count(),
        trailer_reconstructed: diagnostics.iter().any(|diagnostic| {
            matches!(
                diagnostic.code.as_str(),
                "missing_startxref_repaired" | "xref_chain_rebuilt_from_object_scan"
            )
        }),
        page_tree_reconstructed: false,
        recovered_page_objects: scanned
            .iter()
            .filter_map(|object| object.is_page.then_some(object.number))
            .collect(),
        parse_failures: diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.category == ParserCategory::ObjectSyntax)
            .map(|diagnostic| diagnostic.message.clone())
            .collect(),
        byte_ranges_skipped: scanned
            .iter()
            .filter_map(|object| object.truncated.then_some((object.offset, data.len())))
            .collect(),
        confidence: "high".to_string(),
    };

    if let Some(expected) = summary.total_objects_expected_from_xref {
        let scanned_numbers = scanned
            .iter()
            .map(|object| object.number)
            .collect::<HashSet<_>>();
        summary.missing_objects = (1..expected)
            .filter_map(|number| {
                let number = u32::try_from(number).ok()?;
                (!scanned_numbers.contains(&number)).then_some(number)
            })
            .take(100)
            .collect();
    }
    summary.duplicate_objects.sort_unstable();
    summary.truncated_objects.sort_unstable();
    summary.recovered_page_objects.sort_unstable();
    summary.recovered_page_objects.dedup();

    if reader.and_then(PdfReader::root_reference).is_none()
        && !summary.recovered_page_objects.is_empty()
    {
        summary.page_tree_reconstructed = true;
        summary.confidence = "audit_only_recovered_pages".to_string();
    } else if summary.stream_end_inferred > 0 || !summary.truncated_objects.is_empty() {
        summary.confidence = "partial".to_string();
    }
    summary
}

fn scan_indirect_objects(data: &[u8]) -> Vec<ScannedObject> {
    let mut objects = Vec::new();
    let mut seen_offsets = HashSet::new();
    for marker in find_marker_positions(data, b" obj") {
        let Some(offset) = indirect_header_start(data, marker) else {
            continue;
        };
        if !seen_offsets.insert(offset) {
            continue;
        }
        let Some((number, _generation)) = parse_indirect_header_prefix(&data[offset..marker])
        else {
            continue;
        };
        let endobj = find_after(data, marker + b" obj".len(), b"endobj");
        let truncated = endobj.is_none();
        let search_end = endobj.unwrap_or(data.len());
        let has_stream = find_after(data, marker, b"stream").is_some_and(|pos| pos < search_end);
        let missing_endstream = has_stream
            && find_after(data, marker, b"endstream").is_none_or(|pos| pos >= search_end);
        let stream_length_mismatch = has_stream && stream_length_mismatch(data, offset, search_end);
        let is_page = object_has_page_type(data, offset, search_end);
        objects.push(ScannedObject {
            number,
            offset,
            truncated,
            missing_endstream,
            stream_length_mismatch,
            is_page,
        });
    }
    objects.sort_by_key(|object| object.offset);
    objects
}

fn indirect_header_start(data: &[u8], marker: usize) -> Option<usize> {
    let mut pos = marker;
    while pos > 0 && data[pos - 1].is_ascii_whitespace() {
        pos -= 1;
    }
    while pos > 0 && data[pos - 1].is_ascii_digit() {
        pos -= 1;
    }
    while pos > 0 && data[pos - 1].is_ascii_whitespace() {
        pos -= 1;
    }
    while pos > 0 && data[pos - 1].is_ascii_digit() {
        pos -= 1;
    }
    (pos < marker).then_some(pos)
}

fn parse_indirect_header_prefix(prefix: &[u8]) -> Option<(u32, u16)> {
    let text = std::str::from_utf8(prefix).ok()?;
    let mut parts = text.split_whitespace().rev();
    let generation = parts.next()?.parse::<u16>().ok()?;
    let number = parts.next()?.parse::<u32>().ok()?;
    Some((number, generation))
}

fn stream_length_mismatch(data: &[u8], offset: usize, search_end: usize) -> bool {
    let slice = &data[offset..search_end.min(data.len())];
    let Ok(text) = std::str::from_utf8(slice) else {
        return false;
    };
    if !text.contains("/Length") {
        return false;
    }
    let Some(stream_pos) = text.find("stream") else {
        return false;
    };
    let Some(endstream_pos) = text.find("endstream") else {
        return false;
    };
    let mut parts = text[..stream_pos].split_whitespace();
    while let Some(part) = parts.next() {
        if part == "/Length" {
            if let Some(value) = parts.next().and_then(|value| value.parse::<usize>().ok()) {
                let observed = endstream_pos.saturating_sub(stream_pos + "stream".len());
                return observed.abs_diff(value) > 4;
            }
        }
    }
    false
}

fn object_has_page_type(data: &[u8], offset: usize, search_end: usize) -> bool {
    data.get(offset..search_end.min(data.len()))
        .is_some_and(|slice| {
            slice
                .windows(b"/Type /Page".len())
                .any(|window| window == b"/Type /Page")
                && !slice
                    .windows(b"/Type /Pages".len())
                    .any(|window| window == b"/Type /Pages")
        })
}

fn detect_linearization(data: &[u8]) -> LinearizationInfo {
    let mut diagnostics = Vec::new();
    let mut info = LinearizationInfo {
        is_linearized: false,
        damaged: false,
        valid: false,
        first_object_number: None,
        length: None,
        hint_table_offset: None,
        hint_table_length: None,
        end_of_first_page_section: None,
        declared_page_count: None,
        actual_page_count: None,
        main_xref_offset: None,
        main_xref_status: "not_linearized".to_string(),
        first_page_fast_open_candidate: false,
        diagnostics: Vec::new(),
    };
    let prefix_len = data.len().min(2048);
    let prefix = &data[..prefix_len];
    if !prefix
        .windows(b"/Linearized".len())
        .any(|w| w == b"/Linearized")
    {
        return info;
    }
    info.is_linearized = true;
    let Some((object_number, dict)) = find_linearization_dictionary(data, prefix_len) else {
        info.damaged = true;
        info.main_xref_status = "dictionary_not_parseable".to_string();
        info.diagnostics.push(
            ParserDiagnostic::new(
                ParserSeverity::RecoverableError,
                ParserCategory::Linearization,
                "linearization_dictionary_unparseable",
                "found /Linearized near the file beginning but could not parse a valid linearization dictionary",
            )
            .with_source("linearization_validator")
            .incomplete(),
        );
        return info;
    };
    info.first_object_number = Some(object_number);
    if dict
        .get("Linearized")
        .and_then(crate::object::PdfObject::as_number)
        .is_none()
    {
        diagnostics.push(linearization_diag(
            "linearization_value_invalid",
            "/Linearized must be a number",
        ));
    }
    info.length = dict.get_integer("L");
    match info.length {
        Some(length) if length == i64::try_from(data.len()).unwrap_or(i64::MAX) => {}
        Some(length) => diagnostics.push(linearization_diag(
            "linearization_length_mismatch",
            format!(
                "/L declares {length} bytes, actual file length is {}",
                data.len()
            ),
        )),
        None => diagnostics.push(linearization_diag(
            "linearization_length_missing",
            "/L file length is missing or not an integer",
        )),
    }
    if let Some(h) = dict.get_array("H") {
        if h.len() >= 2 {
            info.hint_table_offset = h.first().and_then(crate::object::PdfObject::as_integer);
            info.hint_table_length = h.get(1).and_then(crate::object::PdfObject::as_integer);
        } else {
            diagnostics.push(linearization_diag(
                "linearization_hint_table_invalid",
                "/H hint table array must contain at least offset and length",
            ));
        }
    } else {
        diagnostics.push(linearization_diag(
            "linearization_hint_table_missing",
            "/H hint table entry is missing or not an array",
        ));
    }
    info.end_of_first_page_section = dict.get_integer("E");
    info.declared_page_count = dict.get_integer("N");
    info.main_xref_offset = dict.get_integer("T");
    if let Some(offset) = info.main_xref_offset {
        if let Ok(offset) = usize::try_from(offset) {
            if offset < data.len() && looks_like_xref_at(data, offset) {
                info.main_xref_status = "valid_xref_candidate".to_string();
            } else {
                info.main_xref_status = "not_xref_at_declared_offset".to_string();
                diagnostics.push(linearization_diag(
                    "linearization_main_xref_invalid",
                    format!("/T points to {offset}, which is not a valid xref candidate"),
                ));
            }
        } else {
            info.main_xref_status = "out_of_range".to_string();
            diagnostics.push(linearization_diag(
                "linearization_main_xref_out_of_range",
                "/T is negative or does not fit this platform",
            ));
        }
    } else {
        info.main_xref_status = "missing".to_string();
        diagnostics.push(linearization_diag(
            "linearization_main_xref_missing",
            "/T main xref offset is missing or not an integer",
        ));
    }
    if let Some(end) = info.end_of_first_page_section {
        if usize::try_from(end).map_or(true, |end| end > data.len()) {
            diagnostics.push(linearization_diag(
                "linearization_first_page_end_out_of_range",
                "/E end-of-first-page section is outside the file",
            ));
        }
    } else {
        diagnostics.push(linearization_diag(
            "linearization_first_page_end_missing",
            "/E end-of-first-page section is missing or not an integer",
        ));
    }
    if dict.get_integer("O").is_none() {
        diagnostics.push(linearization_diag(
            "linearization_first_page_object_missing",
            "/O first page object number is missing or not an integer",
        ));
    }
    if info.declared_page_count.is_none() {
        diagnostics.push(linearization_diag(
            "linearization_page_count_missing",
            "/N page count is missing or not an integer",
        ));
    }
    info.damaged = diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.severity,
            ParserSeverity::RecoverableError
                | ParserSeverity::FatalError
                | ParserSeverity::SecurityLimit
        )
    });
    info.valid = !info.damaged;
    info.first_page_fast_open_candidate = info.valid
        && info
            .end_of_first_page_section
            .and_then(|value| usize::try_from(value).ok())
            .is_some_and(|end| end <= data.len())
        && matches!(info.main_xref_status.as_str(), "valid_xref_candidate");
    info.diagnostics = diagnostics;
    info
}

fn find_linearization_dictionary(
    data: &[u8],
    prefix_len: usize,
) -> Option<(u32, crate::object::PdfDictionary)> {
    let limit = prefix_len.min(data.len());
    for pos in 0..limit {
        if !data[pos].is_ascii_digit() {
            continue;
        }
        let mut parser = crate::parser::PdfParser::new(data, pos).ok()?;
        if let Ok(indirect) = parser.parse_indirect_object() {
            if let crate::object::PdfObject::Dictionary(dict) = indirect.object {
                if dict.contains_key("Linearized") {
                    return Some((indirect.number, dict));
                }
            }
        }
    }
    None
}

fn linearization_diag(code: impl Into<String>, message: impl Into<String>) -> ParserDiagnostic {
    ParserDiagnostic::new(
        ParserSeverity::RecoverableError,
        ParserCategory::Linearization,
        code,
        message,
    )
    .with_source("linearization_validator")
    .incomplete()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_pdf() -> Vec<u8> {
        let mut pdf = Vec::new();
        pdf.extend_from_slice(b"%PDF-1.7\n");
        let obj1 = pdf.len();
        pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        let obj2 = pdf.len();
        pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Count 0 /Kids [] >>\nendobj\n");
        let xref = pdf.len();
        pdf.extend_from_slice(
            format!(
                "xref\n0 3\n0000000000 65535 f\n{obj1:010} 00000 n\n{obj2:010} 00000 n\ntrailer\n<< /Size 3 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n"
            )
            .as_bytes(),
        );
        pdf
    }

    fn tiny_pdf_with_bad_stream_filter() -> Vec<u8> {
        let mut pdf = Vec::new();
        pdf.extend_from_slice(b"%PDF-1.7\n");
        let obj1 = pdf.len();
        pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        let obj2 = pdf.len();
        pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Count 0 /Kids [] >>\nendobj\n");
        let obj3 = pdf.len();
        pdf.extend_from_slice(
            b"3 0 obj\n<< /Length 3 /Filter /BogusDecode >>\nstream\nabc\nendstream\nendobj\n",
        );
        let xref = pdf.len();
        pdf.extend_from_slice(
            format!(
                "xref\n0 4\n0000000000 65535 f\n{obj1:010} 00000 n\n{obj2:010} 00000 n\n{obj3:010} 00000 n\ntrailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n"
            )
            .as_bytes(),
        );
        pdf
    }

    #[test]
    fn audit_report_distinguishes_strict_failure_from_repair_success() {
        let mut pdf = tiny_pdf();
        let marker = pdf
            .windows(b"startxref".len())
            .rposition(|w| w == b"startxref")
            .unwrap();
        pdf.truncate(marker);
        pdf.extend_from_slice(b"%%EOF\n");

        let report = parser_report_bytes(&pdf, ParserMode::Audit);

        assert!(report.opened);
        assert_eq!(report.strict_opened, Some(false));
        assert_eq!(report.repair_opened, Some(true));
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.code == "missing_startxref"));
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.code == "strict_failed_repair_succeeded"));
    }

    #[test]
    fn strict_report_opens_clean_pdf() {
        let report = parser_report_bytes(&tiny_pdf(), ParserMode::Strict);
        assert!(report.opened);
        assert_eq!(report.strict_opened, Some(true));
        assert_eq!(report.source_metrics.objects_parsed_during_open, 0);
    }

    #[test]
    fn revision_history_reports_incremental_duplicate_winner() {
        let mut pdf = tiny_pdf();
        let prev = find_startxref_offset(&pdf).unwrap().1;
        let updated = pdf.len();
        pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Count 1 /Kids [] >>\nendobj\n");
        let xref = pdf.len();
        pdf.extend_from_slice(
            format!(
                "xref\n2 1\n{updated:010} 00000 n\ntrailer\n<< /Size 3 /Root 1 0 R /Prev {prev} >>\nstartxref\n{xref}\n%%EOF\n"
            )
            .as_bytes(),
        );

        let report = parser_report_bytes(&pdf, ParserMode::Audit);

        assert!(report.revision_history.contains_incremental_updates);
        assert_eq!(report.revision_history.section_count, 2);
        assert!(report.revision_history.duplicate_objects.contains(&2));
        assert_eq!(
            report.revision_history.winning_revision_by_object.get(&2),
            Some(&0)
        );
    }

    #[test]
    fn linearization_validation_reports_bad_offsets_and_length() {
        let pdf = b"%PDF-1.5\n1 0 obj\n<< /Linearized 1 /L 9999 /H [50 20] /O 3 /E 500 /N 2 /T 4096 >>\nendobj\n%%EOF\n";

        let report = parser_report_bytes(pdf, ParserMode::Audit);

        assert!(report.linearization.is_linearized);
        assert!(!report.linearization.valid);
        assert_eq!(report.linearization.length, Some(9999));
        assert!(report
            .linearization
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "linearization_length_mismatch"));
        assert!(report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "linearization_main_xref_invalid"));
    }

    #[test]
    fn repair_summary_reports_truncated_carved_object() {
        let pdf = b"%PDF-1.7\n1 0 obj\n<< /Type /Page >>\n";

        let report = parser_report_bytes(pdf, ParserMode::Audit);

        assert_eq!(report.repair_summary.total_objects_recovered_from_scan, 1);
        assert!(report.repair_summary.truncated_objects.contains(&1));
        assert!(report.repair_summary.recovered_page_objects.contains(&1));
        assert!(report.repair_summary.page_tree_reconstructed);
    }

    #[test]
    fn parser_report_can_include_decode_diagnostics() {
        let options = ParserReportOptions {
            include_decode: true,
            decode_limits: DecodeLimits::default(),
        };
        let report = parser_report_bytes_with_options(
            &tiny_pdf_with_bad_stream_filter(),
            ParserMode::Audit,
            b"",
            options,
        );
        let decode = report.decode.as_ref().expect("decode report");
        assert_eq!(decode.metrics.streams_seen, 1);
        assert_eq!(decode.metrics.unsupported_filters, 1);
        assert!(report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "decode_unsupported_filter"));
    }
}
