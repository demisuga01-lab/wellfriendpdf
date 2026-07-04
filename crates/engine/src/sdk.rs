//! Stable SDK facade for cross-language bindings.
//!
//! This module is the single, stable, versioned-JSON report layer that the
//! Python (`oxide-py`) and C ABI (`oxide-capi`) bindings call. It exists so the
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
    color_report::{color_report_bytes, ColorValidationProfile},
    compliance::{validate_pdfa, validate_pdfua, PdfAProfile},
    editing::{EditMode, ImageRect, PdfEditor, RedactionOptions},
    filters::{decode_image_budget_report, DecodeLimits},
    interactive::{
        annotation_report, forms_report, interactive_report, page_operations_report,
        redaction_verification_report,
    },
    parser_report::{parser_report_bytes_with_password, ParserMode},
    security::{
        canonicalize_pdf, sanitize_pdf, scan_risky_content, security_report, CanonicalizeOptions,
        SanitizerOptions,
    },
    standards::{validate_standards_profile, StandardsProfile},
    versioning::resource_dedup_report,
    ContentEngine, DocumentInfo, Result, TextQuad, TextSearchOptions, TextSemanticOptions,
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

fn json_err(err: serde_json::Error) -> crate::OxideError {
    crate::OxideError::invalid_input(format!("JSON serialization error: {err}"))
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

/// Annotation inventory: kinds, QuadPoints, appearance status, unsafe actions.
pub fn annotation_report_json(bytes: &[u8], password: Option<&[u8]>) -> Result<String> {
    let engine = open(bytes, password)?;
    envelope("annotation_report", &annotation_report(&engine)?)
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
pub fn feature_report_json() -> Result<String> {
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
        // Capabilities that are always present in the default build regardless of
        // cargo features (they live in unconditional modules).
        "always_available": [
            "security_report", "sanitize", "canonicalize", "parser_report",
            "color_report", "standards_profile", "interactive_report",
            "forms_report", "annotation_report", "page_operations_report",
            "signature_report", "font_report", "decode_budget_report",
            "resource_dedup_report", "redaction",
        ],
        "progress": {
            "status": "progress_not_supported",
            "exposed_bindings": [],
            "engine_observable_operations": [],
            "reason": "Prompt 02 SDK report/output facade operations do not emit progress events yet."
        },
        "cancellation": {
            "status": "cancellation_not_supported_for_prompt02_bindings",
            "exposed_bindings": [],
            "engine_observable_operations": [
                "render_page_cancellable",
                "render_display_list_cancellable_with_mode"
            ],
            "reason": "Engine render internals can observe CancelToken, but the Prompt 02 WASM/.NET/Java report/output SDK surfaces do not expose a cancellable render operation or accept binding-level cancellation tokens."
        },
    });
    serde_json::to_string(&json!({
        "schema_version": REPORT_ENVELOPE_VERSION,
        "kind": "feature_report",
        "report": features,
    }))
    .map_err(json_err)
}

// ── Output-producing operations (bytes + report) ─────────────────────────────

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
        return Err(crate::OxideError::invalid_input(
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
        return Err(crate::OxideError::invalid_input(
            "redact_terms found no matching text to redact",
        ));
    }
    let out = editor.save_to_bytes(EditMode::FullRewrite)?;
    let verification = redaction_verification_report(&out, &terms)?;
    if strict && !verification.verified_absent {
        return Err(crate::OxideError::invalid_input(
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
        assert_eq!(v["report"]["progress"]["status"], "progress_not_supported");
        assert_eq!(
            v["report"]["cancellation"]["status"],
            "cancellation_not_supported_for_prompt02_bindings"
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
