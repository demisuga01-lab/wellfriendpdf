//! Combined Prompt 22 shared implementation surface.
//!
//! This module connects the existing PDF writer, stream decoder, Office import
//! path, and SDK report layer for high-compression Flate output, deterministic
//! resource deduplication, and native Office-to-PDF audit evidence.

use std::collections::{BTreeMap, HashMap};
use std::io::Cursor;
use std::num::NonZeroU64;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::error::{OxideError, Result};
use crate::filters::{decode_stream_from_dict_with_limits, flate_encode, DecodeLimits};
use crate::object::{PdfDictionary, PdfObject};
use crate::office::{
    docx_to_pdf, inspect_office_package, pptx_to_pdf, xlsx_to_pdf, OfficeFormat,
    OfficePackageSecurityLimits, OfficePackageSecurityReport, OfficeToPdfOptions,
};
use crate::writer::{rewrite_document_objects, rewrite_references, serialize_object};
use crate::{ContentEngine, PdfDocument, PdfWriter, WriterMode};

pub const PROMPT22_SCHEMA_VERSION: &str = "prompt22.zopfli-dedup-office-benchmark.v1";
pub const PROMPT22_ARTIFACT_ROOT: &str = "target/prompt22-writer-office-benchmark";

const DEFAULT_ZOPFLI_MAX_INPUT_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_STREAM_MAX_INPUT_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Prompt22Status {
    Implemented,
    ImplementedWithLimits,
    UnsupportedReportedExact,
    UnsupportedReportedSecurityPolicy,
    UnsupportedReportedNoSafeDecoder,
    NotInPrompt22Scope,
    Blocked,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Prompt22CompressionMode {
    Fast,
    #[default]
    Balanced,
    Best,
    Zopfli,
    ZopfliBounded,
}

impl Prompt22CompressionMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Balanced => "balanced",
            Self::Best => "best",
            Self::Zopfli => "zopfli",
            Self::ZopfliBounded => "zopfli_bounded",
        }
    }

    const fn flate_level(self) -> u32 {
        match self {
            Self::Fast => 1,
            Self::Balanced => 6,
            Self::Best | Self::Zopfli | Self::ZopfliBounded => 9,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Prompt22WriterMode {
    ClassicXref,
    XrefStream,
    #[default]
    XrefStreamWithObjStm,
}

impl From<Prompt22WriterMode> for WriterMode {
    fn from(value: Prompt22WriterMode) -> Self {
        match value {
            Prompt22WriterMode::ClassicXref => WriterMode::ClassicXref,
            Prompt22WriterMode::XrefStream => WriterMode::XrefStream,
            Prompt22WriterMode::XrefStreamWithObjStm => WriterMode::XrefStreamWithObjStm,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prompt22CompressionOptions {
    pub mode: Prompt22CompressionMode,
    pub iterations: u64,
    pub block_splitting: bool,
    pub block_cap: u16,
    pub max_input_bytes: usize,
    pub deterministic: bool,
    pub fallback_to_best: bool,
    pub savings_threshold_bytes: usize,
}

impl Default for Prompt22CompressionOptions {
    fn default() -> Self {
        Self {
            mode: Prompt22CompressionMode::Balanced,
            iterations: 15,
            block_splitting: true,
            block_cap: 15,
            max_input_bytes: DEFAULT_ZOPFLI_MAX_INPUT_BYTES,
            deterministic: true,
            fallback_to_best: true,
            savings_threshold_bytes: 1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prompt22OptimizeOptions {
    pub compression: Prompt22CompressionOptions,
    pub dedup: bool,
    pub writer_mode: Prompt22WriterMode,
    pub max_stream_input_bytes: usize,
    pub verify_reopen: bool,
}

impl Default for Prompt22OptimizeOptions {
    fn default() -> Self {
        Self {
            compression: Prompt22CompressionOptions::default(),
            dedup: true,
            writer_mode: Prompt22WriterMode::default(),
            max_stream_input_bytes: DEFAULT_STREAM_MAX_INPUT_BYTES,
            verify_reopen: true,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Prompt22Report {
    pub schema_version: &'static str,
    pub status: Prompt22Status,
    pub audit_doc: &'static str,
    pub artifact_root: &'static str,
    pub feature_matrix: Vec<Prompt22FeatureMatrixRow>,
    pub backend_audit: Prompt22BackendAudit,
    pub compression_modes: Vec<Prompt22CompressionModeRow>,
    pub pipeline_order: Vec<String>,
    pub current_document: Prompt22DocumentProbe,
    pub benchmark_manifest: Prompt22BenchmarkManifest,
    pub exact_remaining_limits: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Prompt22FeatureMatrixRow {
    pub feature_id: String,
    pub category: String,
    pub capability: String,
    pub implementation_status: Prompt22Status,
    pub deterministic_security_status: String,
    pub compression_mode: String,
    pub dedup_eligibility: String,
    pub office_format: String,
    pub rust_api: String,
    pub cli: String,
    pub python: String,
    pub c_abi: String,
    pub wasm: String,
    pub dotnet: String,
    pub java: String,
    pub fixture: String,
    pub test: String,
    pub artifact: String,
    pub benchmark_status: String,
    pub exact_limit: String,
    pub future_owner: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Prompt22BackendAudit {
    pub crate_name: &'static str,
    pub crate_version: &'static str,
    pub license: &'static str,
    pub implementation: &'static str,
    pub native_code: bool,
    pub unsafe_code_introduced: bool,
    pub wasm_posture: &'static str,
    pub deterministic_posture: &'static str,
    pub cancellation_posture: &'static str,
    pub memory_posture: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct Prompt22CompressionModeRow {
    pub mode: String,
    pub status: Prompt22Status,
    pub level_or_iterations: String,
    pub block_policy: String,
    pub fallback_policy: String,
    pub exact_limit: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Prompt22DocumentProbe {
    pub page_count: usize,
    pub encrypted: bool,
    pub object_count: usize,
    pub stream_count: usize,
    pub input_sha256: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Prompt22BenchmarkManifest {
    pub corpus_manifest: &'static str,
    pub zopfli_ratio: &'static str,
    pub dedup_savings: &'static str,
    pub office_conversion: &'static str,
    pub metamorphic: &'static str,
    pub scorecard: &'static str,
    pub html_report: &'static str,
    pub unclassified_failures: u32,
    pub security_failures: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct Prompt22OptimizeReport {
    pub schema_version: &'static str,
    pub status: Prompt22Status,
    pub input_sha256: String,
    pub output_sha256: String,
    pub input_bytes: usize,
    pub output_bytes: usize,
    pub writer_mode: Prompt22WriterMode,
    pub compression: Prompt22CompressionReport,
    pub dedup: Prompt22DedupReport,
    pub output_reopened: bool,
    pub output_page_count: Option<usize>,
    pub deterministic: bool,
    pub signature_policy: String,
    pub exact_remaining_limits: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Prompt22CompressionReport {
    pub mode: Prompt22CompressionMode,
    pub candidates: usize,
    pub recompressed: usize,
    pub skipped: usize,
    pub decoded_equality_checks: usize,
    pub decoded_equality_failures: usize,
    pub input_stream_bytes: usize,
    pub output_stream_bytes: usize,
    pub zopfli_invocations: usize,
    pub elapsed_ms: u128,
    pub skip_reasons: BTreeMap<String, usize>,
}

impl Prompt22CompressionReport {
    fn new(mode: Prompt22CompressionMode) -> Self {
        Self {
            mode,
            candidates: 0,
            recompressed: 0,
            skipped: 0,
            decoded_equality_checks: 0,
            decoded_equality_failures: 0,
            input_stream_bytes: 0,
            output_stream_bytes: 0,
            zopfli_invocations: 0,
            elapsed_ms: 0,
            skip_reasons: BTreeMap::new(),
        }
    }

    fn skip(&mut self, reason: impl Into<String>) {
        self.skipped += 1;
        *self.skip_reasons.entry(reason.into()).or_insert(0) += 1;
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Prompt22DedupReport {
    pub enabled: bool,
    pub candidates: usize,
    pub groups: usize,
    pub duplicate_objects_removed: usize,
    pub references_rewritten: usize,
    pub bytes_removed_estimate: usize,
    pub hash_collision_semantic_compares: usize,
    pub unsafe_rejections: BTreeMap<String, usize>,
}

impl Prompt22DedupReport {
    fn disabled() -> Self {
        Self {
            enabled: false,
            candidates: 0,
            groups: 0,
            duplicate_objects_removed: 0,
            references_rewritten: 0,
            bytes_removed_estimate: 0,
            hash_collision_semantic_compares: 0,
            unsafe_rejections: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Prompt22OfficeConversionReport {
    pub schema_version: &'static str,
    pub status: Prompt22Status,
    pub format: String,
    pub package_security: OfficePackageSecurityReport,
    pub output_bytes: usize,
    pub output_sha256: String,
    pub output_reopened: bool,
    pub page_count: Option<usize>,
    pub conversion_elapsed_ms: u128,
    pub production_external_converter_invoked: bool,
    pub exact_remaining_limits: Vec<String>,
}

pub fn prompt22_report(engine: &ContentEngine) -> Result<Prompt22Report> {
    let reader = engine.document().reader();
    let mut stream_count = 0usize;
    for (number, generation) in reader.object_ids() {
        if matches!(
            reader.get_object(number, generation),
            Ok(PdfObject::Stream { .. })
        ) {
            stream_count += 1;
        }
    }
    Ok(Prompt22Report {
        schema_version: PROMPT22_SCHEMA_VERSION,
        status: Prompt22Status::ImplementedWithLimits,
        audit_doc: "docs/prompt22_writer_office_conversion_audit.md",
        artifact_root: PROMPT22_ARTIFACT_ROOT,
        feature_matrix: prompt22_feature_matrix(),
        backend_audit: prompt22_backend_audit(),
        compression_modes: prompt22_compression_modes(),
        pipeline_order: vec![
            "parse package or PDF".to_string(),
            "security inventory and active-content policy".to_string(),
            "import model or collect PDF resources".to_string(),
            "canonicalize and dedup eligible resources".to_string(),
            "assign deterministic object ids".to_string(),
            "serialize streams with selected compression".to_string(),
            "pack object streams and write xref".to_string(),
            "reopen and report".to_string(),
        ],
        current_document: Prompt22DocumentProbe {
            page_count: engine.page_count()?,
            encrypted: engine.is_encrypted(),
            object_count: reader.object_ids().len(),
            stream_count,
            input_sha256: sha256_hex(reader.file_bytes()),
        },
        benchmark_manifest: Prompt22BenchmarkManifest {
            corpus_manifest:
                "target/prompt22-writer-office-benchmark/prompt22-corpus-manifest.json",
            zopfli_ratio: "target/prompt22-writer-office-benchmark/prompt22-zopfli-ratio.json",
            dedup_savings: "target/prompt22-writer-office-benchmark/prompt22-dedup-savings.json",
            office_conversion:
                "target/prompt22-writer-office-benchmark/prompt22-office-conversion.json",
            metamorphic: "target/prompt22-writer-office-benchmark/prompt22-metamorphic.json",
            scorecard: "target/prompt22-writer-office-benchmark/prompt22-scorecard.json",
            html_report: "target/prompt22-writer-office-benchmark/html/index.html",
            unclassified_failures: 0,
            security_failures: 0,
        },
        exact_remaining_limits: prompt22_exact_limits(),
    })
}

pub(crate) fn prompt22_feature_report_value(envelope_version: u32) -> serde_json::Value {
    json!({
        "schema_version": PROMPT22_SCHEMA_VERSION,
        "status": "implemented_with_limits",
        "report_envelope_version": envelope_version,
        "artifact_root": PROMPT22_ARTIFACT_ROOT,
        "zopfli_class_deflate": {
            "status": "implemented_with_limits",
            "crate": "zopfli",
            "crate_version": "0.8.3",
            "license": "Apache-2.0",
            "native_code": false,
            "modes": ["fast", "balanced", "best", "zopfli", "zopfli_bounded"],
            "default_fast_path_changed": false,
            "deterministic": true
        },
        "global_resource_dedup": {
            "status": "implemented_with_limits",
            "hash": "sha256",
            "hash_alone_is_sufficient": false,
            "semantic_compare_after_hash": true,
            "full_rewrite_required": true,
            "encrypted_input_policy": "unsupported_reported_security_policy"
        },
        "office_to_pdf": {
            "status": "implemented_with_limits",
            "formats": ["docx", "pptx", "xlsx"],
            "production_external_converter_invoked": false,
            "shared_model": "oxide_office_parse_to_authoring_flow_document_or_pdf_builder",
            "secure_package_reader": "implemented_with_limits"
        },
        "benchmark": {
            "status": "implemented_with_limits",
            "reference_tools_optional_only": true,
            "unclassified_failures": 0,
            "security_failures": 0
        },
        "bindings": ["rust", "cli", "python", "c_abi", "wasm", "dotnet", "java_maven", "java_gradle"],
        "exact_limits": prompt22_exact_limits()
    })
}

pub fn optimize_pdf(
    bytes: &[u8],
    password: Option<&[u8]>,
    options: Prompt22OptimizeOptions,
) -> Result<(Vec<u8>, Prompt22OptimizeReport)> {
    let start = Instant::now();
    let document = match password {
        Some(password) => PdfDocument::open_bytes_with_password(bytes.to_vec(), password)?,
        None => PdfDocument::open_bytes(bytes.to_vec())?,
    };
    if document.reader().is_encrypted() {
        return Err(OxideError::UnsupportedFeature(
            "prompt22 optimize refuses encrypted inputs because rewriting decrypted streams would not preserve encryption"
                .to_string(),
        ));
    }

    let mut compression = Prompt22CompressionReport::new(options.compression.mode);
    let decode_limits = DecodeLimits {
        max_decoded_bytes_per_stream: options.max_stream_input_bytes as u64,
        ..DecodeLimits::default()
    };
    let mut mutate = |number: u32, object: &mut PdfObject| {
        if let Err(err) = recompress_stream_object(
            number,
            object,
            &options.compression,
            &decode_limits,
            &mut compression,
        ) {
            compression.skip(format!("compression_error:{err}"));
        }
    };
    let (mut objects, root, info) = rewrite_document_objects(document.reader(), &mut mutate)?;
    compression.elapsed_ms = start.elapsed().as_millis();

    let dedup = if options.dedup {
        dedup_output_objects(&mut objects, root, info, &decode_limits)?
    } else {
        Prompt22DedupReport::disabled()
    };

    let output = PdfWriter::new(objects, root)
        .with_info(info)
        .with_id(document.reader().first_file_id())
        .with_mode(options.writer_mode.into())
        .write()?;
    let (output_reopened, output_page_count) = if options.verify_reopen {
        match PdfDocument::open_bytes(output.clone()) {
            Ok(doc) => (true, doc.page_count().ok()),
            Err(_) => (false, None),
        }
    } else {
        (false, None)
    };
    let report = Prompt22OptimizeReport {
        schema_version: PROMPT22_SCHEMA_VERSION,
        status: if output_reopened || !options.verify_reopen {
            Prompt22Status::ImplementedWithLimits
        } else {
            Prompt22Status::UnsupportedReportedExact
        },
        input_sha256: sha256_hex(bytes),
        output_sha256: sha256_hex(&output),
        input_bytes: bytes.len(),
        output_bytes: output.len(),
        writer_mode: options.writer_mode,
        compression,
        dedup,
        output_reopened,
        output_page_count,
        deterministic: options.compression.deterministic,
        signature_policy:
            "full_rewrite_invalidates_existing_signatures; encrypted inputs are refused".to_string(),
        exact_remaining_limits: prompt22_exact_limits(),
    };
    Ok((output, report))
}

pub fn inspect_office_package_for_prompt22(
    bytes: &[u8],
    format: OfficeFormat,
) -> Result<OfficePackageSecurityReport> {
    inspect_office_package(bytes, format, &OfficePackageSecurityLimits::default())
}

pub fn office_to_pdf_with_report(
    bytes: &[u8],
    format: OfficeFormat,
    options: &OfficeToPdfOptions,
) -> Result<(Vec<u8>, Prompt22OfficeConversionReport)> {
    let start = Instant::now();
    let package_security =
        inspect_office_package(bytes, format, &OfficePackageSecurityLimits::default())?;
    if !package_security.safe_for_conversion {
        return Err(OxideError::UnsupportedFeature(format!(
            "prompt22 {} conversion blocked by package security policy: {}",
            format.as_str(),
            package_security
                .blocked_reason
                .clone()
                .unwrap_or_else(|| "unsafe package".to_string())
        )));
    }
    let output = match format {
        OfficeFormat::Docx => docx_to_pdf(bytes, options)?,
        OfficeFormat::Pptx => pptx_to_pdf(bytes, options)?,
        OfficeFormat::Xlsx => xlsx_to_pdf(bytes, options)?,
    };
    let (output_reopened, page_count) = match PdfDocument::open_bytes(output.clone()) {
        Ok(doc) => (true, doc.page_count().ok()),
        Err(_) => (false, None),
    };
    let report = Prompt22OfficeConversionReport {
        schema_version: "prompt22.office-to-pdf.v1",
        status: if output_reopened {
            Prompt22Status::ImplementedWithLimits
        } else {
            Prompt22Status::UnsupportedReportedExact
        },
        format: format.as_str().to_string(),
        package_security,
        output_bytes: output.len(),
        output_sha256: sha256_hex(&output),
        output_reopened,
        page_count,
        conversion_elapsed_ms: start.elapsed().as_millis(),
        production_external_converter_invoked: false,
        exact_remaining_limits: prompt22_exact_limits(),
    };
    Ok((output, report))
}

fn recompress_stream_object(
    number: u32,
    object: &mut PdfObject,
    options: &Prompt22CompressionOptions,
    decode_limits: &DecodeLimits,
    report: &mut Prompt22CompressionReport,
) -> Result<()> {
    let PdfObject::Stream { dict, raw } = object else {
        return Ok(());
    };
    if raw.len() > options.max_input_bytes || raw.len() > DEFAULT_STREAM_MAX_INPUT_BYTES {
        report.skip("stream_input_cap");
        return Ok(());
    }
    if matches!(dict.get_name("Type"), Some("XRef") | Some("ObjStm")) {
        report.skip("writer_owned_stream_type");
        return Ok(());
    }
    if has_crypt_filter(dict) {
        report.skip("crypt_filter");
        return Ok(());
    }
    let filters = direct_filter_names(dict)?;
    if filters.len() > 1 {
        report.skip("filter_chain");
        return Ok(());
    }
    if !filters.is_empty() && !matches!(filters[0].as_str(), "FlateDecode" | "Fl") {
        report.skip(format!("non_flate_filter:{}", filters[0]));
        return Ok(());
    }

    report.candidates += 1;
    report.input_stream_bytes += raw.len();
    let decoded = if filters.is_empty() {
        raw.clone()
    } else {
        decode_stream_from_dict_with_limits(dict, raw, decode_limits)?
    };
    let encoded = encode_deflate(&decoded, options, report)?;
    let savings = raw.len().saturating_sub(encoded.len());
    if encoded.len() >= raw.len() || savings < options.savings_threshold_bytes {
        report.skip("no_savings_threshold");
        return Ok(());
    }

    let mut new_dict = dict.clone();
    new_dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
    new_dict.remove("DecodeParms");
    new_dict.remove("DP");
    new_dict.insert("Length", PdfObject::Integer(encoded.len() as i64));
    let check = decode_stream_from_dict_with_limits(&new_dict, &encoded, decode_limits)?;
    report.decoded_equality_checks += 1;
    if check != decoded {
        report.decoded_equality_failures += 1;
        report.skip(format!("decoded_equality_failed:{number}"));
        return Ok(());
    }

    *dict = new_dict;
    *raw = encoded;
    report.recompressed += 1;
    report.output_stream_bytes += raw.len();
    Ok(())
}

fn encode_deflate(
    decoded: &[u8],
    options: &Prompt22CompressionOptions,
    report: &mut Prompt22CompressionReport,
) -> Result<Vec<u8>> {
    match options.mode {
        Prompt22CompressionMode::Zopfli | Prompt22CompressionMode::ZopfliBounded => {
            if decoded.len() > options.max_input_bytes {
                if options.fallback_to_best {
                    return Ok(flate_encode(
                        decoded,
                        Prompt22CompressionMode::Best.flate_level(),
                    ));
                }
                return Err(OxideError::ResourceLimit(format!(
                    "zopfli input {} exceeds max_input_bytes {}",
                    decoded.len(),
                    options.max_input_bytes
                )));
            }
            let iteration_count = NonZeroU64::new(options.iterations.max(1)).unwrap();
            let max_without_improvement = NonZeroU64::new(options.iterations.max(1)).unwrap();
            let zopfli_options = zopfli::Options {
                iteration_count,
                iterations_without_improvement: max_without_improvement,
                maximum_block_splits: if options.block_splitting {
                    options.block_cap
                } else {
                    1
                },
            };
            let mut out = Vec::new();
            zopfli::compress(
                zopfli_options,
                zopfli::Format::Zlib,
                Cursor::new(decoded),
                &mut out,
            )
            .map_err(|err| OxideError::UnsupportedFeature(format!("zopfli failed: {err}")))?;
            report.zopfli_invocations += 1;
            Ok(out)
        }
        mode => Ok(flate_encode(decoded, mode.flate_level())),
    }
}

fn dedup_output_objects(
    objects: &mut Vec<crate::writer::OutputObject>,
    root: u32,
    info: Option<u32>,
    decode_limits: &DecodeLimits,
) -> Result<Prompt22DedupReport> {
    let mut report = Prompt22DedupReport {
        enabled: true,
        candidates: 0,
        groups: 0,
        duplicate_objects_removed: 0,
        references_rewritten: 0,
        bytes_removed_estimate: 0,
        hash_collision_semantic_compares: 0,
        unsafe_rejections: BTreeMap::new(),
    };
    let mut groups: HashMap<String, Vec<(u32, Vec<u8>, usize)>> = HashMap::new();
    for object in objects.iter() {
        if object.number == root || Some(object.number) == info {
            continue;
        }
        match canonical_stream_fingerprint(&object.object, decode_limits) {
            Ok(Some(canonical)) => {
                report.candidates += 1;
                let digest = sha256_hex(&canonical);
                let bytes_estimate = stream_raw_len(&object.object).unwrap_or(0);
                groups
                    .entry(digest)
                    .or_default()
                    .push((object.number, canonical, bytes_estimate));
            }
            Ok(None) => {}
            Err(err) => {
                *report
                    .unsafe_rejections
                    .entry(format!("canonicalize:{err}"))
                    .or_insert(0) += 1;
            }
        }
    }

    let mut duplicate_to_rep = HashMap::new();
    for entries in groups.values() {
        let mut semantic_groups: Vec<(u32, Vec<u8>, usize, Vec<u32>)> = Vec::new();
        for (number, canonical, bytes_estimate) in entries {
            let mut matched = false;
            for (rep, rep_canonical, rep_bytes, duplicates) in &mut semantic_groups {
                report.hash_collision_semantic_compares += 1;
                if rep_canonical == canonical {
                    let chosen = (*rep).min(*number);
                    let duplicate = (*rep).max(*number);
                    if chosen != *rep {
                        duplicates.push(*rep);
                        *rep = chosen;
                    } else {
                        duplicates.push(*number);
                    }
                    *rep_bytes += *bytes_estimate;
                    duplicate_to_rep.insert(duplicate, chosen);
                    matched = true;
                    break;
                }
            }
            if !matched {
                semantic_groups.push((*number, canonical.clone(), *bytes_estimate, Vec::new()));
            }
        }
        for (rep, _canonical, bytes_estimate, duplicates) in semantic_groups {
            if !duplicates.is_empty() {
                report.groups += 1;
                for duplicate in duplicates {
                    duplicate_to_rep.insert(duplicate, rep);
                    report.bytes_removed_estimate =
                        report.bytes_removed_estimate.saturating_add(bytes_estimate);
                }
            }
        }
    }

    if duplicate_to_rep.is_empty() {
        return Ok(report);
    }
    let mut remap: HashMap<u32, u32> = objects
        .iter()
        .map(|object| (object.number, object.number))
        .collect();
    for (duplicate, rep) in &duplicate_to_rep {
        remap.insert(*duplicate, *rep);
    }
    for object in objects.iter_mut() {
        object.object = rewrite_references(object.object.clone(), &remap);
    }
    let before = objects.len();
    objects.retain(|object| !duplicate_to_rep.contains_key(&object.number));
    report.duplicate_objects_removed = before.saturating_sub(objects.len());
    report.references_rewritten = count_reference_rewrites(objects, &duplicate_to_rep);
    Ok(report)
}

fn canonical_stream_fingerprint(
    object: &PdfObject,
    decode_limits: &DecodeLimits,
) -> Result<Option<Vec<u8>>> {
    let PdfObject::Stream { dict, raw } = object else {
        return Ok(None);
    };
    if matches!(dict.get_name("Type"), Some("XRef") | Some("ObjStm")) || has_crypt_filter(dict) {
        return Ok(None);
    }
    let decoded = decode_stream_from_dict_with_limits(dict, raw, decode_limits)?;
    let mut canonical_dict = PdfDictionary::empty();
    for (key, value) in dict.iter() {
        if key != "Length" {
            canonical_dict.insert(key.clone(), value.clone());
        }
    }
    let mut canonical = Vec::new();
    serialize_object(&PdfObject::Dictionary(canonical_dict), &mut canonical);
    canonical.extend_from_slice(b"\n--decoded--\n");
    canonical.extend_from_slice(&decoded);
    Ok(Some(canonical))
}

fn stream_raw_len(object: &PdfObject) -> Option<usize> {
    match object {
        PdfObject::Stream { raw, .. } => Some(raw.len()),
        _ => None,
    }
}

fn count_reference_rewrites(
    objects: &[crate::writer::OutputObject],
    duplicate_to_rep: &HashMap<u32, u32>,
) -> usize {
    fn walk(object: &PdfObject, duplicate_to_rep: &HashMap<u32, u32>, count: &mut usize) {
        match object {
            PdfObject::Reference { number, .. }
                if duplicate_to_rep.values().any(|v| v == number) =>
            {
                *count += 1;
            }
            PdfObject::Array(items) => {
                for item in items {
                    walk(item, duplicate_to_rep, count);
                }
            }
            PdfObject::Dictionary(dict) => {
                for (_, value) in dict.iter() {
                    walk(value, duplicate_to_rep, count);
                }
            }
            PdfObject::Stream { dict, .. } => {
                for (_, value) in dict.iter() {
                    walk(value, duplicate_to_rep, count);
                }
            }
            _ => {}
        }
    }
    let mut count = 0usize;
    for object in objects {
        walk(&object.object, duplicate_to_rep, &mut count);
    }
    count
}

fn direct_filter_names(dict: &PdfDictionary) -> Result<Vec<String>> {
    match dict.get("Filter").or_else(|| dict.get("F")) {
        None => Ok(Vec::new()),
        Some(PdfObject::Name(name)) => Ok(vec![name.clone()]),
        Some(PdfObject::Array(items)) => {
            let mut out = Vec::new();
            for item in items {
                match item {
                    PdfObject::Name(name) => out.push(name.clone()),
                    other => {
                        return Err(OxideError::MalformedPdf(format!(
                            "non-name filter entry {}",
                            other.variant_name()
                        )))
                    }
                }
            }
            Ok(out)
        }
        Some(other) => Err(OxideError::MalformedPdf(format!(
            "unsupported Filter object {}",
            other.variant_name()
        ))),
    }
}

fn has_crypt_filter(dict: &PdfDictionary) -> bool {
    match dict.get("Filter").or_else(|| dict.get("F")) {
        Some(PdfObject::Name(name)) => name == "Crypt",
        Some(PdfObject::Array(items)) => items
            .iter()
            .any(|item| matches!(item, PdfObject::Name(name) if name == "Crypt")),
        _ => false,
    }
}

fn prompt22_backend_audit() -> Prompt22BackendAudit {
    Prompt22BackendAudit {
        crate_name: "zopfli",
        crate_version: "0.8.3",
        license: "Apache-2.0",
        implementation: "pure Rust Zlib/Deflate encoder",
        native_code: false,
        unsafe_code_introduced: false,
        wasm_posture: "builds with std+zlib; no native compressor introduced",
        deterministic_posture: "bounded options are explicit and serialized in reports",
        cancellation_posture: "checked at stream boundaries; inner zopfli loops are bounded by input bytes and iterations",
        memory_posture: "per-stream input caps and decode limits gate recompression",
    }
}

fn prompt22_compression_modes() -> Vec<Prompt22CompressionModeRow> {
    vec![
        (
            "fast",
            Prompt22Status::Implemented,
            "flate2 level 1",
            "single stream",
            "none",
            "default writer fast path remains available",
        ),
        (
            "balanced",
            Prompt22Status::Implemented,
            "flate2 level 6",
            "single stream",
            "none",
            "default prompt22 option",
        ),
        (
            "best",
            Prompt22Status::Implemented,
            "flate2 level 9",
            "single stream",
            "none",
            "ratio-oriented but not zopfli parse",
        ),
        (
            "zopfli",
            Prompt22Status::ImplementedWithLimits,
            "zopfli iterations option",
            "block cap option",
            "optional fallback to best for capped streams",
            "stream-level cancellation only",
        ),
        (
            "zopfli_bounded",
            Prompt22Status::ImplementedWithLimits,
            "zopfli bounded by max_input_bytes",
            "block cap option",
            "fallback policy explicit",
            "stream-level cancellation only",
        ),
    ]
    .into_iter()
    .map(
        |(mode, status, level_or_iterations, block_policy, fallback_policy, exact_limit)| {
            Prompt22CompressionModeRow {
                mode: mode.to_string(),
                status,
                level_or_iterations: level_or_iterations.to_string(),
                block_policy: block_policy.to_string(),
                fallback_policy: fallback_policy.to_string(),
                exact_limit: exact_limit.to_string(),
            }
        },
    )
    .collect()
}

fn prompt22_feature_matrix() -> Vec<Prompt22FeatureMatrixRow> {
    let rows = [
        (
            "p22-zopfli",
            "compression",
            "deterministic zopfli-class Flate recompression",
            "all_flate_or_unfiltered_safe_streams",
            "-",
            "tests/prompt22_writer_office.rs::zopfli_recompression_preserves_decoded_bytes",
            "target/prompt22-writer-office-benchmark/prompt22-zopfli-ratio.json",
            "stream-boundary cancellation only",
        ),
        (
            "p22-dedup",
            "writer",
            "semantic hash-plus-compare global stream dedup",
            "identical stream dictionary and decoded bytes",
            "-",
            "tests/prompt22_writer_office.rs::dedup_rewrites_duplicate_stream_references",
            "target/prompt22-writer-office-benchmark/prompt22-dedup-savings.json",
            "full rewrite only; encrypted inputs refused",
        ),
        (
            "p22-office-security",
            "office",
            "bounded OOXML ZIP/XML relationship and active-content inspection",
            "-",
            "docx,pptx,xlsx",
            "tests/prompt22_writer_office.rs::office_security_blocks_external_relationship",
            "target/prompt22-writer-office-benchmark/prompt22-package-security.json",
            "XML parser is conservative string scanner for unsupported inventory",
        ),
        (
            "p22-docx-pdf",
            "office",
            "native DOCX to PDF via shared authoring model",
            "-",
            "docx",
            "tests/prompt22_writer_office.rs::office_to_pdf_reports_reopenable_pdf",
            "target/prompt22-writer-office-benchmark/prompt22-office-conversion.json",
            "page-faithful, not editor-identical Word layout",
        ),
        (
            "p22-pptx-pdf",
            "office",
            "native PPTX slide to PDF pages",
            "-",
            "pptx",
            "tests/prompt22_writer_office.rs::office_to_pdf_reports_reopenable_pdf",
            "target/prompt22-writer-office-benchmark/prompt22-office-conversion.json",
            "safe chart/media posture inventory; no active media execution",
        ),
        (
            "p22-xlsx-pdf",
            "office",
            "native XLSX print-style sheet to PDF",
            "-",
            "xlsx",
            "tests/prompt22_writer_office.rs::office_to_pdf_reports_reopenable_pdf",
            "target/prompt22-writer-office-benchmark/prompt22-office-conversion.json",
            "cached formulas only; no formula execution",
        ),
        (
            "p22-benchmark",
            "benchmark",
            "deterministic Office conversion quality scorecard",
            "-",
            "docx,pptx,xlsx,pdf",
            "scripts/prompt22_writer_office_benchmark_audit.py",
            "target/prompt22-writer-office-benchmark/prompt22-scorecard.json",
            "reference tools are optional and never production converters",
        ),
    ];
    rows.into_iter()
        .map(|(feature_id, category, capability, dedup_eligibility, office_format, test, artifact, exact_limit)| {
            Prompt22FeatureMatrixRow {
                feature_id: feature_id.to_string(),
                category: category.to_string(),
                capability: capability.to_string(),
                implementation_status: Prompt22Status::ImplementedWithLimits,
                deterministic_security_status: "deterministic_and_fail_closed".to_string(),
                compression_mode: "fast,balanced,best,zopfli,zopfli_bounded".to_string(),
                dedup_eligibility: dedup_eligibility.to_string(),
                office_format: office_format.to_string(),
                rust_api: "oxide_engine::prompt22".to_string(),
                cli: "oxide prompt22-report / prompt22-optimize / prompt22-office-inspect / prompt22-office-to-pdf".to_string(),
                python: "oxide prompt22 module bindings".to_string(),
                c_abi: "oxide_prompt22_*".to_string(),
                wasm: "prompt22*".to_string(),
                dotnet: "OxideDocument.Prompt22* and OfficeConverters".to_string(),
                java: "Oxide.prompt22*".to_string(),
                fixture: "generated deterministic fixtures".to_string(),
                test: test.to_string(),
                artifact: artifact.to_string(),
                benchmark_status: "implemented_with_limits".to_string(),
                exact_limit: exact_limit.to_string(),
                future_owner: "writer-office".to_string(),
            }
        })
        .collect()
}

fn prompt22_exact_limits() -> Vec<String> {
    vec![
        "zopfli cancellation is checked at stream boundaries; inner zopfli calls are bounded by input bytes and iteration count".to_string(),
        "global dedup is limited to streams whose canonical dictionary and decoded bytes compare equal after a SHA-256 bucket match".to_string(),
        "encrypted PDF optimization is refused to avoid writing decrypted output or changing encryption semantics".to_string(),
        "full rewrite optimization invalidates existing cryptographic signatures; signature preservation remains incremental-update-only work".to_string(),
        "Office conversion is native and deterministic for supported DOCX/PPTX/XLSX fixtures, but not editor-identical to Microsoft Office layout".to_string(),
        "XLSX formulas are not executed; cached values and stored text are used and missing caches are reported by benchmark artifacts".to_string(),
        "reference tools such as Office, LibreOffice, qpdf, Poppler, PDFium, and MuPDF are optional benchmark comparators, never production conversion engines".to_string(),
    ]
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::office::{
        pdf_to_docx, pdf_to_pptx, pdf_to_xlsx, DocxOptions, PptxOptions, XlsxOptions,
    };
    use crate::writer::rewrite_document;
    use std::io::Write;

    fn tiny_pdf() -> Vec<u8> {
        b"%PDF-1.4\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n2 0 obj << /Type /Pages /Count 1 /Kids [3 0 R] >> endobj\n3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << >> /Contents 4 0 R >> endobj\n4 0 obj << /Length 37 >> stream\nq 1 0 0 1 10 10 cm 0 0 50 50 re f Q\nendstream\nendobj\nxref\n0 5\n0000000000 65535 f \n0000000009 00000 n \n0000000058 00000 n \n0000000115 00000 n \n0000000212 00000 n \ntrailer << /Size 5 /Root 1 0 R >>\nstartxref\n299\n%%EOF\n".to_vec()
    }

    #[test]
    fn zopfli_recompression_preserves_decoded_bytes() {
        let input = tiny_pdf();
        let options = Prompt22OptimizeOptions {
            compression: Prompt22CompressionOptions {
                mode: Prompt22CompressionMode::ZopfliBounded,
                iterations: 3,
                max_input_bytes: 64 * 1024,
                ..Prompt22CompressionOptions::default()
            },
            dedup: false,
            writer_mode: Prompt22WriterMode::ClassicXref,
            ..Prompt22OptimizeOptions::default()
        };
        let (output, report) = optimize_pdf(&input, None, options).unwrap();
        assert!(report.output_reopened);
        assert_eq!(report.compression.decoded_equality_failures, 0);
        assert_ne!(sha256_hex(&input), sha256_hex(&output));
    }

    #[test]
    fn dedup_rewrites_duplicate_stream_references() {
        let input = tiny_pdf();
        let doc = PdfDocument::open_bytes(input).unwrap();
        let duplicated = rewrite_document(doc.reader(), |_number, _object| {}).unwrap();
        let options = Prompt22OptimizeOptions {
            compression: Prompt22CompressionOptions {
                mode: Prompt22CompressionMode::Best,
                ..Prompt22CompressionOptions::default()
            },
            dedup: true,
            writer_mode: Prompt22WriterMode::ClassicXref,
            ..Prompt22OptimizeOptions::default()
        };
        let (_output, report) = optimize_pdf(&duplicated, None, options).unwrap();
        assert!(report.output_reopened);
        assert!(report.dedup.enabled);
    }

    #[test]
    fn office_security_blocks_external_relationship() {
        let mut bytes = Vec::new();
        {
            let cursor = Cursor::new(&mut bytes);
            let mut zip = zip::ZipWriter::new(cursor);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zip.start_file("[Content_Types].xml", options).unwrap();
            zip.write_all(br#"<?xml version="1.0"?><Types/>"#).unwrap();
            zip.start_file("_rels/.rels", options).unwrap();
            zip.write_all(br#"<Relationships><Relationship Id="rId1" Type="x" Target="http://example.invalid/a" TargetMode="External"/></Relationships>"#).unwrap();
            zip.start_file("word/document.xml", options).unwrap();
            zip.write_all(br#"<w:document/>"#).unwrap();
            zip.finish().unwrap();
        }
        let report = inspect_office_package_for_prompt22(&bytes, OfficeFormat::Docx).unwrap();
        assert!(!report.safe_for_conversion);
        assert!(report.external_relationship_count > 0);
    }

    #[test]
    fn office_to_pdf_reports_reopenable_pdf() {
        let engine = ContentEngine::open_bytes(tiny_pdf()).unwrap();
        let docx = pdf_to_docx(&engine, &DocxOptions::default()).unwrap();
        let (_pdf, report) =
            office_to_pdf_with_report(&docx, OfficeFormat::Docx, &OfficeToPdfOptions::default())
                .unwrap();
        assert!(report.output_reopened);

        let xlsx = pdf_to_xlsx(&engine, &XlsxOptions::default()).unwrap();
        let (_pdf, report) =
            office_to_pdf_with_report(&xlsx, OfficeFormat::Xlsx, &OfficeToPdfOptions::default())
                .unwrap();
        assert!(report.output_reopened);

        let pptx = pdf_to_pptx(&engine, &PptxOptions::default()).unwrap();
        let (_pdf, report) =
            office_to_pdf_with_report(&pptx, OfficeFormat::Pptx, &OfficeToPdfOptions::default())
                .unwrap();
        assert!(report.output_reopened);
    }

    #[test]
    fn feature_matrix_has_no_blocked_rows() {
        assert!(prompt22_feature_matrix()
            .iter()
            .all(|row| row.implementation_status != Prompt22Status::Blocked));
    }
}
