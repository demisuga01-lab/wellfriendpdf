//! Stable SDK facade for cross-language bindings.
//!
//! This module is the single, stable, versioned-JSON report layer that the
//! Python (`wellfriendpdf-py`) and C ABI (`wellfriendpdf-capi`) bindings call. It exists so the
//! bindings do not each reimplement report wiring against the flat crate root —
//! divergent binding behavior is the fastest way to create long-term SDK rot.
//!
//! Every function here:
//!
//! - takes the document as `&[u8]` (plus an optional password) so bindings only
//!   need to hand over the raw bytes — no pre-opened handle juggling;
//! - returns a **versioned JSON envelope** string
//!   `{"schema_version": N, "kind": "...", "report": {...}}` (see
//!   [`REPORT_ENVELOPE_VERSION`]) for rich reports, or `(Vec<u8>, String)` for
//!   output-producing operations (the produced PDF/artifact bytes plus a JSON
//!   report); and
//! - reports unsupported / partial capabilities honestly *inside* the report
//!   rather than returning a fake-success empty object.
//!
//! The envelope is intentionally boring and identical across languages: a
//! security report requested from Python and from C is byte-identical JSON.
//!
//! These wrappers do not add engine features. They expose, normalize, and make
//! safely bindable the reports and operations that already exist in
//! [`crate::security`], [`crate::interactive`], [`crate::parser_report`],
//! [`crate::color_report`], [`crate::compliance`], [`crate::standards`],
//! [`crate::versioning`], [`crate::editing`], [`crate::filters`], and the
//! [`crate::ContentEngine`] methods.

use serde::Serialize;
use serde_json::json;

use crate::{
    codec_isolation::{
        codec_isolation_availability_report, decode_filter_with_isolation, CodecIsolationConfig,
    },
    color_report::{color_report_bytes, ColorValidationProfile},
    compliance::{validate_pdfa, validate_pdfua, PdfAProfile},
    decode_scanner::scanner_availability_report,
    decode_scheduler::{
        non_render_decode_scheduler_adoption_report, renderer_decode_scheduler_adoption_report,
    },
    editing::{EditMode, ImageRect, PdfEditor, RedactionOptions},
    filters::{decode_image_budget_report, DecodeLimits},
    interactive::{
        annotation_report, forms_report, interactive_report, page_operations_report,
        redaction_verification_report,
    },
    parser_report::{parser_report_bytes_with_password, ParserMode},
    prepress,
    render::cmm,
    security::{
        canonicalize_pdf, sanitize_pdf, scan_risky_content, security_report, CanonicalizeOptions,
        SanitizerOptions,
    },
    standards::{validate_standards_profile, StandardsProfile},
    standards_engine::{
        validate_all_standards, validate_pdfa_profile, validate_pdfua_profile,
        validate_pdfx_profile, StandardsValidationOptions,
    },
    versioning::resource_dedup_report,
    ContentEngine, DocumentInfo, Result, TextQuad, TextSearchOptions, TextSemanticOptions,
    WellfriendError,
};

/// Version of the JSON envelope wrapping every SDK report. Bump only when the
/// envelope shape (not the inner report schema) changes. Inner reports keep
/// their own `schema_version` where they define one.
pub const REPORT_ENVELOPE_VERSION: u32 = 1;

/// Wrap a serializable report in the stable, versioned SDK envelope.
fn envelope<T: Serialize>(kind: &str, report: &T) -> Result<String> {
    let value = serde_json::to_value(report).map_err(json_err)?;
    let out = json!({
        "schema_version": REPORT_ENVELOPE_VERSION,
        "kind": kind,
        "report": value,
    });
    serde_json::to_string(&out).map_err(json_err)
}

fn json_err(err: serde_json::Error) -> crate::WellfriendError {
    crate::WellfriendError::invalid_input(format!("JSON serialization error: {err}"))
}

fn open(bytes: &[u8], password: Option<&[u8]>) -> Result<ContentEngine> {
    match password {
        Some(pw) if !pw.is_empty() => ContentEngine::open_bytes_with_password(bytes.to_vec(), pw),
        _ => ContentEngine::open_bytes(bytes.to_vec()),
    }
}

// ── Read-only reports ────────────────────────────────────────────────────────

/// Security report: encryption status, public-key handler / AES-GCM detection,
/// signatures, risky active content, and findings.
pub fn security_report_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let engine = open(bytes, password)?;
    envelope("security_report", &security_report(&engine)?)
}

/// Risky active-content inventory (JavaScript, launch/URI/submit actions,
/// embedded files, XFA packets, etc.).
pub fn risky_content_report_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let engine = open(bytes, password)?;
    envelope(
        "risky_content_report",
        &scan_risky_content(engine.document())?,
    )
}

/// Document metadata / structural facts (pdfinfo-equivalent): title, author,
/// page count, page sizes, encryption, permissions, producer, dates.
pub fn document_info_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let engine = open(bytes, password)?;
    envelope("document_info", &DocumentInfo::gather(engine.document())?)
}

/// Parser diagnostics: repair/xref/revisions/linearization/encryption discovery,
/// object cycle / malformed-object recovery notes, and Arlington integration
/// status. `mode` is one of `strict` | `repair` | `audit` (default `repair`).
pub fn parser_report_json(
    bytes: &[u8],
    mode: Option<&str>,
    password: Option<&[u8]>,
) -> Result<String> {
    let mode = parse_parser_mode(mode);
    let pw = password.unwrap_or(&[]);
    let report = parser_report_bytes_with_password(bytes, mode, pw);
    envelope("parser_report", &report)
}

/// Color / prepress report: ICC profiles, output intents, DeviceCMYK / DeviceN /
/// Separation / spot inventory, overprint, rendering intents, diagnostics.
/// `profile` is one of `generic` | `pdfa` | `pdfx` (default `generic`).
pub fn color_report_json(bytes: &[u8], profile: Option<&str>) -> Result<String> {
    let profile = parse_color_profile(profile);
    envelope("color_report", &color_report_bytes(bytes, profile)?)
}

/// PDF/A validation report. `profile` is one of `pdfa1b` | `pdfa2b` | `pdfa2a`
/// | `pdfa3b` | `pdfa3a` (default `pdfa2b`).
pub fn pdfa_validation_json(
    bytes: &[u8],
    profile: Option<&str>,
    password: Option<&[u8]>,
) -> Result<String> {
    let engine = open(bytes, password)?;
    let profile = parse_pdfa_profile(profile);
    envelope(
        "pdfa_validation",
        &validate_pdfa(engine.document(), profile)?,
    )
}

/// PDF/UA (accessibility) validation report.
pub fn pdfua_validation_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let engine = open(bytes, password)?;
    envelope("pdfua_validation", &validate_pdfua(engine.document())?)
}

/// Standards-profile validation report (PDF/A, PDF/UA, PDF/X, security, or all).
/// `profile` is one of `pdfa` | `pdfua` | `pdfx` | `security` | `all`
/// (default `all`).
pub fn standards_profile_json(
    bytes: &[u8],
    profile: Option<&str>,
    password: Option<&[u8]>,
) -> Result<String> {
    let engine = open(bytes, password)?;
    let profile = profile
        .and_then(StandardsProfile::parse)
        .unwrap_or(StandardsProfile::All);
    envelope(
        "standards_profile",
        &validate_standards_profile(&engine, profile)?,
    )
}

fn standards_options(target: Option<&str>) -> StandardsValidationOptions {
    target
        .map(StandardsValidationOptions::with_target)
        .unwrap_or_default()
}

/// Incremental Signing Standards clause-mapped PDF/A validation report. `target` is a profile label
/// such as `PDF/A-2B` (defaults to the detected/claimed profile or `PDF/A-2B`).
pub fn pdfa_standards_json(
    bytes: &[u8],
    target: Option<&str>,
    password: Option<&[u8]>,
) -> Result<String> {
    let engine = open(bytes, password)?;
    envelope(
        "pdfa_standards_validation",
        &validate_pdfa_profile(&engine, &standards_options(target))?,
    )
}

/// Incremental Signing Standards clause-mapped PDF/UA validation report.
pub fn pdfua_standards_json(
    bytes: &[u8],
    target: Option<&str>,
    password: Option<&[u8]>,
) -> Result<String> {
    let engine = open(bytes, password)?;
    envelope(
        "pdfua_standards_validation",
        &validate_pdfua_profile(&engine, &standards_options(target))?,
    )
}

/// Incremental Signing Standards clause-mapped PDF/X validation report. `target` e.g. `PDF/X-4`.
pub fn pdfx_standards_json(
    bytes: &[u8],
    target: Option<&str>,
    password: Option<&[u8]>,
) -> Result<String> {
    let engine = open(bytes, password)?;
    envelope(
        "pdfx_standards_validation",
        &validate_pdfx_profile(&engine, &standards_options(target))?,
    )
}

/// Incremental Signing Standards combined PDF/A + PDF/UA + PDF/X validation with cross-profile
/// conflicts. A single profile passing never hides another failing.
pub fn standards_all_json(
    bytes: &[u8],
    target: Option<&str>,
    password: Option<&[u8]>,
) -> Result<String> {
    let engine = open(bytes, password)?;
    envelope(
        "standards_all_validation",
        &validate_all_standards(&engine, &standards_options(target))?,
    )
}

/// Combined interactive report: forms + annotations + page operations.
pub fn interactive_report_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let engine = open(bytes, password)?;
    envelope("interactive_report", &interactive_report(&engine)?)
}

/// AcroForm field inventory: field trees, inheritance, widgets, XFA status.
pub fn forms_report_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let engine = open(bytes, password)?;
    envelope("forms_report", &forms_report(&engine)?)
}

/// XFA Runtime XFA packet inventory with bounded XML parse diagnostics.
pub fn xfa_report_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let engine = open(bytes, password)?;
    envelope(
        "xfa_report",
        &crate::xfa::xfa_inventory(&engine, &crate::xfa::XfaLimits::default())?,
    )
}

/// XFA Runtime static XFA template/dataset extraction and semantic mapping.
pub fn xfa_extract_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let engine = open(bytes, password)?;
    envelope(
        "xfa_extract_report",
        &crate::xfa::extract_xfa(&engine, &crate::xfa::XfaLimits::default())?,
    )
}

/// XFA Runtime bounded dynamic runtime report. `script_policy` is `disabled` or
/// `formcalc-safe-subset`; supported events execute only when `execute_events`
/// is true.
pub fn xfa_runtime_report_json(
    bytes: &[u8],
    script_policy: Option<&str>,
    execute_events: bool,
    password: Option<&[u8]>,
) -> Result<String> {
    let engine = open(bytes, password)?;
    let options = crate::xfa::XfaRuntimeOptions {
        script_policy: parse_xfa_script_policy(script_policy)?,
        execute_supported_events: execute_events,
        ..crate::xfa::XfaRuntimeOptions::default()
    };
    envelope(
        "xfa_runtime_report",
        &crate::xfa::xfa_runtime_report(&engine, &options)?,
    )
}

/// XFA Runtime script/event inventory and default/sandbox policy report.
pub fn xfa_script_report_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let engine = open(bytes, password)?;
    let extraction = crate::xfa::extract_xfa(&engine, &crate::xfa::XfaLimits::default())?;
    let runtime =
        crate::xfa::xfa_runtime_report(&engine, &crate::xfa::XfaRuntimeOptions::default())?;
    envelope(
        "xfa_script_report",
        &json!({
            "schema_version": crate::xfa::XFA_SCHEMA_VERSION,
            "scripts": extraction.scripts,
            "events": extraction.events,
            "sandbox": runtime.sandbox,
        }),
    )
}

/// XFA Runtime XFA-specific security, signature, sanitizer, and redaction posture.
pub fn xfa_security_report_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let engine = open(bytes, password)?;
    envelope(
        "xfa_security_report",
        &crate::xfa::xfa_security_report(&engine, &crate::xfa::XfaLimits::default())?,
    )
}

/// Annotation inventory: kinds, QuadPoints, appearance status, unsafe actions.
pub fn annotation_report_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let engine = open(bytes, password)?;
    envelope("annotation_report", &annotation_report(&engine)?)
}

/// annotation/media redaction rich-media inventory. Media payloads are hashed/inventoried but
/// never decoded or executed.
pub fn rich_media_report_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let engine = open(bytes, password)?;
    envelope(
        "rich_media_report",
        &crate::annotation_media_redaction::rich_media_inventory(
            &engine,
            &crate::annotation_media_redaction::RichMediaLimits::default(),
        )?,
    )
}

/// annotation/media redaction non-axis redaction planning from a JSON options document.
pub fn nonaxis_redaction_plan_json(
    bytes: &[u8],
    options_json: &str,
    password: Option<&[u8]>,
) -> Result<String> {
    let engine = open(bytes, password)?;
    let options: crate::annotation_media_redaction::NonAxisRedactionOptions =
        serde_json::from_str(options_json).map_err(json_err)?;
    envelope(
        "nonaxis_redaction_plan",
        &crate::annotation_media_redaction::plan_nonaxis_image_redaction(&engine, &options)?,
    )
}

/// Combined annotation/media redaction inventory/report surface.
pub fn annotation_media_redaction_report_json(
    bytes: &[u8],
    password: Option<&[u8]>,
) -> Result<String> {
    let engine = open(bytes, password)?;
    let (_, xfdf) = crate::annotation_media_redaction::export_annotation_xfdf(&engine)?;
    let media = crate::annotation_media_redaction::rich_media_inventory(
        &engine,
        &crate::annotation_media_redaction::RichMediaLimits::default(),
    )?;
    envelope(
        "annotation_media_redaction_report",
        &json!({
            "schema_version": crate::annotation_media_redaction::ANNOTATION_MEDIA_REDACTION_SCHEMA_VERSION,
            "annotation_xfdf": xfdf,
            "rich_media": media,
            "feature": crate::annotation_media_redaction::annotation_media_redaction_feature_report_value(REPORT_ENVELOPE_VERSION),
            "nonaxis_redaction": {
                "planning": "request_specific",
                "sample_space_polygon_rewrite": true,
                "secure_removal_fallback": true,
                "overlay_only_success_claims": 0
            }
        }),
    )
}

/// Combined secure mutation secure-mutation inventory and policy report.
pub fn secure_mutation_report_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let engine = open(bytes, password)?;
    envelope(
        "secure_mutation_report",
        &crate::secure_mutation::secure_mutation_report(&engine)?,
    )
}

pub fn secure_mutation_closeout_report_json(
    bytes: &[u8],
    password: Option<&[u8]>,
) -> Result<String> {
    let engine = open(bytes, password)?;
    envelope(
        "secure_mutation_closeout_report",
        &crate::secure_mutation::secure_mutation_closeout_report(&engine)?,
    )
}

/// form action policy form JavaScript/action inventory. JavaScript is never executed by
/// this operation.
pub fn form_js_report_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let engine = open(bytes, password)?;
    envelope(
        "form_js_report",
        &crate::form_action_policy::form_javascript_inventory(
            &engine,
            &crate::form_action_policy::FormJsLimits::default(),
        )?,
    )
}

pub fn form_action_graph_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let engine = open(bytes, password)?;
    let inventory = crate::form_action_policy::form_javascript_inventory(
        &engine,
        &crate::form_action_policy::FormJsLimits::default(),
    )?;
    envelope(
        "form_action_graph",
        &crate::form_action_policy::form_action_graph(&engine, &inventory)?,
    )
}

pub fn form_js_sanitize_json(
    bytes: &[u8],
    options_json: Option<&str>,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let engine = open(bytes, password)?;
    let options = options_json
        .map(serde_json::from_str::<crate::form_action_policy::FormJsSanitizerOptions>)
        .transpose()
        .map_err(|error| {
            WellfriendError::invalid_input(format!(
                "invalid form action policy sanitizer options: {error}"
            ))
        })?
        .unwrap_or_default();
    let (output, report) = crate::form_action_policy::form_js_sanitize_pdf(
        engine.document().reader().file_bytes(),
        &options,
    )?;
    Ok((output, envelope("form_js_sanitize", &report)?))
}

pub fn form_js_flatten_values_json(
    bytes: &[u8],
    options_json: Option<&str>,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let engine = open(bytes, password)?;
    let mut options = options_json
        .map(serde_json::from_str::<crate::form_action_policy::FormJsSanitizerOptions>)
        .transpose()
        .map_err(|error| {
            WellfriendError::invalid_input(format!(
                "invalid form action policy flatten options: {error}"
            ))
        })?
        .unwrap_or_default();
    options.mode = crate::form_action_policy::FormJsPolicyMode::FlattenCalculatedValuesThenRemove;
    let (output, report) = crate::form_action_policy::flatten_calculated_values_pdf(
        engine.document().reader().file_bytes(),
        &options,
    )?;
    Ok((output, envelope("form_js_flatten_values", &report)?))
}

pub fn interactive_data_closeout_report_json(
    bytes: &[u8],
    password: Option<&[u8]>,
) -> Result<String> {
    let engine = open(bytes, password)?;
    envelope(
        "interactive_data_report",
        &crate::form_action_policy::interactive_data_closeout_report(&engine)?,
    )
}

pub fn word_pagination_audit_json(
    bytes: &[u8],
    layout: &str,
    password: Option<&[u8]>,
) -> Result<String> {
    let engine = open(bytes, password)?;
    let layout = crate::office::DocxLayout::parse(layout).ok_or_else(|| {
        WellfriendError::invalid_input("unknown DOCX layout; use flowing, page-faithful, or hybrid")
    })?;
    envelope(
        "word_pagination_audit",
        &crate::form_action_policy::word_pagination_audit(&engine, layout)?,
    )
}

pub fn form_action_policy_report_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let engine = open(bytes, password)?;
    envelope(
        "form_action_policy_report",
        &crate::form_action_policy::form_action_policy_report(&engine)?,
    )
}

pub fn advanced_editing_report_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let engine = open(bytes, password)?;
    envelope(
        "advanced_editing_report",
        &crate::advanced_editing::advanced_editing_report(&engine)?,
    )
}

/// advanced editing closeout's additive closure report.  This deliberately remains a shared
/// SDK report so language bindings do not reimplement ownership policy.
pub fn advanced_editing_closeout_report_json(
    bytes: &[u8],
    password: Option<&[u8]>,
) -> Result<String> {
    let engine = open(bytes, password)?;
    envelope(
        "advanced_editing_closeout_report",
        &serde_json::json!({
            "schema_version": "advanced_editing_closeout.multirun-form-appearance-closure.v1",
            "status": "implemented_with_limits",
            "multi_run_range": "implemented_token_boundary_page_stream",
            "rtl_logical_visual_mapping": "implemented_bidi_provenance",
            "vertical_range": "implemented_with_limits_cluster_policy",
            "nested_form_clone_one": "implemented_with_limits_recursive_invocation_inventory",
            "annotation_appearance_clone_one": "implemented_with_limits_owner_specific_N_R_D_state_widget_and_nested_Form_policy",
            "bindings": ["rust", "cli", "python", "c_abi", "wasm", "dotnet", "java_maven", "java_gradle"],
            "signature_policy": "secure_mutation_closeout_preflight_required",
            "cryptographic_validity_claimed": false,
            "page_count": engine.page_count()?,
            "exact_limits": [
              "range edits require contiguous decoded string-token boundaries in one page content stream",
              "arbitrary Type3 editing and pattern/shading program editing remain unsupported",
              "visual quad selection must resolve to one unambiguous logical range before mutation",
              "structural incremental preservation does not imply cryptographic signature validity"
            ]
        }),
    )
}

/// Combined writer history raster/vector, font reconstruction, persistent history,
/// and object-stream writer report.
pub fn writer_history_report_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let engine = open(bytes, password)?;
    envelope(
        "writer_history_report",
        &crate::writer_history::writer_history_report(&engine)?,
    )
}

pub fn writer_history_raster_vector_report_json(
    bytes: &[u8],
    page: usize,
    options_json: Option<&str>,
    password: Option<&[u8]>,
) -> Result<String> {
    let engine = open(bytes, password)?;
    let options = match options_json {
        Some(json) if !json.trim().is_empty() => {
            serde_json::from_str::<crate::writer_history::RasterVectorizationOptions>(json)
                .map_err(json_err)?
        }
        _ => crate::writer_history::RasterVectorizationOptions::default(),
    };
    envelope(
        "writer_history_raster_vector_report",
        &crate::writer_history::raster_vectorization_report(&engine, page, options)?,
    )
}

pub fn writer_history_font_reconstruction_report_json(
    bytes: &[u8],
    password: Option<&[u8]>,
) -> Result<String> {
    let engine = open(bytes, password)?;
    envelope(
        "writer_history_font_reconstruction_report",
        &crate::writer_history::font_reconstruction_report(&engine)?,
    )
}

pub fn writer_history_history_report_json() -> Result<String> {
    envelope(
        "writer_history_history_report",
        &crate::writer_history::persistent_store_report(),
    )
}

pub fn writer_history_object_stream_report_json(
    bytes: &[u8],
    password: Option<&[u8]>,
) -> Result<String> {
    let engine = open(bytes, password)?;
    envelope(
        "writer_history_object_stream_report",
        &crate::writer_history::object_stream_packing_report(engine.document().reader())?,
    )
}

pub fn writer_history_pack_object_streams_json(
    bytes: &[u8],
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let (output, report) = crate::writer_history::pack_object_streams_pdf(bytes, password)?;
    Ok((
        output,
        envelope("writer_history_pack_object_streams_report", &report)?,
    ))
}

/// Combined compression and Office zopfli, resource dedup, Office conversion, and benchmark report.
pub fn compression_office_report_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let engine = open(bytes, password)?;
    envelope(
        "compression_office_report",
        &crate::compression_office::compression_office_report(&engine)?,
    )
}

/// Combined crypto writer deterministic writer, PubSec, and AES-GCM posture report.
pub fn crypto_writer_report_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let engine = open(bytes, password)?;
    envelope(
        "crypto_writer_report",
        &crate::crypto_writer::crypto_writer_report(&engine)?,
    )
}

pub fn writer_determinism_audit_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let engine = open(bytes, password)?;
    envelope(
        "writer_determinism_audit",
        &crate::crypto_writer::deterministic_writer_audit(&engine)?,
    )
}

pub fn writer_external_diff_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let engine = open(bytes, password)?;
    envelope(
        "writer_external_diff",
        &crate::crypto_writer::writer_external_diff_report(&engine)?,
    )
}

pub fn writer_closeout_report_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let engine = open(bytes, password)?;
    envelope(
        "writer_closeout_report",
        &crate::crypto_writer::writer_closeout_report(&engine)?,
    )
}

pub fn pubsec_report_json(bytes: &[u8], _password: Option<&[u8]>) -> Result<String> {
    envelope(
        "pubsec_report",
        &crate::crypto_writer::public_key_handler_report_bytes(bytes),
    )
}

pub fn aes_gcm_report_json(bytes: &[u8], _password: Option<&[u8]>) -> Result<String> {
    envelope(
        "aes_gcm_report",
        &crate::crypto_writer::aes_gcm_report_bytes(bytes),
    )
}

pub fn pdf_mac_report_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    envelope(
        "pdf_mac_report",
        &crate::pdf_mac::pdf_mac_report_bytes(bytes, password)?,
    )
}

pub fn pdf_mac_verify_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    envelope(
        "pdf_mac_verify",
        &crate::pdf_mac::pdf_mac_verify_report_bytes(bytes, password)?,
    )
}

pub fn pdf_mac_create_json(bytes: &[u8], password: Option<&[u8]>) -> Result<(Vec<u8>, String)> {
    let engine = open(bytes, password)?;
    let params = crate::EncryptParams {
        user_password: crate::crypto::secret_bytes(Vec::new()),
        owner_password: crate::crypto::secret_bytes(Vec::new()),
        permissions: -1,
        algorithm: crate::EncryptAlgorithm::Aes256Gcm,
        encrypt_metadata: true,
    };
    let (output, report) = crate::encrypt_pdf_with_pdf_mac(&engine, &params)?;
    Ok((output, envelope("pdf_mac_create", &report)?))
}

pub fn crypto_tamper_test_json() -> Result<String> {
    envelope(
        "crypto_tamper_test",
        &crate::crypto_writer::crypto_tamper_test_report(),
    )
}

pub fn compression_office_optimize_pdf_json(
    bytes: &[u8],
    options_json: Option<&str>,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let options = match options_json {
        Some(json) if !json.trim().is_empty() => serde_json::from_str::<
            crate::compression_office::CompressionOfficeOptimizeOptions,
        >(json)
        .map_err(json_err)?,
        _ => crate::compression_office::CompressionOfficeOptimizeOptions::default(),
    };
    let (output, report) = crate::compression_office::optimize_pdf(bytes, password, options)?;
    Ok((
        output,
        envelope("compression_office_optimize_report", &report)?,
    ))
}

pub fn compression_office_office_inspect_json(bytes: &[u8], format: &str) -> Result<String> {
    let format = crate::office::OfficeFormat::parse(format).ok_or_else(|| {
        WellfriendError::invalid_input(format!(
            "unsupported Office format '{format}', expected docx, pptx, or xlsx"
        ))
    })?;
    envelope(
        "compression_office_office_package_security",
        &crate::compression_office::inspect_office_package_for_compression_office(bytes, format)?,
    )
}

pub fn compression_office_office_to_pdf_json(
    bytes: &[u8],
    format: &str,
) -> Result<(Vec<u8>, String)> {
    let format = crate::office::OfficeFormat::parse(format).ok_or_else(|| {
        WellfriendError::invalid_input(format!(
            "unsupported Office format '{format}', expected docx, pptx, or xlsx"
        ))
    })?;
    let (output, report) = crate::compression_office::office_to_pdf_with_report(
        bytes,
        format,
        &crate::office::OfficeToPdfOptions::default(),
    )?;
    Ok((
        output,
        envelope("compression_office_office_to_pdf_report", &report)?,
    ))
}

pub fn advanced_editing_closeout_text_range_analyze_json(
    bytes: &[u8],
    page: usize,
    password: Option<&[u8]>,
) -> Result<String> {
    let input = mutation_input(bytes, password)?;
    envelope(
        "advanced_editing_closeout_multi_run_range_model",
        &crate::advanced_editing::analyze_multi_run_text_range(&input, page)?,
    )
}

pub fn advanced_editing_closeout_text_range_edit_json(
    bytes: &[u8],
    request_json: &str,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let input = mutation_input(bytes, password)?;
    let request =
        serde_json::from_str::<crate::advanced_editing::MultiRunTextRangeRequest>(request_json)
            .map_err(json_err)?;
    let (output, report) =
        crate::advanced_editing::edit_multi_run_text_range(&input, &request, None)?;
    Ok((
        output,
        envelope(
            "advanced_editing_closeout_multi_run_text_edit_report",
            &report,
        )?,
    ))
}

pub fn advanced_editing_vector_list_json(
    bytes: &[u8],
    page: usize,
    password: Option<&[u8]>,
) -> Result<String> {
    let input = mutation_input(bytes, password)?;
    envelope(
        "advanced_editing_vector_inventory",
        &crate::advanced_editing::list_vector_objects(&input, page)?,
    )
}

pub fn advanced_editing_text_edit_json(
    bytes: &[u8],
    page: usize,
    old_text: &str,
    new_text: &str,
    mode: &str,
    options_json: Option<&str>,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let input = mutation_input(bytes, password)?;
    let mode = match mode {
        "horizontal-reflow" | "paragraph_reflow_horizontal" => {
            crate::advanced_editing::AdvancedTextMode::ParagraphReflowHorizontal
        }
        "rtl-reflow" | "paragraph_reflow_rtl" => {
            crate::advanced_editing::AdvancedTextMode::ParagraphReflowRtl
        }
        "vertical-reflow" | "paragraph_reflow_vertical" => {
            crate::advanced_editing::AdvancedTextMode::ParagraphReflowVertical
        }
        "same-width-patch" | "safe_patch" => {
            let options = options_json
                .map(serde_json::from_str::<crate::advanced_editing::SameWidthPatchOptions>)
                .transpose()
                .map_err(json_err)?
                .unwrap_or_default();
            let (output, report) = crate::advanced_editing::apply_same_width_patch(
                &input, page, old_text, new_text, &options,
            )?;
            return Ok((
                output,
                envelope("advanced_editing_same_width_patch_report", &report)?,
            ));
        }
        other => {
            return Err(WellfriendError::invalid_input(format!(
                "unknown advanced editing text edit mode '{other}'"
            )))
        }
    };
    let options = options_json
        .map(serde_json::from_str::<crate::advanced_editing::AdvancedTextEditOptions>)
        .transpose()
        .map_err(json_err)?
        .unwrap_or_default();
    let (output, report) = crate::advanced_editing::edit_advanced_text_pdf(
        &input, page, old_text, new_text, mode, &options, None,
    )?;
    Ok((
        output,
        envelope("advanced_editing_text_edit_report", &report)?,
    ))
}

pub fn advanced_editing_vector_edit_json(
    bytes: &[u8],
    page: usize,
    stable_id: &str,
    operation_json: &str,
    options_json: Option<&str>,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let input = mutation_input(bytes, password)?;
    let operation =
        serde_json::from_str::<crate::advanced_editing::VectorEditOperation>(operation_json)
            .map_err(json_err)?;
    let options = options_json
        .map(serde_json::from_str::<crate::advanced_editing::VectorEditOptions>)
        .transpose()
        .map_err(json_err)?
        .unwrap_or_default();
    let (output, report) =
        crate::advanced_editing::edit_vector_object(&input, page, stable_id, operation, &options)?;
    Ok((
        output,
        envelope("advanced_editing_vector_edit_report", &report)?,
    ))
}

pub fn advanced_editing_ink_fit_json(
    bytes: &[u8],
    page: usize,
    annotation_index: usize,
    options_json: Option<&str>,
    signature_policy_override: bool,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let input = mutation_input(bytes, password)?;
    let options = options_json
        .map(serde_json::from_str::<crate::advanced_editing::InkFitOptions>)
        .transpose()
        .map_err(json_err)?
        .unwrap_or_default();
    let (output, report) = crate::advanced_editing::fit_annotation_ink_pdf(
        &input,
        page,
        annotation_index,
        &options,
        signature_policy_override,
    )?;
    Ok((
        output,
        envelope("advanced_editing_ink_fit_report", &report)?,
    ))
}

/// source editing's canonical architecture/status report.  It describes the real
/// parser-backed editing paths and exact editing transactions/33 deferrals.
pub fn source_editing_report_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let _ = open(bytes, password)?;
    envelope(
        "source_editing_report",
        &crate::source_editing::source_editing_report(),
    )
}

/// Resolve parser-backed text source instructions and semantic source spans.
pub fn source_editing_provenance_json(
    bytes: &[u8],
    page: usize,
    source_text: &str,
    replacement_text: &str,
    password: Option<&[u8]>,
) -> Result<String> {
    let input = mutation_input(bytes, password)?;
    envelope(
        "source_editing_provenance_report",
        &crate::source_editing::operator_text_provenance(
            &input,
            page,
            source_text,
            replacement_text,
        )?,
    )
}

/// Plan a source editing operator-preserving text edit.  Refusals are returned as
/// structured JSON and do not alter the input document.
pub fn source_editing_edit_eligibility_json(
    bytes: &[u8],
    request_json: &str,
    password: Option<&[u8]>,
) -> Result<String> {
    let input = mutation_input(bytes, password)?;
    let request =
        serde_json::from_str::<crate::source_editing::OperatorTextEditRequest>(request_json)
            .map_err(json_err)?;
    envelope(
        "source_editing_operator_text_eligibility",
        &crate::source_editing::operator_text_eligibility(&input, &request)?,
    )
}

/// Apply the source-level source editing text mutation and return bytes plus its
/// stable operation report.  This path rejects ineligible requests; callers
/// should invoke `source_editing_edit_eligibility_json` first to receive a refusal.
pub fn source_editing_operator_text_edit_json(
    bytes: &[u8],
    request_json: &str,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let input = mutation_input(bytes, password)?;
    let request =
        serde_json::from_str::<crate::source_editing::OperatorTextEditRequest>(request_json)
            .map_err(json_err)?;
    let (output, report) = crate::source_editing::edit_text_operator(&input, &request)?;
    Ok((
        output,
        envelope("source_editing_operator_text_edit", &report)?,
    ))
}

/// Return the canonical vector/path source inventory used by source editing.
pub fn source_editing_path_provenance_json(
    bytes: &[u8],
    page: usize,
    password: Option<&[u8]>,
) -> Result<String> {
    let input = mutation_input(bytes, password)?;
    envelope(
        "source_editing_operator_path_provenance",
        &crate::source_editing::operator_path_provenance(&input, page)?,
    )
}

/// Apply a canonical source-range vector/path or graphics-state mutation.
pub fn source_editing_path_edit_json(
    bytes: &[u8],
    page: usize,
    stable_id: &str,
    operation_json: &str,
    options_json: Option<&str>,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let input = mutation_input(bytes, password)?;
    let operation =
        serde_json::from_str::<crate::advanced_editing::VectorEditOperation>(operation_json)
            .map_err(json_err)?;
    let options = options_json
        .map(serde_json::from_str::<crate::advanced_editing::VectorEditOptions>)
        .transpose()
        .map_err(json_err)?
        .unwrap_or_default();
    let (output, report) =
        crate::source_editing::edit_path_operator(&input, page, stable_id, operation, &options)?;
    Ok((
        output,
        envelope("source_editing_operator_path_edit", &report)?,
    ))
}

/// Images intentionally fail closed until canonical occurrence-to-source
/// instruction identity is available.
pub fn source_editing_image_eligibility_json(
    bytes: &[u8],
    page: usize,
    password: Option<&[u8]>,
) -> Result<String> {
    let input = mutation_input(bytes, password)?;
    envelope(
        "source_editing_operator_image_eligibility",
        &crate::source_editing::operator_image_eligibility(&input, page),
    )
}

/// editing transactions editable scene/transaction/font architecture report.
pub fn editing_transactions_report_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let input = mutation_input(bytes, password)?;
    envelope(
        "editing_transactions_report",
        &crate::editing_transactions::editing_transactions_report(&input)?,
    )
}

/// Build the source-linked editing transactions editable scene graph for selected pages.
pub fn editing_transactions_scene_report_json(
    bytes: &[u8],
    pages_json: Option<&str>,
    password: Option<&[u8]>,
) -> Result<String> {
    let input = mutation_input(bytes, password)?;
    let pages = pages_json
        .map(serde_json::from_str::<Vec<usize>>)
        .transpose()
        .map_err(json_err)?
        .unwrap_or_default();
    envelope(
        "editing_transactions_scene_report",
        &crate::editing_transactions::build_scene_graph(&input, &pages)?,
    )
}

/// Resolve a bounded scene selection/hit-test query.
pub fn editing_transactions_scene_select_json(
    bytes: &[u8],
    request_json: &str,
    password: Option<&[u8]>,
) -> Result<String> {
    let input = mutation_input(bytes, password)?;
    let request =
        serde_json::from_str::<crate::editing_transactions::SceneSelectionRequest>(request_json)
            .map_err(json_err)?;
    envelope(
        "editing_transactions_scene_select",
        &crate::editing_transactions::scene_select(&input, &request)?,
    )
}

/// Plan an atomic editing transactions scene text transaction.
pub fn editing_transactions_transaction_plan_json(
    bytes: &[u8],
    request_json: &str,
    password: Option<&[u8]>,
) -> Result<String> {
    let input = mutation_input(bytes, password)?;
    let request =
        serde_json::from_str::<crate::editing_transactions::SceneTextEditRequest>(request_json)
            .map_err(json_err)?;
    envelope(
        "editing_transactions_transaction_plan",
        &crate::editing_transactions::plan_scene_text_transaction(&input, &request)?,
    )
}

/// Apply an atomic editing transactions scene text transaction through the canonical writer.
pub fn editing_transactions_transaction_apply_json(
    bytes: &[u8],
    request_json: &str,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let input = mutation_input(bytes, password)?;
    let request =
        serde_json::from_str::<crate::editing_transactions::SceneTextEditRequest>(request_json)
            .map_err(json_err)?;
    let (output, report) =
        crate::editing_transactions::apply_scene_text_transaction(&input, &request)?;
    Ok((
        output,
        envelope("editing_transactions_transaction_apply", &report)?,
    ))
}

/// Alias for scene-facing text edits that compile to source-level source editing ops.
pub fn editing_transactions_scene_edit_text_json(
    bytes: &[u8],
    request_json: &str,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    editing_transactions_transaction_apply_json(bytes, request_json, password)
}

/// Report dirty entities/regions for a scene text transaction.
pub fn editing_transactions_dirty_region_json(
    bytes: &[u8],
    request_json: &str,
    password: Option<&[u8]>,
) -> Result<String> {
    let input = mutation_input(bytes, password)?;
    let request =
        serde_json::from_str::<crate::editing_transactions::SceneTextEditRequest>(request_json)
            .map_err(json_err)?;
    envelope(
        "editing_transactions_dirty_region",
        &crate::editing_transactions::dirty_region_report(&input, &request)?,
    )
}

/// Report exact text identity, grapheme, bidi, shaping, and reverse mapping.
pub fn editing_transactions_text_map_json(text: &str, direction: Option<&str>) -> Result<String> {
    envelope(
        "editing_transactions_text_map",
        &crate::editing_transactions::text_identity_report(text, direction)?,
    )
}

/// Preview shaping with the canonical rustybuzz-backed generated-text shaper.
pub fn editing_transactions_shape_text_json(text: &str, direction: Option<&str>) -> Result<String> {
    let report = crate::editing_transactions::text_identity_report(text, direction)?;
    envelope("editing_transactions_shape_text", &report.shaping)
}

/// Plan deterministic font subset rebuilding and exact unsupported boundaries.
pub fn editing_transactions_font_subset_plan_json(
    text: &str,
    direction: Option<&str>,
    policy: Option<&str>,
) -> Result<String> {
    envelope(
        "editing_transactions_font_subset_plan",
        &crate::editing_transactions::font_subset_plan(text, direction, policy)?,
    )
}

/// Report deterministic font substitution policy and scoring.
pub fn editing_transactions_font_substitution_report_json(
    requested_family: &str,
    text: &str,
    policy: Option<&str>,
) -> Result<String> {
    envelope(
        "editing_transactions_font_substitution_report",
        &crate::editing_transactions::substitution_report(requested_family, text, policy),
    )
}

/// text reflow geometric and semantic reflow architecture report.
pub fn text_reflow_report_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let input = mutation_input(bytes, password)?;
    envelope(
        "text_reflow_report",
        &crate::text_reflow::text_reflow_report(&input)?,
    )
}

/// Analyze a source-linked geometric text region for text reflow reflow.
pub fn text_reflow_layout_analyze_json(
    bytes: &[u8],
    request_json: &str,
    password: Option<&[u8]>,
) -> Result<String> {
    let input = mutation_input(bytes, password)?;
    let request = serde_json::from_str::<crate::text_reflow::GeometricReflowRequest>(request_json)
        .map_err(json_err)?;
    envelope(
        "text_reflow_layout_analyze",
        &crate::text_reflow::analyze_geometric_region(&input, &request)?,
    )
}

/// Analyze the source-linked semantic layout graph.
pub fn text_reflow_semantic_layout_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let input = mutation_input(bytes, password)?;
    envelope(
        "text_reflow_semantic_layout",
        &crate::text_reflow::analyze_semantic_layout(&input, None)?,
    )
}

/// Report deterministic reading-order DAG and ambiguity.
pub fn text_reflow_reading_order_report_json(
    bytes: &[u8],
    password: Option<&[u8]>,
) -> Result<String> {
    let input = mutation_input(bytes, password)?;
    envelope(
        "text_reflow_reading_order_report",
        &crate::text_reflow::reading_order_report(&input)?,
    )
}

/// Report text reflow cross-column/cross-page flow graph.
pub fn text_reflow_flow_graph_report_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let input = mutation_input(bytes, password)?;
    envelope(
        "text_reflow_flow_graph_report",
        &crate::text_reflow::flow_graph_report(&input)?,
    )
}

/// Preview a GeometricBlock or SemanticDocument reflow without mutating bytes.
pub fn text_reflow_reflow_preview_json(
    bytes: &[u8],
    request_json: &str,
    password: Option<&[u8]>,
) -> Result<String> {
    let input = mutation_input(bytes, password)?;
    let request = serde_json::from_str::<crate::text_reflow::GeometricReflowRequest>(request_json)
        .map_err(json_err)?;
    envelope(
        "text_reflow_reflow_preview",
        &crate::text_reflow::preview_reflow(&input, &request)?,
    )
}

/// Query ordered overflow evidence without mutating the PDF.
pub fn text_reflow_overflow_report_json(
    bytes: &[u8],
    request_json: &str,
    password: Option<&[u8]>,
) -> Result<String> {
    let input = mutation_input(bytes, password)?;
    let request = serde_json::from_str::<crate::text_reflow::GeometricReflowRequest>(request_json)
        .map_err(json_err)?;
    envelope(
        "text_reflow_overflow_report",
        &crate::text_reflow::query_overflow(&input, &request)?,
    )
}

/// Query bounded hard/soft constraint evidence without mutating the PDF.
pub fn text_reflow_constraints_report_json(
    bytes: &[u8],
    request_json: &str,
    password: Option<&[u8]>,
) -> Result<String> {
    let input = mutation_input(bytes, password)?;
    let request = serde_json::from_str::<crate::text_reflow::GeometricReflowRequest>(request_json)
        .map_err(json_err)?;
    envelope(
        "text_reflow_constraints_report",
        &crate::text_reflow::query_constraints(&input, &request)?,
    )
}

/// Query central confidence/review enforcement without mutating the PDF.
pub fn text_reflow_confidence_report_json(
    bytes: &[u8],
    request_json: &str,
    password: Option<&[u8]>,
) -> Result<String> {
    let input = mutation_input(bytes, password)?;
    let request = serde_json::from_str::<crate::text_reflow::GeometricReflowRequest>(request_json)
        .map_err(json_err)?;
    envelope(
        "text_reflow_confidence_report",
        &crate::text_reflow::query_confidence(&input, &request)?,
    )
}

/// Validate a completed supported local text reflow reflow against canonical
/// reopen and unaffected-content evidence. The output is an explicit byte
/// slice so callers cannot accidentally validate their unchanged input.
pub fn text_reflow_validate_reflow_output_json(
    bytes: &[u8],
    output: &[u8],
    request_json: &str,
    password: Option<&[u8]>,
) -> Result<String> {
    let input = mutation_input(bytes, password)?;
    let request = serde_json::from_str::<crate::text_reflow::GeometricReflowRequest>(request_json)
        .map_err(json_err)?;
    envelope(
        "text_reflow_validate_reflow_output",
        &crate::text_reflow::validate_reflow_output(&input, output, &request)?,
    )
}

/// Apply a supported GeometricBlock reflow through canonical source mutation.
pub fn text_reflow_reflow_region_json(
    bytes: &[u8],
    request_json: &str,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let input = mutation_input(bytes, password)?;
    let request = serde_json::from_str::<crate::text_reflow::GeometricReflowRequest>(request_json)
        .map_err(json_err)?;
    let (output, report) = crate::text_reflow::apply_reflow_region(&input, &request)?;
    Ok((output, envelope("text_reflow_reflow_region", &report)?))
}

/// Apply a supported SemanticDocument reflow boundary through the canonical path.
pub fn text_reflow_reflow_document_json(
    bytes: &[u8],
    request_json: &str,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let input = mutation_input(bytes, password)?;
    let request = serde_json::from_str::<crate::text_reflow::GeometricReflowRequest>(request_json)
        .map_err(json_err)?;
    let (output, report) = crate::text_reflow::apply_reflow_document(&input, &request)?;
    Ok((output, envelope("text_reflow_reflow_document", &report)?))
}

/// Execute text reflow's canonical inverse operation against an explicit output
/// byte buffer. The engine replays the requested operation from this document,
/// rejects stale/non-deterministic output, then performs the transaction undo
/// and returns owned restored bytes plus typed proof.
pub fn text_reflow_undo_reflow_json(
    bytes: &[u8],
    output: &[u8],
    request_json: &str,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let input = mutation_input(bytes, password)?;
    let request = serde_json::from_str::<crate::text_reflow::GeometricReflowRequest>(request_json)
        .map_err(json_err)?;
    let (restored, report) = crate::text_reflow::undo_reflow_from_replay(&input, output, &request)?;
    Ok((restored, envelope("text_reflow_undo_reflow", &report)?))
}

/// Store/preview a user correction to inferred semantic structure.
pub fn text_reflow_reflow_approve_structure_json(
    bytes: &[u8],
    correction_json: &str,
    password: Option<&[u8]>,
) -> Result<String> {
    let input = mutation_input(bytes, password)?;
    envelope(
        "text_reflow_reflow_approve_structure",
        &crate::text_reflow::approve_structure_correction(&input, correction_json)?,
    )
}

/// Report text reflow transaction/undo/redo policy for a reflow request.
pub fn text_reflow_reflow_operation_report_json(
    bytes: &[u8],
    request_json: &str,
    password: Option<&[u8]>,
) -> Result<String> {
    let input = mutation_input(bytes, password)?;
    let request = serde_json::from_str::<crate::text_reflow::GeometricReflowRequest>(request_json)
        .map_err(json_err)?;
    envelope(
        "text_reflow_reflow_operation_report",
        &crate::text_reflow::transaction_undo_report(&input, &request)?,
    )
}

/// Report no-overlay/no-clipping evidence for a text reflow reflow request.
pub fn text_reflow_no_overlay_no_clipping_json(
    bytes: &[u8],
    request_json: &str,
    password: Option<&[u8]>,
) -> Result<String> {
    let input = mutation_input(bytes, password)?;
    let request = serde_json::from_str::<crate::text_reflow::GeometricReflowRequest>(request_json)
        .map_err(json_err)?;
    envelope(
        "text_reflow_no_overlay_no_clipping",
        &crate::text_reflow::no_overlay_no_clipping_report(&input, &request)?,
    )
}

/// document subsystems feature and supported-boundary report.
pub fn document_subsystems_report_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let input = mutation_input(bytes, password)?;
    envelope(
        "document_subsystems_report",
        &serde_json::json!({
            "feature_matrix": crate::document_subsystems::document_subsystems_feature_matrix(),
            "source_bytes": input.len()
        }),
    )
}

/// Analyze document subsystems source-linked tables, math, OCR, annotations, forms, and XFA.
pub fn document_subsystems_analyze_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let input = mutation_input(bytes, password)?;
    envelope(
        "document_subsystems_analyze",
        &crate::document_subsystems::analyze_document_subsystems(&input)?,
    )
}

/// Plan a supported document subsystems operation without mutating bytes.
pub fn document_subsystems_plan_json(
    bytes: &[u8],
    request_json: &str,
    password: Option<&[u8]>,
) -> Result<String> {
    let input = mutation_input(bytes, password)?;
    let request =
        serde_json::from_str::<crate::document_subsystems::DocumentSubsystemsRequest>(request_json)
            .map_err(json_err)?;
    envelope(
        "document_subsystems_plan",
        &crate::document_subsystems::plan_document_subsystems(&input, &request)?,
    )
}

/// Apply one supported document subsystems operation through the canonical writer.
pub fn document_subsystems_apply_json(
    bytes: &[u8],
    request_json: &str,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let input = mutation_input(bytes, password)?;
    let request =
        serde_json::from_str::<crate::document_subsystems::DocumentSubsystemsRequest>(request_json)
            .map_err(json_err)?;
    let (output, report) = crate::document_subsystems::apply_document_subsystems(&input, &request)?;
    Ok((output, envelope("document_subsystems_apply", &report)?))
}

/// Restore the immutable document subsystems transaction preimage after output verification.
pub fn document_subsystems_undo_json(
    bytes: &[u8],
    output: &[u8],
    request_json: &str,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let input = mutation_input(bytes, password)?;
    let request =
        serde_json::from_str::<crate::document_subsystems::DocumentSubsystemsRequest>(request_json)
            .map_err(json_err)?;
    let (restored, report) =
        crate::document_subsystems::undo_document_subsystems(&input, output, &request)?;
    Ok((restored, envelope("document_subsystems_undo", &report)?))
}

/// document security tagged-PDF accessibility, redaction, sanitization, and residual
/// verification feature report.
pub fn document_security_report_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let input = mutation_input(bytes, password)?;
    envelope(
        "document_security_report",
        &serde_json::json!({
            "feature_matrix": crate::document_security::document_security_feature_matrix(),
            "source_bytes": input.len()
        }),
    )
}

/// Analyze document security structure, accessibility, redaction, and sanitizer state.
pub fn document_security_analyze_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let input = mutation_input(bytes, password)?;
    envelope(
        "document_security_analyze",
        &crate::document_security::analyze_document_security(&input)?,
    )
}

/// Plan a typed document security operation without mutating bytes.
pub fn document_security_plan_json(
    bytes: &[u8],
    request_json: &str,
    password: Option<&[u8]>,
) -> Result<String> {
    let input = mutation_input(bytes, password)?;
    let request =
        serde_json::from_str::<crate::document_security::DocumentSecurityRequest>(request_json)
            .map_err(json_err)?;
    envelope(
        "document_security_plan",
        &crate::document_security::plan_document_security(&input, &request)?,
    )
}

/// Apply one supported document security operation through the canonical writer.
pub fn document_security_apply_json(
    bytes: &[u8],
    request_json: &str,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let input = mutation_input(bytes, password)?;
    let request =
        serde_json::from_str::<crate::document_security::DocumentSecurityRequest>(request_json)
            .map_err(json_err)?;
    let (output, report) = crate::document_security::apply_document_security(&input, &request)?;
    Ok((output, envelope("document_security_apply", &report)?))
}

/// Restore the immutable document security transaction preimage after output verification.
pub fn document_security_undo_json(
    bytes: &[u8],
    output: &[u8],
    request_json: &str,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let input = mutation_input(bytes, password)?;
    let request =
        serde_json::from_str::<crate::document_security::DocumentSecurityRequest>(request_json)
            .map_err(json_err)?;
    let (restored, report) =
        crate::document_security::undo_document_security(&input, output, &request)?;
    Ok((restored, envelope("document_security_undo", &report)?))
}

/// Run document security residual-data verification without mutating the document.
pub fn document_security_verify_residual_json(
    bytes: &[u8],
    terms_json: &str,
    password: Option<&[u8]>,
) -> Result<String> {
    let input = mutation_input(bytes, password)?;
    let terms = serde_json::from_str::<Vec<String>>(terms_json).map_err(json_err)?;
    envelope(
        "document_security_verify_residual",
        &crate::document_security::verify_residual_data(&input, &terms)?,
    )
}

/// secure mutation mask/soft-mask inventory and secure fallback posture.
pub fn mask_redaction_report_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let engine = open(bytes, password)?;
    envelope(
        "mask_redaction_report",
        &crate::secure_mutation::mask_redaction_inventory(&engine)?,
    )
}

/// Inventory all supported embedded/associated-file locations.
pub fn associated_files_report_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let engine = open(bytes, password)?;
    envelope(
        "associated_files_report",
        &crate::secure_mutation::associated_files_inventory(&engine)?,
    )
}

/// Structural and cryptographic signature-impact analysis for an edit class.
pub fn edit_policy_report_json(
    bytes: &[u8],
    operation: &str,
    password: Option<&[u8]>,
) -> Result<String> {
    let engine = open(bytes, password)?;
    let operation = parse_secure_mutation_edit_operation(operation)?;
    envelope(
        "edit_policy_report",
        &crate::secure_mutation::analyze_edit_policy(&engine, operation)?,
    )
}

pub fn edit_signature_impact_json(
    bytes: &[u8],
    operation: &str,
    password: Option<&[u8]>,
) -> Result<String> {
    edit_policy_report_json(bytes, operation, password)
}

/// Page-operations report: page boxes, labels/outlines/destinations, and
/// page-operation preservation risks.
pub fn page_operations_report_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let engine = open(bytes, password)?;
    envelope("page_operations_report", &page_operations_report(&engine)?)
}

/// Signature report (pdfsig-equivalent): validity, trust, coverage, LTV, and
/// certificate details for every signature in the document.
pub fn signature_report_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let engine = open(bytes, password)?;
    envelope("signature_report", &engine.verify_signatures()?)
}

/// Signature Validation signature report with explicit trust/evidence options.
///
/// `options_json` is parsed by [`crate::signature::verify_options_from_json`].
/// The report remains deterministic for the same bytes, options, trust anchors,
/// evidence, and validation time. Network retrieval remains disabled unless the
/// options explicitly enable the bounded shared retrieval policy.
pub fn signature_report_with_options_json(
    bytes: &[u8],
    options_json: &str,
    password: Option<&[u8]>,
) -> Result<String> {
    let engine = open(bytes, password)?;
    let options = crate::signature::verify_options_from_json(options_json)?;
    envelope(
        "signature_report",
        &engine.verify_signatures_with_options(&options)?,
    )
}

/// Signature Validation signature validation plus an explicit, portable evidence bundle.
///
/// This is deliberately separate from [`signature_report_with_options_json`]:
/// normal reports expose only evidence hashes and provenance, while this
/// opt-in surface includes the bounded raw DER needed for offline replay.
pub fn signature_validation_with_evidence_json(
    bytes: &[u8],
    options_json: &str,
    password: Option<&[u8]>,
) -> Result<String> {
    let engine = open(bytes, password)?;
    let options = crate::signature::verify_options_from_json(options_json)?;
    envelope(
        "signature_validation_outcome",
        &engine.verify_signatures_with_options_and_evidence(&options)?,
    )
}

/// Pades LTV RFC 3161 signature timestamp-token validation.
///
/// `signature_value` is the exact CMS `SignerInfo.signature` octet string the
/// timestamp token claims to bind through TSTInfo.messageImprint. The token is
/// not considered valid merely because it parses or is signed.
pub fn timestamp_token_validation_json(
    token_der: &[u8],
    signature_value: &[u8],
    options_json: &str,
) -> Result<String> {
    let options = crate::signature::verify_options_from_json(options_json)?;
    envelope(
        "timestamp_token_validation",
        &crate::signature::verify_signature_timestamp_token_der(
            token_der,
            signature_value,
            &options,
        )?,
    )
}

/// Font inventory (pdffonts-equivalent): name, type, embedding status,
/// subsetting, encoding.
pub fn font_report_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let engine = open(bytes, password)?;
    envelope("font_report", &engine.list_fonts()?)
}

/// Decode budget report for a hypothetical image stream. This surfaces the
/// engine's decode-limit / decompression-bomb policy without needing an actual
/// oversized stream: pass the declared `filter`, `width`, `height`, and
/// `components` and receive the diagnostics that decoding it would produce.
pub fn decode_budget_report_json(
    filter: &str,
    width: u32,
    height: u32,
    components: u8,
) -> Result<String> {
    let report =
        decode_image_budget_report(filter, width, height, components, &DecodeLimits::default());
    envelope("decode_budget_report", &report)
}

/// Codec isolation report for a caller-supplied stream payload. `policy` is one
/// of `in_process`, `isolated_preferred`, `isolated_required`, `report_only`,
/// or `disabled`. The report names whether a subprocess worker was used,
/// failed closed, or fell back by explicit policy.
pub fn codec_isolation_report_json(
    filter: &str,
    input: &[u8],
    policy: Option<&str>,
) -> Result<String> {
    let config = CodecIsolationConfig::from_policy_str(policy)?;
    let decoded = decode_filter_with_isolation(filter, input, &config);
    envelope("codec_isolation_report", &decoded.report)
}

/// Resource-dedup report over caller-supplied resource byte buffers. Groups
/// byte-identical resources by content digest — the writer's dedup evidence.
pub fn resource_dedup_report_json(resources: &[Vec<u8>]) -> Result<String> {
    envelope("resource_dedup_report", &resource_dedup_report(resources))
}

/// Text semantic model: pages → blocks → paragraphs → lines → words/spans with
/// geometry, confidence, provenance, and reading order.
pub fn text_semantic_json(
    bytes: &[u8],
    pages: &[usize],
    password: Option<&[u8]>,
) -> Result<String> {
    let engine = open(bytes, password)?;
    let model = engine.extract_text_semantic_model(pages, TextSemanticOptions::default())?;
    envelope("text_semantic", &model)
}

/// RAG-ready semantic chunks of the document (canonical model → chunk set).
pub fn chunk_report_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let engine = open(bytes, password)?;
    let document = engine.parse_document(&crate::ParseOptions::default())?;
    let set = document.chunk(&crate::ChunkOptions::default());
    envelope("chunk_set", &set)
}

/// Semantic Closeout provenance-aware RAG chunks. This is additive to `chunk_set` and
/// includes stable hashes, source spans, table/cell ids, CJK token metadata,
/// ParentTree status, and security posture.
pub fn advanced_chunk_report_json(
    bytes: &[u8],
    pages: &[usize],
    password: Option<&[u8]>,
) -> Result<String> {
    let engine = open(bytes, password)?;
    let report = engine.semantic_binding_report(&crate::SemanticBindingOptions {
        pages: pages.to_vec(),
        ..crate::SemanticBindingOptions::default()
    })?;
    envelope("advanced_rag_chunk_set", &report.rag_chunks)
}

/// Semantic Closeout full semantic binding bundle. All bindings consume this same
/// versioned JSON shape instead of duplicating deep object ownership graphs.
pub fn semantic_binding_report_json(
    bytes: &[u8],
    pages: &[usize],
    password: Option<&[u8]>,
) -> Result<String> {
    let engine = open(bytes, password)?;
    let report = engine.semantic_binding_report(&crate::SemanticBindingOptions {
        pages: pages.to_vec(),
        ..crate::SemanticBindingOptions::default()
    })?;
    envelope("semantic_binding_report", &report)
}

/// Provenance-aware semantic + dictionary-token search. Raw semantic text
/// search remains the fallback when a dictionary has no matching token.
pub fn semantic_search_report_json(
    bytes: &[u8],
    pages: &[usize],
    query: &str,
    password: Option<&[u8]>,
) -> Result<String> {
    let engine = open(bytes, password)?;
    let report = engine.semantic_search_report(pages, query, None)?;
    envelope("semantic_search_report", &report)
}

/// TableFormer/Table Transformer hook and backend availability status. This
/// call loads no model and performs no network request.
pub fn table_proposal_status_json() -> Result<String> {
    envelope(
        "table_proposal_status",
        &crate::table_model_backend_status_report(),
    )
}

/// Tagged-structure semantic document (structure tree / MCID model, if present).
pub fn semantic_document_json(
    bytes: &[u8],
    pages: &[usize],
    password: Option<&[u8]>,
) -> Result<String> {
    let engine = open(bytes, password)?;
    envelope(
        "semantic_document",
        &engine.extract_semantic_document(pages)?,
    )
}

/// Feature / capability report: SDK version, envelope version, and which
/// optional engine capabilities are compiled into this build. Bindings expose
/// this so integrators can query availability instead of guessing.
pub fn annotation_ocg_rendering_renderer_report_json() -> Result<String> {
    envelope(
        "annotation_ocg_rendering_renderer_report",
        &annotation_ocg_rendering_renderer_report_value(),
    )
}

pub fn renderer_validation_validation_report_json() -> Result<String> {
    envelope(
        "renderer_validation_validation_report",
        &renderer_validation_validation_report_value(),
    )
}

pub fn multilingual_color_glyphs_renderer_report_json() -> Result<String> {
    envelope(
        "multilingual_color_glyphs_renderer_report",
        &multilingual_color_glyphs_renderer_report_value(),
    )
}

pub fn cjk_rtl_color_glyph_closeout_closure_report_json() -> Result<String> {
    envelope(
        "cjk_rtl_color_glyph_closeout_closure_report",
        &cjk_rtl_color_glyph_closeout_closure_report_value(),
    )
}

pub fn color_glyph_hinting_closure_report_json() -> Result<String> {
    envelope(
        "color_glyph_hinting_closure_report",
        &color_glyph_hinting_closure_report_value(),
    )
}

pub fn colrv_svg_bitmap_closure_report_json() -> Result<String> {
    envelope(
        "colrv_svg_bitmap_closure_report",
        &colrv_svg_bitmap_closure_report_value(),
    )
}

pub fn colrv_gradient_composite_closure_report_json() -> Result<String> {
    envelope(
        "colrv_gradient_composite_closure_report",
        &colrv_gradient_composite_closure_report_value(),
    )
}

pub fn porterduff_radial_color_glyph_closure_report_json() -> Result<String> {
    envelope(
        "porterduff_radial_color_glyph_closure_report",
        &porterduff_radial_color_glyph_closure_report_value(),
    )
}

fn annotation_ocg_rendering_renderer_report_value() -> serde_json::Value {
    json!({
        "status": "implemented_with_bounded_unsupported_reports",
        "artifact_root": "target/annotation_ocg_rendering-annotation-ocg-progressive-cache",
        "audit_doc": "docs/annotation_ocg_rendering_annotation_ocg_progressive_cache_audit.md",
        "known_limits_doc": "docs/annotation_ocg_rendering_known_limits.md",
        "audit_script": "scripts/annotation_ocg_rendering_annotation_ocg_progressive_cache_audit.py",
        "reference_policy": {
            "reference_engines": ["Poppler", "PDFium", "MuPDF"],
            "bootstrap_source": "Reference Renderer reference-tool manifest and bootstrap scripts",
            "missing_reference_policy": "affected rows are partial unless target-local bootstrap proves unavailable"
        },
        "annotation_rendering": {
            "status": "widget_ap_native_form_path_with_subtype_taxonomy",
            "implemented": [
                "widget_AP_N_streams",
                "widget_AP_N_state_dictionary_selection_via_AS",
                "widget_text_button_choice_synthesis_when_NeedAppearances_requires_bounded_generation",
                "annotation_hidden_and_no_view_flags",
                "annotation_OC_visibility",
                "annotation_AP_Form_resources_BBox_Matrix_transparency_group_soft_mask_pattern_shading_replay",
                "annotation_page_rotation_rect_mapping",
                "malformed_AP_fail_closed_without_panic"
            ],
            "unsupported_reported": [
                "generated_non_widget_Text_icons",
                "generated_FreeText_layout",
                "generated_Line_PolyLine_Square_Circle_Polygon_Ink_markup_and_stamp_shapes",
                "caret_file_attachment_sound_movie_rich_media_playback",
                "dynamic_XFA"
            ],
            "matrix_artifact": "target/annotation_ocg_rendering-annotation-ocg-progressive-cache/annotation-matrix.json"
        },
        "optional_content": {
            "status": "default_view_configuration_evaluator",
            "implemented": [
                "catalog_OCProperties_discovery",
                "OCG_inventory",
                "default_configuration",
                "BaseState_ON_OFF_arrays",
                "Intent_matching",
                "Usage_View_state",
                "RBGroups_and_Order_and_Locked_metadata_reporting",
                "OCMD_AnyOn_AllOn_AnyOff_AllOff",
                "marked_content_visibility_stack",
                "XObject_OC_visibility",
                "annotation_OC_visibility",
                "pattern_and_shading_OC_visibility",
                "OCG_visibility_fingerprint_for_cache_keys"
            ],
            "unsupported_reported": [
                "alternate_configuration_selection_public_option",
                "Usage_Print_Export_active_mode_selection",
                "malformed_or_cyclic_OCG_references_fail_open_with_diagnostic"
            ],
            "matrix_artifact": "target/annotation_ocg_rendering-annotation-ocg-progressive-cache/ocg-layer-matrix.json"
        },
        "progressive_render": {
            "status": "engine_tile_checkpoint_resume_model",
            "granularity": ["tile"],
            "implemented": [
                "ProgressiveRenderJob",
                "ProgressiveRenderToken",
                "tile_level_progress",
                "cancelled_resumable_step_reports",
                "partial_surface_preservation_in_process",
                "full_vs_progressive_equivalence_tests",
                "page_box_rotation_render_mode_and_OCG_visibility_fingerprint_in_token"
            ],
            "binding_limit": "Rust engine surface is available; callback-style Python/C/WASM/.NET/Java cancellation/progress tokens remain later binding work",
            "matrix_artifact": "target/annotation_ocg_rendering-annotation-ocg-progressive-cache/progressive-render-matrix.json"
        },
        "tile_band_cache_performance": {
            "status": "deterministic_compatibility_safe_tile_band_cache_path",
            "implemented": [
                "tile_scheduler_full_page_crop_equivalence",
                "band_renderer_vertical_band_equivalence",
                "byte_budgeted_LRU_render_tile_cache",
                "deterministic_cache_metrics",
                "OCG_visibility_fingerprint_in_cache_key",
                "memory_budget_eviction",
                "large_page_fail_closed_via_render_pixel_cap"
            ],
            "unsupported_reported": [
                "global_image_Form_pattern_shading_clip_mask_surface_caches_beyond_tile_cache",
                "parallel_tile_renderer_enabled_by_default"
            ],
            "matrix_artifact": "target/annotation_ocg_rendering-annotation-ocg-progressive-cache/cache-performance-matrix.json"
        },
        "closure_gates": {
            "wellfriendpdf_outlier_failures": 0,
            "unclassified_failures": 0,
            "memory_cap_mb": 4096,
            "public_report_schema": "additive_feature_report_annotation_ocg_rendering"
        }
    })
}

fn renderer_validation_validation_report_value() -> serde_json::Value {
    json!({
        "status": "implemented_and_proven",
        "artifact_root": "target/annotation_ocg_rendering-annotation-ocg-progressive-cache",
        "audit_doc": "docs/renderer_validation_validation_closure_audit.md",
        "audit_script": "scripts/renderer_validation_validation_closure_audit.py",
        "annotation_parity": {
            "status": "matrix_proven_with_bounded_non_widget_policy",
            "subtype_style_rows": 25,
            "native_rendered": 1,
            "appearance_stream_rendered": 4,
            "generated_appearance_rendered": 4,
            "policy_reported_not_rendered": 8,
            "unsupported_reported": 8,
            "matrix_artifact": "target/annotation_ocg_rendering-annotation-ocg-progressive-cache/annotation-appearance-matrix-renderer_validation.json"
        },
        "ocg_validation": {
            "status": "default_view_ocg_ocmd_visibility_proven",
            "marked_content": "proven",
            "xobjects": "proven",
            "annotations": "proven",
            "patterns_shadings": "proven",
            "cache_fingerprint": "proven",
            "matrix_artifact": "target/annotation_ocg_rendering-annotation-ocg-progressive-cache/ocg-layer-matrix-renderer_validation.json",
            "cache_fingerprint_artifact": "target/annotation_ocg_rendering-annotation-ocg-progressive-cache/ocg-cache-key-fingerprint-renderer_validation.json"
        },
        "progressive_resume_equivalence": {
            "status": "full_vs_resumed_equivalent",
            "granularity": "tile",
            "invalid_token_handling": "mismatched page/DPI/render_mode/tile_geometry/cursor/OCG_fingerprint rejected",
            "artifact": "target/annotation_ocg_rendering-annotation-ocg-progressive-cache/progressive-resume-equivalence-renderer_validation.json"
        },
        "tile_band_cache_equivalence": {
            "tile_full": "proven",
            "band_full": "proven",
            "cache_no_cache": "proven",
            "performance_artifact": "target/annotation_ocg_rendering-annotation-ocg-progressive-cache/tile-band-cache-performance-renderer_validation.json",
            "memory_artifact": "target/annotation_ocg_rendering-annotation-ocg-progressive-cache/tile-band-cache-memory-renderer_validation.json"
        },
        "multi_reference_audit": {
            "status": "renderer_validation_corpus_classified",
            "reference_engines": ["Poppler", "PDFium", "MuPDF"],
            "artifact": "target/annotation_ocg_rendering-annotation-ocg-progressive-cache/multi-reference-render-results-renderer_validation.json",
            "diff_metrics": "target/annotation_ocg_rendering-annotation-ocg-progressive-cache/multi-reference-diff-metrics-renderer_validation.json",
            "wellfriendpdf_outlier_failures": 0,
            "unclassified_failures": 0
        },
        "public_report_parity": {
            "schema_change": "additive_section_only",
            "report_envelope_version": REPORT_ENVELOPE_VERSION,
            "bindings": ["Rust SDK", "CLI", "Python", "C ABI", "WASM", ".NET", "Java Maven", "Java Gradle"]
        },
        "remaining_bounded_limits": [
            "non_widget_generated_annotation_shapes remain policy-reported unless an author AP stream exists",
            "alternate OCG configuration selection remains parsed/report-only without public selection API",
            "binding-level progressive callbacks remain later binding work",
            "global image/Form/pattern/shading resource caches remain outside Annotation Ocg Rendering tile-cache closure"
        ]
    })
}

fn multilingual_color_glyphs_renderer_report_value() -> serde_json::Value {
    json!({
        "status": "implemented_with_bounded_unsupported_reports",
        "artifact_root": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference",
        "audit_doc": "docs/multilingual_color_glyphs_cjk_rtl_color_glyph_reference_harness.md",
        "audit_script": "scripts/multilingual_color_glyphs_cjk_rtl_color_glyph_reference_harness.py",
        "reference_policy": {
            "reference_engines": ["Poppler", "PDFium", "MuPDF"],
            "bootstrap_source": "scripts/reference_renderer_bootstrap_reference_renderers.ps1 reused with Multilingual Color Glyphs artifact manifests",
            "missing_reference_policy": "Multilingual Color Glyphs bootstrap fails the direct audit unless all three reference renderers are available"
        },
        "cjk_raster_hinting": {
            "status": "direct_corpus_audit_with_existing_pdf_glyph_painting_boundary",
            "fixture_categories": [
                "simplified_chinese",
                "traditional_chinese_or_cjk_variant",
                "japanese_horizontal",
                "japanese_vertical",
                "mixed_latin_cjk",
                "type0_cid",
                "identity_h",
                "identity_v",
                "cid_to_gid",
                "missing_tounicode",
                "malformed_cmap"
            ],
            "rendering_boundary": "visual glyph painting maps PDF character codes through CMap/CID/GID/font data and stays independent from ToUnicode extraction",
            "hinting_posture": "pure-rust analytic/light grid-fitting raster path; no new native hinting dependency is enabled by default",
            "diagnostics": [
                "font.type0.descendant_missing",
                "font.cmap.predefined.unsupported",
                "font.cmap.identity",
                "font.cmap.vertical",
                "font.tounicode.missing_type0"
            ]
        },
        "rtl_raster_shaping": {
            "status": "generated_text_complex_shaping_boundary_documented",
            "fixture_categories": ["arabic", "hebrew", "mixed_bidi", "pre_shaped_pdf_text", "rtl_annotation_appearance"],
            "painting_boundary": "existing PDF content streams preserve encoded glyph order and PDF text-state positioning; Wellfriend does not blindly reshape painted PDF glyph streams",
            "generated_text_boundary": "rustybuzz is used for generated/fallback text paths that own Unicode-to-glyph layout",
            "shaped_scripts": ["Arabic", "Hebrew", "Indic complex-script families"]
        },
        "color_glyph_rendering": {
            "status": "unsupported_color_tables_are_detected_and_reported",
            "implemented_formats": [],
            "unsupported_reported": [
                "COLR/CPAL v0 layered glyphs",
                "COLR/CPAL v1 paint graph glyphs",
                "CBDT/CBLC bitmap strikes",
                "sbix bitmap strikes",
                "SVG-in-OpenType static or scripted glyph documents"
            ],
            "report_fields": [
                "color_font_tables",
                "color_glyph_status",
                "color_glyph_supported_tables",
                "color_glyph_unsupported_tables",
                "diagnostics"
            ],
            "security_boundary": "SVG-in-OpenType is not executed and external references are not dereferenced"
        },
        "pdfium_direct_harness": {
            "status": "target_local_direct_renderer",
            "wrapper_choices": ["pdfium_test when configured", "target-local pypdfium2 wrapper fallback"],
            "version_checksum_manifest": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/reference-tool-manifest-multilingual_color_glyphs.json",
            "command_normalization": ["PNG output", "explicit page range", "explicit DPI", "white background/form drawing where wrapper supports it"]
        },
        "mupdf_direct_harness": {
            "status": "target_local_direct_renderer",
            "wrapper_choices": ["mutool draw"],
            "version_checksum_manifest": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/reference-tool-manifest-multilingual_color_glyphs.json",
            "command_normalization": ["PNG output by extension", "explicit page", "explicit DPI", "target-local checksum posture"]
        },
        "multi_reference_audit": {
            "status": "multilingual_color_glyphs_corpus_classified_by_direct_harness",
            "corpus_manifest": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/corpus-manifest-multilingual_color_glyphs.json",
            "render_results": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/multi-reference-render-results-multilingual_color_glyphs.json",
            "diff_metrics": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/multi-reference-diff-metrics-multilingual_color_glyphs.json",
            "reference_disagreement_summary": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/reference-disagreement-summary-multilingual_color_glyphs.json",
            "html_report": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/html-report/index.html"
        },
        "public_report_parity": {
            "schema_change": "additive_section_only",
            "report_envelope_version": REPORT_ENVELOPE_VERSION,
            "bindings": ["Rust SDK", "CLI", "Python", "C ABI", "WASM", ".NET", "Java Maven", "Java Gradle"]
        },
        "closure_gates": {
            "memory_cap_mb": 4096,
            "public_report_schema": "additive_feature_report_multilingual_color_glyphs"
        },
        "remaining_bounded_limits": [
            "color glyph tables are exposed as precise unsupported diagnostics rather than rendered color layers",
            "complex CID-keyed CFF geometry under real-world text clipping is policy-reported when it falls outside the reference cluster",
            "existing PDF content-stream glyph painting is not reshaped as authoring text",
            "native hinting backends remain outside the default dependency boundary"
        ]
    })
}

fn cjk_rtl_color_glyph_closeout_closure_report_value() -> serde_json::Value {
    json!({
        "status": "complete",
        "artifact_root": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference",
        "audit_doc": "docs/cjk_rtl_color_glyph_closeout_color_glyph_cjk_rtl_closure_audit.md",
        "audit_script": "scripts/cjk_rtl_color_glyph_closeout_color_glyph_cjk_rtl_closure.py",
        "closure_audit": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/cjk_rtl_color_glyph_closeout-closure-audit.json",
        "color_glyph_rendering": {
            "status": "implemented_with_precise_security_and_exotic_limits",
            "colr_cpal": {
                "status": "implemented_and_proven",
                "supported": ["COLR/CPAL v0 solid layered glyphs", "palette 0", "graphics alpha", "text transform", "text clipping outline", "Form XObject transparency group"],
                "artifact": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/color-glyph-colr-cpal-matrix-cjk_rtl_color_glyph_closeout.json",
                "remaining_limits": ["COLRv1 gradients/transforms/compositing remain unsupported_reported_exotic_case"]
            },
            "cbdt_cblc": {
                "status": "implemented_and_proven_shared_raster_branch",
                "supported": ["CBDT/CBLC PNG and bounded bitmap strikes through ttf-parser RasterGlyphImage"],
                "artifact": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/color-glyph-cbdt-cblc-matrix-cjk_rtl_color_glyph_closeout.json",
                "remaining_limits": ["malformed, incomplete, oversized, or unavailable CBDT/CBLC payloads fail closed"]
            },
            "sbix": {
                "status": "implemented_and_proven",
                "supported": ["sbix PNG strikes", "origin offsets", "scaling", "graphics alpha"],
                "artifact": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/color-glyph-sbix-matrix-cjk_rtl_color_glyph_closeout.json",
                "remaining_limits": ["sbix JPEG/TIFF/PDF/mask payloads remain unsupported_reported_exotic_case"]
            },
            "svg_opentype": {
                "status": "unsupported_reported_security_policy",
                "blocked": ["script", "event attributes", "external references", "remote resources", "foreignObject", "animation", "network"],
                "artifact": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/color-glyph-svg-opentype-policy-cjk_rtl_color_glyph_closeout.json"
            }
        },
        "cjk_rtl_fixture_fidelity": {
            "korean": {
                "status": "implemented_and_proven",
                "coverage": ["embedded Korean font", "Hangul syllables", "compatibility jamo", "Identity-H glyph painting", "ToUnicode-independent rendering"],
                "artifact": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/korean-render-fixture-matrix-cjk_rtl_color_glyph_closeout.json"
            },
            "hebrew": {
                "status": "implemented_and_proven",
                "coverage": ["embedded Hebrew font", "explicit positioned RTL visual order", "PDF glyph painting separated from generated rustybuzz shaping"],
                "artifact": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/hebrew-render-fixture-matrix-cjk_rtl_color_glyph_closeout.json"
            }
        },
        "cid_keyed_cff_clipping": {
            "status": "unsupported_reported_exotic_case",
            "supported_path": "CID-keyed CFF glyph outlines clip when the font subsystem exposes real charstring path geometry",
            "unsupported_policy": "advanced CID-keyed CFF clipping geometry remains unsupported only when real charstring path geometry is unavailable or outside the reference cluster; no bbox fake clipping",
            "artifact": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/cid-keyed-cff-clipping-matrix-cjk_rtl_color_glyph_closeout.json"
        },
        "hinting_posture": {
            "status": "pure_rust_reference_cluster_accepted",
            "native_hinting": "not required and not added as a native dependency",
            "artifact": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/hinting-posture-cjk_rtl_color_glyph_closeout.json"
        },
        "multi_reference_audit": {
            "status": "cjk_rtl_color_glyph_closeout_corpus_classified",
            "fixture_count": 5,
            "reference_engines": ["Poppler", "PDFium", "MuPDF"],
            "render_results": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/cjk_rtl_color_glyph_closeout-multi-reference-render-results.json",
            "diff_metrics": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/cjk_rtl_color_glyph_closeout-multi-reference-diff-metrics.json",
            "reference_disagreement_summary": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/cjk_rtl_color_glyph_closeout-reference-disagreement-summary.json",
            "html_report": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/cjk_rtl_color_glyph_closeout-html-report/index.html",
            "wellfriendpdf_outlier_failures": 0,
            "unclassified_failures": 0,
            "reference_disagreements": ["sbix PNG reference disagreement with Wellfriend inside PDFium/MuPDF cluster"],
            "unsupported_rows": ["advanced CID-keyed CFF clipping geometry"]
        },
        "public_report_parity": {
            "schema_change": "additive_section_only",
            "report_envelope_version": REPORT_ENVELOPE_VERSION,
            "bindings": ["Rust SDK", "CLI", "Python", "C ABI", "WASM", ".NET", "Java Maven", "Java Gradle"]
        },
        "closure_gates": {
            "memory_cap_mb": 4096,
            "public_report_schema": "additive_feature_report_cjk_rtl_color_glyph_closeout",
            "wellfriendpdf_outlier_failures": 0,
            "unclassified_failures": 0
        },
        "remaining_bounded_limits": [
            "COLRv1 gradients/transforms/compositing are unsupported_reported_exotic_case",
            "SVG-in-OpenType remains blocked by security policy until a static no-network sanitizer is implemented",
            "sbix JPEG/TIFF/PDF/mask payloads are unsupported_reported_exotic_case",
            "advanced CID-keyed CFF clipping geometry remains unsupported only when real charstring path geometry is unavailable or outside the reference cluster",
            "native hinting is a future optional feature, not a CJK RTL Color Glyph Closeout blocker"
        ]
    })
}

fn color_glyph_hinting_svg_policy_samples() -> serde_json::Value {
    let samples = [
        (
            "safe_static_path",
            r##"<svg viewBox="0 0 64 64"><path d="M8 8 L56 8 L56 56 Z" fill="#ffcc00"/></svg>"##,
        ),
        (
            "blocked_script",
            r#"<svg><script>alert(1)</script><path d="M0 0 L1 1"/></svg>"#,
        ),
        (
            "blocked_external_reference",
            r#"<svg><image href="https://example.invalid/a.png"/></svg>"#,
        ),
        (
            "unsupported_static_use",
            r##"<svg><defs><path id="p" d="M0 0 L1 1"/></defs><use href="#p"/></svg>"##,
        ),
    ];
    let rows: Vec<_> = samples
        .iter()
        .map(|(id, svg)| {
            let policy = crate::render::color_glyph::classify_svg_glyph_document(svg);
            json!({
                "id": id,
                "status": policy.status(),
                "reason": policy.reason()
            })
        })
        .collect();
    json!(rows)
}

fn color_glyph_hinting_closure_report_value() -> serde_json::Value {
    json!({
        "status": "complete",
        "artifact_root": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference",
        "audit_doc": "docs/color_glyph_hinting_color_glyph_hinting_cff_closure_audit.md",
        "overview_doc": "docs/color_glyph_hinting_color_glyph_hinting_cff_closure.md",
        "audit_script": "scripts/color_glyph_hinting_color_glyph_hinting_cff_closure.py",
        "closure_audit": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/color_glyph_hinting-closure-audit.json",
        "colrv1": {
            "status": "implemented_with_operator_level_limits",
            "implemented_operators": [
                "PaintSolid",
                "PaintColrGlyph",
                "PaintTransform",
                "PaintTranslate",
                "PaintScale",
                "PaintRotate",
                "PaintSkew",
                "PaintComposite SourceOver"
            ],
            "unsupported_operators": [
                "PaintLinearGradient",
                "PaintRadialGradient",
                "PaintSweepGradient",
                "PaintClip",
                "PaintClipBox",
                "PaintComposite non-SourceOver"
            ],
            "safety_caps": {
                "paint_layer_cap": 256,
                "transform_depth_cap": 32,
                "parser_recursion_cap": 64,
                "finite_transform_required": true
            },
            "artifacts": {
                "matrix": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/color-glyph-colrv1-matrix-color_glyph_hinting.json",
                "reference_results": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/color-glyph-colrv1-reference-results-color_glyph_hinting.json"
            }
        },
        "svg_in_opentype": {
            "status": "static_subset_classified_with_security_blocking",
            "safe_static_subset": [
                "svg root viewBox",
                "g grouping",
                "path",
                "rect/circle/ellipse/line/polyline/polygon when reduced by future static renderer",
                "finite transform attributes after parser admission"
            ],
            "blocked_active_features": [
                "script",
                "event attributes",
                "network/file/javascript URLs",
                "foreignObject",
                "animation",
                "CSS import",
                "remote or embedded SVG fonts",
                "external image resources",
                "filters",
                "masks",
                "path/depth bombs"
            ],
            "classifier_samples": color_glyph_hinting_svg_policy_samples(),
            "artifacts": {
                "matrix": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/color-glyph-svg-static-subset-matrix-color_glyph_hinting.json",
                "security_policy": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/color-glyph-svg-security-policy-color_glyph_hinting.json",
                "reference_results": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/color-glyph-svg-reference-results-color_glyph_hinting.json"
            }
        },
        "bitmap_color_glyphs": {
            "cbdt_cblc": {
                "status": "png_and_bounded_bitmap_preserved_with_exact_non_png_policy",
                "supported": ["PNG RasterGlyphImage payloads", "bounded raw/grayscale/color bitmap strikes when ttf-parser exposes safe RasterGlyphImage metadata"],
                "unsupported_payloads": ["ambiguous compressed payloads", "malformed strike tables", "oversized dimensions", "invalid offsets or lengths", "mismatched glyph strike references"]
            },
            "sbix": {
                "status": "png_rendered_non_png_payloads_reported_by_tag",
                "supported": ["sbix PNG", "dupe references resolving to supported PNG"],
                "unsupported_payloads": ["sbix JPEG", "sbix TIFF", "sbix PDF", "sbix mask", "unknown sbix graphicType tags"]
            },
            "malformed_behavior": "fail_closed_without_monochrome_fallback_for_known_color_payloads",
            "artifacts": {
                "matrix": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/color-glyph-bitmap-payload-matrix-color_glyph_hinting.json",
                "cbdt_cblc_results": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/color-glyph-cbdt-cblc-results-color_glyph_hinting.json",
                "sbix_results": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/color-glyph-sbix-results-color_glyph_hinting.json"
            }
        },
        "hinting_posture": {
            "status": "pure_rust_reference_cluster_accepted",
            "native_backend": "not added; no silent native dependency and WASM/default builds stay portable",
            "backend_report_field": "pure_rust_analytic_aa",
            "artifact": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/hinting-posture-color_glyph_hinting.json"
        },
        "cid_keyed_cff_clipping": {
            "status": "narrow_exotic_policy_with_real_geometry_only",
            "implemented_geometry_path": [
                "encoded bytes to CMap/CID/GID mapping",
                "FDArray/FDSelect diagnostics",
                "subr bias and charstring depth diagnostics",
                "FontMatrix/text matrix/CTM/rise/scaling preserved when outlines are exposed"
            ],
            "unsupported_policy": "only missing or unsafe charstring geometry remains unsupported; bbox fake clipping is forbidden",
            "artifacts": {
                "matrix": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/cid-keyed-cff-clipping-matrix-color_glyph_hinting.json",
                "reference_results": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/cid-keyed-cff-clipping-reference-results-color_glyph_hinting.json"
            }
        },
        "multi_reference_audit": {
            "status": "color_glyph_hinting_corpus_classified",
            "reference_engines": ["Poppler", "PDFium", "MuPDF"],
            "fixture_count": 9,
            "render_results": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/multi-reference-render-results-color_glyph_hinting.json",
            "diff_metrics": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/multi-reference-diff-metrics-color_glyph_hinting.json",
            "reference_disagreement_summary": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/reference-disagreement-summary-color_glyph_hinting.json",
            "html_report": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/color_glyph_hinting-html-report/index.html",
            "wellfriendpdf_outlier_failures": 0,
            "unclassified_failures": 0
        },
        "public_report_parity": {
            "schema_change": "additive_section_only",
            "report_envelope_version": REPORT_ENVELOPE_VERSION,
            "bindings": ["Rust SDK", "CLI", "Python", "C ABI", "WASM", ".NET", "Java Maven", "Java Gradle"]
        },
        "closure_gates": {
            "memory_cap_mb": 4096,
            "public_report_schema": "additive_feature_report_color_glyph_hinting",
            "wellfriendpdf_outlier_failures": 0,
            "unclassified_failures": 0
        },
        "remaining_bounded_limits": [
            "COLRv1 gradient and clip paint operators are exact unsupported_reported_exotic_format rows until mapped to renderer shading/clip primitives",
            "SVG-in-OpenType safe static candidates are classified but not executed through a general SVG engine; active and external features remain security-blocked",
            "CBDT/CBLC non-PNG or ambiguous compressed payloads are exact unsupported_reported_exotic_format rows unless exposed as safe RasterGlyphImage metadata",
            "sbix JPEG/TIFF/PDF/mask and unknown graphicType payloads are exact unsupported_reported_exotic_format rows",
            "native hinting remains a future optional feature because pure-Rust output is accepted by the Color Glyph Hinting reference cluster",
            "CID-keyed CFF clipping fails closed only for missing or unsafe charstring geometry"
        ]
    })
}

fn colrv_svg_bitmap_svg_policy_samples() -> serde_json::Value {
    let samples = [
        (
            "safe_static_path",
            r##"<svg viewBox="0 0 1000 1000"><path d="M100 100 L900 100 L900 900 Z" fill="#ffcc00"/></svg>"##,
        ),
        (
            "safe_static_shape_opacity",
            r##"<svg viewBox="0 0 1000 1000"><g opacity="0.75"><circle cx="500" cy="500" r="300" fill="red"/></g></svg>"##,
        ),
        (
            "blocked_script",
            r#"<svg><script>alert(1)</script><path d="M0 0 L1 1"/></svg>"#,
        ),
        (
            "blocked_event",
            r#"<svg><path onload="alert(1)" d="M0 0 L1 1"/></svg>"#,
        ),
        (
            "blocked_external_reference",
            r#"<svg><image href="https://example.invalid/a.png"/></svg>"#,
        ),
        (
            "blocked_foreign_object",
            r#"<svg><foreignObject><body>blocked</body></foreignObject></svg>"#,
        ),
    ];
    let rows: Vec<_> = samples
        .iter()
        .map(|(id, svg)| {
            let policy = crate::render::color_glyph::classify_svg_glyph_document(svg);
            json!({
                "id": id,
                "status": policy.status(),
                "reason": policy.reason()
            })
        })
        .collect();
    json!(rows)
}

fn colrv_svg_bitmap_closure_report_value() -> serde_json::Value {
    json!({
        "status": "complete",
        "artifact_root": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference",
        "audit_doc": "docs/colrv_svg_bitmap_colrv1_svg_bitmap_closure_audit.md",
        "overview_doc": "docs/colrv_svg_bitmap_full_colrv1_svg_color_glyph_closure.md",
        "audit_script": "scripts/colrv_svg_bitmap_full_colrv1_svg_color_glyph_closure.py",
        "closure_audit": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/colrv_svg_bitmap-closure-audit.json",
        "colrv1_gradients": {
            "status": "unsupported_reported_exotic_operator",
            "implemented_operators": [],
            "unsupported_operators": [
                "PaintLinearGradient",
                "PaintRadialGradient",
                "PaintSweepGradient"
            ],
            "reason": "ttf-parser Painter callbacks expose gradient operators but not a bounded renderer paint tree/offscreen surface mapping; Colrv Svg Bitmap keeps operator-level fail-closed diagnostics instead of monochrome fallback",
            "artifacts": {
                "linear": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/colrv1-linear-gradient-matrix-colrv_svg_bitmap.json",
                "radial": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/colrv1-radial-gradient-matrix-colrv_svg_bitmap.json",
                "sweep": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/colrv1-sweep-gradient-matrix-colrv_svg_bitmap.json",
                "reference_results": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/colrv1-gradient-reference-results-colrv_svg_bitmap.json"
            }
        },
        "colrv1_clip": {
            "status": "unsupported_reported_exotic_operator",
            "unsupported_operators": ["PaintClip", "PaintClipBox"],
            "reason": "COLRv1 clip graphs need nested glyph-path clip stack execution; current safe collector reports exact operators and fails closed",
            "artifacts": {
                "matrix": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/colrv1-clip-matrix-colrv_svg_bitmap.json",
                "reference_results": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/colrv1-clip-reference-results-colrv_svg_bitmap.json"
            }
        },
        "colrv1_composite": {
            "status": "source_over_preserved_non_source_over_reported",
            "implemented_modes": ["SourceOver"],
            "unsupported_modes": [
                "Clear", "Source", "Destination", "DestinationOver", "SourceIn", "DestinationIn",
                "SourceOut", "DestinationOut", "SourceAtop", "DestinationAtop", "Xor", "Plus",
                "Screen", "Overlay", "Darken", "Lighten", "ColorDodge", "ColorBurn",
                "HardLight", "SoftLight", "Difference", "Exclusion", "Multiply", "Hue",
                "Saturation", "Color", "Luminosity"
            ],
            "reason": "non-SourceOver COLRv1 composites require isolated bounded paint surfaces in glyph space; existing Transparency Rendering blend modes are not invoked without that safe surface model",
            "artifacts": {
                "matrix": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/colrv1-composite-matrix-colrv_svg_bitmap.json",
                "reference_results": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/colrv1-composite-reference-results-colrv_svg_bitmap.json"
            }
        },
        "svg_in_opentype": {
            "status": "safe_static_subset_rendered_active_constructs_blocked",
            "safe_static_subset": [
                "svg root with viewBox metadata",
                "g grouping with inherited style/transform",
                "path M/L/H/V/C/Q/Z subset",
                "rect",
                "circle",
                "ellipse",
                "line",
                "polyline",
                "polygon",
                "quoted fill/stroke/stroke-width",
                "opacity/fill-opacity/stroke-opacity",
                "matrix/translate/scale/rotate/skew transforms with finite checks"
            ],
            "blocked_active_features": [
                "script",
                "event attributes",
                "animation",
                "foreignObject",
                "external images",
                "network/file/javascript URLs",
                "CSS style blocks and imports",
                "remote or embedded SVG fonts",
                "filters",
                "masks",
                "recursive use",
                "URL paint-server references",
                "path/depth bombs"
            ],
            "classifier_samples": colrv_svg_bitmap_svg_policy_samples(),
            "artifacts": {
                "matrix": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/svg-opentype-static-rendering-matrix-colrv_svg_bitmap.json",
                "security_policy": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/svg-opentype-security-policy-colrv_svg_bitmap.json",
                "reference_results": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/svg-opentype-reference-results-colrv_svg_bitmap.json"
            }
        },
        "bitmap_color_glyphs": {
            "cbdt_cblc": {
                "status": "png_raw_gray_color_strikes_preserved_exact_unsupported_for_ambiguous_payloads",
                "supported": ["PNG RasterGlyphImage payloads", "BitmapPremulBgra32", "BitmapGray8/4/2", "BitmapMono and packed variants when metadata is sufficient"],
                "unsupported_payloads": ["ambiguous compressed payloads not exposed as safe RasterGlyphImage metadata", "malformed strike tables", "oversized dimensions", "invalid offsets or lengths"]
            },
            "sbix": {
                "status": "png_and_jpeg_rendered_tiff_other_precisely_reported",
                "supported": ["sbix PNG", "sbix JPEG through bounded DCT decoder", "dupe references resolving to supported payloads"],
                "unsupported_payloads": ["sbix TIFF when no existing safe TIFF decoder is available", "sbix PDF", "sbix mask", "unknown graphicType tags"]
            },
            "malformed_behavior": "fail_closed_without_monochrome_fallback_for_known_color_payloads",
            "artifacts": {
                "matrix": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/bitmap-color-glyph-nonpng-matrix-colrv_svg_bitmap.json",
                "cbdt_cblc_results": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/cbdt-cblc-nonpng-results-colrv_svg_bitmap.json",
                "sbix_results": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/sbix-nonpng-results-colrv_svg_bitmap.json"
            }
        },
        "multi_reference_audit": {
            "status": "colrv_svg_bitmap_corpus_classified",
            "reference_engines": ["Poppler", "PDFium", "MuPDF"],
            "fixture_count": 19,
            "render_results": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/multi-reference-render-results-colrv_svg_bitmap.json",
            "diff_metrics": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/multi-reference-diff-metrics-colrv_svg_bitmap.json",
            "reference_disagreement_summary": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/reference-disagreement-summary-colrv_svg_bitmap.json",
            "html_report": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/colrv_svg_bitmap-html-report/index.html",
            "wellfriendpdf_outlier_failures": 0,
            "unclassified_failures": 0
        },
        "public_report_parity": {
            "schema_change": "additive_section_only",
            "report_envelope_version": REPORT_ENVELOPE_VERSION,
            "bindings": ["Rust SDK", "CLI", "Python", "C ABI", "WASM", ".NET", "Java Maven", "Java Gradle"]
        },
        "closure_gates": {
            "memory_cap_mb": 4096,
            "public_report_schema": "additive_feature_report_colrv_svg_bitmap",
            "wellfriendpdf_outlier_failures": 0,
            "unclassified_failures": 0
        },
        "remaining_bounded_limits": [
            "Colrv Svg Bitmap-era COLRv1 gradient limits are superseded by Colrv Gradient Composite gradient rendering and Porterduff Radial Color Glyph exact moving-center radial closure",
            "Colrv Svg Bitmap-era PaintClip/PaintClipBox limits are superseded by Colrv Gradient Composite glyph paint clip stack closure",
            "Colrv Svg Bitmap-era non-SourceOver composite limits are superseded by Colrv Gradient Composite blend composites and Porterduff Radial Color Glyph Porter-Duff/Plus closure",
            "SVG gradients, clipPath, filters, masks, use references, CSS blocks, external resources, and active constructs remain blocked or exact unsupported rows",
            "sbix TIFF/PDF/mask and unknown graphicType payloads remain exact unsupported rows when no existing safe decoder is available"
        ]
    })
}

fn colrv_gradient_composite_closure_report_value() -> serde_json::Value {
    json!({
        "status": "complete",
        "artifact_root": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference",
        "audit_doc": "docs/colrv_gradient_composite_colrv1_gradient_clip_composite_closure_audit.md",
        "overview_doc": "docs/colrv_gradient_composite_colrv1_gradient_clip_composite_closure.md",
        "audit_script": "scripts/colrv_gradient_composite_colrv1_gradient_clip_composite_closure.py",
        "closure_audit": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/colrv_gradient_composite-closure-audit.json",
        "colrv1_gradients": {
            "status": "implemented_with_limits",
            "implemented_operators": [
                "PaintLinearGradient",
                "PaintRadialGradient",
                "PaintSweepGradient"
            ],
            "extend_modes": ["pad", "repeat", "reflect"],
            "safety_caps": {
                "gradient_stop_cap": 16,
                "paint_layer_cap": 256,
                "transform_depth_cap": 32
            },
            "limits": [
                "linear gradients sample along the primary COLRv1 line with finite p2 validation",
                "radial gradients are exact for same-center circles and bounded for moving-center cases",
                "sweep gradients use deterministic angular interpolation in glyph paint space"
            ],
            "artifacts": {
                "matrix": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/colrv1-gradient-matrix-colrv_gradient_composite.json",
                "reference_results": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/colrv1-gradient-reference-results-colrv_gradient_composite.json",
                "limit_diagnostics": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/colrv1-gradient-limit-diagnostics-colrv_gradient_composite.json"
            }
        },
        "colrv1_clip_stack": {
            "status": "implemented",
            "implemented_operators": ["PaintClip", "PaintClipBox"],
            "behavior": [
                "glyph clips use real outline masks",
                "clip boxes use transformed rectangular clip masks",
                "nested clips intersect through the renderer clip stack",
                "clip applies to solids, gradients, nested glyph paints, and composites"
            ],
            "bbox_fake_clipping": false,
            "artifacts": {
                "matrix": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/colrv1-clip-stack-matrix-colrv_gradient_composite.json",
                "reference_results": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/colrv1-clip-reference-results-colrv_gradient_composite.json",
                "limit_diagnostics": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/colrv1-clip-limit-diagnostics-colrv_gradient_composite.json"
            }
        },
        "colrv1_composites": {
            "status": "implemented_with_exact_mode_limits",
            "implemented_modes": [
                "SourceOver",
                "Multiply",
                "Screen",
                "Overlay",
                "Darken",
                "Lighten",
                "ColorDodge",
                "ColorBurn",
                "HardLight",
                "SoftLight",
                "Difference",
                "Exclusion",
                "Hue",
                "Saturation",
                "Color",
                "Luminosity"
            ],
            "unsupported_modes": [
                "Clear",
                "Source",
                "Destination",
                "DestinationOver",
                "SourceIn",
                "DestinationIn",
                "SourceOut",
                "DestinationOut",
                "SourceAtop",
                "DestinationAtop",
                "Xor",
                "Plus"
            ],
            "superseded_by_porterduff_radial_color_glyph": true,
            "reason_for_unsupported_modes": "Porter-Duff and Plus modes require source/backdrop ownership semantics that are not equivalent to the existing Transparency Rendering PDF blend modes in the current glyph surface model",
            "artifacts": {
                "matrix": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/colrv1-composite-surface-matrix-colrv_gradient_composite.json",
                "reference_results": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/colrv1-composite-reference-results-colrv_gradient_composite.json",
                "limit_diagnostics": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/colrv1-composite-limit-diagnostics-colrv_gradient_composite.json"
            }
        },
        "glyph_paint_surface_model": {
            "status": "scheduler_bounded",
            "allocation": "renderer offscreen scheduler token",
            "pixel_format": "transparent PixelBuffer in active render mode",
            "tracked_state": [
                "palette",
                "alpha",
                "transform stack",
                "clip stack",
                "blend mode",
                "paint graph depth"
            ],
            "cache_scheduler": {
                "cache_key_posture": "color glyph mode plus font/glyph/palette/transform/clip/composite feature versions prevent monochrome or stale color reuse",
                "surface_denial": "fail_closed_with_diagnostic",
                "artifacts": {
                    "surface_model": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/colrv1-glyph-paint-surface-model-colrv_gradient_composite.json",
                    "cache_scheduler": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/colrv1-cache-scheduler-matrix-colrv_gradient_composite.json",
                    "tile_band_progressive": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/colrv1-tile-band-progressive-equivalence-colrv_gradient_composite.json",
                    "determinism": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/colrv1-determinism-report-colrv_gradient_composite.json"
                }
            }
        },
        "multi_reference_audit": {
            "status": "colrv_gradient_composite_corpus_classified",
            "reference_engines": ["Poppler", "PDFium", "MuPDF"],
            "fixture_count": 24,
            "render_results": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/multi-reference-render-results-colrv_gradient_composite.json",
            "diff_metrics": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/multi-reference-diff-metrics-colrv_gradient_composite.json",
            "reference_disagreement_summary": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/reference-disagreement-summary-colrv_gradient_composite.json",
            "html_report": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/colrv_gradient_composite-html-report/index.html",
            "wellfriendpdf_outlier_failures": 0,
            "unclassified_failures": 0
        },
        "public_report_parity": {
            "schema_change": "additive_section_only",
            "report_envelope_version": REPORT_ENVELOPE_VERSION,
            "bindings": ["Rust SDK", "CLI", "Python", "C ABI", "WASM", ".NET", "Java Maven", "Java Gradle"]
        },
        "closure_gates": {
            "memory_cap_mb": 4096,
            "public_report_schema": "additive_feature_report_colrv_gradient_composite",
            "wellfriendpdf_outlier_failures": 0,
            "unclassified_failures": 0
        },
        "remaining_bounded_limits": [
            "Colrv Gradient Composite's Porter-Duff/Plus and moving-center radial limits are superseded by porterduff_radial_color_glyph_colrv1_porterduff_radial_closure",
            "COLRv1 glyph paint surfaces are scheduler-bounded full render buffers; cropped glyph-space allocation is an optimization, not a Multilingual Color Glyphs correctness blocker"
        ]
    })
}

fn porterduff_radial_color_glyph_closure_report_value() -> serde_json::Value {
    json!({
        "status": "complete",
        "artifact_root": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference",
        "audit_doc": "docs/porterduff_radial_color_glyph_porterduff_radial_closure_audit.md",
        "overview_doc": "docs/porterduff_radial_color_glyph_colrv1_porterduff_radial_closure.md",
        "audit_script": "scripts/porterduff_radial_color_glyph_colrv1_porterduff_radial_closure.py",
        "closure_audit": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/porterduff_radial_color_glyph-closure-audit.json",
        "porter_duff_plus_composites": {
            "status": "implemented",
            "implemented_modes": [
                "Clear",
                "Source",
                "Destination",
                "DestinationOver",
                "SourceIn",
                "DestinationIn",
                "SourceOut",
                "DestinationOut",
                "SourceAtop",
                "DestinationAtop",
                "Xor",
                "Plus"
            ],
            "non_applicable_modes": [],
            "source_surface_model": "scheduler-reserved transparent glyph-local source surface composited against the current glyph-local backdrop",
            "artifacts": {
                "matrix": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/colrv1-porterduff-composite-matrix-porterduff_radial_color_glyph.json",
                "reference_results": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/colrv1-porterduff-composite-reference-results-porterduff_radial_color_glyph.json",
                "scheduler_cache": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/colrv1-composite-scheduler-cache-porterduff_radial_color_glyph.json"
            }
        },
        "exact_moving_center_radial": {
            "status": "implemented_with_reference_equivalence",
            "implementation": "analytic two-circle per-pixel solve with largest finite non-negative-radius root",
            "supported_cases": [
                "same-center radial",
                "moving-center small offset",
                "moving-center large offset",
                "different start/end radii",
                "pad/repeat/reflect extend",
                "transformed radial glyph paint",
                "clipped radial glyph paint",
                "composite radial source"
            ],
            "artifacts": {
                "matrix": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/colrv1-exact-radial-gradient-matrix-porterduff_radial_color_glyph.json",
                "reference_results": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/colrv1-exact-radial-gradient-reference-results-porterduff_radial_color_glyph.json",
                "error_bound": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/colrv1-radial-error-bound-porterduff_radial_color_glyph.json"
            }
        },
        "cache_scheduler_determinism": {
            "status": "implemented",
            "cache_key_inputs": [
                "font identity",
                "glyph id",
                "palette",
                "COLRv1 graph digest/font hash",
                "composite mode",
                "Porter-Duff mode",
                "radial gradient parameters",
                "clip stack digest",
                "transform state",
                "render scale/options"
            ],
            "scheduler_paths": [
                "isolated glyph paint surface",
                "Porter-Duff source paint surface",
                "clip masks",
                "transformed glyph paint surfaces"
            ],
            "artifacts": {
                "cache_key": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/colrv1-cache-key-porterduff_radial_color_glyph.json",
                "scheduler_memory": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/colrv1-scheduler-memory-porterduff_radial_color_glyph.json",
                "determinism": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/colrv1-determinism-porterduff_radial_color_glyph.json"
            }
        },
        "multi_reference_audit": {
            "status": "porterduff_radial_color_glyph_corpus_classified",
            "reference_engines": ["Poppler", "PDFium", "MuPDF"],
            "fixture_count": 38,
            "rendered_page_count": 31,
            "render_results": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/multi-reference-render-results-porterduff_radial_color_glyph.json",
            "diff_metrics": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/multi-reference-diff-metrics-porterduff_radial_color_glyph.json",
            "reference_disagreement_summary": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/reference-disagreement-summary-porterduff_radial_color_glyph.json",
            "html_report": "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/porterduff_radial_color_glyph-html-report/index.html",
            "wellfriendpdf_outlier_failures": 0,
            "unclassified_failures": 0
        },
        "public_report_parity": {
            "schema_change": "additive_section_only",
            "report_envelope_version": REPORT_ENVELOPE_VERSION,
            "bindings": ["Rust SDK", "CLI", "Python", "C ABI", "WASM", ".NET", "Java Maven", "Java Gradle"]
        },
        "closure_gates": {
            "memory_cap_mb": 4096,
            "public_report_schema": "additive_feature_report_porterduff_radial_color_glyph",
            "wellfriendpdf_outlier_failures": 0,
            "unclassified_failures": 0
        },
        "remaining_bounded_limits": [
            "No Multilingual Color Glyphs color-glyph blockers remain; future work is limited to performance optimizations such as cropped glyph-space intermediate surfaces"
        ]
    })
}

fn renderer_fuzz_cmm_renderer_fuzz_cmm_closeout_report_value() -> serde_json::Value {
    json!({
        "status": "complete_with_native_cmm_hard_blocked_precise",
        "artifact_root": "target/renderer_fuzz_cmm-renderer-cmm-closeout",
        "audit_doc": "docs/renderer_fuzz_cmm_renderer_cmm_audit.md",
        "fuzz_doc": "docs/renderer_fuzz_cmm_renderer_fuzz_metamorphic_campaign.md",
        "closeout_doc": "docs/renderer_fuzz_cmm_renderer_parity_closeout.md",
        "native_cmm_audit_doc": "docs/renderer_fuzz_cmm_native_cmm_feasibility_safety_audit.md",
        "native_cmm_backend_doc": "docs/renderer_fuzz_cmm_native_cmm_backend.md",
        "audit_script": "scripts/renderer_fuzz_cmm_renderer_fuzz_cmm_closeout.py",
        "renderer_fuzz": {
            "status": "implemented_with_short_smoke_and_release_duration_deferred",
            "fuzz_target_count": 25,
            "new_target": "renderer_renderer_fuzz_cmm",
            "target_inventory": "target/renderer_fuzz_cmm-renderer-cmm-closeout/renderer-fuzz-target-inventory-renderer_fuzz_cmm.json",
            "seed_corpus_manifest": "target/renderer_fuzz_cmm-renderer-cmm-closeout/renderer-seed-corpus-manifest-renderer_fuzz_cmm.json",
            "structure_aware_mutator": "target/renderer_fuzz_cmm-renderer-cmm-closeout/renderer-mutator-report-renderer_fuzz_cmm.json",
            "smoke_status": "fuzz_bin_compile_plus_mutator_corpus_runner",
            "smoke_report": "target/renderer_fuzz_cmm-renderer-cmm-closeout/renderer-fuzz-smoke-report-renderer_fuzz_cmm.json",
            "release_duration_fuzzing": "deferred_release_duration",
            "crash_minimization_workflow": "target/renderer_fuzz_cmm-renderer-cmm-closeout/renderer-crash-minimization-workflow-renderer_fuzz_cmm.md",
            "unclassified_crashes_hangs_ooms": 0
        },
        "metamorphic_testing": {
            "status": "implemented",
            "test_file": "crates/engine/tests/renderer_fuzz_cmm_renderer_metamorphic.rs",
            "comparison": "byte_exact_rgba",
            "threshold": 0,
            "full_tile_band": "target/renderer_fuzz_cmm-renderer-cmm-closeout/full-tile-band-equivalence-renderer_fuzz_cmm.json",
            "cache_no_cache": "target/renderer_fuzz_cmm-renderer-cmm-closeout/cache-no-cache-equivalence-renderer_fuzz_cmm.json",
            "progressive_resume": "target/renderer_fuzz_cmm-renderer-cmm-closeout/progressive-equivalence-renderer_fuzz_cmm.json",
            "ocg_cache_separation": "target/renderer_fuzz_cmm-renderer-cmm-closeout/ocg-cache-separation-renderer_fuzz_cmm.json",
            "unclassified_failures": 0,
            "stale_cache_failures": 0,
            "progressive_mismatch_failures": 0
        },
        "renderer_closeout": {
            "status": "implemented",
            "reference_engines": ["Poppler", "PDFium", "MuPDF", "Wellfriend"],
            "corpus_manifest": "target/renderer_fuzz_cmm-renderer-cmm-closeout/renderer-closeout-corpus-manifest-renderer_fuzz_cmm.json",
            "render_results": "target/renderer_fuzz_cmm-renderer-cmm-closeout/renderer-closeout-render-results-renderer_fuzz_cmm.json",
            "diff_metrics": "target/renderer_fuzz_cmm-renderer-cmm-closeout/renderer-closeout-diff-metrics-renderer_fuzz_cmm.json",
            "reference_disagreements": "target/renderer_fuzz_cmm-renderer-cmm-closeout/renderer-closeout-reference-disagreements-renderer_fuzz_cmm.json",
            "fallback_taxonomy": "target/renderer_fuzz_cmm-renderer-cmm-closeout/renderer-closeout-fallback-taxonomy-renderer_fuzz_cmm.json",
            "performance_memory": "target/renderer_fuzz_cmm-renderer-cmm-closeout/renderer-closeout-performance-memory-renderer_fuzz_cmm.json",
            "html_report": "target/renderer_fuzz_cmm-renderer-cmm-closeout/renderer-closeout-html-report/index.html",
            "visual_threshold": "mean_abs_channel_difference <= 2.0 OR changed_pixel_threshold8_percentage <= 0.02",
            "wellfriendpdf_outlier_failures": 0,
            "unclassified_failures": 0,
            "verdict": "advanced CMM/prepress may begin with exact CMM limits carried forward"
        },
        "native_cmm_audit": {
            "decision": "littlecms_native_backend_hard_blocked_pending_audited_native_boundary",
            "backend_candidate": "LittleCMS lcms2",
            "license_posture": "generally compatible but not vendored_or_linked_in_renderer_fuzz_cmm",
            "security_posture": "no unsafe/native boundary inside wellfriendpdf-engine default build",
            "dependency_policy": "no silent native dependency",
            "feature_flag": "reserved_native-cmm-lcms2_not_compiled",
            "default_build_posture": "no_native_cmm_dependency",
            "wasm_posture": "native_cmm_disabled_qcms_default_path_only",
            "package_impact": "target/renderer_fuzz_cmm-renderer-cmm-closeout/native-cmm-package-impact-renderer_fuzz_cmm.json",
            "audit_artifact": "target/renderer_fuzz_cmm-renderer-cmm-closeout/native-cmm-feasibility-renderer_fuzz_cmm.json"
        },
        "native_cmm_backend": {
            "status": "hard_blocked_precise_no_default_native_dependency",
            "backend_used_in_current_build": "safe-rust-plus-qcms",
            "native_backend_used_in_current_build": false,
            "feature_flag_status": "reserved_not_available",
            "implemented_default_transforms": [
                "ICCBased profile-to-sRGB preview through qcms",
                "DeviceCMYK deterministic process-ink preview",
                "CalRGB/CalGray/Lab to sRGB fallback",
                "rendering intent carried into qcms transform options"
            ],
            "output_intent_behavior": "reported; destination-output proofing transform remains later owner",
            "image_integration": "ICCBased image source to sRGB preview where qcms accepts the profile",
            "shading_integration": "current Device/Cal/Lab color model only",
            "pattern_integration": "current Device/Cal/Lab color model only",
            "transparency_group_integration": "RGB framebuffer preview only",
            "transform_tests": "qcms_identity_vectors_no_native_claim",
            "cache_memory": "target/renderer_fuzz_cmm-renderer-cmm-closeout/native-cmm-cache-memory-renderer_fuzz_cmm.json",
            "backend_matrix": "target/renderer_fuzz_cmm-renderer-cmm-closeout/native-cmm-backend-matrix-renderer_fuzz_cmm.json",
            "not_claimed": [
                "LittleCMS native transforms",
                "device-link ICC",
                "multicolor ICC",
                "true black-point compensation",
                "separation/DeviceN plate framebuffer",
                "overprint proofing"
            ]
        },
        "public_report_parity": {
            "schema_change": "additive_section_only",
            "report_envelope_version": REPORT_ENVELOPE_VERSION,
            "bindings": ["Rust SDK", "CLI", "Python", "C ABI", "WASM", ".NET", "Java Maven", "Java Gradle"],
            "artifact": "target/renderer_fuzz_cmm-renderer-cmm-closeout/public-feature-report-renderer_fuzz_cmm.json"
        },
        "closure_gates": {
            "memory_cap_mb": 4096,
            "public_report_schema": "additive_feature_report_renderer_fuzz_cmm",
            "wellfriendpdf_outlier_failures": 0,
            "unclassified_failures": 0,
            "native_cmm_backend_status": "hard_blocked_precise",
            "default_build_native_dependency": false,
            "wasm_native_dependency": false
        },
        "remaining_bounded_limits": [
            "release-duration coverage-guided renderer fuzzing remains a release-hardening run over the Renderer Fuzz CMM targets and promoted corpus",
            "LittleCMS/native CMM is not linked until a separate audited native boundary and package policy are accepted",
            "output-intent destination proofing, device-link ICC, multicolor ICC, true BPC, spot/DeviceN plates, separation framebuffers, and overprint proofing remain later CMM/prepress owners",
            "qcms/default ICCBased transforms are sRGB preview transforms, not full prepress parity"
        ]
    })
}

fn native_cmm_backend_native_littlecms_cmm_backend_closure_report_value() -> serde_json::Value {
    let native = cmm::native_cmm_status();
    let backend_status = if native.available {
        "implemented_native_lcms2_active"
    } else if native.compiled {
        "compiled_native_lcms2_unavailable_on_target"
    } else {
        "portable_fallback_qcms_active_native_feature_not_compiled"
    };
    json!({
        "status": "complete",
        "artifact_root": "target/renderer_fuzz_cmm-renderer-cmm-closeout",
        "audit_doc": "docs/native_cmm_backend_native_cmm_safety_audit.md",
        "backend_doc": "docs/native_cmm_backend_native_littlecms_cmm_backend_closure.md",
        "selection_doc": "docs/native_cmm_backend_cmm_backend_selection.md",
        "native_cmm_compiled": native.compiled,
        "native_cmm_available_at_runtime": native.available,
        "backend_selected": native.selected_backend,
        "native_backend_version": native.native_version,
        "native_backend_crates": {
            "lcms2": cmm::LCMS2_CRATE_VERSION,
            "lcms2_sys": cmm::LCMS2_SYS_CRATE_VERSION
        },
        "feature_flag": {
            "name": native.feature_flag,
            "enabled_in_current_build": native.compiled,
            "default_enabled": false
        },
        "backend_status": backend_status,
        "profile_size_cap": cmm::DEFAULT_MAX_ICC_PROFILE_BYTES,
        "transform_cache_cap": cmm::DEFAULT_TRANSFORM_CACHE_ENTRIES,
        "output_intent_proofing_status": if native.available {
            "implemented_basic_lcms2_soft_proofing_helper_and_report_path"
        } else {
            "reported_unavailable_uses_fallback_color_report_only"
        },
        "bpc_status": if native.available {
            "implemented_for_lcms2_transform_flags_on_request"
        } else {
            "unsupported_in_default_qcms_fallback_reported"
        },
        "rendering_intent_status": {
            "supported": ["perceptual", "relative_colorimetric", "saturation", "absolute_colorimetric"],
            "unsupported": []
        },
        "icc_transform_support": {
            "gray": if native.available { "lcms2_profile_to_srgb" } else { "qcms_fallback_profile_to_srgb" },
            "rgb": if native.available { "lcms2_profile_to_srgb" } else { "qcms_fallback_profile_to_srgb" },
            "cmyk": if native.available { "lcms2_profile_to_srgb_for_valid_cmyk_icc_profiles" } else { "qcms_fallback_profile_to_srgb_where_qcms_accepts_profile" },
            "malformed_profiles": "fail_closed_structured_diagnostics",
            "oversized_profiles": "fail_closed_16_mib_cap"
        },
        "wasm_native_unavailable_posture": {
            "target_arch_wasm32": cfg!(target_arch = "wasm32"),
            "native_cmm_available": false,
            "fallback_backend": "qcms/default portable color path"
        },
        "package_posture": {
            "rust_sdk": "native CMM available only with source build feature native-cmm-lcms2",
            "cli": "native CMM available only when CLI is built with native-cmm-lcms2 feature",
            "python": "fresh default wheel does not bundle lcms2; report says native unavailable unless built from source with feature",
            "c_abi": "native payload not silently bundled; feature build reports lcms2 only when compiled",
            "wasm": "native CMM unavailable; fallback remains active",
            "dotnet": "package does not silently bundle lcms2; report exposes fallback/native state from bundled native library",
            "java_maven": "package does not silently bundle lcms2; report exposes fallback/native state from bundled native library",
            "java_gradle": "package does not silently bundle lcms2; report exposes fallback/native state from bundled native library"
        },
        "native_boundary": {
            "unsafe_policy": native.unsafe_boundary,
            "linking_posture": native.linking_posture,
            "default_build_native_dependency": native.default_build_native_dependency,
            "license": "lcms2 and lcms2-sys Rust crates MIT; bundled LittleCMS source has MIT-style LittleCMS license notice when static fallback is used"
        },
        "validation_status": "native_cmm_backend_native_cmm_audit_required",
        "artifacts": {
            "audit": "target/renderer_fuzz_cmm-renderer-cmm-closeout/native_cmm_backend-native-cmm-audit.json",
            "build_matrix": "target/renderer_fuzz_cmm-renderer-cmm-closeout/native-cmm-build-matrix-native_cmm_backend.json",
            "package_matrix": "target/renderer_fuzz_cmm-renderer-cmm-closeout/native-cmm-package-matrix-native_cmm_backend.json",
            "transform_matrix": "target/renderer_fuzz_cmm-renderer-cmm-closeout/native-cmm-transform-matrix-native_cmm_backend.json",
            "binding_parity": "target/renderer_fuzz_cmm-renderer-cmm-closeout/native-cmm-binding-report-parity-native_cmm_backend.json",
            "html_report": "target/renderer_fuzz_cmm-renderer-cmm-closeout/native_cmm_backend-html-report/index.html"
        },
        "remaining_exact_limits": [
            "device-link ICC execution is reserved for Prepress CMM",
            "multicolor ICC and n-color transforms are reserved for Prepress CMM",
            "true separation framebuffer and spot/DeviceN plate preview are reserved for Prepress CMM/13",
            "Prepress Proofing owns bounded overprint/prepress close-out; certification-grade PDF/X proofing is not claimed",
            "default Python/.NET/Java packages do not silently bundle LittleCMS"
        ],
        "closure_gates": {
            "public_report_schema": "additive_feature_report_native_cmm_backend",
            "schema_change": "additive_section_only",
            "no_silent_native_dependency": true,
            "default_build_portable": true,
            "wasm_build_portable": true
        }
    })
}

fn prepress_cmm_prepress_cmm_device_link_separation_plates_report_value() -> serde_json::Value {
    let native = cmm::native_cmm_status();
    json!({
        "status": "complete",
        "artifact_root": "target/prepress_cmm-prepress-cmm",
        "audit_doc": "docs/prepress_cmm_prepress_cmm_audit.md",
        "device_link_doc": "docs/prepress_cmm_device_link_icc.md",
        "multicolor_doc": "docs/prepress_cmm_multicolor_icc.md",
        "bpc_intent_doc": "docs/prepress_cmm_bpc_rendering_intents.md",
        "separation_framebuffer_doc": "docs/prepress_cmm_separation_framebuffer.md",
        "plate_rendering_doc": "docs/prepress_cmm_spot_devicen_plate_rendering.md",
        "plate_preview_doc": "docs/prepress_cmm_plate_preview.md",
        "audit_script": "scripts/prepress_cmm_prepress_cmm_audit.py",
        "native_cmm_compiled": native.compiled,
        "native_cmm_available_at_runtime": native.available,
        "backend_selected": native.selected_backend,
        "device_link_icc": {
            "detection": "ICC header profile-class detection for scnr/mntr/prtr/link/spac/abst/nmcl plus native validation where available",
            "native_transform_behavior": if native.available {
                "lcms2 path accepts legal device-link source/destination channel shapes; ambiguous or mismatched contexts fail closed"
            } else {
                "not active in current build"
            },
            "fallback_behavior": "default/WASM inventories device-link profiles and reports unsupported transform status; alternate spaces are preview-only",
            "diagnostics": [
                "profile_hash",
                "object_id",
                "profile_class",
                "input_channels",
                "output_channels",
                "reason"
            ],
            "artifacts": [
                "device-link-icc-matrix-prepress_cmm.json",
                "device-link-transform-results-prepress_cmm.json",
                "device-link-fallback-results-prepress_cmm.json",
                "device-link-malformed-results-prepress_cmm.json"
            ]
        },
        "multicolor_icc": {
            "inventory": "nCLR signatures are detected with channel counts and profile hashes",
            "native_behavior": "safe Gray/RGB/CMYK transforms remain active; higher-channel multicolor ICC is fail-closed/report-only until a safe renderer pixel format is available",
            "fallback_behavior": "unsupported_multicolor_transform_alternate_preview_only_when_pdf_supplies_safe_alternate",
            "devicen_interaction": "DeviceN component names and tint values are preserved in the sparse plate framebuffer/report model"
        },
        "rendering_intents_bpc": {
            "supported_intents": ["perceptual", "relative_colorimetric", "saturation", "absolute_colorimetric"],
            "bpc_native": if native.available { "wired_to_littlecms_flags_on_request" } else { "native_lcms2_unavailable_current_build" },
            "bpc_fallback": "bpc_unsupported_in_fallback_reported",
            "cache_key_fields": [
                "backend",
                "profile_hash",
                "input_channels",
                "output_channels",
                "rendering_intent",
                "black_point_compensation",
                "output_intent",
                "plate_cache_fingerprint"
            ]
        },
        "separation_framebuffer": {
            "status": "implemented_sparse_plate_contribution_model",
            "storage_model": "sparse_tile_local_plate_contributions",
            "max_prepress_plates": prepress::MAX_PREPRESS_PLATES,
            "memory_budget_bytes": prepress::DEFAULT_SEPARATION_FRAMEBUFFER_BUDGET_BYTES,
            "scheduler_accounted": true,
            "cache_key_includes_plate_state": true,
            "progressive_tile_band_posture": "plate fingerprint participates in render cache keys; Prepress CMM artifacts prove representative equivalence"
        },
        "spot_devicen_plates": {
            "separation_support": "spot name, tint value, alternate preview, alpha, object provenance, and Prepress Proofing overprint posture are preserved",
            "devicen_support": "component names and per-component tints are preserved; process components remain distinct from named spot plates",
            "tint_transforms": "existing bounded PDF function evaluator is used for preview; malformed/excessive functions are reported",
            "plate_preview": "report hashes are emitted under plate-preview-results-prepress_cmm.json"
        },
        "public_reports": {
            "color_report": "additive prepress_cmm_prepress_cmm_device_link_separation_plates section",
            "feature_report": "additive_feature_report_prepress_cmm",
            "bindings": ["Rust", "CLI", "Python", "C ABI", "WASM", ".NET", "Java Maven", "Java Gradle"]
        },
        "reference_audit": {
            "reference_engines": ["Poppler", "PDFium", "MuPDF"],
            "wellfriendpdf_outlier_failures": 0,
            "unclassified_failures": 0,
            "disagreement_policy": "spot/DeviceN flattening differences are classified; Wellfriend internal plate artifacts prove plate state"
        },
        "remaining_exact_limits": [
            "Prepress Proofing owns bounded overprint close-out; Prepress CMM remains the CMM/plate baseline",
            "certification-grade PDF/X validation is later standards work",
            "Nchannel Plate Prepress owns n-channel output closure and exact high-channel transform limits"
        ],
        "closure_gates": {
            "public_report_schema": "additive_feature_report_prepress_cmm",
            "schema_change": "additive_section_only",
            "default_build_portable": true,
            "wasm_build_portable": true,
            "no_silent_rgb_flattening_claimed_as_proof": true
        }
    })
}

fn nchannel_plate_prepress_nchannel_plate_reference_closure_report_value() -> serde_json::Value {
    let native = cmm::native_cmm_status();
    json!({
        "status": "complete",
        "artifact_root": "target/prepress_cmm-prepress-cmm",
        "audit_doc": "docs/nchannel_plate_prepress_prepress_nchannel_plate_closure_audit.md",
        "audit_script": "scripts/nchannel_plate_prepress_prepress_nchannel_plate_closure.py",
        "nchannel_pixel_format": {
            "status": "implemented_bounded_internal_sample_surface",
            "storage_model": "dynamic_channel_vector_samples_backed_by_sparse_tile_local_plate_planes",
            "min_channels": 1,
            "max_channels": prepress::MAX_NCHANNEL_OUTPUT_CHANNELS,
            "channel_labels_preserved": true,
            "process_vs_named_distinction": true,
            "alpha_coverage_preserved": true,
            "memory_budget_bytes": prepress::DEFAULT_SEPARATION_FRAMEBUFFER_BUDGET_BYTES,
            "channel_cap_fail_closed": true,
            "cache_key_fields": [
                "backend",
                "profile_hash",
                "input_channels",
                "output_channels",
                "channel_labels",
                "rendering_intent",
                "black_point_compensation",
                "output_intent",
                "plate_fingerprint"
            ]
        },
        "device_link_transform_status": if native.available {
            "native_lcms2_device_link_path_validates_link_class_source_destination_shape_and_prevents_output_intent_double_proofing"
        } else {
            "unsupported_reported_no_native_backend_default_wasm_preview_only"
        },
        "multicolor_icc_transform_status": if native.available {
            "native_lcms2_transform_setup_runs_where_the_safe_wrapper_exposes_pixel_formats; 2CLR_through_FCLR_inventory_and_nchannel_output_representation_are_implemented"
        } else {
            "unsupported_reported_no_native_backend_default_wasm_preview_only"
        },
        "bpc_rendering_intent_status": if native.available {
            "all_four_intents_threaded; black_point_compensation_in_lcms2_flags_and_transform_cache_keys"
        } else {
            "all_four_intents_reported; black_point_compensation_unsupported_in_fallback"
        },
        "separation_framebuffer_status": {
            "architecture": "sampled_nchannel_plate_surface_with_sparse_tile_local_plane_storage",
            "scheduler_accounted": true,
            "per_page_memory_cap_bytes": prepress::DEFAULT_SEPARATION_FRAMEBUFFER_BUDGET_BYTES,
            "plate_count_cap": prepress::MAX_PREPRESS_PLATES,
            "channel_count_cap": prepress::MAX_NCHANNEL_OUTPUT_CHANNELS,
            "tile_band_progressive_cache_equivalence": "proved_by_nchannel_plate_prepress_audit_artifacts"
        },
        "plate_writing": {
            "text": "implemented_for_simple_type0_cid_type1_truetype_and_supported_type3_path_geometry",
            "vector": "implemented_for_fill_stroke_fill_stroke_even_odd_nonzero_dash_cap_join_geometry",
            "images": "implemented_for_stencil_masks_and_named_separation_devicen_image_color_space_samples",
            "shadings": "implemented_for_named_separation_devicen_shading_color_space_samples",
            "patterns": "implemented_for_colored_tiling_uncolored_caller_color_and_shading_pattern_plate_samples",
            "provenance": "page_object_operation_color_space_plate_and_cache_fingerprint_recorded"
        },
        "reference_audit": {
            "poppler": "required_and_run_by_nchannel_plate_prepress_audit",
            "pdfium": "required_and_run_by_nchannel_plate_prepress_audit",
            "mupdf": "required_and_run_by_nchannel_plate_prepress_audit",
            "wellfriendpdf_default": "run",
            "wellfriendpdf_native_lcms2": if native.available { "run_current_feature_build" } else { "run_when_feature_gate_enabled_in_validation" },
            "wellfriendpdf_outlier_failures": 0,
            "unclassified_failures": 0
        },
        "native_fallback_backend_status": {
            "native_cmm_compiled": native.compiled,
            "native_cmm_available_at_runtime": native.available,
            "backend_selected": native.selected_backend,
            "fallback_wasm_posture": "no_native_nchannel_transform_claim; inventory_and_preview_only"
        },
        "public_reports": {
            "feature_report": "additive_feature_report_nchannel_plate_prepress",
            "color_report": "additive nchannel_plate_prepress_nchannel_plate_reference_closure section",
            "bindings": ["Rust", "CLI", "Python", "C ABI", "WASM", ".NET", "Java Maven", "Java Gradle"]
        },
        "remaining_exact_limits": [
            "Prepress Proofing owns bounded overprint close-out; Nchannel Plate Prepress remains the n-channel baseline",
            "certification-grade PDF/X validation is later standards work",
            "resource-heavy Type3 charprocs that invoke XObjects/shadings/images are fail-closed until recursive Type3 resource execution owns those resources",
            "ICC profiles whose n-channel pixel format is not exposed by the safe LittleCMS wrapper are inventory plus unsupported_reported_unsafe_profile rather than transformed"
        ],
        "closure_gates": {
            "public_report_schema": "additive_feature_report_nchannel_plate_prepress",
            "schema_change": "additive_section_only",
            "default_build_portable": true,
            "wasm_build_portable": true,
            "pdfium_reference_run_required": true,
            "mupdf_reference_run_required": true,
            "wellfriendpdf_outlier_failures": 0,
            "unclassified_failures": 0
        }
    })
}

fn prepress_proofing_full_overprint_prepress_closeout_report_value() -> serde_json::Value {
    let native = cmm::native_cmm_status();
    json!({
        "status": "complete",
        "artifact_root": "target/prepress_proofing-prepress-closeout",
        "audit_doc": "docs/prepress_proofing_prepress_closeout_audit.md",
        "overprint_doc": "docs/prepress_proofing_full_overprint_simulation.md",
        "shading_pattern_doc": "docs/prepress_proofing_color_managed_shadings_patterns.md",
        "benchmark_doc": "docs/prepress_proofing_prepress_proofing_benchmark.md",
        "scorecard_doc": "docs/prepress_proofing_advanced_cmm_prepress_scorecard.md",
        "cache_scheduler_doc": "docs/prepress_proofing_prepress_cache_scheduler.md",
        "known_limits_doc": "docs/prepress_proofing_known_limits.md",
        "release_verdict_doc": "docs/prepress_proofing_release_verdict.md",
        "audit_script": "scripts/prepress_proofing_prepress_benchmark.py",
        "overprint": {
            "simulation_status": "implemented_with_limits",
            "op_opm_status": "OP_stroke_and_op_fill_are_distinct; OPM_0_and_OPM_1_are_modeled_for_supported_process_named_plate_paths",
            "fill_overprint": "implemented_for_DeviceCMYK_Separation_DeviceN_and_named_plate_contribution_paths",
            "stroke_overprint": "implemented_for_DeviceCMYK_Separation_DeviceN_and_named_plate_contribution_paths",
            "text": "implemented_for_text_fill_text_stroke_Type0_CID_simple_fonts_and_safe_Type3_path_geometry; resource_heavy_Type3_exact_limit_reported",
            "vector": "implemented_for_fill_stroke_fill_stroke_even_odd_nonzero_dash_cap_join_geometry",
            "image": "implemented_for_stencil_current_color_named_separation_devicen_samples_and_CMYK_component_report_paths; unsafe_high_channel_layouts_fail_closed",
            "shading": "implemented_for_axial_radial_mesh_patch_and_function_color_paths_already_supported_by_renderer_CMM_layer",
            "pattern": "implemented_for_colored_tiling_uncolored_caller_color_pattern_resource_color_spaces_and_cache_fingerprints",
            "transparency_interaction": "implemented_with_limits_for_alpha_and_non_recursive_group_plate_contributions; soft_mask_and_knockout_variants_are_exact_matrix_rows",
            "plate_preview_consistency": "plate_hashes_RGB_preview_hashes_and_overprint_posture_are_written_by_prepress_proofing_benchmark",
            "remaining_limits": [
                "vendor-specific RIP quirks without reference evidence are not claimed",
                "recursive resource-heavy Type3 charprocs remain fail-closed",
                "unsafe high-channel image or ICC pixel formats not exposed by the safe wrapper are unsupported_reported_exact"
            ]
        },
        "color_managed_shadings_patterns": {
            "shading_behavior": "axial_radial_mesh_patch_and_function_interpolation_route_through ColorSpaceHandler, ICC/Cal/Lab/Separation/DeviceN tint transforms, and native/fallback CMM reporting",
            "pattern_behavior": "colored_and_uncolored_tiling_patterns_preserve_caller_color_space_pattern_resource_color_space_matrix_cell_cache_and_plate_contribution_provenance",
            "native_behavior": if native.available {
                "native_lcms2_active_for_supported_Gray_RGB_CMYK_device_link_and_output_intent_contexts"
            } else {
                "native_lcms2_not_available_in_current_build"
            },
            "fallback_wasm_behavior": "fallback_and_wasm_preserve_plate_metadata_and_label_output_as_preview_only_where_native_proofing_is_absent",
            "artifacts": [
                "color-managed-shadings-matrix-prepress_proofing.json",
                "color-managed-patterns-matrix-prepress_proofing.json",
                "shading-pattern-native-fallback-comparison-prepress_proofing.json",
                "shading-pattern-plate-output-prepress_proofing.json",
                "shading-pattern-cache-equivalence-prepress_proofing.json"
            ]
        },
        "prepress_benchmark": {
            "status": "deterministic_prepress_proofing_suite",
            "fixture_categories": 18,
            "dimensions": [
                "page_count",
                "fixture_category",
                "input_pdf_hash",
                "output_preview_hash",
                "plate_output_hash",
                "native_fallback_backend",
                "rendering_intent",
                "black_point_compensation",
                "output_intent_hash",
                "profile_hashes",
                "plate_names",
                "channel_counts",
                "tile_band_progressive_equivalence",
                "cache_hits_misses_evictions",
                "peak_memory",
                "elapsed_ms",
                "diagnostics_count",
                "unsupported_exact_rows",
                "reference_renderer_status"
            ],
            "references": {
                "wellfriendpdf_default": "run",
                "wellfriendpdf_native_lcms2": if native.available { "run_current_feature_build" } else { "feature_build_required_or_reported_unavailable" },
                "poppler": "required_and_run_when_target_local_tool_available",
                "pdfium": "required_and_run_when_target_local_tool_available",
                "mupdf": "required_and_run_when_target_local_tool_available"
            },
            "wellfriendpdf_outliers": 0,
            "unclassified_failures": 0
        },
        "advanced_cmm_prepress_scorecard": {
            "native_cmm": if native.available { "active_supported_profile_shapes" } else { "not_active_current_build" },
            "fallback_qcms_default": "preview_only_for_prepress_proofing_limits",
            "wasm_backend": "no_native_lcms2_dependency; preview_only_limits_reported",
            "device_link_multicolor": "inventory_transform_setup_and_safe_pixel_format_limits_are_exact",
            "bpc_intents": "four_intents_threaded; BPC_in_native_flags_and_cache_keys_where_supported",
            "plates_overprint": "DeviceCMYK_Separation_DeviceN_true_separation_framebuffer_and_overprint_posture_are_reported",
            "final_verdict": "credible_bounded_prepress_simulation_not_certification_not_RIP_replacement"
        },
        "tile_band_progressive_cache": {
            "full_vs_tile": "proved_by_prepress_proofing_artifact",
            "full_vs_band": "proved_by_prepress_proofing_artifact",
            "full_vs_progressive": "proved_by_prepress_proofing_artifact",
            "cache_no_cache": "proved_by_prepress_proofing_artifact",
            "invalidates_on_output_intent": true,
            "invalidates_on_bpc": true,
            "invalidates_on_rendering_intent": true,
            "invalidates_on_plate_visibility": true,
            "invalidates_on_overprint_state": true,
            "scheduler_caps_enforced": true,
            "memory_budget_bytes": prepress::DEFAULT_SEPARATION_FRAMEBUFFER_BUDGET_BYTES,
            "max_prepress_plates": prepress::MAX_PREPRESS_PLATES,
            "max_nchannel_output_channels": prepress::MAX_NCHANNEL_OUTPUT_CHANNELS
        },
        "public_reports": {
            "feature_report": "additive_feature_report_prepress_proofing",
            "color_report": "additive prepress_proofing_full_overprint_prepress_closeout section",
            "bindings": ["Rust", "CLI", "Python", "C ABI", "WASM", ".NET", "Java Maven", "Java Gradle"]
        },
        "reference_audit": {
            "poppler": "required_and_run_by_prepress_proofing_benchmark",
            "pdfium": "required_and_run_by_prepress_proofing_benchmark",
            "mupdf": "required_and_run_by_prepress_proofing_benchmark",
            "wellfriendpdf_outlier_failures": 0,
            "unclassified_failures": 0,
            "disagreement_policy": "reference_renderer_differences_are_classified; Wellfriend_outlier_failures_must_be_zero"
        },
        "remaining_exact_limits": [
            "certification-grade PDF/X validation remains later standards work",
            "vendor-specific RIP behavior not covered by Poppler/PDFium/MuPDF/Wellfriend evidence is not claimed",
            "profiles or image layouts whose high-channel pixel format is not exposed by the safe native wrapper are unsupported_reported_exact",
            "malformed recursive resource bombs fail closed under scheduler and resource caps"
        ],
        "closure_gates": {
            "public_report_schema": "additive_feature_report_prepress_proofing",
            "schema_change": "additive_section_only",
            "default_build_portable": true,
            "wasm_build_portable": true,
            "pdfium_reference_run_required": true,
            "mupdf_reference_run_required": true,
            "wellfriendpdf_outlier_failures": 0,
            "unclassified_failures": 0
        }
    })
}

pub fn feature_report_json() -> Result<String> {
    let codec_isolation = codec_isolation_availability_report();
    let native_codec_boundary = codec_isolation["native_codec_boundary"].clone();
    let features = json!({
        "engine_version": crate::ENGINE_VERSION,
        "report_envelope_version": REPORT_ENVELOPE_VERSION,
        "capabilities": {
            "parse": cfg!(feature = "parse"),
            "extract": cfg!(feature = "extract"),
            "render": cfg!(feature = "render"),
            "create": cfg!(feature = "create"),
            "edit": cfg!(feature = "edit"),
            "structural": cfg!(feature = "structural"),
            "sign": cfg!(feature = "sign"),
            "pdfa": cfg!(feature = "pdfa"),
            "ocr": cfg!(feature = "ocr"),
        },
        "runtime": {
            "schema_version": crate::runtime::RUNTIME_CONFIG_SCHEMA_VERSION,
            "public_modes": ["standard", "research"],
            "default_mode": "standard",
            "standard_gpu_required": false,
            "minimum_standard": {
                "vcpu": crate::runtime::MINIMUM_STANDARD_VCPU,
                "ram_bytes": crate::runtime::MINIMUM_STANDARD_RAM_BYTES
            },
            "recommended_standard": {
                "vcpu": crate::runtime::RECOMMENDED_STANDARD_VCPU,
                "ram_bytes": crate::runtime::RECOMMENDED_STANDARD_RAM_BYTES
            },
            "ocr_provider_families": ["hosted_api", "self_hosted", "cloud_document_intelligence"],
            "binding_surfaces": ["Rust", "CLI", "Python", "C ABI", "WASM", ".NET", "Java", "server"]
        },
        "codec_isolation": codec_isolation,
        "codec_boundary": {
            "native_codec_boundary": native_codec_boundary,
            "scanner": scanner_availability_report(),
            "renderer_decode_scheduler": renderer_decode_scheduler_adoption_report(),
            "rlbox_wasm": {
                "status": "hard_blocked_with_codec_boundary_evidence",
                "report_artifact": "target/codec_boundary-codec-boundary-scheduler/rlbox-wasm-feasibility.json"
            }
        },
        "decode_scheduler": {
            "decode_scheduler": non_render_decode_scheduler_adoption_report(),
            "hostile_corpus": {
                "status": "deterministic_generated_corpus_with_local_runner",
                "generator": "scripts/decode_scheduler_hostile_codec_corpus.py",
                "manifest_artifact": "target/decode_scheduler-codec-closeout/hostile-corpus-manifest.json",
                "run_artifact": "target/decode_scheduler-codec-closeout/hostile-corpus-run.json"
            },
            "fuzz_campaign": {
                "status": "campaign_scripts_and_smoke_artifacts",
                "script": "scripts/decode_scheduler_codec_fuzz_campaign.py",
                "target_inventory_artifact": "target/decode_scheduler-codec-closeout/fuzz-target-inventory.json",
                "smoke_artifact": "target/decode_scheduler-codec-closeout/fuzz-smoke-report.json"
            },
            "closeout": {
                "status": "decode_scheduler_closeout_artifacts_required_for_release_grade_verdict",
                "script": "scripts/decode_scheduler_codec_closeout.py",
                "performance_artifact": "target/decode_scheduler-codec-closeout/performance-report.json",
                "verdict_artifact": "target/decode_scheduler-codec-closeout/closeout-verdict.json"
            }
        },
        "native_renderer": {
            "renderer_parity_audit": {
                "status": "reference_aware_corpus_harness",
                "script": "scripts/native_renderer_renderer_parity_audit.py",
                "baseline_artifact": "target/native_renderer-renderer-native-replay/parity-baseline.json",
                "post_native_artifact": "target/native_renderer-renderer-native-replay/parity-after-native-replay.json",
                "reference_availability_artifact": "target/native_renderer-renderer-native-replay/reference-availability.json"
            },
            "native_replay": {
                "status": "native_text_image_form_display_list_foundation",
                "text": "BT/ET state and common text-showing operators are represented as native display-list operations",
                "image": "Image XObject and inline image operations are represented as native display-list operations while decode remains in renderer paths",
                "form_xobject": "Form XObject invocations are represented as native display-list operations with fallback diagnostics for unsupported groups and limits",
                "counter_artifact": "target/native_renderer-renderer-native-replay/native-replay-counters.json",
                "regression_script": "scripts/native_renderer_native_replay_regression.py"
            },
            "compatibility_fallback_policy": {
                "status": "measured_by_operation_kind_and_reason",
                "measured_reasons": [
                    "unsupported_operator_shading",
                    "unsupported_operator_pattern",
                    "unsupported_graphics_state",
                    "unsupported_xobject_subtype",
                    "safety_limit_exceeded",
                    "malformed_content"
                ],
                "policy_doc": "docs/native_renderer_compatibility_fallback_policy.md"
            },
            "failure_taxonomy": {
                "status": "json_taxonomy_for_reference_and_wellfriendpdf_failures",
                "artifact": "target/native_renderer-renderer-native-replay/failure-taxonomy.json",
                "doc": "docs/native_renderer_renderer_failure_taxonomy.md"
            },
            "reference_renderer_multi_reference_audit": {
                "status": "multi_reference_audit_complete",
                "bootstrap_script": "scripts/reference_renderer_bootstrap_reference_renderers.ps1",
                "audit_script": "scripts/reference_renderer_multi_reference_audit.ps1",
                "tool_manifest_artifact": "target/native_renderer-renderer-native-replay/reference-tool-manifest-reference_renderer.json",
                "corpus_manifest_artifact": "target/native_renderer-renderer-native-replay/multi-reference-corpus-manifest-reference_renderer.json",
                "render_results_artifact": "target/native_renderer-renderer-native-replay/multi-reference-render-results-reference_renderer.json",
                "diff_metrics_artifact": "target/native_renderer-renderer-native-replay/multi-reference-diff-metrics-reference_renderer.json",
                "disagreement_summary_artifact": "target/native_renderer-renderer-native-replay/reference-disagreement-summary-reference_renderer.json",
                "taxonomy_artifact": "target/native_renderer-renderer-native-replay/renderer-parity-taxonomy-reference_renderer.json",
                "html_report": "target/native_renderer-renderer-native-replay/reference_renderer-html-report/index.html",
                "reference_engines": {
                    "poppler": "pdftoppm",
                    "pdfium": "target-local pypdfium2/pdfium_test-compatible wrapper",
                    "mupdf": "target-local mutool"
                },
                "corpus_page_count": 13,
                "total_pairwise_comparisons": 78,
                "known_later_owned_renderer_categories": [
                    "pattern/later",
                    "shading/later",
                    "transparency/later"
                ],
                "multi_reference_audit_complete": true
            }
        },
        "transparency_rendering_transparency_compositing": {
            "status": "native_foundation_with_transparency_closeout_closure",
            "audit_script": "scripts/transparency_rendering_transparency_compositing_audit.py",
            "powershell_wrapper": "scripts/transparency_rendering_transparency_compositing_audit.ps1",
            "artifacts": {
                "corpus_manifest": "target/transparency_rendering-transparency-compositing/corpus-manifest.json",
                "baseline_results": "target/transparency_rendering-transparency-compositing/baseline-render-results.json",
                "post_results": "target/transparency_rendering-transparency-compositing/post-implementation-render-results.json",
                "reference_disagreement_summary": "target/transparency_rendering-transparency-compositing/reference-disagreement-summary.json",
                "blend_mode_matrix": "target/transparency_rendering-transparency-compositing/blend-mode-matrix.json",
                "soft_mask_matrix": "target/transparency_rendering-transparency-compositing/soft-mask-matrix.json",
                "group_isolation_knockout_matrix": "target/transparency_rendering-transparency-compositing/group-isolation-knockout-matrix.json",
                "fallback_taxonomy": "target/transparency_rendering-transparency-compositing/fallback-taxonomy.json",
                "memory_budget_report": "target/transparency_rendering-transparency-compositing/memory-budget-report.json",
                "html_report": "target/transparency_rendering-transparency-compositing/html-report/index.html",
                "transparency_closeout_closure_audit": "target/transparency_rendering-transparency-compositing/transparency_closeout-closure-audit.json"
            },
            "transparency_groups": {
                "status": "native_common_path",
                "implemented": [
                    "group_dictionary_detection",
                    "form_xobject_group_integration",
                    "isolated_group_backdrop",
                    "non_isolated_group_backdrop",
                    "bbox_clipping",
                    "nested_group_stack",
                    "group_compositing_back_to_parent",
                    "malformed_group_diagnostics"
                ],
                "bounded_memory": "transparency group RGBA surfaces reserve scheduler memory before allocation",
                "memory_denial_unit_test": "renderer_offscreen_surface_fails_closed_over_budget",
                "color_space_status": "DeviceGray_DeviceRGB_DeviceCMYK_common_group_paths_exercised_by_transparency_closeout",
                "unsupported_reported": [
                    "advanced_icc_device_link_multicolor_group_color_management",
                    "cropped_coordinate_offscreen_surfaces"
                ]
            },
            "blend_modes": {
                "status": "implemented_central_dispatch",
                "implemented": [
                    "Normal",
                    "Multiply",
                    "Screen",
                    "Overlay",
                    "Darken",
                    "Lighten",
                    "ColorDodge",
                    "ColorBurn",
                    "HardLight",
                    "SoftLight",
                    "Difference",
                    "Exclusion",
                    "Hue",
                    "Saturation",
                    "Color",
                    "Luminosity"
                ],
                "posture": "separable and nonseparable formulas are centralized in the render buffer/compositing path"
            },
            "soft_masks": {
                "status": "alpha_and_luminosity_common_path",
                "implemented": [
                    "SMask_graphics_state_detection",
                    "alpha_soft_mask",
                    "luminosity_soft_mask",
                    "mask_bbox_clipping",
                    "mask_matrix_posture",
                    "text_image_form_sources",
                    "transfer_function_lut",
                    "malformed_mask_fail_closed"
                ],
                "bounded_memory": "soft mask group RGBA surfaces reserve scheduler memory before allocation",
                "matte_background_status": "image_smask_matte_and_extgstate_bc_backdrop_closed_by_transparency_closeout",
                "luminosity_color_spaces": ["DeviceGray", "DeviceRGB", "DeviceCMYK"],
                "unsupported_reported": [
                    "advanced_icc_device_link_matte_conversion",
                    "advanced_icc_calibrated_luminosity_cmm_parity"
                ]
            },
            "knockout_isolation": {
                "status": "common_path_native_with_exact_interior_overlap_for_supported_groups",
                "implemented": [
                    "isolated_group_flag",
                    "non_isolated_backdrop",
                    "knockout_group_flag",
                    "nested_isolated_group",
                    "nested_knockout_group",
                    "interior_knockout_overlap",
                    "state_stack_restore",
                    "fallback_metrics"
                ],
                "unsupported_reported": [
                    "text_clipping_inside_knockout_groups",
                    "pattern_and_shading_paints_inside_knockout_groups"
                ]
            },
            "reference_audit": {
                "status": "poppler_pdfium_mupdf_required",
                "tool_manifest": "target/native_renderer-renderer-native-replay/reference-tool-manifest-reference_renderer.json",
                "fixture_count": 47,
                "memory_cap_mb": 4096,
                "classification_artifact": "target/transparency_rendering-transparency-compositing/transparency_closeout-reference-disagreement-summary.json",
                "wellfriendpdf_outlier_failures": 0,
                "unclassified_failures": 0
            },
            "known_limits": [
                "Advanced ICC/device-link/multicolor CMM parity remains unsupported-reported",
                "Offscreen buffers are scheduler-bounded page-coordinate surfaces with bbox clipping rather than cropped coordinate surfaces"
            ]
        },
        "transparency_closeout_transparency_closure": {
            "status": "complete",
            "audit_script": "scripts/transparency_closeout_transparency_closure_audit.py",
            "artifacts": {
                "reference_tool_manifest": "target/transparency_rendering-transparency-compositing/transparency_closeout-reference-tool-manifest.json",
                "corpus_manifest": "target/transparency_rendering-transparency-compositing/transparency_closeout-corpus-manifest.json",
                "render_results": "target/transparency_rendering-transparency-compositing/transparency_closeout-render-results.json",
                "diff_metrics": "target/transparency_rendering-transparency-compositing/transparency_closeout-diff-metrics.json",
                "reference_disagreement_summary": "target/transparency_rendering-transparency-compositing/transparency_closeout-reference-disagreement-summary.json",
                "transparency_matrix": "target/transparency_rendering-transparency-compositing/transparency_closeout-transparency-matrix.json",
                "memory_report": "target/transparency_rendering-transparency-compositing/transparency_closeout-memory-report.json",
                "closure_audit": "target/transparency_rendering-transparency-compositing/transparency_closeout-closure-audit.json",
                "html_report": "target/transparency_rendering-transparency-compositing/transparency_closeout-html-report/index.html"
            },
            "alpha_image": {
                "status": "closed",
                "root_cause": "image_painter_ignored_graphics_state_nonstroking_alpha",
                "fixture": "alpha_image",
                "classification": "all_references_agree_and_wellfriendpdf_passes"
            },
            "soft_mask_matte_background": {
                "status": "closed",
                "implemented": [
                    "image_smask_matte_unblend_for_common_device_spaces",
                    "extgstate_alpha_smask_bc_backdrop"
                ],
                "fixtures": ["image_smask_matte", "softmask_alpha_bc_background"],
                "unsupported_reported": ["advanced_icc_device_link_matte_conversion"]
            },
            "luminosity_soft_mask_color_spaces": {
                "status": "closed",
                "supported": ["DeviceGray", "DeviceRGB", "DeviceCMYK"],
                "fixtures": [
                    "softmask_luminosity_devicegray",
                    "softmask_luminosity_devicergb",
                    "softmask_luminosity_devicecmyk"
                ],
                "unsupported_reported": ["ICCBased_exact_CMM", "CalGray_CalRGB_exact_CMM"]
            },
            "transparency_group_color_spaces": {
                "status": "closed_for_common_device_spaces",
                "supported": ["DeviceGray", "DeviceRGB", "DeviceCMYK"],
                "fixtures": [
                    "group_colorspace_devicegray",
                    "group_colorspace_devicergb",
                    "group_colorspace_devicecmyk"
                ],
                "unsupported_reported": ["advanced_icc_device_link_multicolor_group_blending"]
            },
            "knockout_overlap": {
                "status": "closed",
                "implemented": ["initial_backdrop_per_pixel_knockout_for_vector_and_form_groups"],
                "fixtures": ["knockout_overlap_exact", "knockout_overlap_nested_form"],
                "unsupported_reported": ["text_clipping_and_pattern_shading_inside_knockout_groups"]
            },
            "reference_audit": {
                "fixture_count": 47,
                "classification_counts": {
                    "all_references_agree_and_wellfriendpdf_passes": 41,
                    "references_disagree_and_wellfriendpdf_within_cluster": 5,
                    "malformed_or_reference_failure": 1
                },
                "wellfriendpdf_outlier_failures": 0,
                "unclassified_failures": 0,
                "memory_cap_mb": 4096
            },
            "remaining_bounded_limits": [
                "advanced ICC/device-link/multicolor CMM parity",
                "cropped coordinate offscreen surfaces"
            ]
        },
        "advanced_rendering_text_clipping_shading_patterns": {
            "status": "native_common_paths_with_bounded_unsupported_reports",
            "artifacts": {
                "starting_state": "target/advanced_rendering-text-shading-patterns/starting-state.json",
                "corpus_manifest": "target/advanced_rendering-text-shading-patterns/corpus-manifest.json",
                "reference_tool_manifest": "target/advanced_rendering-text-shading-patterns/reference-tool-manifest.json",
                "text_clipping_matrix": "target/advanced_rendering-text-shading-patterns/text-clipping-matrix.json",
                "axial_radial_shading_matrix": "target/advanced_rendering-text-shading-patterns/axial-radial-shading-matrix.json",
                "mesh_patch_shading_matrix": "target/advanced_rendering-text-shading-patterns/mesh-patch-shading-matrix.json",
                "tiling_pattern_matrix": "target/advanced_rendering-text-shading-patterns/tiling-pattern-matrix.json",
                "fallback_taxonomy": "target/advanced_rendering-text-shading-patterns/fallback-taxonomy.json",
                "render_results": "target/advanced_rendering-text-shading-patterns/multi-reference-render-results.json",
                "diff_metrics": "target/advanced_rendering-text-shading-patterns/visual-diff-metrics.json",
                "reference_disagreement_summary": "target/advanced_rendering-text-shading-patterns/reference-disagreement-summary.json",
                "memory_scheduler_report": "target/advanced_rendering-text-shading-patterns/memory-scheduler-report.json",
                "public_feature_report": "target/advanced_rendering-text-shading-patterns/public-feature-report.json",
                "html_report": "target/advanced_rendering-text-shading-patterns/html-report/index.html"
            },
            "text_clipping": {
                "status": "implemented_with_type3_cid_rendering_type3_cid_closure",
                "rendering_modes": [4, 5, 6, 7],
                "accumulation": "glyph outline masks accumulate during BT/ET and intersect the current clip at ET",
                "interactions_tested": [
                    "subsequent_fill",
                    "image_xobject",
                    "form_xobject",
                    "axial_shading",
                    "colored_tiling_pattern"
                ],
                "unsupported_reported": [
                    "image_or_resource_only_Type3_charprocs_that_do_not_yield_safe_path_geometry",
                    "fonts_or_glyphs_without_extractable_outlines"
                ],
                "type3_cid_rendering_closure": "target/type3_cid_rendering-type3-cid-tensor/type3_cid_rendering-reference-disagreement-summary.json"
            },
            "axial_radial_shadings": {
                "status": "native",
                "shading_types": [2, 3],
                "function_types": [0, 2, 3, 4],
                "implemented": [
                    "domain",
                    "coords",
                    "extend_flags",
                    "bbox_clip",
                    "ctm_transform",
                    "current_clip_and_text_clip",
                    "DeviceGray_DeviceRGB_DeviceCMYK_current_color_model"
                ],
                "unsupported_reported": ["advanced ICC/device-link/multicolor CMM exactness"]
            },
            "mesh_patch_shadings": {
                "status": "native_common_path",
                "shading_types": [4, 5, 6, 7],
                "implemented": [
                    "BitsPerCoordinate",
                    "BitsPerComponent",
                    "BitsPerFlag",
                    "Decode_arrays",
                    "Type4_triangle_connectivity",
                    "Type5_lattice_triangles",
                    "Type6_Coons_patch_tessellation",
                    "Type7_tensor_product_patch_interpolation_with_interior_controls",
                    "malformed_stream_fail_closed"
                ],
                "limits": {
                    "type7_tensor_exactness": "closed by Type3 CID Rendering for the device-color corpus with tensor-product interior evaluation",
                    "tessellation": "deterministic curvature-scaled bounded subdivision and triangle rasterization"
                }
            },
            "tiling_patterns": {
                "status": "native_common_path",
                "paint_types": ["colored", "uncolored"],
                "implemented": [
                    "PatternType_1",
                    "BBox",
                    "XStep_YStep_validation",
                    "Pattern_matrix",
                    "resource_dictionary_merge",
                    "cell_content_stream_interpretation",
                    "caller_color_for_uncolored_patterns",
                    "cell_clipping",
                    "recursion_depth_cap",
                    "tile_count_cap",
                    "scheduler_bounded_stream_decode"
                ],
                "limits": {
                    "cache": "deterministic per-render execution with bounded tile count; no unbounded global pattern cache",
                    "advanced_color": "Pattern color spaces use the current color model rather than advanced CMM"
                }
            },
            "reference_audit": {
                "status": "multi_reference_audit_complete",
                "fixture_count": 26,
                "reference_engines": ["Poppler", "PDFium", "MuPDF"],
                "memory_cap_mb": 4096,
                "classification_artifact": "target/advanced_rendering-text-shading-patterns/reference-disagreement-summary.json",
                "classification_counts": {
                    "all_references_agree_wellfriendpdf_passes": 19,
                    "references_disagree_wellfriendpdf_within_cluster": 3,
                    "unsupported_reported_expected": 3,
                    "malformed_reference_failure": 1
                },
                "wellfriendpdf_outlier_failures": 0,
                "advanced_rendering_cluster_tolerance_acceptances": 2
            },
            "fallback_taxonomy": {
                "removed_vague_buckets": [
                    "text_clipping/later",
                    "shading/later",
                    "pattern/later"
                ],
                "remaining_precise_limits": [
                    "advanced_icc_device_link_multicolor_cmm",
                    "image_or_resource_only_Type3_charprocs_fail_closed",
                    "exotic_missing_glyph_outline_for_text_clip",
                    "cropped_coordinate_offscreen_optimization"
                ]
            }
        },
        "type3_cid_rendering_type3_cid_tensor_closure": {
            "status": "complete_native_common_paths_with_reference_cluster_limits",
            "artifacts": {
                "corpus_manifest": "target/type3_cid_rendering-type3-cid-tensor/type3_cid_rendering-corpus-manifest.json",
                "reference_tool_manifest": "target/type3_cid_rendering-type3-cid-tensor/type3_cid_rendering-reference-tool-manifest.json",
                "render_results": "target/type3_cid_rendering-type3-cid-tensor/type3_cid_rendering-render-results.json",
                "diff_metrics": "target/type3_cid_rendering-type3-cid-tensor/type3_cid_rendering-diff-metrics.json",
                "reference_disagreement_summary": "target/type3_cid_rendering-type3-cid-tensor/type3_cid_rendering-reference-disagreement-summary.json",
                "text_clipping_matrix": "target/type3_cid_rendering-type3-cid-tensor/type3_cid_rendering-text-clipping-matrix.json",
                "type3_clip_matrix": "target/type3_cid_rendering-type3-cid-tensor/type3_cid_rendering-type3-clip-matrix.json",
                "cid_clip_matrix": "target/type3_cid_rendering-type3-cid-tensor/type3_cid_rendering-cid-clip-matrix.json",
                "type7_tensor_matrix": "target/type3_cid_rendering-type3-cid-tensor/type3_cid_rendering-type7-tensor-matrix.json",
                "fallback_taxonomy": "target/type3_cid_rendering-type3-cid-tensor/type3_cid_rendering-fallback-taxonomy.json",
                "memory_scheduler_report": "target/type3_cid_rendering-type3-cid-tensor/type3_cid_rendering-memory-scheduler-report.json",
                "html_report": "target/type3_cid_rendering-type3-cid-tensor/type3_cid_rendering-html-report/index.html"
            },
            "type3_text_clipping": {
                "status": "native_charproc_path_collection",
                "rendering_modes": [4, 5, 6, 7],
                "implemented": [
                    "path_construction_operators",
                    "fillable_path_collection",
                    "stroked_path_outline_collection",
                    "font_matrix_text_matrix_rise_horizontal_scaling_ctm_transform",
                    "multi_glyph_accumulation_until_ET",
                    "fail_closed_for_image_only_or_resource_heavy_charprocs"
                ],
                "reference_cluster_status": "Poppler/PDFium/MuPDF render the generated Type3 Tr clipping fixtures without the Type3 clip; Wellfriend native output is recorded as unsupported_reported_expected reference limitation rather than bbox fallback"
            },
            "cid_cmap_text_clipping": {
                "status": "native_common_identity_h_embedded_outline_path",
                "mapping_path": "encoded bytes to CMap CID to CIDToGIDMap or embedded font glyph ID to outline",
                "fixtures": [
                    "cid_identity_h_image_clip",
                    "cid_multibyte_two_glyph_clip",
                    "cid_form_clip",
                    "cid_axial_shading_clip",
                    "cid_tiling_pattern_clip"
                ],
                "diagnostics": ["missing CID outline fails closed with font/CID/GID context"]
            },
            "type7_tensor_patch": {
                "status": "native_tensor_product_interior",
                "implemented": [
                    "flagged patch decoding",
                    "16 tensor control points",
                    "bicubic Bernstein interior evaluation",
                    "curvature_scaled_deterministic_subdivision",
                    "patch_count_cap",
                    "truncated_stream_fail_closed"
                ],
                "color_scope": "DeviceGray/DeviceRGB/DeviceCMYK via current renderer color model; advanced ICC/device-link/multicolor remains later CMM"
            },
            "reference_audit": {
                "status": "multi_reference_audit_complete",
                "fixture_count": 21,
                "reference_engines": ["Poppler", "PDFium", "MuPDF"],
                "memory_cap_mb": 4096,
                "classification_counts": {
                    "all_references_agree_wellfriendpdf_passes": 11,
                    "unsupported_reported_expected": 10
                },
                "wellfriendpdf_outlier_failures": 0,
                "unclassified_failures": 0
            },
            "fallback_taxonomy": {
                "removed_vague_buckets": [
                    "type3_text_clip_outline_extraction",
                    "missing_glyph_outline_for_common_cid_text_clip",
                    "type7_exact_tensor_interior_interpolation"
                ],
                "remaining_precise_limits": [
                    "advanced_icc_device_link_multicolor_cmm",
                    "exotic_font_outline_absence_unsupported_reported",
                    "unsafe_recursive_type3_or_pattern_resource_bomb_fail_closed",
                    "cropped_coordinate_offscreen_optimization"
                ]
            }
        },
        "annotation_ocg_rendering_annotation_ocg_progressive_cache": annotation_ocg_rendering_renderer_report_value(),
        "renderer_validation_annotation_progressive_cache_validation": renderer_validation_validation_report_value(),
        "multilingual_color_glyphs_cjk_rtl_color_glyph_reference_harness": multilingual_color_glyphs_renderer_report_value(),
        "cjk_rtl_color_glyph_closeout_color_glyph_cjk_rtl_fidelity_closure": cjk_rtl_color_glyph_closeout_closure_report_value(),
        "color_glyph_hinting_color_glyph_hinting_cff_closure": color_glyph_hinting_closure_report_value(),
        "colrv_svg_bitmap_full_colrv1_svg_color_glyph_closure": colrv_svg_bitmap_closure_report_value(),
        "colrv_gradient_composite_colrv1_gradient_clip_composite_closure": colrv_gradient_composite_closure_report_value(),
        "porterduff_radial_color_glyph_colrv1_porterduff_radial_closure": porterduff_radial_color_glyph_closure_report_value(),
        "renderer_fuzz_cmm_renderer_fuzz_cmm_closeout": renderer_fuzz_cmm_renderer_fuzz_cmm_closeout_report_value(),
        "native_cmm_backend_native_littlecms_cmm_backend_closure": native_cmm_backend_native_littlecms_cmm_backend_closure_report_value(),
        "prepress_cmm_prepress_cmm_device_link_separation_plates": prepress_cmm_prepress_cmm_device_link_separation_plates_report_value(),
        "nchannel_plate_prepress_nchannel_plate_reference_closure": nchannel_plate_prepress_nchannel_plate_reference_closure_report_value(),
        "prepress_proofing_full_overprint_prepress_closeout": prepress_proofing_full_overprint_prepress_closeout_report_value(),
        "semantic_intelligence_semantic_intelligence_parenttree_cjk_ml_layout": crate::semantic_intelligence::semantic_intelligence_semantic_intelligence_report_value(),
        "cjk_dictionary_layout_cjk_dictionary_layout_backend_closure": crate::semantic_intelligence::cjk_dictionary_layout_cjk_dictionary_layout_backend_closure_report_value(),
        "semantic_closeout_semantic_binding_rag_benchmark_closeout": crate::semantic_intelligence::semantic_closeout_semantic_binding_rag_benchmark_closeout_report_value(),
        "xfa_runtime_xfa_runtime_sandbox_closure": crate::xfa::xfa_runtime_feature_report_value(REPORT_ENVELOPE_VERSION),
        "annotation_media_redaction_annotation_xfdf_media_nonaxis_redaction": crate::annotation_media_redaction::annotation_media_redaction_feature_report_value(REPORT_ENVELOPE_VERSION),
        "secure_mutation_mask_inline_associated_signature_safe_edits": crate::secure_mutation::secure_mutation_feature_report_value(REPORT_ENVELOPE_VERSION),
        "secure_mutation_closeout_advanced_secure_mutation_closure": crate::secure_mutation::secure_mutation_closeout_feature_report_value(REPORT_ENVELOPE_VERSION),
        "form_action_policy_form_js_interactive_docx_layout": crate::form_action_policy::form_action_policy_feature_report_value(REPORT_ENVELOPE_VERSION),
        "advanced_editing_vertical_rtl_patch_vector_ink_editing": crate::advanced_editing::advanced_editing_feature_report_value(REPORT_ENVELOPE_VERSION),
        "advanced_editing_closeout_multirun_form_appearance_closure": crate::advanced_editing::advanced_editing_closeout_feature_report_value(REPORT_ENVELOPE_VERSION),
        "writer_history_raster_vector_font_persistent_object_stream": crate::writer_history::writer_history_feature_report_value(REPORT_ENVELOPE_VERSION),
        "compression_office_zopfli_dedup_office_to_pdf_benchmark": crate::compression_office::compression_office_feature_report_value(REPORT_ENVELOPE_VERSION),
        "compression_office_closeout_resource_dedup_office_benchmark_closure": crate::compression_office::compression_office_closeout_feature_report_value(REPORT_ENVELOPE_VERSION),
        "crypto_writer_deterministic_writer_pubsec_aesgcm": crate::crypto_writer::crypto_writer_feature_report_value(REPORT_ENVELOPE_VERSION),
        "signature_validation_certificate_trust_pades_ocsp_crl_validation": crate::signature::signature_validation_feature_report_value(REPORT_ENVELOPE_VERSION),
        // Capabilities that are always present in the default build regardless of
        // cargo features (they live in unconditional modules).
        "always_available": [
            "security_report", "sanitize", "canonicalize", "parser_report",
            "color_report", "standards_profile", "interactive_report",
            "forms_report", "annotation_report", "page_operations_report",
            "signature_report", "font_report", "decode_budget_report",
            "resource_dedup_report", "redaction", "semantic_binding_report",
            "advanced_rag_chunk_set", "semantic_search_report", "table_proposal_status",
            "xfa_report", "xfa_extract_report", "xfa_runtime_report",
            "xfa_script_report", "xfa_security_report", "xfa_render_preview",
            "xfa_flatten", "xfa_sanitize",
            "annotation_xfdf_export", "annotation_xfdf_import",
            "annotation_appearance_generate", "annotation_appearance_report",
            "rich_media_report", "rich_media_sanitize", "rich_media_flatten_poster",
            "nonaxis_redaction_plan", "nonaxis_redaction_apply", "annotation_media_redaction_report",
            "redact_image_mask", "redact_inline_image", "associated_files_report",
            "associated_files_extract", "associated_files_add", "associated_files_remove",
            "associated_files_sanitize", "edit_signature_impact", "edit_policy_report",
            "secure_mutation_report", "secure_mutation_closeout_report", "associated_files_update_owner",
            "associated_files_remove_owner", "incremental_form_edit",
            "incremental_annotation_edit", "incremental_page_property_edit",
            "form_js_report", "form_action_graph", "form_js_sanitize",
            "form_js_flatten_values", "interactive_data_report",
            "word_pagination_audit", "form_action_policy_report", "advanced_editing_report",
            "advanced_editing_closeout_report", "writer_history_report", "writer_history_raster_vector_report",
            "writer_history_font_reconstruction_report", "writer_history_history_report",
            "writer_history_object_stream_report", "writer_history_pack_object_streams",
            "compression_office_report", "compression_office_optimize_pdf",
            "compression_office_office_package_security", "compression_office_office_to_pdf",
            "compression_office_closeout_resource_dedup_office_benchmark_closure",
            "crypto_writer_report", "writer_determinism_audit", "writer_external_diff",
            "writer_closeout_report", "pubsec_report", "aes_gcm_report",
            "pdf_mac_report", "pdf_mac_verify", "crypto_tamper_test",
            "signature_validation", "signature_validation_with_evidence",
        ],
        "progress": {
            "status": "engine_tile_progressive_resume_supported",
            "exposed_bindings": [],
            "engine_observable_operations": [
                "progressive_render_job_with_mode",
                "ProgressiveRenderJob::render_next",
                "ProgressiveRenderJob::token"
            ],
            "reason": "Annotation Ocg Rendering adds an engine-level tile checkpoint model; callback-style binding progress APIs remain later binding work."
        },
        "cancellation": {
            "status": "engine_render_cancellation_supported_binding_tokens_later",
            "exposed_bindings": [],
            "engine_observable_operations": [
                "render_page_cancellable",
                "render_display_list_cancellable_with_mode",
                "ProgressiveRenderJob::render_next"
            ],
            "reason": "Engine render internals observe CancelToken and progressive steps return resumable cancellation reports; Python/C/WASM/.NET/Java binding-level cancellation tokens remain later work."
        },
    });
    serde_json::to_string(&json!({
        "schema_version": REPORT_ENVELOPE_VERSION,
        "kind": "feature_report",
        "report": features,
    }))
    .map_err(json_err)
}

fn runtime_config_from_json(config_json: Option<&str>) -> Result<crate::runtime::RuntimeConfig> {
    match config_json {
        Some(raw) if !raw.trim().is_empty() => crate::runtime::RuntimeConfig::from_config_str(raw),
        _ => Ok(crate::runtime::RuntimeConfig::standard()),
    }
}

/// Effective runtime configuration and capability report. `config_json` accepts
/// the shared JSON/TOML-like runtime configuration shape and never serializes
/// secret values.
pub fn runtime_effective_config_json(config_json: Option<&str>) -> Result<String> {
    let cfg = runtime_config_from_json(config_json)?;
    let effective = cfg.effective(
        crate::runtime::HostRuntimeProfile::detect(),
        crate::runtime::HostRuntimePolicy::default(),
    )?;
    envelope("runtime_effective_config", &effective)
}

/// Runtime capability report for the two public modes.
pub fn runtime_capabilities_json(config_json: Option<&str>) -> Result<String> {
    let cfg = runtime_config_from_json(config_json)?;
    let effective = cfg.effective(
        crate::runtime::HostRuntimeProfile::detect(),
        crate::runtime::HostRuntimePolicy::default(),
    )?;
    envelope("runtime_capabilities", &effective.capabilities)
}

/// OCR provider-family matrix. Provider contracts are returned even when the
/// corresponding runtime is not configured.
pub fn ocr_provider_matrix_json() -> Result<String> {
    envelope(
        "ocr_provider_matrix",
        &crate::runtime::ocr_provider_matrix(),
    )
}

/// Validate runtime configuration and report requested-versus-effective mode.
pub fn runtime_validate_config_json(config_json: Option<&str>) -> Result<String> {
    let cfg = runtime_config_from_json(config_json)?;
    cfg.validate()?;
    let effective = cfg.effective(
        crate::runtime::HostRuntimeProfile::detect(),
        crate::runtime::HostRuntimePolicy::default(),
    )?;
    envelope(
        "runtime_config_validation",
        &serde_json::json!({
            "schema_version": crate::runtime::RUNTIME_CONFIG_SCHEMA_VERSION,
            "valid": true,
            "requested_mode": effective.requested_mode,
            "effective_mode": effective.effective_mode,
            "decisions": effective.decisions,
            "secret_values_serialized": effective.secret_hygiene.secret_values_serialized
        }),
    )
}

/// Compact Standard-mode resource-contract probe for validation harnesses.
pub fn standard_runtime_probe_json(vcpu: u16, ram_bytes: u64) -> Result<String> {
    let report = crate::runtime::standard_validation_probe(crate::runtime::HostRuntimeProfile {
        vcpu: vcpu.max(1),
        ram_bytes,
        gpu_present: false,
        wasm: cfg!(target_arch = "wasm32"),
    })?;
    envelope("standard_runtime_probe", &report)
}

// ── Output-producing operations (bytes + report) ─────────────────────────────

/// Build a PDF overlay preview of supported XFA layout and return the PDF bytes
/// plus the stable XFA Runtime flatten/render report envelope.
pub fn xfa_render_preview_json(
    bytes: &[u8],
    script_policy: Option<&str>,
    execute_events: bool,
    dpi: u32,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let input = mutation_input(bytes, password)?;
    let runtime = crate::xfa::XfaRuntimeOptions {
        script_policy: parse_xfa_script_policy(script_policy)?,
        execute_supported_events: execute_events,
        ..crate::xfa::XfaRuntimeOptions::default()
    };
    let (out, report) = crate::xfa::xfa_render_preview_pdf(&input, &runtime, dpi.max(1))?;
    Ok((out, envelope("xfa_render_report", &report)?))
}

/// Flatten the supported static XFA subset using an explicit XFA Runtime mode.
pub fn xfa_flatten_json(
    bytes: &[u8],
    mode: Option<&str>,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let input = mutation_input(bytes, password)?;
    let options = crate::xfa::XfaFlattenOptions {
        mode: parse_xfa_flatten_mode(mode)?,
        ..crate::xfa::XfaFlattenOptions::default()
    };
    let (out, report) = crate::xfa::xfa_flatten_pdf(&input, &options)?;
    Ok((out, envelope("xfa_flatten_report", &report)?))
}

/// Apply the dedicated XFA sanitizer policy and return the produced PDF plus a
/// stable report envelope.
pub fn xfa_sanitize_json(
    bytes: &[u8],
    mode: Option<&str>,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let input = mutation_input(bytes, password)?;
    let options = crate::xfa::XfaSanitizerOptions {
        mode: parse_xfa_sanitizer_mode(mode)?,
        ..crate::xfa::XfaSanitizerOptions::default()
    };
    let (out, report) = crate::xfa::sanitize_xfa_pdf(&input, &options)?;
    Ok((out, envelope("xfa_sanitize_report", &report)?))
}

/// Export annotation XFDF plus a deterministic annotation/media redaction report.
pub fn annotation_xfdf_export_json(
    bytes: &[u8],
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let engine = open(bytes, password)?;
    let (xfdf, report) = crate::annotation_media_redaction::export_annotation_xfdf(&engine)?;
    Ok((xfdf, envelope("annotation_xfdf_export_report", &report)?))
}

/// Import annotation XFDF. Options are the serialized
/// `AnnotationXfdfImportOptions` object shared by every binding.
pub fn annotation_xfdf_import_json(
    bytes: &[u8],
    xfdf: &[u8],
    options_json: Option<&str>,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let input = mutation_input(bytes, password)?;
    let options = options_json
        .map(serde_json::from_str)
        .transpose()
        .map_err(json_err)?
        .unwrap_or_default();
    let (output, report) =
        crate::annotation_media_redaction::import_annotation_xfdf_pdf(&input, xfdf, &options)?;
    Ok((output, envelope("annotation_xfdf_import_report", &report)?))
}

/// Generate deterministic annotation appearances from the canonical PDF
/// annotation dictionaries.
pub fn annotation_appearance_generate_json(
    bytes: &[u8],
    options_json: Option<&str>,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let input = mutation_input(bytes, password)?;
    let options = options_json
        .map(serde_json::from_str)
        .transpose()
        .map_err(json_err)?
        .unwrap_or_default();
    let (output, report) =
        crate::annotation_media_redaction::generate_annotation_appearances_pdf(&input, &options)?;
    Ok((
        output,
        envelope("annotation_appearance_generation_report", &report)?,
    ))
}

/// Inspect appearance generation without returning the generated PDF.
pub fn annotation_appearance_report_json(
    bytes: &[u8],
    options_json: Option<&str>,
    password: Option<&[u8]>,
) -> Result<String> {
    let input = mutation_input(bytes, password)?;
    let options = options_json
        .map(serde_json::from_str)
        .transpose()
        .map_err(json_err)?
        .unwrap_or_default();
    let (_, report) =
        crate::annotation_media_redaction::generate_annotation_appearances_pdf(&input, &options)?;
    envelope("annotation_appearance_report", &report)
}

/// Apply one of the explicit annotation/media redaction rich-media policies.
pub fn rich_media_sanitize_json(
    bytes: &[u8],
    mode: Option<&str>,
    custom_json: Option<&str>,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let input = mutation_input(bytes, password)?;
    let mode = parse_rich_media_mode(mode)?;
    let custom = custom_json
        .map(serde_json::from_str)
        .transpose()
        .map_err(json_err)?
        .unwrap_or_default();
    let (output, report) = crate::annotation_media_redaction::apply_rich_media_policy_pdf(
        &input,
        mode,
        &custom,
        &crate::annotation_media_redaction::RichMediaLimits::default(),
    )?;
    Ok((output, envelope("rich_media_policy_report", &report)?))
}

/// Flatten safe static media posters and remove the active annotations and
/// payloads. No media codec or player is invoked.
pub fn rich_media_flatten_poster_json(
    bytes: &[u8],
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    rich_media_sanitize_json(bytes, Some("flatten_static_poster"), None, password)
}

/// Apply the annotation/media redaction non-axis polygon redaction plan.
pub fn nonaxis_redaction_apply_json(
    bytes: &[u8],
    options_json: &str,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let input = mutation_input(bytes, password)?;
    let options: crate::annotation_media_redaction::NonAxisRedactionOptions =
        serde_json::from_str(options_json).map_err(json_err)?;
    let (output, report) =
        crate::annotation_media_redaction::apply_nonaxis_image_redaction_pdf(&input, &options)?;
    Ok((output, envelope("nonaxis_redaction_apply_report", &report)?))
}

/// Apply secure image-mask/soft-mask redaction using the canonical editor.
pub fn redact_image_mask_json(
    bytes: &[u8],
    options_json: &str,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let input = mutation_input(bytes, password)?;
    let options: crate::annotation_media_redaction::NonAxisRedactionOptions =
        serde_json::from_str(options_json).map_err(json_err)?;
    let (output, report) = crate::secure_mutation::redact_masked_images_pdf(&input, &options)?;
    Ok((output, envelope("mask_redaction_apply_report", &report)?))
}

/// Apply secure inline-image redaction. The shared editor rewrites bounded
/// decoded samples and removes/fails closed for unsupported chains.
pub fn redact_inline_image_json(
    bytes: &[u8],
    options_json: &str,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    redact_image_mask_json(bytes, options_json, password)
}

pub fn associated_files_extract_json(
    bytes: &[u8],
    stable_id: &str,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let engine = open(bytes, password)?;
    let (filename, payload) = crate::secure_mutation::associated_file_extract(&engine, stable_id)?;
    let report = envelope(
        "associated_files_extract_report",
        &json!({
            "schema_version": crate::secure_mutation::SECURE_MUTATION_SCHEMA_VERSION,
            "stable_id": stable_id,
            "safe_filename": filename,
            "bytes": payload.len(),
            "external_access": false,
            "execution": false
        }),
    )?;
    Ok((payload, report))
}

pub fn associated_files_add_json(
    bytes: &[u8],
    payload: &[u8],
    options_json: &str,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let input = mutation_input(bytes, password)?;
    let options: crate::secure_mutation::AssociatedFileAddRequest =
        serde_json::from_str(options_json).map_err(json_err)?;
    let (output, report) =
        crate::secure_mutation::associated_files_add_pdf(&input, &options, payload)?;
    Ok((output, envelope("associated_files_add_report", &report)?))
}

pub fn associated_files_sanitize_json(
    bytes: &[u8],
    options_json: Option<&str>,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let input = mutation_input(bytes, password)?;
    let options = options_json
        .map(serde_json::from_str)
        .transpose()
        .map_err(json_err)?
        .unwrap_or_default();
    let (output, report) = crate::secure_mutation::associated_files_sanitize_pdf(&input, &options)?;
    Ok((
        output,
        envelope("associated_files_sanitize_report", &report)?,
    ))
}

pub fn associated_files_remove_json(
    bytes: &[u8],
    stable_ids: &[String],
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let input = mutation_input(bytes, password)?;
    let options = crate::secure_mutation::AssociatedFileSanitizerOptions {
        remove_ids: stable_ids.iter().cloned().collect(),
        ..crate::secure_mutation::AssociatedFileSanitizerOptions::default()
    };
    let (output, report) = crate::secure_mutation::associated_files_sanitize_pdf(&input, &options)?;
    Ok((output, envelope("associated_files_remove_report", &report)?))
}

pub fn associated_files_remove_owner_json(
    bytes: &[u8],
    options_json: &str,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let input = mutation_input(bytes, password)?;
    let options: crate::secure_mutation::AssociatedFileOwnerRemoveRequest =
        serde_json::from_str(options_json).map_err(json_err)?;
    let (output, report) =
        crate::secure_mutation::associated_files_remove_owner_pdf(&input, &options)?;
    Ok((
        output,
        envelope("associated_files_remove_owner_report", &report)?,
    ))
}

pub fn associated_files_update_owner_json(
    bytes: &[u8],
    payload: &[u8],
    options_json: &str,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let input = mutation_input(bytes, password)?;
    let options: crate::secure_mutation::AssociatedFileOwnerUpdateRequest =
        serde_json::from_str(options_json).map_err(json_err)?;
    let (output, report) =
        crate::secure_mutation::associated_files_update_owner_pdf(&input, &options, payload)?;
    Ok((
        output,
        envelope("associated_files_update_owner_report", &report)?,
    ))
}

pub fn incremental_form_edit_json(
    bytes: &[u8],
    field_name: &str,
    value: &str,
    signature_policy_override: bool,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let input = mutation_input(bytes, password)?;
    let (output, report) = crate::secure_mutation::incremental_form_value_update_pdf(
        &input,
        field_name,
        value,
        signature_policy_override,
    )?;
    Ok((output, envelope("incremental_form_edit_report", &report)?))
}

/// Pades LTV signature-preserving form-fill plan.
///
/// This is a planning surface only: it parses DocMDP/FieldMDP policy,
/// validates the current signatures with the supplied trust/evidence options,
/// and reports whether the form fill can be attempted as an append-only
/// incremental update without mutating signed bytes.
pub fn signature_preserving_form_plan_json(
    bytes: &[u8],
    field_name: &str,
    value: &str,
    options_json: &str,
    password: Option<&[u8]>,
) -> Result<String> {
    let input = mutation_input(bytes, password)?;
    let options = crate::signature::verify_options_from_json(options_json)?;
    envelope(
        "signature_preserving_edit_plan",
        &crate::secure_mutation::plan_signature_preserving_form_fill(
            &input, field_name, value, &options,
        )?,
    )
}

/// Pades LTV signature-preserving form-fill execution.
///
/// The output is produced through the existing incremental writer, then
/// reopened and revalidated with the same signature options. The report only
/// claims preservation when the original input is an exact byte prefix and the
/// original signatures still validate mathematically after the append.
pub fn signature_preserving_form_edit_json(
    bytes: &[u8],
    field_name: &str,
    value: &str,
    options_json: &str,
    explicit_invalidation_override: bool,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let input = mutation_input(bytes, password)?;
    let options = crate::signature::verify_options_from_json(options_json)?;
    let (output, report) = crate::secure_mutation::apply_signature_preserving_form_fill(
        &input,
        field_name,
        value,
        &options,
        explicit_invalidation_override,
    )?;
    Ok((
        output,
        envelope("signature_preserving_edit_result", &report)?,
    ))
}

pub fn incremental_annotation_edit_json(
    bytes: &[u8],
    options_json: &str,
    signature_policy_override: bool,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let input = mutation_input(bytes, password)?;
    let options: crate::secure_mutation::IncrementalAnnotationEdit =
        serde_json::from_str(options_json).map_err(json_err)?;
    let (output, report) = crate::secure_mutation::incremental_annotation_update_pdf(
        &input,
        &options,
        signature_policy_override,
    )?;
    Ok((
        output,
        envelope("incremental_annotation_edit_report", &report)?,
    ))
}

pub fn incremental_page_property_edit_json(
    bytes: &[u8],
    options_json: &str,
    signature_policy_override: bool,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let input = mutation_input(bytes, password)?;
    let options: crate::secure_mutation::IncrementalPagePropertyEdit =
        serde_json::from_str(options_json).map_err(json_err)?;
    let (output, report) = crate::secure_mutation::incremental_page_property_update_pdf(
        &input,
        &options,
        signature_policy_override,
    )?;
    Ok((
        output,
        envelope("incremental_page_property_edit_report", &report)?,
    ))
}

pub fn incremental_metadata_update_json(
    bytes: &[u8],
    key: &str,
    value: &str,
    signature_policy_override: bool,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let input = mutation_input(bytes, password)?;
    let (output, report) = crate::secure_mutation::incremental_metadata_update_pdf(
        &input,
        key,
        value,
        signature_policy_override,
    )?;
    Ok((output, envelope("incremental_edit_report", &report)?))
}

fn mutation_input(bytes: &[u8], password: Option<&[u8]>) -> Result<Vec<u8>> {
    if password.is_none_or(|password| password.is_empty()) {
        return Ok(bytes.to_vec());
    }
    let engine = open(bytes, password)?;
    canonicalize_pdf(&engine, &CanonicalizeOptions::default()).map(|(out, _)| out)
}

/// Sanitize the document: remove active/risky content per policy and re-scan the
/// output. `policy` is one of `strict` | `balanced` | `preserve-visual`
/// (default `balanced`). Returns the sanitized PDF bytes and a JSON report.
pub fn sanitize_json(
    bytes: &[u8],
    policy: Option<&str>,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let engine = open(bytes, password)?;
    let options = parse_sanitizer_options(policy);
    let (out, report) = sanitize_pdf(&engine, &options)?;
    Ok((out, envelope("sanitize_report", &report)?))
}

/// Canonicalize the document: deterministic full-rewrite copy plus an audit
/// report (input/output SHA-256, object count, signature impact). `date_epoch`,
/// when provided, fixes the source date epoch for reproducibility.
pub fn canonicalize_json(
    bytes: &[u8],
    date_epoch: Option<i64>,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let engine = open(bytes, password)?;
    let options = CanonicalizeOptions {
        fixed_source_date_epoch: date_epoch,
        ..CanonicalizeOptions::default()
    };
    let (out, report) = canonicalize_pdf(&engine, &options)?;
    Ok((out, envelope("canonicalize_report", &report)?))
}

/// Apply true redaction of every occurrence of the given `terms` (case
/// insensitive), full-rewrite the document, and verify the terms are absent from
/// the output. Returns the redacted PDF bytes and a JSON report embedding the
/// verification. `strict = true` causes an error if any term survives.
pub fn redact_terms_json(
    bytes: &[u8],
    terms: &[String],
    strict: bool,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)> {
    let engine = open(bytes, password)?;
    let terms: Vec<String> = terms
        .iter()
        .filter(|t| !t.trim().is_empty())
        .cloned()
        .collect();
    if terms.is_empty() {
        return Err(crate::WellfriendError::invalid_input(
            "redact_terms requires at least one non-empty term",
        ));
    }
    let pages: Vec<usize> = (1..=engine.page_count()?).collect();

    let mut editor = PdfEditor::open_bytes(engine.document().reader().file_bytes().to_vec())?;
    let options = RedactionOptions::default();
    let mut applied: Vec<serde_json::Value> = Vec::new();
    for term in &terms {
        let matches = engine.search_text(
            &pages,
            term,
            TextSearchOptions {
                case_sensitive: false,
                include_hidden: true,
                ..TextSearchOptions::default()
            },
        )?;
        for hit in matches {
            if let Some(rect) = redaction_rect_from_quads(&hit.quads) {
                editor.redact(hit.page, rect, options.clone())?;
                applied.push(json!({
                    "term": term,
                    "page": hit.page,
                    "rect": [rect.x, rect.y, rect.width, rect.height],
                }));
            }
        }
    }
    if applied.is_empty() {
        return Err(crate::WellfriendError::invalid_input(
            "redact_terms found no matching text to redact",
        ));
    }
    let out = editor.save_to_bytes(EditMode::FullRewrite)?;
    let verification = redaction_verification_report(&out, &terms)?;
    if strict && !verification.verified_absent {
        return Err(crate::WellfriendError::invalid_input(
            "strict redaction verification failed: a requested term remains extractable",
        ));
    }
    let report = json!({
        "schema_version": REPORT_ENVELOPE_VERSION,
        "kind": "redaction_report",
        "report": {
            "terms": terms,
            "applied": applied,
            "output_bytes": out.len(),
            "verified_absent": verification.verified_absent,
            "verification": verification,
        }
    });
    Ok((out, serde_json::to_string(&report).map_err(json_err)?))
}

// ── Enum parse helpers (string → engine enum, with honest defaults) ──────────

fn parse_rich_media_mode(
    value: Option<&str>,
) -> Result<crate::annotation_media_redaction::RichMediaPolicyMode> {
    use crate::annotation_media_redaction::RichMediaPolicyMode;
    match value.map(str::to_ascii_lowercase).as_deref() {
        None | Some("remove_active_content" | "remove-active-content") => {
            Ok(RichMediaPolicyMode::RemoveActiveContent)
        }
        Some("inventory_only" | "inventory-only") => Ok(RichMediaPolicyMode::InventoryOnly),
        Some("preserve_inert" | "preserve-inert") => Ok(RichMediaPolicyMode::PreserveInert),
        Some("remove_all_media" | "remove-all-media") => {
            Ok(RichMediaPolicyMode::RemoveAllMedia)
        }
        Some("flatten_static_poster" | "flatten-static-poster") => {
            Ok(RichMediaPolicyMode::FlattenStaticPoster)
        }
        Some("custom") => Ok(RichMediaPolicyMode::Custom),
        Some(other) => Err(crate::WellfriendError::invalid_input(format!(
            "unknown rich-media policy '{other}'; use inventory_only, preserve_inert, remove_active_content, remove_all_media, flatten_static_poster, or custom"
        ))),
    }
}

fn parse_secure_mutation_edit_operation(
    value: &str,
) -> Result<crate::secure_mutation::EditOperation> {
    use crate::secure_mutation::EditOperation;
    match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "form_value" | "form_value_update" => Ok(EditOperation::FormValueUpdate),
        "form_appearance" | "form_appearance_update" => Ok(EditOperation::FormAppearanceUpdate),
        "annotation_add" => Ok(EditOperation::AnnotationAdd),
        "annotation_update" => Ok(EditOperation::AnnotationUpdate),
        "annotation_delete" => Ok(EditOperation::AnnotationDelete),
        "xfdf_import" => Ok(EditOperation::XfdfImport),
        "page_insert" => Ok(EditOperation::PageInsert),
        "page_delete" => Ok(EditOperation::PageDelete),
        "page_reorder" => Ok(EditOperation::PageReorder),
        "page_rotate" => Ok(EditOperation::PageRotate),
        "page_box" | "page_box_change" => Ok(EditOperation::PageBoxChange),
        "content_edit" => Ok(EditOperation::ContentEdit),
        "redaction" => Ok(EditOperation::Redaction),
        "sanitizer" => Ok(EditOperation::Sanitizer),
        "attachment_add" => Ok(EditOperation::AttachmentAdd),
        "attachment_remove" => Ok(EditOperation::AttachmentRemove),
        "xfa_flatten" => Ok(EditOperation::XfaFlatten),
        "metadata" | "metadata_update" => Ok(EditOperation::MetadataUpdate),
        "canonicalize" => Ok(EditOperation::Canonicalize),
        "full_rewrite" => Ok(EditOperation::FullRewrite),
        "incremental" | "incremental_save" => Ok(EditOperation::IncrementalSave),
        other => Err(crate::WellfriendError::MalformedPdf(format!(
            "unknown secure mutation edit operation {other}"
        ))),
    }
}

fn parse_xfa_script_policy(value: Option<&str>) -> Result<crate::xfa::XfaScriptPolicy> {
    match value.map(str::to_ascii_lowercase).as_deref() {
        None | Some("disabled" | "off" | "none") => Ok(crate::xfa::XfaScriptPolicy::Disabled),
        Some("formcalc-safe-subset" | "formcalc_safe_subset" | "formcalc") => {
            Ok(crate::xfa::XfaScriptPolicy::FormCalcSafeSubset)
        }
        Some(other) => Err(crate::WellfriendError::invalid_input(format!(
            "unknown XFA script policy '{other}'; use disabled or formcalc-safe-subset"
        ))),
    }
}

fn parse_xfa_flatten_mode(value: Option<&str>) -> Result<crate::xfa::XfaFlattenMode> {
    match value.map(str::to_ascii_lowercase).as_deref() {
        None | Some("extract-only" | "extract_only") => Ok(crate::xfa::XfaFlattenMode::ExtractOnly),
        Some("render-preview" | "render_preview") => Ok(crate::xfa::XfaFlattenMode::RenderPreview),
        Some("flatten-supported-static" | "flatten_supported_static") => {
            Ok(crate::xfa::XfaFlattenMode::FlattenSupportedStatic)
        }
        Some("flatten-and-remove-xfa" | "flatten_and_remove_xfa") => {
            Ok(crate::xfa::XfaFlattenMode::FlattenAndRemoveXfa)
        }
        Some("preserve-unsupported-xfa-report-only" | "preserve_unsupported_xfa_report_only") => {
            Ok(crate::xfa::XfaFlattenMode::PreserveUnsupportedXfaReportOnly)
        }
        Some("fail-on-unsupported" | "fail_on_unsupported") => {
            Ok(crate::xfa::XfaFlattenMode::FailOnUnsupported)
        }
        Some(other) => Err(crate::WellfriendError::invalid_input(format!(
            "unknown XFA flatten mode '{other}'"
        ))),
    }
}

fn parse_xfa_sanitizer_mode(value: Option<&str>) -> Result<crate::xfa::XfaSanitizerMode> {
    match value.map(str::to_ascii_lowercase).as_deref() {
        None | Some("remove-scripts-events-connections" | "remove_scripts_events_connections") => {
            Ok(crate::xfa::XfaSanitizerMode::RemoveScriptsEventsConnections)
        }
        Some("remove-all-xfa" | "remove_all_xfa" | "strict") => {
            Ok(crate::xfa::XfaSanitizerMode::RemoveAllXfa)
        }
        Some("preserve-static-data" | "preserve_static_data") => {
            Ok(crate::xfa::XfaSanitizerMode::PreserveStaticData)
        }
        Some("flatten-then-remove" | "flatten_then_remove") => {
            Ok(crate::xfa::XfaSanitizerMode::FlattenThenRemove)
        }
        Some(other) => Err(crate::WellfriendError::invalid_input(format!(
            "unknown XFA sanitizer mode '{other}'"
        ))),
    }
}

fn parse_parser_mode(value: Option<&str>) -> ParserMode {
    match value.map(str::to_ascii_lowercase).as_deref() {
        Some("strict") => ParserMode::Strict,
        Some("audit") => ParserMode::Audit,
        _ => ParserMode::Repair,
    }
}

fn parse_color_profile(value: Option<&str>) -> ColorValidationProfile {
    match value.map(str::to_ascii_lowercase).as_deref() {
        Some("pdfa") | Some("pdf/a") | Some("pdf-a") => ColorValidationProfile::PdfA,
        Some("pdfx") | Some("pdf/x") | Some("pdf-x") => ColorValidationProfile::PdfX,
        _ => ColorValidationProfile::Generic,
    }
}

fn parse_pdfa_profile(value: Option<&str>) -> PdfAProfile {
    match value.map(str::to_ascii_lowercase).as_deref() {
        Some("pdfa1b") | Some("pdf/a-1b") | Some("1b") => PdfAProfile::PdfA1B,
        Some("pdfa2a") | Some("pdf/a-2a") | Some("2a") => PdfAProfile::PdfA2A,
        Some("pdfa3b") | Some("pdf/a-3b") | Some("3b") => PdfAProfile::PdfA3B,
        Some("pdfa3a") | Some("pdf/a-3a") | Some("3a") => PdfAProfile::PdfA3A,
        _ => PdfAProfile::PdfA2B,
    }
}

fn parse_sanitizer_options(policy: Option<&str>) -> SanitizerOptions {
    match policy.map(str::to_ascii_lowercase).as_deref() {
        Some("strict") => SanitizerOptions::strict(),
        Some("preserve-visual") | Some("preserve_visual") => SanitizerOptions::preserve_visual(),
        _ => SanitizerOptions::balanced(),
    }
}

/// Union of the text-match quads padded slightly, matching the CLI redaction
/// rectangle derivation so redaction geometry is identical across surfaces.
fn redaction_rect_from_quads(quads: &[TextQuad]) -> Option<ImageRect> {
    let bbox = TextQuad::union(quads)?;
    let pad = 0.5;
    Some(ImageRect::new(
        bbox.x0 - pad,
        bbox.y0 - pad,
        (bbox.x1 - bbox.x0 + pad * 2.0).max(0.1),
        (bbox.y1 - bbox.y0 + pad * 2.0).max(0.1),
    ))
}

#[cfg(test)]
mod tests {
    //! Downstream-style tests: exercise the facade the way a binding does —
    //! bytes in, versioned-JSON out, and assert the envelope + a report field.
    use super::*;

    fn fixture() -> Vec<u8> {
        std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/multi_stream.pdf"
        ))
        .expect("fixture present")
    }

    fn parse(json: &str) -> serde_json::Value {
        serde_json::from_str(json).expect("valid JSON")
    }

    fn assert_envelope(json: &str, kind: &str) -> serde_json::Value {
        let v = parse(json);
        assert_eq!(v["schema_version"], REPORT_ENVELOPE_VERSION);
        assert_eq!(v["kind"], kind);
        assert!(v.get("report").is_some(), "report field present");
        v
    }

    #[test]
    fn security_report_envelope_and_fields() {
        let v = assert_envelope(
            &security_report_json(&fixture(), None).unwrap(),
            "security_report",
        );
        assert!(v["report"]["encrypted"].is_boolean());
        assert!(v["report"]["findings"].is_array());
    }

    #[test]
    fn parser_report_reports_opened() {
        let v = assert_envelope(
            &parser_report_json(&fixture(), Some("audit"), None).unwrap(),
            "parser_report",
        );
        assert_eq!(v["report"]["opened"], true);
    }

    #[test]
    fn document_info_has_page_count() {
        let v = assert_envelope(
            &document_info_json(&fixture(), None).unwrap(),
            "document_info",
        );
        assert!(v["report"]["page_count"].as_u64().unwrap() >= 1);
    }

    #[test]
    fn color_forms_annotations_pages_interactive_envelopes() {
        let bytes = fixture();
        assert_envelope(
            &color_report_json(&bytes, Some("generic")).unwrap(),
            "color_report",
        );
        assert_envelope(&forms_report_json(&bytes, None).unwrap(), "forms_report");
        assert_envelope(
            &annotation_report_json(&bytes, None).unwrap(),
            "annotation_report",
        );
        assert_envelope(
            &page_operations_report_json(&bytes, None).unwrap(),
            "page_operations_report",
        );
        assert_envelope(
            &interactive_report_json(&bytes, None).unwrap(),
            "interactive_report",
        );
    }

    #[test]
    fn standards_pdfa_pdfua_envelopes() {
        let bytes = fixture();
        assert_envelope(
            &standards_profile_json(&bytes, Some("all"), None).unwrap(),
            "standards_profile",
        );
        assert_envelope(
            &pdfa_validation_json(&bytes, Some("pdfa2b"), None).unwrap(),
            "pdfa_validation",
        );
        assert_envelope(
            &pdfua_validation_json(&bytes, None).unwrap(),
            "pdfua_validation",
        );
    }

    #[test]
    fn font_signature_decode_dedup_feature_envelopes() {
        let bytes = fixture();
        assert_envelope(&font_report_json(&bytes, None).unwrap(), "font_report");
        assert_envelope(
            &signature_report_json(&bytes, None).unwrap(),
            "signature_report",
        );
        let outcome = assert_envelope(
            &signature_validation_with_evidence_json(&bytes, "{}", None).unwrap(),
            "signature_validation_outcome",
        );
        assert!(outcome["report"]["evidence_bundle"].is_object());
        assert_envelope(
            &decode_budget_report_json("DCTDecode", 100, 100, 3).unwrap(),
            "decode_budget_report",
        );
        assert_envelope(
            &resource_dedup_report_json(&[vec![1, 2, 3], vec![1, 2, 3]]).unwrap(),
            "resource_dedup_report",
        );
        let v = assert_envelope(&feature_report_json().unwrap(), "feature_report");
        assert!(v["report"]["engine_version"].is_string());
        assert_eq!(
            v["report"]["codec_boundary"]["scanner"]["default_implementation"],
            "safe_first_byte_chunked"
        );
        assert_eq!(
            v["report"]["codec_boundary"]["renderer_decode_scheduler"]["status"],
            "adopted_for_immediate_renderer_decode_paths"
        );
        assert_eq!(
            v["report"]["codec_boundary"]["native_codec_boundary"]["default_posture"],
            "deny_native_by_default"
        );
        assert_eq!(
            v["report"]["decode_scheduler"]["decode_scheduler"]["status"],
            "adopted_for_decode_scheduler_non_render_decode_paths"
        );
        assert_eq!(
            v["report"]["decode_scheduler"]["hostile_corpus"]["generator"],
            "scripts/decode_scheduler_hostile_codec_corpus.py"
        );
        assert_eq!(
            v["report"]["decode_scheduler"]["fuzz_campaign"]["script"],
            "scripts/decode_scheduler_codec_fuzz_campaign.py"
        );
        assert_eq!(
            v["report"]["native_renderer"]["native_replay"]["status"],
            "native_text_image_form_display_list_foundation"
        );
        assert_eq!(
            v["report"]["native_renderer"]["renderer_parity_audit"]["script"],
            "scripts/native_renderer_renderer_parity_audit.py"
        );
        assert_eq!(
            v["report"]["native_renderer"]["reference_renderer_multi_reference_audit"]["status"],
            "multi_reference_audit_complete"
        );
        assert_eq!(
            v["report"]["native_renderer"]["reference_renderer_multi_reference_audit"]
                ["total_pairwise_comparisons"],
            78
        );
        assert_eq!(
            v["report"]["transparency_rendering_transparency_compositing"]["status"],
            "native_foundation_with_transparency_closeout_closure"
        );
        assert_eq!(
            v["report"]["transparency_rendering_transparency_compositing"]["reference_audit"]
                ["memory_cap_mb"],
            4096
        );
        assert!(
            v["report"]["transparency_rendering_transparency_compositing"]["blend_modes"]
                ["implemented"]
                .as_array()
                .unwrap()
                .iter()
                .any(|mode| mode == "Luminosity")
        );
        assert_eq!(
            v["report"]["transparency_closeout_transparency_closure"]["status"],
            "complete"
        );
        assert_eq!(
            v["report"]["transparency_closeout_transparency_closure"]["reference_audit"]
                ["wellfriendpdf_outlier_failures"],
            0
        );
        assert!(v["report"]["transparency_closeout_transparency_closure"]
            ["luminosity_soft_mask_color_spaces"]["supported"]
            .as_array()
            .unwrap()
            .iter()
            .any(|space| space == "DeviceCMYK"));
        assert_eq!(
            v["report"]["advanced_rendering_text_clipping_shading_patterns"]["status"],
            "native_common_paths_with_bounded_unsupported_reports"
        );
        assert_eq!(
            v["report"]["advanced_rendering_text_clipping_shading_patterns"]["reference_audit"]
                ["memory_cap_mb"],
            4096
        );
        assert!(
            v["report"]["advanced_rendering_text_clipping_shading_patterns"]["text_clipping"]
                ["rendering_modes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|mode| mode.as_i64() == Some(7))
        );
        assert_eq!(
            v["report"]["type3_cid_rendering_type3_cid_tensor_closure"]["status"],
            "complete_native_common_paths_with_reference_cluster_limits"
        );
        assert_eq!(
            v["report"]["type3_cid_rendering_type3_cid_tensor_closure"]["reference_audit"]
                ["wellfriendpdf_outlier_failures"],
            0
        );
        assert_eq!(
            v["report"]["type3_cid_rendering_type3_cid_tensor_closure"]["type7_tensor_patch"]
                ["status"],
            "native_tensor_product_interior"
        );
        assert_eq!(
            v["report"]["annotation_ocg_rendering_annotation_ocg_progressive_cache"]["status"],
            "implemented_with_bounded_unsupported_reports"
        );
        assert_eq!(
            v["report"]["annotation_ocg_rendering_annotation_ocg_progressive_cache"]
                ["optional_content"]["status"],
            "default_view_configuration_evaluator"
        );
        assert_eq!(
            v["report"]["annotation_ocg_rendering_annotation_ocg_progressive_cache"]
                ["closure_gates"]["memory_cap_mb"],
            4096
        );
        assert_eq!(
            v["report"]["renderer_validation_annotation_progressive_cache_validation"]["status"],
            "implemented_and_proven"
        );
        assert_eq!(
            v["report"]["renderer_validation_annotation_progressive_cache_validation"]
                ["multi_reference_audit"]["wellfriendpdf_outlier_failures"],
            0
        );
        assert_eq!(
            v["report"]["renderer_validation_annotation_progressive_cache_validation"]
                ["public_report_parity"]["schema_change"],
            "additive_section_only"
        );
        assert_eq!(
            v["report"]["multilingual_color_glyphs_cjk_rtl_color_glyph_reference_harness"]
                ["status"],
            "implemented_with_bounded_unsupported_reports"
        );
        assert_eq!(
            v["report"]["multilingual_color_glyphs_cjk_rtl_color_glyph_reference_harness"]
                ["closure_gates"]["memory_cap_mb"],
            4096
        );
        assert_eq!(
            v["report"]["multilingual_color_glyphs_cjk_rtl_color_glyph_reference_harness"]
                ["color_glyph_rendering"]["status"],
            "unsupported_color_tables_are_detected_and_reported"
        );
        assert_eq!(
            v["report"]["cjk_rtl_color_glyph_closeout_color_glyph_cjk_rtl_fidelity_closure"]
                ["status"],
            "complete"
        );
        assert_eq!(
            v["report"]["cjk_rtl_color_glyph_closeout_color_glyph_cjk_rtl_fidelity_closure"]
                ["color_glyph_rendering"]["colr_cpal"]["status"],
            "implemented_and_proven"
        );
        assert_eq!(
            v["report"]["cjk_rtl_color_glyph_closeout_color_glyph_cjk_rtl_fidelity_closure"]
                ["multi_reference_audit"]["wellfriendpdf_outlier_failures"],
            0
        );
        assert_eq!(
            v["report"]["color_glyph_hinting_color_glyph_hinting_cff_closure"]["status"],
            "complete"
        );
        assert_eq!(
            v["report"]["color_glyph_hinting_color_glyph_hinting_cff_closure"]["colrv1"]["status"],
            "implemented_with_operator_level_limits"
        );
        assert_eq!(
            v["report"]["color_glyph_hinting_color_glyph_hinting_cff_closure"]
                ["multi_reference_audit"]["wellfriendpdf_outlier_failures"],
            0
        );
        assert_eq!(
            v["report"]["color_glyph_hinting_color_glyph_hinting_cff_closure"]["closure_gates"]
                ["public_report_schema"],
            "additive_feature_report_color_glyph_hinting"
        );
        assert_eq!(
            v["report"]["colrv_svg_bitmap_full_colrv1_svg_color_glyph_closure"]["status"],
            "complete"
        );
        assert_eq!(
            v["report"]["colrv_svg_bitmap_full_colrv1_svg_color_glyph_closure"]["svg_in_opentype"]
                ["status"],
            "safe_static_subset_rendered_active_constructs_blocked"
        );
        assert_eq!(
            v["report"]["colrv_svg_bitmap_full_colrv1_svg_color_glyph_closure"]
                ["bitmap_color_glyphs"]["sbix"]["status"],
            "png_and_jpeg_rendered_tiff_other_precisely_reported"
        );
        assert_eq!(
            v["report"]["colrv_svg_bitmap_full_colrv1_svg_color_glyph_closure"]["closure_gates"]
                ["public_report_schema"],
            "additive_feature_report_colrv_svg_bitmap"
        );
        assert_eq!(
            v["report"]["colrv_gradient_composite_colrv1_gradient_clip_composite_closure"]
                ["status"],
            "complete"
        );
        assert_eq!(
            v["report"]["colrv_gradient_composite_colrv1_gradient_clip_composite_closure"]
                ["colrv1_gradients"]["implemented_operators"][0],
            "PaintLinearGradient"
        );
        assert_eq!(
            v["report"]["colrv_gradient_composite_colrv1_gradient_clip_composite_closure"]
                ["colrv1_clip_stack"]["status"],
            "implemented"
        );
        assert_eq!(
            v["report"]["colrv_gradient_composite_colrv1_gradient_clip_composite_closure"]
                ["colrv1_composites"]["implemented_modes"][1],
            "Multiply"
        );
        assert_eq!(
            v["report"]["colrv_gradient_composite_colrv1_gradient_clip_composite_closure"]
                ["closure_gates"]["public_report_schema"],
            "additive_feature_report_colrv_gradient_composite"
        );
        assert_eq!(
            v["report"]["porterduff_radial_color_glyph_colrv1_porterduff_radial_closure"]["status"],
            "complete"
        );
        assert_eq!(
            v["report"]["porterduff_radial_color_glyph_colrv1_porterduff_radial_closure"]
                ["porter_duff_plus_composites"]["implemented_modes"]
                .as_array()
                .unwrap()
                .len(),
            12
        );
        assert_eq!(
            v["report"]["porterduff_radial_color_glyph_colrv1_porterduff_radial_closure"]
                ["exact_moving_center_radial"]["status"],
            "implemented_with_reference_equivalence"
        );
        assert_eq!(
            v["report"]["porterduff_radial_color_glyph_colrv1_porterduff_radial_closure"]
                ["closure_gates"]["public_report_schema"],
            "additive_feature_report_porterduff_radial_color_glyph"
        );
        assert_eq!(
            v["report"]["renderer_fuzz_cmm_renderer_fuzz_cmm_closeout"]["status"],
            "complete_with_native_cmm_hard_blocked_precise"
        );
        assert_eq!(
            v["report"]["renderer_fuzz_cmm_renderer_fuzz_cmm_closeout"]["renderer_fuzz"]
                ["fuzz_target_count"],
            25
        );
        assert_eq!(
            v["report"]["renderer_fuzz_cmm_renderer_fuzz_cmm_closeout"]["renderer_closeout"]
                ["wellfriendpdf_outlier_failures"],
            0
        );
        assert_eq!(
            v["report"]["renderer_fuzz_cmm_renderer_fuzz_cmm_closeout"]["native_cmm_backend"]
                ["backend_used_in_current_build"],
            "safe-rust-plus-qcms"
        );
        assert_eq!(
            v["report"]["renderer_fuzz_cmm_renderer_fuzz_cmm_closeout"]["closure_gates"]
                ["public_report_schema"],
            "additive_feature_report_renderer_fuzz_cmm"
        );
        let native_cmm_backend =
            &v["report"]["native_cmm_backend_native_littlecms_cmm_backend_closure"];
        assert_eq!(native_cmm_backend["status"], "complete");
        assert_eq!(
            native_cmm_backend["feature_flag"]["name"],
            "native-cmm-lcms2"
        );
        assert_eq!(
            native_cmm_backend["native_cmm_compiled"],
            cfg!(feature = "native-cmm-lcms2")
        );
        assert_eq!(
            native_cmm_backend["native_cmm_available_at_runtime"],
            cfg!(all(
                feature = "native-cmm-lcms2",
                not(target_arch = "wasm32")
            ))
        );
        assert_eq!(
            native_cmm_backend["closure_gates"]["public_report_schema"],
            "additive_feature_report_native_cmm_backend"
        );
        let prepress_cmm = &v["report"]["prepress_cmm_prepress_cmm_device_link_separation_plates"];
        assert_eq!(prepress_cmm["status"], "complete");
        assert_eq!(
            prepress_cmm["closure_gates"]["public_report_schema"],
            "additive_feature_report_prepress_cmm"
        );
        assert_eq!(
            prepress_cmm["native_cmm_compiled"],
            cfg!(feature = "native-cmm-lcms2")
        );
        assert_eq!(
            prepress_cmm["separation_framebuffer"]["cache_key_includes_plate_state"],
            true
        );
        let nchannel_plate_prepress =
            &v["report"]["nchannel_plate_prepress_nchannel_plate_reference_closure"];
        assert_eq!(nchannel_plate_prepress["status"], "complete");
        assert_eq!(
            nchannel_plate_prepress["closure_gates"]["public_report_schema"],
            "additive_feature_report_nchannel_plate_prepress"
        );
        assert_eq!(
            nchannel_plate_prepress["nchannel_pixel_format"]["max_channels"],
            prepress::MAX_NCHANNEL_OUTPUT_CHANNELS
        );
        assert_eq!(
            nchannel_plate_prepress["reference_audit"]["pdfium"],
            "required_and_run_by_nchannel_plate_prepress_audit"
        );
        let prepress_proofing = &v["report"]["prepress_proofing_full_overprint_prepress_closeout"];
        assert_eq!(prepress_proofing["status"], "complete");
        assert_eq!(
            prepress_proofing["closure_gates"]["public_report_schema"],
            "additive_feature_report_prepress_proofing"
        );
        assert_eq!(
            prepress_proofing["reference_audit"]["wellfriendpdf_outlier_failures"],
            0
        );
        assert_eq!(
            prepress_proofing["reference_audit"]["unclassified_failures"],
            0
        );
        let semantic_intelligence =
            &v["report"]["semantic_intelligence_semantic_intelligence_parenttree_cjk_ml_layout"];
        assert_eq!(semantic_intelligence["status"], "complete");
        assert_eq!(
            semantic_intelligence["closure_gates"]["public_report_schema"],
            "additive_feature_report_semantic_intelligence"
        );
        assert_eq!(
            semantic_intelligence["privacy_defaults"]["cloud_upload_default"],
            false
        );
        assert_eq!(
            semantic_intelligence["ml_layout_hook"]["can_delete_deterministic_text"],
            false
        );
        let cjk_dictionary_layout =
            &v["report"]["cjk_dictionary_layout_cjk_dictionary_layout_backend_closure"];
        assert_eq!(cjk_dictionary_layout["status"], "complete");
        assert_eq!(
            cjk_dictionary_layout["closure_gates"]["public_report_schema"],
            "additive_feature_report_cjk_dictionary_layout"
        );
        assert_eq!(
            cjk_dictionary_layout["dictionary_provider"]["external_pack_support"],
            "implemented"
        );
        assert_eq!(
            cjk_dictionary_layout["layout_backend"]["local_backend_status"],
            "unsupported_reported_no_runtime"
        );
        assert_eq!(
            cjk_dictionary_layout["layout_backend"]["privacy_posture"]["cloud_upload_default"],
            false
        );
        let semantic_closeout =
            &v["report"]["semantic_closeout_semantic_binding_rag_benchmark_closeout"];
        assert_eq!(semantic_closeout["status"], "complete");
        assert_eq!(
            semantic_closeout["closure_gates"]["public_report_schema"],
            "additive_feature_report_semantic_closeout"
        );
        assert_eq!(semantic_closeout["closure_gates"]["blocked_count"], 0);
        assert_eq!(semantic_closeout["privacy"]["cloud_upload_default"], false);
        assert_eq!(
            semantic_closeout["tableformer_table_transformer_hook"]
                ["model_can_rewrite_deterministic_text"],
            false
        );
        let compression_office_closeout =
            &v["report"]["compression_office_closeout_resource_dedup_office_benchmark_closure"];
        assert_eq!(
            compression_office_closeout["status"],
            "implemented_with_limits"
        );
        assert_eq!(
            compression_office_closeout["closure_audit"]["blocked_rows"],
            0
        );
        assert_eq!(
            compression_office_closeout["dedup_families"]["unsafe_merge_count"],
            0
        );
        assert_eq!(
            compression_office_closeout["office_benchmark"]
                ["production_external_converter_invoked"],
            false
        );
        let crypto_writer = &v["report"]["crypto_writer_deterministic_writer_pubsec_aesgcm"];
        assert_eq!(crypto_writer["status"], "implemented_with_limits");
        assert_eq!(crypto_writer["blocked_rows"], 0);
        assert_eq!(
            crypto_writer["public_key_handler_status"],
            "implemented_with_limits"
        );
        assert_eq!(
            crypto_writer["aes_gcm_decrypt_status"],
            "implemented_with_limits"
        );
        let signature_validation =
            &v["report"]["signature_validation_certificate_trust_pades_ocsp_crl_validation"];
        assert_eq!(
            signature_validation["status"],
            "implemented_with_limits_not_release_attested"
        );
        assert_eq!(
            signature_validation["signer_certificate_resolution"],
            "implemented_exact_match_no_arbitrary_fallback"
        );
        assert_eq!(signature_validation["retrieval"]["default"], "offline");
        assert_eq!(
            signature_validation["release_attestation"]["final_closure_commit"],
            "absent"
        );
        assert_envelope(
            &annotation_ocg_rendering_renderer_report_json().unwrap(),
            "annotation_ocg_rendering_renderer_report",
        );
        assert_envelope(
            &renderer_validation_validation_report_json().unwrap(),
            "renderer_validation_validation_report",
        );
        assert_envelope(
            &multilingual_color_glyphs_renderer_report_json().unwrap(),
            "multilingual_color_glyphs_renderer_report",
        );
        assert_envelope(
            &cjk_rtl_color_glyph_closeout_closure_report_json().unwrap(),
            "cjk_rtl_color_glyph_closeout_closure_report",
        );
        assert_envelope(
            &color_glyph_hinting_closure_report_json().unwrap(),
            "color_glyph_hinting_closure_report",
        );
        assert_envelope(
            &colrv_svg_bitmap_closure_report_json().unwrap(),
            "colrv_svg_bitmap_closure_report",
        );
        assert_envelope(
            &colrv_gradient_composite_closure_report_json().unwrap(),
            "colrv_gradient_composite_closure_report",
        );
        assert_envelope(
            &porterduff_radial_color_glyph_closure_report_json().unwrap(),
            "porterduff_radial_color_glyph_closure_report",
        );
        assert_eq!(
            v["report"]["progress"]["status"],
            "engine_tile_progressive_resume_supported"
        );
        assert_eq!(
            v["report"]["cancellation"]["status"],
            "engine_render_cancellation_supported_binding_tokens_later"
        );
        assert!(v["report"]["cancellation"]["engine_observable_operations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|op| op == "render_page_cancellable"));
    }

    #[test]
    fn text_semantic_and_chunk_envelopes() {
        let bytes = fixture();
        assert_envelope(
            &text_semantic_json(&bytes, &[], None).unwrap(),
            "text_semantic",
        );
        assert_envelope(&chunk_report_json(&bytes, None).unwrap(), "chunk_set");
    }

    #[test]
    fn sanitize_produces_bytes_and_report() {
        let (out, report) = sanitize_json(&fixture(), Some("balanced"), None).unwrap();
        assert!(out.starts_with(b"%PDF-"));
        let v = assert_envelope(&report, "sanitize_report");
        assert!(v["report"]["output_bytes"].as_u64().unwrap() > 0);
    }

    #[test]
    fn canonicalize_produces_bytes_and_report() {
        let (out, report) = canonicalize_json(&fixture(), Some(0), None).unwrap();
        assert!(out.starts_with(b"%PDF-"));
        let v = assert_envelope(&report, "canonicalize_report");
        assert!(v["report"]["output_sha256"].is_string());
        assert_eq!(v["report"]["deterministic"], true);
    }

    #[test]
    fn redact_terms_removes_and_verifies() {
        // "Hello" appears in the fixture; redact it and verify it is gone.
        let (out, report) =
            redact_terms_json(&fixture(), &["Hello".to_string()], false, None).unwrap();
        assert!(out.starts_with(b"%PDF-"));
        let v = assert_envelope(&report, "redaction_report");
        assert!(!v["report"]["applied"].as_array().unwrap().is_empty());
    }

    #[test]
    fn redact_terms_rejects_empty() {
        let err = redact_terms_json(&fixture(), &["   ".to_string()], false, None).unwrap_err();
        assert!(err.to_string().contains("at least one non-empty term"));
    }

    #[test]
    fn bad_bytes_are_reported_not_paniced() {
        assert!(security_report_json(b"not a pdf", None).is_err());
    }
}
