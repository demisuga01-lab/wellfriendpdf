use std::thread;

use flate2::write::ZlibEncoder;
use flate2::Compression;
use oxide_engine::{
    codec_backend_registry, codec_dimension_report, decode_filter_with_isolation,
    native_codec_dependency_allowlist, select_codec_backend, validate_codec_registry_policy,
    CodecBackendPreference, CodecIsolationConfig, CodecIsolationPolicy,
};
use std::io::Write as _;
use std::path::PathBuf;

fn worker_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_oxide-codec-worker"))
}

fn flate_bytes(data: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

fn config(policy: CodecIsolationPolicy) -> CodecIsolationConfig {
    CodecIsolationConfig::with_policy(policy)
        .with_worker_path(worker_path())
        .with_timeout_ms(500)
        .with_max_decoded_bytes(1024 * 1024)
}

#[test]
fn isolated_required_decodes_flate_successfully() {
    let input = flate_bytes(b"hello isolated codec");
    let result = decode_filter_with_isolation(
        "FlateDecode",
        &input,
        &config(CodecIsolationPolicy::IsolatedRequired),
    );
    assert_eq!(
        result.decoded.as_deref(),
        Some(&b"hello isolated codec"[..])
    );
    assert_eq!(result.report.status, "success");
    assert_eq!(result.report.isolation_mode, "subprocess");
    assert!(result.report.worker_used);
}

#[test]
fn malformed_input_is_structured_failure() {
    let result = decode_filter_with_isolation(
        "FlateDecode",
        b"not flate",
        &config(CodecIsolationPolicy::IsolatedRequired),
    );
    assert!(result.decoded.is_none());
    assert!(!result.report.ok);
    assert!(result
        .report
        .errors
        .iter()
        .any(|err| err.contains("decode failure") || err.contains("FlateDecode")));
}

#[test]
fn missing_worker_fails_closed_when_required() {
    let mut cfg = CodecIsolationConfig::with_policy(CodecIsolationPolicy::IsolatedRequired)
        .with_worker_path("target/no-such-codec-worker.exe");
    cfg.limits.timeout_milliseconds = 100;
    let result = decode_filter_with_isolation("FlateDecode", &flate_bytes(b"x"), &cfg);
    assert!(result.decoded.is_none());
    assert_eq!(result.report.status, "failed_closed");
    assert!(result
        .report
        .errors
        .iter()
        .any(|e| e.contains("does not exist")));
}

#[test]
fn isolated_preferred_reports_fallback() {
    let mut cfg = CodecIsolationConfig::with_policy(CodecIsolationPolicy::IsolatedPreferred)
        .with_worker_path("target/no-such-codec-worker.exe");
    cfg.limits.timeout_milliseconds = 100;
    let result = decode_filter_with_isolation("FlateDecode", &flate_bytes(b"fallback"), &cfg);
    assert_eq!(result.decoded.as_deref(), Some(&b"fallback"[..]));
    assert_eq!(result.report.status, "fallback_success");
    assert!(result.report.fallback_used);
    assert_eq!(
        result.report.fallback_reason.as_deref(),
        Some("worker_unavailable_or_failed")
    );
}

#[test]
fn report_only_does_not_decode() {
    let result = decode_filter_with_isolation(
        "FlateDecode",
        &flate_bytes(b"hidden"),
        &CodecIsolationConfig::with_policy(CodecIsolationPolicy::ReportOnly),
    );
    assert!(result.decoded.is_none());
    assert_eq!(result.report.status, "report_only");
}

#[test]
fn disabled_reports_unavailable() {
    let result = decode_filter_with_isolation(
        "FlateDecode",
        &flate_bytes(b"hidden"),
        &CodecIsolationConfig::with_policy(CodecIsolationPolicy::Disabled),
    );
    assert!(result.decoded.is_none());
    assert_eq!(result.report.status, "disabled");
}

#[test]
fn central_codec_registry_enforces_pure_rust_defaults() {
    let entries = codec_backend_registry();
    assert!(
        entries.iter().any(|entry| entry.codec_kind == "FlateDecode"
            && entry.implementation_language == "rust"
            && entry.default_enabled
            && entry.worker_supported),
        "FlateDecode must be represented in the central registry"
    );
    assert!(
        entries
            .iter()
            .filter(|entry| entry.native_dependency.is_some())
            .all(|entry| !entry.default_enabled
                && entry.feature_flag == Some("native-codecs")
                && entry.worker_required_for_native
                && !entry.in_process_allowed_by_default),
        "native entries must be denied by default and worker-gated"
    );
    assert!(
        validate_codec_registry_policy().is_empty(),
        "registry policy errors: {:?}",
        validate_codec_registry_policy()
    );
    assert_eq!(native_codec_dependency_allowlist().len(), 0);
}

#[test]
fn native_backend_selection_is_denied_by_default() {
    let selection = select_codec_backend(
        "DCTDecode",
        CodecBackendPreference::NativeInProcess,
        &CodecIsolationPolicy::InProcess,
    );
    assert!(!selection.ok);
    assert_eq!(selection.status, "native_backend_blocked");
    assert!(!selection.native_codecs_compiled);
    assert_eq!(
        selection.reason.as_deref(),
        Some("no native backend is registered and allowlisted for this codec")
    );
}

#[test]
fn codec_isolation_report_exposes_backend_boundary_fields() {
    let result = decode_filter_with_isolation(
        "FlateDecode",
        &flate_bytes(b"boundary fields"),
        &CodecIsolationConfig::with_policy(CodecIsolationPolicy::InProcess),
    );
    assert_eq!(result.decoded.as_deref(), Some(&b"boundary fields"[..]));
    assert_eq!(
        result.report.backend_selection.selected_backend.as_deref(),
        Some("oxide-rust-flate2")
    );
    assert_eq!(
        result
            .report
            .backend_selection
            .implementation_language
            .as_deref(),
        Some("rust")
    );
    assert!(result.report.native_boundary.pure_rust_default);
    assert!(
        result
            .report
            .native_boundary
            .unknown_native_dependencies_fail_closed
    );
}

#[test]
fn worker_nonzero_and_crash_are_contained() {
    for mode in ["nonzero", "crash"] {
        let mut cfg = config(CodecIsolationPolicy::IsolatedRequired);
        cfg.worker_test_mode = Some(mode.to_string());
        let result = decode_filter_with_isolation("FlateDecode", &flate_bytes(b"x"), &cfg);
        assert!(result.decoded.is_none(), "mode {mode}");
        assert_eq!(result.report.status, "failed_closed", "mode {mode}");
        assert!(
            result.report.errors.iter().any(|e| e.contains("exited"))
                || result.report.errors.iter().any(|e| e.contains("failed")),
            "mode {mode}: {:?}",
            result.report.errors
        );
    }
}

#[test]
fn worker_timeout_is_contained() {
    let mut cfg = config(CodecIsolationPolicy::IsolatedRequired).with_timeout_ms(50);
    cfg.worker_test_mode = Some("timeout".to_string());
    let result = decode_filter_with_isolation("FlateDecode", &flate_bytes(b"x"), &cfg);
    assert!(result.decoded.is_none());
    assert_eq!(result.report.status, "failed_closed");
    assert!(result.report.errors.iter().any(|e| e.contains("timed out")));
}

#[test]
fn malformed_worker_response_is_rejected() {
    let mut cfg = config(CodecIsolationPolicy::IsolatedRequired);
    cfg.worker_test_mode = Some("malformed".to_string());
    let result = decode_filter_with_isolation("FlateDecode", &flate_bytes(b"x"), &cfg);
    assert!(result.decoded.is_none());
    assert_eq!(result.report.status, "failed_closed");
    assert!(result
        .report
        .errors
        .iter()
        .any(|e| e.contains("invalid worker response JSON")));
}

#[test]
fn wrong_request_id_is_rejected() {
    let mut cfg = config(CodecIsolationPolicy::IsolatedRequired);
    cfg.worker_test_mode = Some("wrong-id".to_string());
    let result = decode_filter_with_isolation("FlateDecode", &flate_bytes(b"x"), &cfg);
    assert!(result.decoded.is_none());
    assert_eq!(result.report.status, "failed_closed");
    assert!(result
        .report
        .errors
        .iter()
        .any(|e| e.contains("request_id")));
}

#[test]
fn worker_returning_too_many_bytes_is_rejected() {
    let mut cfg = config(CodecIsolationPolicy::IsolatedRequired).with_max_decoded_bytes(4);
    cfg.worker_test_mode = Some("oversized".to_string());
    let result = decode_filter_with_isolation("FlateDecode", &flate_bytes(b"x"), &cfg);
    assert!(result.decoded.is_none());
    assert_eq!(result.report.status, "failed_closed");
    assert!(result
        .report
        .errors
        .iter()
        .any(|e| e.contains("max_decoded_bytes")));
}

#[test]
fn unsupported_codec_is_structured() {
    let result = decode_filter_with_isolation(
        "DCTDecode",
        b"not really jpeg",
        &config(CodecIsolationPolicy::IsolatedRequired),
    );
    assert!(result.decoded.is_none());
    assert_eq!(result.report.status, "failed_closed");
    assert!(result
        .report
        .errors
        .iter()
        .any(|e| e.contains("unsupported") || e.contains("not enabled")));
}

#[test]
fn input_and_decoded_output_caps_are_enforced() {
    let mut cfg = config(CodecIsolationPolicy::IsolatedRequired);
    cfg.limits.max_input_bytes = 1;
    let result =
        decode_filter_with_isolation("FlateDecode", &flate_bytes(b"too large input"), &cfg);
    assert_eq!(result.report.status, "input_cap_exceeded");
    assert_eq!(
        result.report.limit_failed.as_deref(),
        Some("max_input_bytes")
    );

    let cfg = config(CodecIsolationPolicy::IsolatedRequired).with_max_decoded_bytes(3);
    let result = decode_filter_with_isolation("FlateDecode", &flate_bytes(b"abcd"), &cfg);
    assert!(result.decoded.is_none());
    assert!(result
        .report
        .errors
        .iter()
        .any(|e| e.contains("byte limit") || e.contains("decode failure")));
}

#[test]
fn dimensions_exceeding_caps_are_reported_before_decode() {
    let mut cfg = config(CodecIsolationPolicy::IsolatedRequired);
    cfg.limits.max_pixels = 10;
    let report = codec_dimension_report("JPXDecode", 4, 4, 3, &cfg);
    assert!(!report.ok);
    assert_eq!(report.status, "dimension_cap_exceeded");
    assert_eq!(report.limit_failed.as_deref(), Some("max_pixels"));
}

#[test]
fn concurrent_worker_requests_complete_independently() {
    let handles: Vec<_> = (0..6)
        .map(|idx| {
            thread::spawn(move || {
                let text = format!("payload-{idx}");
                let result = decode_filter_with_isolation(
                    "FlateDecode",
                    &flate_bytes(text.as_bytes()),
                    &config(CodecIsolationPolicy::IsolatedRequired),
                );
                assert_eq!(result.decoded.as_deref(), Some(text.as_bytes()));
                result.report.request_id
            })
        })
        .collect();
    let mut ids = Vec::new();
    for handle in handles {
        ids.push(handle.join().unwrap());
    }
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 6);
}
