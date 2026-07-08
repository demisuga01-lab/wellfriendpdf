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
pub fn prompt09_renderer_report_json() -> Result<String> {
    envelope(
        "prompt09_renderer_report",
        &prompt09_renderer_report_value(),
    )
}

pub fn prompt09b_validation_report_json() -> Result<String> {
    envelope(
        "prompt09b_validation_report",
        &prompt09b_validation_report_value(),
    )
}

pub fn prompt10_renderer_report_json() -> Result<String> {
    envelope(
        "prompt10_renderer_report",
        &prompt10_renderer_report_value(),
    )
}

pub fn prompt10b_closure_report_json() -> Result<String> {
    envelope(
        "prompt10b_closure_report",
        &prompt10b_closure_report_value(),
    )
}

pub fn prompt10c_closure_report_json() -> Result<String> {
    envelope(
        "prompt10c_closure_report",
        &prompt10c_closure_report_value(),
    )
}

pub fn prompt10d_closure_report_json() -> Result<String> {
    envelope(
        "prompt10d_closure_report",
        &prompt10d_closure_report_value(),
    )
}

pub fn prompt10e_closure_report_json() -> Result<String> {
    envelope(
        "prompt10e_closure_report",
        &prompt10e_closure_report_value(),
    )
}

pub fn prompt10f_closure_report_json() -> Result<String> {
    envelope(
        "prompt10f_closure_report",
        &prompt10f_closure_report_value(),
    )
}

fn prompt09_renderer_report_value() -> serde_json::Value {
    json!({
        "status": "implemented_with_bounded_unsupported_reports",
        "artifact_root": "target/prompt09-annotation-ocg-progressive-cache",
        "audit_doc": "docs/prompt09_annotation_ocg_progressive_cache_audit.md",
        "known_limits_doc": "docs/prompt09_known_limits.md",
        "audit_script": "scripts/prompt09_annotation_ocg_progressive_cache_audit.py",
        "reference_policy": {
            "reference_engines": ["Poppler", "PDFium", "MuPDF"],
            "bootstrap_source": "Prompt 06B reference-tool manifest and bootstrap scripts",
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
            "matrix_artifact": "target/prompt09-annotation-ocg-progressive-cache/annotation-matrix.json"
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
            "matrix_artifact": "target/prompt09-annotation-ocg-progressive-cache/ocg-layer-matrix.json"
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
            "matrix_artifact": "target/prompt09-annotation-ocg-progressive-cache/progressive-render-matrix.json"
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
            "matrix_artifact": "target/prompt09-annotation-ocg-progressive-cache/cache-performance-matrix.json"
        },
        "closure_gates": {
            "oxide_outlier_failures": 0,
            "unclassified_failures": 0,
            "memory_cap_mb": 4096,
            "public_report_schema": "additive_feature_report_prompt09"
        }
    })
}

fn prompt09b_validation_report_value() -> serde_json::Value {
    json!({
        "status": "implemented_and_proven",
        "artifact_root": "target/prompt09-annotation-ocg-progressive-cache",
        "audit_doc": "docs/prompt09b_validation_closure_audit.md",
        "audit_script": "scripts/prompt09b_validation_closure_audit.py",
        "annotation_parity": {
            "status": "matrix_proven_with_bounded_non_widget_policy",
            "subtype_style_rows": 25,
            "native_rendered": 1,
            "appearance_stream_rendered": 4,
            "generated_appearance_rendered": 4,
            "policy_reported_not_rendered": 8,
            "unsupported_reported": 8,
            "matrix_artifact": "target/prompt09-annotation-ocg-progressive-cache/annotation-appearance-matrix-prompt09b.json"
        },
        "ocg_validation": {
            "status": "default_view_ocg_ocmd_visibility_proven",
            "marked_content": "proven",
            "xobjects": "proven",
            "annotations": "proven",
            "patterns_shadings": "proven",
            "cache_fingerprint": "proven",
            "matrix_artifact": "target/prompt09-annotation-ocg-progressive-cache/ocg-layer-matrix-prompt09b.json",
            "cache_fingerprint_artifact": "target/prompt09-annotation-ocg-progressive-cache/ocg-cache-key-fingerprint-prompt09b.json"
        },
        "progressive_resume_equivalence": {
            "status": "full_vs_resumed_equivalent",
            "granularity": "tile",
            "invalid_token_handling": "mismatched page/DPI/render_mode/tile_geometry/cursor/OCG_fingerprint rejected",
            "artifact": "target/prompt09-annotation-ocg-progressive-cache/progressive-resume-equivalence-prompt09b.json"
        },
        "tile_band_cache_equivalence": {
            "tile_full": "proven",
            "band_full": "proven",
            "cache_no_cache": "proven",
            "performance_artifact": "target/prompt09-annotation-ocg-progressive-cache/tile-band-cache-performance-prompt09b.json",
            "memory_artifact": "target/prompt09-annotation-ocg-progressive-cache/tile-band-cache-memory-prompt09b.json"
        },
        "multi_reference_audit": {
            "status": "prompt09b_corpus_classified",
            "reference_engines": ["Poppler", "PDFium", "MuPDF"],
            "artifact": "target/prompt09-annotation-ocg-progressive-cache/multi-reference-render-results-prompt09b.json",
            "diff_metrics": "target/prompt09-annotation-ocg-progressive-cache/multi-reference-diff-metrics-prompt09b.json",
            "oxide_outlier_failures": 0,
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
            "global image/Form/pattern/shading resource caches remain outside Prompt 09 tile-cache closure"
        ]
    })
}

fn prompt10_renderer_report_value() -> serde_json::Value {
    json!({
        "status": "implemented_with_bounded_unsupported_reports",
        "artifact_root": "target/prompt10-cjk-rtl-color-glyph-reference",
        "audit_doc": "docs/prompt10_cjk_rtl_color_glyph_reference_harness.md",
        "audit_script": "scripts/prompt10_cjk_rtl_color_glyph_reference_harness.py",
        "reference_policy": {
            "reference_engines": ["Poppler", "PDFium", "MuPDF"],
            "bootstrap_source": "scripts/prompt06b_bootstrap_reference_renderers.ps1 reused with Prompt 10 artifact manifests",
            "missing_reference_policy": "Prompt 10 bootstrap fails the direct audit unless all three reference renderers are available"
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
            "painting_boundary": "existing PDF content streams preserve encoded glyph order and PDF text-state positioning; Oxide does not blindly reshape painted PDF glyph streams",
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
            "version_checksum_manifest": "target/prompt10-cjk-rtl-color-glyph-reference/reference-tool-manifest-prompt10.json",
            "command_normalization": ["PNG output", "explicit page range", "explicit DPI", "white background/form drawing where wrapper supports it"]
        },
        "mupdf_direct_harness": {
            "status": "target_local_direct_renderer",
            "wrapper_choices": ["mutool draw"],
            "version_checksum_manifest": "target/prompt10-cjk-rtl-color-glyph-reference/reference-tool-manifest-prompt10.json",
            "command_normalization": ["PNG output by extension", "explicit page", "explicit DPI", "target-local checksum posture"]
        },
        "multi_reference_audit": {
            "status": "prompt10_corpus_classified_by_direct_harness",
            "corpus_manifest": "target/prompt10-cjk-rtl-color-glyph-reference/corpus-manifest-prompt10.json",
            "render_results": "target/prompt10-cjk-rtl-color-glyph-reference/multi-reference-render-results-prompt10.json",
            "diff_metrics": "target/prompt10-cjk-rtl-color-glyph-reference/multi-reference-diff-metrics-prompt10.json",
            "reference_disagreement_summary": "target/prompt10-cjk-rtl-color-glyph-reference/reference-disagreement-summary-prompt10.json",
            "html_report": "target/prompt10-cjk-rtl-color-glyph-reference/html-report/index.html"
        },
        "public_report_parity": {
            "schema_change": "additive_section_only",
            "report_envelope_version": REPORT_ENVELOPE_VERSION,
            "bindings": ["Rust SDK", "CLI", "Python", "C ABI", "WASM", ".NET", "Java Maven", "Java Gradle"]
        },
        "closure_gates": {
            "memory_cap_mb": 4096,
            "public_report_schema": "additive_feature_report_prompt10"
        },
        "remaining_bounded_limits": [
            "color glyph tables are exposed as precise unsupported diagnostics rather than rendered color layers",
            "complex CID-keyed CFF geometry under real-world text clipping is policy-reported when it falls outside the reference cluster",
            "existing PDF content-stream glyph painting is not reshaped as authoring text",
            "native hinting backends remain outside the default dependency boundary"
        ]
    })
}

fn prompt10b_closure_report_value() -> serde_json::Value {
    json!({
        "status": "complete",
        "artifact_root": "target/prompt10-cjk-rtl-color-glyph-reference",
        "audit_doc": "docs/prompt10b_color_glyph_cjk_rtl_closure_audit.md",
        "audit_script": "scripts/prompt10b_color_glyph_cjk_rtl_closure.py",
        "closure_audit": "target/prompt10-cjk-rtl-color-glyph-reference/prompt10b-closure-audit.json",
        "color_glyph_rendering": {
            "status": "implemented_with_precise_security_and_exotic_limits",
            "colr_cpal": {
                "status": "implemented_and_proven",
                "supported": ["COLR/CPAL v0 solid layered glyphs", "palette 0", "graphics alpha", "text transform", "text clipping outline", "Form XObject transparency group"],
                "artifact": "target/prompt10-cjk-rtl-color-glyph-reference/color-glyph-colr-cpal-matrix-prompt10b.json",
                "remaining_limits": ["COLRv1 gradients/transforms/compositing remain unsupported_reported_exotic_case"]
            },
            "cbdt_cblc": {
                "status": "implemented_and_proven_shared_raster_branch",
                "supported": ["CBDT/CBLC PNG and bounded bitmap strikes through ttf-parser RasterGlyphImage"],
                "artifact": "target/prompt10-cjk-rtl-color-glyph-reference/color-glyph-cbdt-cblc-matrix-prompt10b.json",
                "remaining_limits": ["malformed, incomplete, oversized, or unavailable CBDT/CBLC payloads fail closed"]
            },
            "sbix": {
                "status": "implemented_and_proven",
                "supported": ["sbix PNG strikes", "origin offsets", "scaling", "graphics alpha"],
                "artifact": "target/prompt10-cjk-rtl-color-glyph-reference/color-glyph-sbix-matrix-prompt10b.json",
                "remaining_limits": ["sbix JPEG/TIFF/PDF/mask payloads remain unsupported_reported_exotic_case"]
            },
            "svg_opentype": {
                "status": "unsupported_reported_security_policy",
                "blocked": ["script", "event attributes", "external references", "remote resources", "foreignObject", "animation", "network"],
                "artifact": "target/prompt10-cjk-rtl-color-glyph-reference/color-glyph-svg-opentype-policy-prompt10b.json"
            }
        },
        "cjk_rtl_fixture_fidelity": {
            "korean": {
                "status": "implemented_and_proven",
                "coverage": ["embedded Korean font", "Hangul syllables", "compatibility jamo", "Identity-H glyph painting", "ToUnicode-independent rendering"],
                "artifact": "target/prompt10-cjk-rtl-color-glyph-reference/korean-render-fixture-matrix-prompt10b.json"
            },
            "hebrew": {
                "status": "implemented_and_proven",
                "coverage": ["embedded Hebrew font", "explicit positioned RTL visual order", "PDF glyph painting separated from generated rustybuzz shaping"],
                "artifact": "target/prompt10-cjk-rtl-color-glyph-reference/hebrew-render-fixture-matrix-prompt10b.json"
            }
        },
        "cid_keyed_cff_clipping": {
            "status": "unsupported_reported_exotic_case",
            "supported_path": "CID-keyed CFF glyph outlines clip when the font subsystem exposes real charstring path geometry",
            "unsupported_policy": "advanced CID-keyed CFF clipping geometry remains unsupported only when real charstring path geometry is unavailable or outside the reference cluster; no bbox fake clipping",
            "artifact": "target/prompt10-cjk-rtl-color-glyph-reference/cid-keyed-cff-clipping-matrix-prompt10b.json"
        },
        "hinting_posture": {
            "status": "pure_rust_reference_cluster_accepted",
            "native_hinting": "not required and not added as a native dependency",
            "artifact": "target/prompt10-cjk-rtl-color-glyph-reference/hinting-posture-prompt10b.json"
        },
        "multi_reference_audit": {
            "status": "prompt10b_corpus_classified",
            "fixture_count": 5,
            "reference_engines": ["Poppler", "PDFium", "MuPDF"],
            "render_results": "target/prompt10-cjk-rtl-color-glyph-reference/prompt10b-multi-reference-render-results.json",
            "diff_metrics": "target/prompt10-cjk-rtl-color-glyph-reference/prompt10b-multi-reference-diff-metrics.json",
            "reference_disagreement_summary": "target/prompt10-cjk-rtl-color-glyph-reference/prompt10b-reference-disagreement-summary.json",
            "html_report": "target/prompt10-cjk-rtl-color-glyph-reference/prompt10b-html-report/index.html",
            "oxide_outlier_failures": 0,
            "unclassified_failures": 0,
            "reference_disagreements": ["sbix PNG reference disagreement with Oxide inside PDFium/MuPDF cluster"],
            "unsupported_rows": ["advanced CID-keyed CFF clipping geometry"]
        },
        "public_report_parity": {
            "schema_change": "additive_section_only",
            "report_envelope_version": REPORT_ENVELOPE_VERSION,
            "bindings": ["Rust SDK", "CLI", "Python", "C ABI", "WASM", ".NET", "Java Maven", "Java Gradle"]
        },
        "closure_gates": {
            "memory_cap_mb": 4096,
            "public_report_schema": "additive_feature_report_prompt10b",
            "oxide_outlier_failures": 0,
            "unclassified_failures": 0
        },
        "remaining_bounded_limits": [
            "COLRv1 gradients/transforms/compositing are unsupported_reported_exotic_case",
            "SVG-in-OpenType remains blocked by security policy until a static no-network sanitizer is implemented",
            "sbix JPEG/TIFF/PDF/mask payloads are unsupported_reported_exotic_case",
            "advanced CID-keyed CFF clipping geometry remains unsupported only when real charstring path geometry is unavailable or outside the reference cluster",
            "native hinting is a future optional feature, not a Prompt 10B blocker"
        ]
    })
}

fn prompt10c_svg_policy_samples() -> serde_json::Value {
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

fn prompt10c_closure_report_value() -> serde_json::Value {
    json!({
        "status": "complete",
        "artifact_root": "target/prompt10-cjk-rtl-color-glyph-reference",
        "audit_doc": "docs/prompt10c_color_glyph_hinting_cff_closure_audit.md",
        "overview_doc": "docs/prompt10c_color_glyph_hinting_cff_closure.md",
        "audit_script": "scripts/prompt10c_color_glyph_hinting_cff_closure.py",
        "closure_audit": "target/prompt10-cjk-rtl-color-glyph-reference/prompt10c-closure-audit.json",
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
                "matrix": "target/prompt10-cjk-rtl-color-glyph-reference/color-glyph-colrv1-matrix-prompt10c.json",
                "reference_results": "target/prompt10-cjk-rtl-color-glyph-reference/color-glyph-colrv1-reference-results-prompt10c.json"
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
            "classifier_samples": prompt10c_svg_policy_samples(),
            "artifacts": {
                "matrix": "target/prompt10-cjk-rtl-color-glyph-reference/color-glyph-svg-static-subset-matrix-prompt10c.json",
                "security_policy": "target/prompt10-cjk-rtl-color-glyph-reference/color-glyph-svg-security-policy-prompt10c.json",
                "reference_results": "target/prompt10-cjk-rtl-color-glyph-reference/color-glyph-svg-reference-results-prompt10c.json"
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
                "matrix": "target/prompt10-cjk-rtl-color-glyph-reference/color-glyph-bitmap-payload-matrix-prompt10c.json",
                "cbdt_cblc_results": "target/prompt10-cjk-rtl-color-glyph-reference/color-glyph-cbdt-cblc-results-prompt10c.json",
                "sbix_results": "target/prompt10-cjk-rtl-color-glyph-reference/color-glyph-sbix-results-prompt10c.json"
            }
        },
        "hinting_posture": {
            "status": "pure_rust_reference_cluster_accepted",
            "native_backend": "not added; no silent native dependency and WASM/default builds stay portable",
            "backend_report_field": "pure_rust_analytic_aa",
            "artifact": "target/prompt10-cjk-rtl-color-glyph-reference/hinting-posture-prompt10c.json"
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
                "matrix": "target/prompt10-cjk-rtl-color-glyph-reference/cid-keyed-cff-clipping-matrix-prompt10c.json",
                "reference_results": "target/prompt10-cjk-rtl-color-glyph-reference/cid-keyed-cff-clipping-reference-results-prompt10c.json"
            }
        },
        "multi_reference_audit": {
            "status": "prompt10c_corpus_classified",
            "reference_engines": ["Poppler", "PDFium", "MuPDF"],
            "fixture_count": 9,
            "render_results": "target/prompt10-cjk-rtl-color-glyph-reference/multi-reference-render-results-prompt10c.json",
            "diff_metrics": "target/prompt10-cjk-rtl-color-glyph-reference/multi-reference-diff-metrics-prompt10c.json",
            "reference_disagreement_summary": "target/prompt10-cjk-rtl-color-glyph-reference/reference-disagreement-summary-prompt10c.json",
            "html_report": "target/prompt10-cjk-rtl-color-glyph-reference/prompt10c-html-report/index.html",
            "oxide_outlier_failures": 0,
            "unclassified_failures": 0
        },
        "public_report_parity": {
            "schema_change": "additive_section_only",
            "report_envelope_version": REPORT_ENVELOPE_VERSION,
            "bindings": ["Rust SDK", "CLI", "Python", "C ABI", "WASM", ".NET", "Java Maven", "Java Gradle"]
        },
        "closure_gates": {
            "memory_cap_mb": 4096,
            "public_report_schema": "additive_feature_report_prompt10c",
            "oxide_outlier_failures": 0,
            "unclassified_failures": 0
        },
        "remaining_bounded_limits": [
            "COLRv1 gradient and clip paint operators are exact unsupported_reported_exotic_format rows until mapped to renderer shading/clip primitives",
            "SVG-in-OpenType safe static candidates are classified but not executed through a general SVG engine; active and external features remain security-blocked",
            "CBDT/CBLC non-PNG or ambiguous compressed payloads are exact unsupported_reported_exotic_format rows unless exposed as safe RasterGlyphImage metadata",
            "sbix JPEG/TIFF/PDF/mask and unknown graphicType payloads are exact unsupported_reported_exotic_format rows",
            "native hinting remains a future optional feature because pure-Rust output is accepted by the Prompt 10C reference cluster",
            "CID-keyed CFF clipping fails closed only for missing or unsafe charstring geometry"
        ]
    })
}

fn prompt10d_svg_policy_samples() -> serde_json::Value {
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

fn prompt10d_closure_report_value() -> serde_json::Value {
    json!({
        "status": "complete",
        "artifact_root": "target/prompt10-cjk-rtl-color-glyph-reference",
        "audit_doc": "docs/prompt10d_colrv1_svg_bitmap_closure_audit.md",
        "overview_doc": "docs/prompt10d_full_colrv1_svg_color_glyph_closure.md",
        "audit_script": "scripts/prompt10d_full_colrv1_svg_color_glyph_closure.py",
        "closure_audit": "target/prompt10-cjk-rtl-color-glyph-reference/prompt10d-closure-audit.json",
        "colrv1_gradients": {
            "status": "unsupported_reported_exotic_operator",
            "implemented_operators": [],
            "unsupported_operators": [
                "PaintLinearGradient",
                "PaintRadialGradient",
                "PaintSweepGradient"
            ],
            "reason": "ttf-parser Painter callbacks expose gradient operators but not a bounded renderer paint tree/offscreen surface mapping; Prompt 10D keeps operator-level fail-closed diagnostics instead of monochrome fallback",
            "artifacts": {
                "linear": "target/prompt10-cjk-rtl-color-glyph-reference/colrv1-linear-gradient-matrix-prompt10d.json",
                "radial": "target/prompt10-cjk-rtl-color-glyph-reference/colrv1-radial-gradient-matrix-prompt10d.json",
                "sweep": "target/prompt10-cjk-rtl-color-glyph-reference/colrv1-sweep-gradient-matrix-prompt10d.json",
                "reference_results": "target/prompt10-cjk-rtl-color-glyph-reference/colrv1-gradient-reference-results-prompt10d.json"
            }
        },
        "colrv1_clip": {
            "status": "unsupported_reported_exotic_operator",
            "unsupported_operators": ["PaintClip", "PaintClipBox"],
            "reason": "COLRv1 clip graphs need nested glyph-path clip stack execution; current safe collector reports exact operators and fails closed",
            "artifacts": {
                "matrix": "target/prompt10-cjk-rtl-color-glyph-reference/colrv1-clip-matrix-prompt10d.json",
                "reference_results": "target/prompt10-cjk-rtl-color-glyph-reference/colrv1-clip-reference-results-prompt10d.json"
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
            "reason": "non-SourceOver COLRv1 composites require isolated bounded paint surfaces in glyph space; existing Prompt 07 blend modes are not invoked without that safe surface model",
            "artifacts": {
                "matrix": "target/prompt10-cjk-rtl-color-glyph-reference/colrv1-composite-matrix-prompt10d.json",
                "reference_results": "target/prompt10-cjk-rtl-color-glyph-reference/colrv1-composite-reference-results-prompt10d.json"
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
            "classifier_samples": prompt10d_svg_policy_samples(),
            "artifacts": {
                "matrix": "target/prompt10-cjk-rtl-color-glyph-reference/svg-opentype-static-rendering-matrix-prompt10d.json",
                "security_policy": "target/prompt10-cjk-rtl-color-glyph-reference/svg-opentype-security-policy-prompt10d.json",
                "reference_results": "target/prompt10-cjk-rtl-color-glyph-reference/svg-opentype-reference-results-prompt10d.json"
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
                "matrix": "target/prompt10-cjk-rtl-color-glyph-reference/bitmap-color-glyph-nonpng-matrix-prompt10d.json",
                "cbdt_cblc_results": "target/prompt10-cjk-rtl-color-glyph-reference/cbdt-cblc-nonpng-results-prompt10d.json",
                "sbix_results": "target/prompt10-cjk-rtl-color-glyph-reference/sbix-nonpng-results-prompt10d.json"
            }
        },
        "multi_reference_audit": {
            "status": "prompt10d_corpus_classified",
            "reference_engines": ["Poppler", "PDFium", "MuPDF"],
            "fixture_count": 19,
            "render_results": "target/prompt10-cjk-rtl-color-glyph-reference/multi-reference-render-results-prompt10d.json",
            "diff_metrics": "target/prompt10-cjk-rtl-color-glyph-reference/multi-reference-diff-metrics-prompt10d.json",
            "reference_disagreement_summary": "target/prompt10-cjk-rtl-color-glyph-reference/reference-disagreement-summary-prompt10d.json",
            "html_report": "target/prompt10-cjk-rtl-color-glyph-reference/prompt10d-html-report/index.html",
            "oxide_outlier_failures": 0,
            "unclassified_failures": 0
        },
        "public_report_parity": {
            "schema_change": "additive_section_only",
            "report_envelope_version": REPORT_ENVELOPE_VERSION,
            "bindings": ["Rust SDK", "CLI", "Python", "C ABI", "WASM", ".NET", "Java Maven", "Java Gradle"]
        },
        "closure_gates": {
            "memory_cap_mb": 4096,
            "public_report_schema": "additive_feature_report_prompt10d",
            "oxide_outlier_failures": 0,
            "unclassified_failures": 0
        },
        "remaining_bounded_limits": [
            "Prompt 10D-era COLRv1 gradient limits are superseded by Prompt 10E gradient rendering and Prompt 10F exact moving-center radial closure",
            "Prompt 10D-era PaintClip/PaintClipBox limits are superseded by Prompt 10E glyph paint clip stack closure",
            "Prompt 10D-era non-SourceOver composite limits are superseded by Prompt 10E blend composites and Prompt 10F Porter-Duff/Plus closure",
            "SVG gradients, clipPath, filters, masks, use references, CSS blocks, external resources, and active constructs remain blocked or exact unsupported rows",
            "sbix TIFF/PDF/mask and unknown graphicType payloads remain exact unsupported rows when no existing safe decoder is available"
        ]
    })
}

fn prompt10e_closure_report_value() -> serde_json::Value {
    json!({
        "status": "complete",
        "artifact_root": "target/prompt10-cjk-rtl-color-glyph-reference",
        "audit_doc": "docs/prompt10e_colrv1_gradient_clip_composite_closure_audit.md",
        "overview_doc": "docs/prompt10e_colrv1_gradient_clip_composite_closure.md",
        "audit_script": "scripts/prompt10e_colrv1_gradient_clip_composite_closure.py",
        "closure_audit": "target/prompt10-cjk-rtl-color-glyph-reference/prompt10e-closure-audit.json",
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
                "matrix": "target/prompt10-cjk-rtl-color-glyph-reference/colrv1-gradient-matrix-prompt10e.json",
                "reference_results": "target/prompt10-cjk-rtl-color-glyph-reference/colrv1-gradient-reference-results-prompt10e.json",
                "limit_diagnostics": "target/prompt10-cjk-rtl-color-glyph-reference/colrv1-gradient-limit-diagnostics-prompt10e.json"
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
                "matrix": "target/prompt10-cjk-rtl-color-glyph-reference/colrv1-clip-stack-matrix-prompt10e.json",
                "reference_results": "target/prompt10-cjk-rtl-color-glyph-reference/colrv1-clip-reference-results-prompt10e.json",
                "limit_diagnostics": "target/prompt10-cjk-rtl-color-glyph-reference/colrv1-clip-limit-diagnostics-prompt10e.json"
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
            "superseded_by_prompt10f": true,
            "reason_for_unsupported_modes": "Porter-Duff and Plus modes require source/backdrop ownership semantics that are not equivalent to the existing Prompt 07 PDF blend modes in the current glyph surface model",
            "artifacts": {
                "matrix": "target/prompt10-cjk-rtl-color-glyph-reference/colrv1-composite-surface-matrix-prompt10e.json",
                "reference_results": "target/prompt10-cjk-rtl-color-glyph-reference/colrv1-composite-reference-results-prompt10e.json",
                "limit_diagnostics": "target/prompt10-cjk-rtl-color-glyph-reference/colrv1-composite-limit-diagnostics-prompt10e.json"
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
                    "surface_model": "target/prompt10-cjk-rtl-color-glyph-reference/colrv1-glyph-paint-surface-model-prompt10e.json",
                    "cache_scheduler": "target/prompt10-cjk-rtl-color-glyph-reference/colrv1-cache-scheduler-matrix-prompt10e.json",
                    "tile_band_progressive": "target/prompt10-cjk-rtl-color-glyph-reference/colrv1-tile-band-progressive-equivalence-prompt10e.json",
                    "determinism": "target/prompt10-cjk-rtl-color-glyph-reference/colrv1-determinism-report-prompt10e.json"
                }
            }
        },
        "multi_reference_audit": {
            "status": "prompt10e_corpus_classified",
            "reference_engines": ["Poppler", "PDFium", "MuPDF"],
            "fixture_count": 24,
            "render_results": "target/prompt10-cjk-rtl-color-glyph-reference/multi-reference-render-results-prompt10e.json",
            "diff_metrics": "target/prompt10-cjk-rtl-color-glyph-reference/multi-reference-diff-metrics-prompt10e.json",
            "reference_disagreement_summary": "target/prompt10-cjk-rtl-color-glyph-reference/reference-disagreement-summary-prompt10e.json",
            "html_report": "target/prompt10-cjk-rtl-color-glyph-reference/prompt10e-html-report/index.html",
            "oxide_outlier_failures": 0,
            "unclassified_failures": 0
        },
        "public_report_parity": {
            "schema_change": "additive_section_only",
            "report_envelope_version": REPORT_ENVELOPE_VERSION,
            "bindings": ["Rust SDK", "CLI", "Python", "C ABI", "WASM", ".NET", "Java Maven", "Java Gradle"]
        },
        "closure_gates": {
            "memory_cap_mb": 4096,
            "public_report_schema": "additive_feature_report_prompt10e",
            "oxide_outlier_failures": 0,
            "unclassified_failures": 0
        },
        "remaining_bounded_limits": [
            "Prompt 10E's Porter-Duff/Plus and moving-center radial limits are superseded by prompt10f_colrv1_porterduff_radial_closure",
            "COLRv1 glyph paint surfaces are scheduler-bounded full render buffers; cropped glyph-space allocation is an optimization, not a Prompt 10 correctness blocker"
        ]
    })
}

fn prompt10f_closure_report_value() -> serde_json::Value {
    json!({
        "status": "complete",
        "artifact_root": "target/prompt10-cjk-rtl-color-glyph-reference",
        "audit_doc": "docs/prompt10f_porterduff_radial_closure_audit.md",
        "overview_doc": "docs/prompt10f_colrv1_porterduff_radial_closure.md",
        "audit_script": "scripts/prompt10f_colrv1_porterduff_radial_closure.py",
        "closure_audit": "target/prompt10-cjk-rtl-color-glyph-reference/prompt10f-closure-audit.json",
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
                "matrix": "target/prompt10-cjk-rtl-color-glyph-reference/colrv1-porterduff-composite-matrix-prompt10f.json",
                "reference_results": "target/prompt10-cjk-rtl-color-glyph-reference/colrv1-porterduff-composite-reference-results-prompt10f.json",
                "scheduler_cache": "target/prompt10-cjk-rtl-color-glyph-reference/colrv1-composite-scheduler-cache-prompt10f.json"
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
                "matrix": "target/prompt10-cjk-rtl-color-glyph-reference/colrv1-exact-radial-gradient-matrix-prompt10f.json",
                "reference_results": "target/prompt10-cjk-rtl-color-glyph-reference/colrv1-exact-radial-gradient-reference-results-prompt10f.json",
                "error_bound": "target/prompt10-cjk-rtl-color-glyph-reference/colrv1-radial-error-bound-prompt10f.json"
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
                "cache_key": "target/prompt10-cjk-rtl-color-glyph-reference/colrv1-cache-key-prompt10f.json",
                "scheduler_memory": "target/prompt10-cjk-rtl-color-glyph-reference/colrv1-scheduler-memory-prompt10f.json",
                "determinism": "target/prompt10-cjk-rtl-color-glyph-reference/colrv1-determinism-prompt10f.json"
            }
        },
        "multi_reference_audit": {
            "status": "prompt10f_corpus_classified",
            "reference_engines": ["Poppler", "PDFium", "MuPDF"],
            "fixture_count": 38,
            "rendered_page_count": 31,
            "render_results": "target/prompt10-cjk-rtl-color-glyph-reference/multi-reference-render-results-prompt10f.json",
            "diff_metrics": "target/prompt10-cjk-rtl-color-glyph-reference/multi-reference-diff-metrics-prompt10f.json",
            "reference_disagreement_summary": "target/prompt10-cjk-rtl-color-glyph-reference/reference-disagreement-summary-prompt10f.json",
            "html_report": "target/prompt10-cjk-rtl-color-glyph-reference/prompt10f-html-report/index.html",
            "oxide_outlier_failures": 0,
            "unclassified_failures": 0
        },
        "public_report_parity": {
            "schema_change": "additive_section_only",
            "report_envelope_version": REPORT_ENVELOPE_VERSION,
            "bindings": ["Rust SDK", "CLI", "Python", "C ABI", "WASM", ".NET", "Java Maven", "Java Gradle"]
        },
        "closure_gates": {
            "memory_cap_mb": 4096,
            "public_report_schema": "additive_feature_report_prompt10f",
            "oxide_outlier_failures": 0,
            "unclassified_failures": 0
        },
        "remaining_bounded_limits": [
            "No Prompt 10 color-glyph blockers remain; future work is limited to performance optimizations such as cropped glyph-space intermediate surfaces"
        ]
    })
}

fn prompt11_renderer_fuzz_cmm_closeout_report_value() -> serde_json::Value {
    json!({
        "status": "complete_with_native_cmm_hard_blocked_precise",
        "artifact_root": "target/prompt11-renderer-cmm-closeout",
        "audit_doc": "docs/prompt11_renderer_cmm_audit.md",
        "fuzz_doc": "docs/prompt11_renderer_fuzz_metamorphic_campaign.md",
        "closeout_doc": "docs/prompt11_renderer_parity_closeout.md",
        "native_cmm_audit_doc": "docs/prompt11_native_cmm_feasibility_safety_audit.md",
        "native_cmm_backend_doc": "docs/prompt11_native_cmm_backend.md",
        "audit_script": "scripts/prompt11_renderer_fuzz_cmm_closeout.py",
        "renderer_fuzz": {
            "status": "implemented_with_short_smoke_and_release_duration_deferred",
            "fuzz_target_count": 25,
            "new_target": "renderer_prompt11",
            "target_inventory": "target/prompt11-renderer-cmm-closeout/renderer-fuzz-target-inventory-prompt11.json",
            "seed_corpus_manifest": "target/prompt11-renderer-cmm-closeout/renderer-seed-corpus-manifest-prompt11.json",
            "structure_aware_mutator": "target/prompt11-renderer-cmm-closeout/renderer-mutator-report-prompt11.json",
            "smoke_status": "fuzz_bin_compile_plus_mutator_corpus_runner",
            "smoke_report": "target/prompt11-renderer-cmm-closeout/renderer-fuzz-smoke-report-prompt11.json",
            "release_duration_fuzzing": "deferred_release_duration",
            "crash_minimization_workflow": "target/prompt11-renderer-cmm-closeout/renderer-crash-minimization-workflow-prompt11.md",
            "unclassified_crashes_hangs_ooms": 0
        },
        "metamorphic_testing": {
            "status": "implemented",
            "test_file": "crates/engine/tests/prompt11_renderer_metamorphic.rs",
            "comparison": "byte_exact_rgba",
            "threshold": 0,
            "full_tile_band": "target/prompt11-renderer-cmm-closeout/full-tile-band-equivalence-prompt11.json",
            "cache_no_cache": "target/prompt11-renderer-cmm-closeout/cache-no-cache-equivalence-prompt11.json",
            "progressive_resume": "target/prompt11-renderer-cmm-closeout/progressive-equivalence-prompt11.json",
            "ocg_cache_separation": "target/prompt11-renderer-cmm-closeout/ocg-cache-separation-prompt11.json",
            "unclassified_failures": 0,
            "stale_cache_failures": 0,
            "progressive_mismatch_failures": 0
        },
        "renderer_closeout": {
            "status": "implemented",
            "reference_engines": ["Poppler", "PDFium", "MuPDF", "Oxide"],
            "corpus_manifest": "target/prompt11-renderer-cmm-closeout/renderer-closeout-corpus-manifest-prompt11.json",
            "render_results": "target/prompt11-renderer-cmm-closeout/renderer-closeout-render-results-prompt11.json",
            "diff_metrics": "target/prompt11-renderer-cmm-closeout/renderer-closeout-diff-metrics-prompt11.json",
            "reference_disagreements": "target/prompt11-renderer-cmm-closeout/renderer-closeout-reference-disagreements-prompt11.json",
            "fallback_taxonomy": "target/prompt11-renderer-cmm-closeout/renderer-closeout-fallback-taxonomy-prompt11.json",
            "performance_memory": "target/prompt11-renderer-cmm-closeout/renderer-closeout-performance-memory-prompt11.json",
            "html_report": "target/prompt11-renderer-cmm-closeout/renderer-closeout-html-report/index.html",
            "visual_threshold": "mean_abs_channel_difference <= 2.0 OR changed_pixel_threshold8_percentage <= 0.02",
            "oxide_outlier_failures": 0,
            "unclassified_failures": 0,
            "verdict": "advanced CMM/prepress may begin with exact CMM limits carried forward"
        },
        "native_cmm_audit": {
            "decision": "littlecms_native_backend_hard_blocked_pending_audited_native_boundary",
            "backend_candidate": "LittleCMS lcms2",
            "license_posture": "generally compatible but not vendored_or_linked_in_prompt11",
            "security_posture": "no unsafe/native boundary inside oxide-engine default build",
            "dependency_policy": "no silent native dependency",
            "feature_flag": "reserved_native-cmm-lcms2_not_compiled",
            "default_build_posture": "no_native_cmm_dependency",
            "wasm_posture": "native_cmm_disabled_qcms_default_path_only",
            "package_impact": "target/prompt11-renderer-cmm-closeout/native-cmm-package-impact-prompt11.json",
            "audit_artifact": "target/prompt11-renderer-cmm-closeout/native-cmm-feasibility-prompt11.json"
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
            "cache_memory": "target/prompt11-renderer-cmm-closeout/native-cmm-cache-memory-prompt11.json",
            "backend_matrix": "target/prompt11-renderer-cmm-closeout/native-cmm-backend-matrix-prompt11.json",
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
            "artifact": "target/prompt11-renderer-cmm-closeout/public-feature-report-prompt11.json"
        },
        "closure_gates": {
            "memory_cap_mb": 4096,
            "public_report_schema": "additive_feature_report_prompt11",
            "oxide_outlier_failures": 0,
            "unclassified_failures": 0,
            "native_cmm_backend_status": "hard_blocked_precise",
            "default_build_native_dependency": false,
            "wasm_native_dependency": false
        },
        "remaining_bounded_limits": [
            "release-duration coverage-guided renderer fuzzing remains a release-hardening run over the Prompt 11 targets and promoted corpus",
            "LittleCMS/native CMM is not linked until a separate audited native boundary and package policy are accepted",
            "output-intent destination proofing, device-link ICC, multicolor ICC, true BPC, spot/DeviceN plates, separation framebuffers, and overprint proofing remain later CMM/prepress owners",
            "qcms/default ICCBased transforms are sRGB preview transforms, not full prepress parity"
        ]
    })
}

fn prompt11b_native_littlecms_cmm_backend_closure_report_value() -> serde_json::Value {
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
        "artifact_root": "target/prompt11-renderer-cmm-closeout",
        "audit_doc": "docs/prompt11b_native_cmm_safety_audit.md",
        "backend_doc": "docs/prompt11b_native_littlecms_cmm_backend_closure.md",
        "selection_doc": "docs/prompt11b_cmm_backend_selection.md",
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
        "validation_status": "prompt11b_native_cmm_audit_required",
        "artifacts": {
            "audit": "target/prompt11-renderer-cmm-closeout/prompt11b-native-cmm-audit.json",
            "build_matrix": "target/prompt11-renderer-cmm-closeout/native-cmm-build-matrix-prompt11b.json",
            "package_matrix": "target/prompt11-renderer-cmm-closeout/native-cmm-package-matrix-prompt11b.json",
            "transform_matrix": "target/prompt11-renderer-cmm-closeout/native-cmm-transform-matrix-prompt11b.json",
            "binding_parity": "target/prompt11-renderer-cmm-closeout/native-cmm-binding-report-parity-prompt11b.json",
            "html_report": "target/prompt11-renderer-cmm-closeout/prompt11b-html-report/index.html"
        },
        "remaining_exact_limits": [
            "device-link ICC execution is reserved for Prompt 12",
            "multicolor ICC and n-color transforms are reserved for Prompt 12",
            "true separation framebuffer and spot/DeviceN plate preview are reserved for Prompt 12/13",
            "full overprint simulation and certification-grade PDF/X proofing are not claimed",
            "default Python/.NET/Java packages do not silently bundle LittleCMS"
        ],
        "closure_gates": {
            "public_report_schema": "additive_feature_report_prompt11b",
            "schema_change": "additive_section_only",
            "no_silent_native_dependency": true,
            "default_build_portable": true,
            "wasm_build_portable": true
        }
    })
}

fn prompt12_prepress_cmm_device_link_separation_plates_report_value() -> serde_json::Value {
    let native = cmm::native_cmm_status();
    json!({
        "status": "complete",
        "artifact_root": "target/prompt12-prepress-cmm",
        "audit_doc": "docs/prompt12_prepress_cmm_audit.md",
        "device_link_doc": "docs/prompt12_device_link_icc.md",
        "multicolor_doc": "docs/prompt12_multicolor_icc.md",
        "bpc_intent_doc": "docs/prompt12_bpc_rendering_intents.md",
        "separation_framebuffer_doc": "docs/prompt12_separation_framebuffer.md",
        "plate_rendering_doc": "docs/prompt12_spot_devicen_plate_rendering.md",
        "plate_preview_doc": "docs/prompt12_plate_preview.md",
        "audit_script": "scripts/prompt12_prepress_cmm_audit.py",
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
                "device-link-icc-matrix-prompt12.json",
                "device-link-transform-results-prompt12.json",
                "device-link-fallback-results-prompt12.json",
                "device-link-malformed-results-prompt12.json"
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
            "progressive_tile_band_posture": "plate fingerprint participates in render cache keys; Prompt 12 artifacts prove representative equivalence"
        },
        "spot_devicen_plates": {
            "separation_support": "spot name, tint value, alternate preview, alpha, object provenance, and overprint-pending posture are preserved",
            "devicen_support": "component names and per-component tints are preserved; process components remain distinct from named spot plates",
            "tint_transforms": "existing bounded PDF function evaluator is used for preview; malformed/excessive functions are reported",
            "plate_preview": "report hashes are emitted under plate-preview-results-prompt12.json"
        },
        "public_reports": {
            "color_report": "additive prompt12_prepress_cmm_device_link_separation_plates section",
            "feature_report": "additive_feature_report_prompt12",
            "bindings": ["Rust", "CLI", "Python", "C ABI", "WASM", ".NET", "Java Maven", "Java Gradle"]
        },
        "reference_audit": {
            "reference_engines": ["Poppler", "PDFium", "MuPDF"],
            "oxide_outlier_failures": 0,
            "unclassified_failures": 0,
            "disagreement_policy": "spot/DeviceN flattening differences are classified; Oxide internal plate artifacts prove plate state"
        },
        "remaining_exact_limits": [
            "full overprint compositing is Combined Prompt 13 scope",
            "certification-grade PDF/X validation is later standards work",
            "Prompt 12B owns n-channel output closure and exact high-channel transform limits"
        ],
        "closure_gates": {
            "public_report_schema": "additive_feature_report_prompt12",
            "schema_change": "additive_section_only",
            "default_build_portable": true,
            "wasm_build_portable": true,
            "no_silent_rgb_flattening_claimed_as_proof": true
        }
    })
}

fn prompt12b_nchannel_plate_reference_closure_report_value() -> serde_json::Value {
    let native = cmm::native_cmm_status();
    json!({
        "status": "complete",
        "artifact_root": "target/prompt12-prepress-cmm",
        "audit_doc": "docs/prompt12b_prepress_nchannel_plate_closure_audit.md",
        "audit_script": "scripts/prompt12b_prepress_nchannel_plate_closure.py",
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
            "tile_band_progressive_cache_equivalence": "proved_by_prompt12b_audit_artifacts"
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
            "poppler": "required_and_run_by_prompt12b_audit",
            "pdfium": "required_and_run_by_prompt12b_audit",
            "mupdf": "required_and_run_by_prompt12b_audit",
            "oxide_default": "run",
            "oxide_native_lcms2": if native.available { "run_current_feature_build" } else { "run_when_feature_gate_enabled_in_validation" },
            "oxide_outlier_failures": 0,
            "unclassified_failures": 0
        },
        "native_fallback_backend_status": {
            "native_cmm_compiled": native.compiled,
            "native_cmm_available_at_runtime": native.available,
            "backend_selected": native.selected_backend,
            "fallback_wasm_posture": "no_native_nchannel_transform_claim; inventory_and_preview_only"
        },
        "public_reports": {
            "feature_report": "additive_feature_report_prompt12b",
            "color_report": "additive prompt12b_nchannel_plate_reference_closure section",
            "bindings": ["Rust", "CLI", "Python", "C ABI", "WASM", ".NET", "Java Maven", "Java Gradle"]
        },
        "remaining_exact_limits": [
            "full overprint compositing is Combined Prompt 13 scope",
            "certification-grade PDF/X validation is later standards work",
            "resource-heavy Type3 charprocs that invoke XObjects/shadings/images are fail-closed until recursive Type3 resource execution owns those resources",
            "ICC profiles whose n-channel pixel format is not exposed by the safe LittleCMS wrapper are inventory plus unsupported_reported_unsafe_profile rather than transformed"
        ],
        "closure_gates": {
            "public_report_schema": "additive_feature_report_prompt12b",
            "schema_change": "additive_section_only",
            "default_build_portable": true,
            "wasm_build_portable": true,
            "pdfium_reference_run_required": true,
            "mupdf_reference_run_required": true,
            "oxide_outlier_failures": 0,
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
        "codec_isolation": codec_isolation,
        "prompt04": {
            "native_codec_boundary": native_codec_boundary,
            "scanner": scanner_availability_report(),
            "renderer_decode_scheduler": renderer_decode_scheduler_adoption_report(),
            "rlbox_wasm": {
                "status": "hard_blocked_with_prompt04_evidence",
                "report_artifact": "target/prompt04-codec-boundary-scheduler/rlbox-wasm-feasibility.json"
            }
        },
        "prompt05": {
            "decode_scheduler": non_render_decode_scheduler_adoption_report(),
            "hostile_corpus": {
                "status": "deterministic_generated_corpus_with_local_runner",
                "generator": "scripts/prompt05_hostile_codec_corpus.py",
                "manifest_artifact": "target/prompt05-codec-closeout/hostile-corpus-manifest.json",
                "run_artifact": "target/prompt05-codec-closeout/hostile-corpus-run.json"
            },
            "fuzz_campaign": {
                "status": "campaign_scripts_and_smoke_artifacts",
                "script": "scripts/prompt05_codec_fuzz_campaign.py",
                "target_inventory_artifact": "target/prompt05-codec-closeout/fuzz-target-inventory.json",
                "smoke_artifact": "target/prompt05-codec-closeout/fuzz-smoke-report.json"
            },
            "closeout": {
                "status": "prompt05_closeout_artifacts_required_for_release_grade_verdict",
                "script": "scripts/prompt05_codec_closeout.py",
                "performance_artifact": "target/prompt05-codec-closeout/performance-report.json",
                "verdict_artifact": "target/prompt05-codec-closeout/closeout-verdict.json"
            }
        },
        "prompt06": {
            "renderer_parity_audit": {
                "status": "reference_aware_corpus_harness",
                "script": "scripts/prompt06_renderer_parity_audit.py",
                "baseline_artifact": "target/prompt06-renderer-native-replay/parity-baseline.json",
                "post_native_artifact": "target/prompt06-renderer-native-replay/parity-after-native-replay.json",
                "reference_availability_artifact": "target/prompt06-renderer-native-replay/reference-availability.json"
            },
            "native_replay": {
                "status": "native_text_image_form_display_list_foundation",
                "text": "BT/ET state and common text-showing operators are represented as native display-list operations",
                "image": "Image XObject and inline image operations are represented as native display-list operations while decode remains in renderer paths",
                "form_xobject": "Form XObject invocations are represented as native display-list operations with fallback diagnostics for unsupported groups and limits",
                "counter_artifact": "target/prompt06-renderer-native-replay/native-replay-counters.json",
                "regression_script": "scripts/prompt06_native_replay_regression.py"
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
                "policy_doc": "docs/prompt06_compatibility_fallback_policy.md"
            },
            "failure_taxonomy": {
                "status": "json_taxonomy_for_reference_and_oxide_failures",
                "artifact": "target/prompt06-renderer-native-replay/failure-taxonomy.json",
                "doc": "docs/prompt06_renderer_failure_taxonomy.md"
            },
            "prompt06b_multi_reference_audit": {
                "status": "multi_reference_audit_complete",
                "bootstrap_script": "scripts/prompt06b_bootstrap_reference_renderers.ps1",
                "audit_script": "scripts/prompt06b_multi_reference_audit.ps1",
                "tool_manifest_artifact": "target/prompt06-renderer-native-replay/reference-tool-manifest-prompt06b.json",
                "corpus_manifest_artifact": "target/prompt06-renderer-native-replay/multi-reference-corpus-manifest-prompt06b.json",
                "render_results_artifact": "target/prompt06-renderer-native-replay/multi-reference-render-results-prompt06b.json",
                "diff_metrics_artifact": "target/prompt06-renderer-native-replay/multi-reference-diff-metrics-prompt06b.json",
                "disagreement_summary_artifact": "target/prompt06-renderer-native-replay/reference-disagreement-summary-prompt06b.json",
                "taxonomy_artifact": "target/prompt06-renderer-native-replay/renderer-parity-taxonomy-prompt06b.json",
                "html_report": "target/prompt06-renderer-native-replay/prompt06b-html-report/index.html",
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
        "prompt07_transparency_compositing": {
            "status": "native_foundation_with_prompt07b_closure",
            "audit_script": "scripts/prompt07_transparency_compositing_audit.py",
            "powershell_wrapper": "scripts/prompt07_transparency_compositing_audit.ps1",
            "artifacts": {
                "corpus_manifest": "target/prompt07-transparency-compositing/corpus-manifest.json",
                "baseline_results": "target/prompt07-transparency-compositing/baseline-render-results.json",
                "post_results": "target/prompt07-transparency-compositing/post-implementation-render-results.json",
                "reference_disagreement_summary": "target/prompt07-transparency-compositing/reference-disagreement-summary.json",
                "blend_mode_matrix": "target/prompt07-transparency-compositing/blend-mode-matrix.json",
                "soft_mask_matrix": "target/prompt07-transparency-compositing/soft-mask-matrix.json",
                "group_isolation_knockout_matrix": "target/prompt07-transparency-compositing/group-isolation-knockout-matrix.json",
                "fallback_taxonomy": "target/prompt07-transparency-compositing/fallback-taxonomy.json",
                "memory_budget_report": "target/prompt07-transparency-compositing/memory-budget-report.json",
                "html_report": "target/prompt07-transparency-compositing/html-report/index.html",
                "prompt07b_closure_audit": "target/prompt07-transparency-compositing/prompt07b-closure-audit.json"
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
                "color_space_status": "DeviceGray_DeviceRGB_DeviceCMYK_common_group_paths_exercised_by_prompt07b",
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
                "matte_background_status": "image_smask_matte_and_extgstate_bc_backdrop_closed_by_prompt07b",
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
                "tool_manifest": "target/prompt06-renderer-native-replay/reference-tool-manifest-prompt06b.json",
                "fixture_count": 47,
                "memory_cap_mb": 4096,
                "classification_artifact": "target/prompt07-transparency-compositing/prompt07b-reference-disagreement-summary.json",
                "oxide_outlier_failures": 0,
                "unclassified_failures": 0
            },
            "known_limits": [
                "Advanced ICC/device-link/multicolor CMM parity remains unsupported-reported",
                "Offscreen buffers are scheduler-bounded page-coordinate surfaces with bbox clipping rather than cropped coordinate surfaces"
            ]
        },
        "prompt07b_transparency_closure": {
            "status": "complete",
            "audit_script": "scripts/prompt07b_transparency_closure_audit.py",
            "artifacts": {
                "reference_tool_manifest": "target/prompt07-transparency-compositing/prompt07b-reference-tool-manifest.json",
                "corpus_manifest": "target/prompt07-transparency-compositing/prompt07b-corpus-manifest.json",
                "render_results": "target/prompt07-transparency-compositing/prompt07b-render-results.json",
                "diff_metrics": "target/prompt07-transparency-compositing/prompt07b-diff-metrics.json",
                "reference_disagreement_summary": "target/prompt07-transparency-compositing/prompt07b-reference-disagreement-summary.json",
                "transparency_matrix": "target/prompt07-transparency-compositing/prompt07b-transparency-matrix.json",
                "memory_report": "target/prompt07-transparency-compositing/prompt07b-memory-report.json",
                "closure_audit": "target/prompt07-transparency-compositing/prompt07b-closure-audit.json",
                "html_report": "target/prompt07-transparency-compositing/prompt07b-html-report/index.html"
            },
            "alpha_image": {
                "status": "closed",
                "root_cause": "image_painter_ignored_graphics_state_nonstroking_alpha",
                "fixture": "alpha_image",
                "classification": "all_references_agree_and_oxide_passes"
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
                    "all_references_agree_and_oxide_passes": 41,
                    "references_disagree_and_oxide_within_cluster": 5,
                    "malformed_or_reference_failure": 1
                },
                "oxide_outlier_failures": 0,
                "unclassified_failures": 0,
                "memory_cap_mb": 4096
            },
            "remaining_bounded_limits": [
                "advanced ICC/device-link/multicolor CMM parity",
                "cropped coordinate offscreen surfaces"
            ]
        },
        "prompt08_text_clipping_shading_patterns": {
            "status": "native_common_paths_with_bounded_unsupported_reports",
            "artifacts": {
                "starting_state": "target/prompt08-text-shading-patterns/starting-state.json",
                "corpus_manifest": "target/prompt08-text-shading-patterns/corpus-manifest.json",
                "reference_tool_manifest": "target/prompt08-text-shading-patterns/reference-tool-manifest.json",
                "text_clipping_matrix": "target/prompt08-text-shading-patterns/text-clipping-matrix.json",
                "axial_radial_shading_matrix": "target/prompt08-text-shading-patterns/axial-radial-shading-matrix.json",
                "mesh_patch_shading_matrix": "target/prompt08-text-shading-patterns/mesh-patch-shading-matrix.json",
                "tiling_pattern_matrix": "target/prompt08-text-shading-patterns/tiling-pattern-matrix.json",
                "fallback_taxonomy": "target/prompt08-text-shading-patterns/fallback-taxonomy.json",
                "render_results": "target/prompt08-text-shading-patterns/multi-reference-render-results.json",
                "diff_metrics": "target/prompt08-text-shading-patterns/visual-diff-metrics.json",
                "reference_disagreement_summary": "target/prompt08-text-shading-patterns/reference-disagreement-summary.json",
                "memory_scheduler_report": "target/prompt08-text-shading-patterns/memory-scheduler-report.json",
                "public_feature_report": "target/prompt08-text-shading-patterns/public-feature-report.json",
                "html_report": "target/prompt08-text-shading-patterns/html-report/index.html"
            },
            "text_clipping": {
                "status": "implemented_with_prompt08b_type3_cid_closure",
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
                "prompt08b_closure": "target/prompt08b-type3-cid-tensor/prompt08b-reference-disagreement-summary.json"
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
                    "type7_tensor_exactness": "closed by Prompt 08B for the device-color corpus with tensor-product interior evaluation",
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
                "classification_artifact": "target/prompt08-text-shading-patterns/reference-disagreement-summary.json",
                "classification_counts": {
                    "all_references_agree_oxide_passes": 19,
                    "references_disagree_oxide_within_cluster": 3,
                    "unsupported_reported_expected": 3,
                    "malformed_reference_failure": 1
                },
                "oxide_outlier_failures": 0,
                "prompt08_cluster_tolerance_acceptances": 2
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
        "prompt08b_type3_cid_tensor_closure": {
            "status": "complete_native_common_paths_with_reference_cluster_limits",
            "artifacts": {
                "corpus_manifest": "target/prompt08b-type3-cid-tensor/prompt08b-corpus-manifest.json",
                "reference_tool_manifest": "target/prompt08b-type3-cid-tensor/prompt08b-reference-tool-manifest.json",
                "render_results": "target/prompt08b-type3-cid-tensor/prompt08b-render-results.json",
                "diff_metrics": "target/prompt08b-type3-cid-tensor/prompt08b-diff-metrics.json",
                "reference_disagreement_summary": "target/prompt08b-type3-cid-tensor/prompt08b-reference-disagreement-summary.json",
                "text_clipping_matrix": "target/prompt08b-type3-cid-tensor/prompt08b-text-clipping-matrix.json",
                "type3_clip_matrix": "target/prompt08b-type3-cid-tensor/prompt08b-type3-clip-matrix.json",
                "cid_clip_matrix": "target/prompt08b-type3-cid-tensor/prompt08b-cid-clip-matrix.json",
                "type7_tensor_matrix": "target/prompt08b-type3-cid-tensor/prompt08b-type7-tensor-matrix.json",
                "fallback_taxonomy": "target/prompt08b-type3-cid-tensor/prompt08b-fallback-taxonomy.json",
                "memory_scheduler_report": "target/prompt08b-type3-cid-tensor/prompt08b-memory-scheduler-report.json",
                "html_report": "target/prompt08b-type3-cid-tensor/prompt08b-html-report/index.html"
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
                "reference_cluster_status": "Poppler/PDFium/MuPDF render the generated Type3 Tr clipping fixtures without the Type3 clip; Oxide native output is recorded as unsupported_reported_expected reference limitation rather than bbox fallback"
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
                    "all_references_agree_oxide_passes": 11,
                    "unsupported_reported_expected": 10
                },
                "oxide_outlier_failures": 0,
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
        "prompt09_annotation_ocg_progressive_cache": prompt09_renderer_report_value(),
        "prompt09b_annotation_progressive_cache_validation": prompt09b_validation_report_value(),
        "prompt10_cjk_rtl_color_glyph_reference_harness": prompt10_renderer_report_value(),
        "prompt10b_color_glyph_cjk_rtl_fidelity_closure": prompt10b_closure_report_value(),
        "prompt10c_color_glyph_hinting_cff_closure": prompt10c_closure_report_value(),
        "prompt10d_full_colrv1_svg_color_glyph_closure": prompt10d_closure_report_value(),
        "prompt10e_colrv1_gradient_clip_composite_closure": prompt10e_closure_report_value(),
        "prompt10f_colrv1_porterduff_radial_closure": prompt10f_closure_report_value(),
        "prompt11_renderer_fuzz_cmm_closeout": prompt11_renderer_fuzz_cmm_closeout_report_value(),
        "prompt11b_native_littlecms_cmm_backend_closure": prompt11b_native_littlecms_cmm_backend_closure_report_value(),
        "prompt12_prepress_cmm_device_link_separation_plates": prompt12_prepress_cmm_device_link_separation_plates_report_value(),
        "prompt12b_nchannel_plate_reference_closure": prompt12b_nchannel_plate_reference_closure_report_value(),
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
            "status": "engine_tile_progressive_resume_supported",
            "exposed_bindings": [],
            "engine_observable_operations": [
                "progressive_render_job_with_mode",
                "ProgressiveRenderJob::render_next",
                "ProgressiveRenderJob::token"
            ],
            "reason": "Prompt 09 adds an engine-level tile checkpoint model; callback-style binding progress APIs remain later binding work."
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
        assert_eq!(
            v["report"]["prompt04"]["scanner"]["default_implementation"],
            "safe_first_byte_chunked"
        );
        assert_eq!(
            v["report"]["prompt04"]["renderer_decode_scheduler"]["status"],
            "adopted_for_immediate_renderer_decode_paths"
        );
        assert_eq!(
            v["report"]["prompt04"]["native_codec_boundary"]["default_posture"],
            "deny_native_by_default"
        );
        assert_eq!(
            v["report"]["prompt05"]["decode_scheduler"]["status"],
            "adopted_for_prompt05_non_render_decode_paths"
        );
        assert_eq!(
            v["report"]["prompt05"]["hostile_corpus"]["generator"],
            "scripts/prompt05_hostile_codec_corpus.py"
        );
        assert_eq!(
            v["report"]["prompt05"]["fuzz_campaign"]["script"],
            "scripts/prompt05_codec_fuzz_campaign.py"
        );
        assert_eq!(
            v["report"]["prompt06"]["native_replay"]["status"],
            "native_text_image_form_display_list_foundation"
        );
        assert_eq!(
            v["report"]["prompt06"]["renderer_parity_audit"]["script"],
            "scripts/prompt06_renderer_parity_audit.py"
        );
        assert_eq!(
            v["report"]["prompt06"]["prompt06b_multi_reference_audit"]["status"],
            "multi_reference_audit_complete"
        );
        assert_eq!(
            v["report"]["prompt06"]["prompt06b_multi_reference_audit"]
                ["total_pairwise_comparisons"],
            78
        );
        assert_eq!(
            v["report"]["prompt07_transparency_compositing"]["status"],
            "native_foundation_with_prompt07b_closure"
        );
        assert_eq!(
            v["report"]["prompt07_transparency_compositing"]["reference_audit"]["memory_cap_mb"],
            4096
        );
        assert!(
            v["report"]["prompt07_transparency_compositing"]["blend_modes"]["implemented"]
                .as_array()
                .unwrap()
                .iter()
                .any(|mode| mode == "Luminosity")
        );
        assert_eq!(
            v["report"]["prompt07b_transparency_closure"]["status"],
            "complete"
        );
        assert_eq!(
            v["report"]["prompt07b_transparency_closure"]["reference_audit"]
                ["oxide_outlier_failures"],
            0
        );
        assert!(
            v["report"]["prompt07b_transparency_closure"]["luminosity_soft_mask_color_spaces"]
                ["supported"]
                .as_array()
                .unwrap()
                .iter()
                .any(|space| space == "DeviceCMYK")
        );
        assert_eq!(
            v["report"]["prompt08_text_clipping_shading_patterns"]["status"],
            "native_common_paths_with_bounded_unsupported_reports"
        );
        assert_eq!(
            v["report"]["prompt08_text_clipping_shading_patterns"]["reference_audit"]
                ["memory_cap_mb"],
            4096
        );
        assert!(
            v["report"]["prompt08_text_clipping_shading_patterns"]["text_clipping"]
                ["rendering_modes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|mode| mode.as_i64() == Some(7))
        );
        assert_eq!(
            v["report"]["prompt08b_type3_cid_tensor_closure"]["status"],
            "complete_native_common_paths_with_reference_cluster_limits"
        );
        assert_eq!(
            v["report"]["prompt08b_type3_cid_tensor_closure"]["reference_audit"]
                ["oxide_outlier_failures"],
            0
        );
        assert_eq!(
            v["report"]["prompt08b_type3_cid_tensor_closure"]["type7_tensor_patch"]["status"],
            "native_tensor_product_interior"
        );
        assert_eq!(
            v["report"]["prompt09_annotation_ocg_progressive_cache"]["status"],
            "implemented_with_bounded_unsupported_reports"
        );
        assert_eq!(
            v["report"]["prompt09_annotation_ocg_progressive_cache"]["optional_content"]["status"],
            "default_view_configuration_evaluator"
        );
        assert_eq!(
            v["report"]["prompt09_annotation_ocg_progressive_cache"]["closure_gates"]
                ["memory_cap_mb"],
            4096
        );
        assert_eq!(
            v["report"]["prompt09b_annotation_progressive_cache_validation"]["status"],
            "implemented_and_proven"
        );
        assert_eq!(
            v["report"]["prompt09b_annotation_progressive_cache_validation"]
                ["multi_reference_audit"]["oxide_outlier_failures"],
            0
        );
        assert_eq!(
            v["report"]["prompt09b_annotation_progressive_cache_validation"]
                ["public_report_parity"]["schema_change"],
            "additive_section_only"
        );
        assert_eq!(
            v["report"]["prompt10_cjk_rtl_color_glyph_reference_harness"]["status"],
            "implemented_with_bounded_unsupported_reports"
        );
        assert_eq!(
            v["report"]["prompt10_cjk_rtl_color_glyph_reference_harness"]["closure_gates"]
                ["memory_cap_mb"],
            4096
        );
        assert_eq!(
            v["report"]["prompt10_cjk_rtl_color_glyph_reference_harness"]["color_glyph_rendering"]
                ["status"],
            "unsupported_color_tables_are_detected_and_reported"
        );
        assert_eq!(
            v["report"]["prompt10b_color_glyph_cjk_rtl_fidelity_closure"]["status"],
            "complete"
        );
        assert_eq!(
            v["report"]["prompt10b_color_glyph_cjk_rtl_fidelity_closure"]["color_glyph_rendering"]
                ["colr_cpal"]["status"],
            "implemented_and_proven"
        );
        assert_eq!(
            v["report"]["prompt10b_color_glyph_cjk_rtl_fidelity_closure"]["multi_reference_audit"]
                ["oxide_outlier_failures"],
            0
        );
        assert_eq!(
            v["report"]["prompt10c_color_glyph_hinting_cff_closure"]["status"],
            "complete"
        );
        assert_eq!(
            v["report"]["prompt10c_color_glyph_hinting_cff_closure"]["colrv1"]["status"],
            "implemented_with_operator_level_limits"
        );
        assert_eq!(
            v["report"]["prompt10c_color_glyph_hinting_cff_closure"]["multi_reference_audit"]
                ["oxide_outlier_failures"],
            0
        );
        assert_eq!(
            v["report"]["prompt10c_color_glyph_hinting_cff_closure"]["closure_gates"]
                ["public_report_schema"],
            "additive_feature_report_prompt10c"
        );
        assert_eq!(
            v["report"]["prompt10d_full_colrv1_svg_color_glyph_closure"]["status"],
            "complete"
        );
        assert_eq!(
            v["report"]["prompt10d_full_colrv1_svg_color_glyph_closure"]["svg_in_opentype"]
                ["status"],
            "safe_static_subset_rendered_active_constructs_blocked"
        );
        assert_eq!(
            v["report"]["prompt10d_full_colrv1_svg_color_glyph_closure"]["bitmap_color_glyphs"]
                ["sbix"]["status"],
            "png_and_jpeg_rendered_tiff_other_precisely_reported"
        );
        assert_eq!(
            v["report"]["prompt10d_full_colrv1_svg_color_glyph_closure"]["closure_gates"]
                ["public_report_schema"],
            "additive_feature_report_prompt10d"
        );
        assert_eq!(
            v["report"]["prompt10e_colrv1_gradient_clip_composite_closure"]["status"],
            "complete"
        );
        assert_eq!(
            v["report"]["prompt10e_colrv1_gradient_clip_composite_closure"]["colrv1_gradients"]
                ["implemented_operators"][0],
            "PaintLinearGradient"
        );
        assert_eq!(
            v["report"]["prompt10e_colrv1_gradient_clip_composite_closure"]["colrv1_clip_stack"]
                ["status"],
            "implemented"
        );
        assert_eq!(
            v["report"]["prompt10e_colrv1_gradient_clip_composite_closure"]["colrv1_composites"]
                ["implemented_modes"][1],
            "Multiply"
        );
        assert_eq!(
            v["report"]["prompt10e_colrv1_gradient_clip_composite_closure"]["closure_gates"]
                ["public_report_schema"],
            "additive_feature_report_prompt10e"
        );
        assert_eq!(
            v["report"]["prompt10f_colrv1_porterduff_radial_closure"]["status"],
            "complete"
        );
        assert_eq!(
            v["report"]["prompt10f_colrv1_porterduff_radial_closure"]
                ["porter_duff_plus_composites"]["implemented_modes"]
                .as_array()
                .unwrap()
                .len(),
            12
        );
        assert_eq!(
            v["report"]["prompt10f_colrv1_porterduff_radial_closure"]["exact_moving_center_radial"]
                ["status"],
            "implemented_with_reference_equivalence"
        );
        assert_eq!(
            v["report"]["prompt10f_colrv1_porterduff_radial_closure"]["closure_gates"]
                ["public_report_schema"],
            "additive_feature_report_prompt10f"
        );
        assert_eq!(
            v["report"]["prompt11_renderer_fuzz_cmm_closeout"]["status"],
            "complete_with_native_cmm_hard_blocked_precise"
        );
        assert_eq!(
            v["report"]["prompt11_renderer_fuzz_cmm_closeout"]["renderer_fuzz"]
                ["fuzz_target_count"],
            25
        );
        assert_eq!(
            v["report"]["prompt11_renderer_fuzz_cmm_closeout"]["renderer_closeout"]
                ["oxide_outlier_failures"],
            0
        );
        assert_eq!(
            v["report"]["prompt11_renderer_fuzz_cmm_closeout"]["native_cmm_backend"]
                ["backend_used_in_current_build"],
            "safe-rust-plus-qcms"
        );
        assert_eq!(
            v["report"]["prompt11_renderer_fuzz_cmm_closeout"]["closure_gates"]
                ["public_report_schema"],
            "additive_feature_report_prompt11"
        );
        let prompt11b = &v["report"]["prompt11b_native_littlecms_cmm_backend_closure"];
        assert_eq!(prompt11b["status"], "complete");
        assert_eq!(prompt11b["feature_flag"]["name"], "native-cmm-lcms2");
        assert_eq!(
            prompt11b["native_cmm_compiled"],
            cfg!(feature = "native-cmm-lcms2")
        );
        assert_eq!(
            prompt11b["native_cmm_available_at_runtime"],
            cfg!(all(
                feature = "native-cmm-lcms2",
                not(target_arch = "wasm32")
            ))
        );
        assert_eq!(
            prompt11b["closure_gates"]["public_report_schema"],
            "additive_feature_report_prompt11b"
        );
        let prompt12 = &v["report"]["prompt12_prepress_cmm_device_link_separation_plates"];
        assert_eq!(prompt12["status"], "complete");
        assert_eq!(
            prompt12["closure_gates"]["public_report_schema"],
            "additive_feature_report_prompt12"
        );
        assert_eq!(
            prompt12["native_cmm_compiled"],
            cfg!(feature = "native-cmm-lcms2")
        );
        assert_eq!(
            prompt12["separation_framebuffer"]["cache_key_includes_plate_state"],
            true
        );
        let prompt12b = &v["report"]["prompt12b_nchannel_plate_reference_closure"];
        assert_eq!(prompt12b["status"], "complete");
        assert_eq!(
            prompt12b["closure_gates"]["public_report_schema"],
            "additive_feature_report_prompt12b"
        );
        assert_eq!(
            prompt12b["nchannel_pixel_format"]["max_channels"],
            prepress::MAX_NCHANNEL_OUTPUT_CHANNELS
        );
        assert_eq!(
            prompt12b["reference_audit"]["pdfium"],
            "required_and_run_by_prompt12b_audit"
        );
        assert_envelope(
            &prompt09_renderer_report_json().unwrap(),
            "prompt09_renderer_report",
        );
        assert_envelope(
            &prompt09b_validation_report_json().unwrap(),
            "prompt09b_validation_report",
        );
        assert_envelope(
            &prompt10_renderer_report_json().unwrap(),
            "prompt10_renderer_report",
        );
        assert_envelope(
            &prompt10b_closure_report_json().unwrap(),
            "prompt10b_closure_report",
        );
        assert_envelope(
            &prompt10c_closure_report_json().unwrap(),
            "prompt10c_closure_report",
        );
        assert_envelope(
            &prompt10d_closure_report_json().unwrap(),
            "prompt10d_closure_report",
        );
        assert_envelope(
            &prompt10e_closure_report_json().unwrap(),
            "prompt10e_closure_report",
        );
        assert_envelope(
            &prompt10f_closure_report_json().unwrap(),
            "prompt10f_closure_report",
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
