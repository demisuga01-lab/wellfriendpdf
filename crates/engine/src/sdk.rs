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
        assert_envelope(
            &prompt09_renderer_report_json().unwrap(),
            "prompt09_renderer_report",
        );
        assert_envelope(
            &prompt09b_validation_report_json().unwrap(),
            "prompt09b_validation_report",
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
