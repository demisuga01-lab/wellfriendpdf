use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde_json::json;
use wellfriendpdf_engine::codec_isolation::{
    codec_backend_registry, codec_native_boundary_report, native_codec_dependency_allowlist,
    select_codec_backend, validate_codec_registry_policy, CodecBackendPreference,
    CodecIsolationPolicy,
};
use wellfriendpdf_engine::decode_scanner::{
    scan_pdf_markers_accelerated, scan_pdf_markers_scalar, scanner_availability_report,
};
use wellfriendpdf_engine::decode_scheduler::renderer_decode_scheduler_adoption_report;
use wellfriendpdf_engine::{sdk, ContentEngine};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/codec_boundary-codec-boundary-scheduler"));
    fs::create_dir_all(&out_dir)?;

    write_json(
        &out_dir.join("native-codec-boundary-report.json"),
        &native_boundary_report(),
    )?;
    write_json(
        &out_dir.join("scanner-parity-report.json"),
        &scanner_parity_report(),
    )?;
    write_json(
        &out_dir.join("scanner-benchmark-report.json"),
        &scanner_benchmark_report(),
    )?;
    write_json(
        &out_dir.join("renderer-scheduler-report.json"),
        &renderer_scheduler_report(),
    )?;
    write_json(
        &out_dir.join("sdk-report-parity.json"),
        &sdk_report_parity()?,
    )?;

    Ok(())
}

fn native_boundary_report() -> serde_json::Value {
    let native_in_process = select_codec_backend(
        "DCTDecode",
        CodecBackendPreference::NativeInProcess,
        &CodecIsolationPolicy::InProcess,
    );
    let default_flate = select_codec_backend(
        "FlateDecode",
        CodecBackendPreference::Default,
        &CodecIsolationPolicy::InProcess,
    );
    json!({
        "schema_version": 1,
        "feature_area": "combined_codec_boundary",
        "policy": codec_native_boundary_report(),
        "registry_policy_errors": validate_codec_registry_policy(),
        "native_dependency_allowlist": native_codec_dependency_allowlist(),
        "registry": codec_backend_registry(),
        "selection_smoke": {
            "default_flate": default_flate,
            "native_in_process_dct": native_in_process
        }
    })
}

fn scanner_parity_report() -> serde_json::Value {
    let mut cases: Vec<Vec<u8>> = vec![
        b"1 0 obj\n<<>>\nstream\nabc\nendstream\nendobj\nstartxref\n0".to_vec(),
        b"% comment with obj endstream trailer\n(literal stream) /Name#20xref".to_vec(),
        b"BI /W 1 /H 1 /CS /RGB ID binary endstream obj EI".to_vec(),
        b"xref\n0 1\n0000000000 65535 f\ntrailer\n<<>>\nstartxref\n0".to_vec(),
    ];
    let mut random = Vec::new();
    let mut state = 0xC0DEC0DE_u64;
    for i in 0..8192 {
        state = state
            .wrapping_mul(2862933555777941757)
            .wrapping_add(3037000493);
        random.push((state >> 24) as u8);
        if i % 1024 == 0 {
            random.extend_from_slice(b" obj stream endstream trailer startxref xref ");
        }
    }
    cases.push(random);

    let mut total_candidates = 0usize;
    let mut mismatches = Vec::new();
    for (idx, data) in cases.iter().enumerate() {
        let scalar = scan_pdf_markers_scalar(data).candidates;
        let accelerated = scan_pdf_markers_accelerated(data).candidates;
        total_candidates += accelerated.len();
        if scalar != accelerated {
            mismatches.push(idx);
        }
    }
    json!({
        "schema_version": 1,
        "feature_area": "combined_codec_boundary",
        "scanner": scanner_availability_report(),
        "cases": cases.len(),
        "total_candidates": total_candidates,
        "scalar_accelerated_equal": mismatches.is_empty(),
        "mismatched_cases": mismatches,
        "malformed_contexts_covered": [
            "comments",
            "literal_strings",
            "name_objects",
            "inline_image_like_binary",
            "xref_and_trailer_markers"
        ]
    })
}

fn scanner_benchmark_report() -> serde_json::Value {
    let mut data = Vec::new();
    for i in 0..4096 {
        data.extend_from_slice(b"0000000000 00000 n \nrandom payload without many markers ");
        if i % 64 == 0 {
            data.extend_from_slice(b"1 0 obj\n<<>>\nstream\npayload\nendstream\nendobj\n");
        }
    }
    let iterations = 100usize;
    let scalar_start = Instant::now();
    let mut scalar_candidates = 0usize;
    for _ in 0..iterations {
        scalar_candidates += scan_pdf_markers_scalar(std::hint::black_box(&data))
            .candidates
            .len();
    }
    let scalar_ns = scalar_start.elapsed().as_nanos();

    let accelerated_start = Instant::now();
    let mut accelerated_candidates = 0usize;
    for _ in 0..iterations {
        accelerated_candidates += scan_pdf_markers_accelerated(std::hint::black_box(&data))
            .candidates
            .len();
    }
    let accelerated_ns = accelerated_start.elapsed().as_nanos();

    json!({
        "schema_version": 1,
        "feature_area": "combined_codec_boundary",
        "input_bytes": data.len(),
        "iterations": iterations,
        "scalar_total_candidates": scalar_candidates,
        "accelerated_total_candidates": accelerated_candidates,
        "candidate_counts_equal": scalar_candidates == accelerated_candidates,
        "scalar_elapsed_ns": scalar_ns,
        "accelerated_elapsed_ns": accelerated_ns,
        "speedup_ratio": if accelerated_ns == 0 { 0.0 } else { scalar_ns as f64 / accelerated_ns as f64 },
        "implementation": "safe_first_byte_chunked"
    })
}

fn renderer_scheduler_report() -> serde_json::Value {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("image_only.pdf");
    let mut deterministic = false;
    let mut first_hash = 0u64;
    let mut second_hash = 0u64;
    let mut render_error = None;
    match ContentEngine::open_path(&fixture) {
        Ok(engine) => {
            let first = engine.render_page(1, 72);
            let second = engine.render_page(1, 72);
            match (first, second) {
                (Ok(first), Ok(second)) => {
                    first_hash = hash_pixels(&first.to_raw_image().pixels);
                    second_hash = hash_pixels(&second.to_raw_image().pixels);
                    deterministic = first_hash == second_hash;
                }
                (Err(err), _) | (_, Err(err)) => render_error = Some(err.to_string()),
            }
        }
        Err(err) => render_error = Some(err.to_string()),
    }

    json!({
        "schema_version": 1,
        "feature_area": "combined_codec_boundary",
        "adoption": renderer_decode_scheduler_adoption_report(),
        "deterministic_image_fixture": deterministic,
        "first_render_hash": first_hash,
        "second_render_hash": second_hash,
        "render_error": render_error,
        "memory_token_tests": [
            "renderer_inline_decode_acquires_scheduler_token",
            "renderer_decode_scheduler_fails_closed_over_budget",
            "renderer_decode_scheduler_observes_cancel_before_decode"
        ]
    })
}

fn sdk_report_parity() -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let feature: serde_json::Value = serde_json::from_str(&sdk::feature_report_json()?)?;
    let codec: serde_json::Value = serde_json::from_str(&sdk::codec_isolation_report_json(
        "FlateDecode",
        &wellfriendpdf_engine::flate_encode(b"codec_boundary parity", 6),
        Some("report_only"),
    )?)?;
    Ok(json!({
        "schema_version": 1,
        "feature_area": "combined_codec_boundary",
        "envelope_version": feature["schema_version"],
        "feature_report_kind": feature["kind"],
        "codec_report_kind": codec["kind"],
        "rust_sdk_codec_boundary_fields": {
            "native_codec_boundary": feature["report"]["codec_boundary"]["native_codec_boundary"].is_object(),
            "scanner": feature["report"]["codec_boundary"]["scanner"]["default_implementation"],
            "renderer_decode_scheduler": feature["report"]["codec_boundary"]["renderer_decode_scheduler"]["status"],
            "rlbox_wasm": feature["report"]["codec_boundary"]["rlbox_wasm"]["status"]
        },
        "codec_report_backend_selection_present": codec["report"]["backend_selection"].is_object(),
        "codec_report_native_boundary_present": codec["report"]["native_boundary"].is_object(),
        "binding_surfaces": [
            "rust_sdk_shared_facade",
            "cli_codec_isolation_report_json_shape",
            "python_shared_facade",
            "c_abi_shared_facade",
            "wasm_shared_facade",
            "dotnet_c_abi_facade",
            "java_c_abi_facade"
        ],
        "schema_versioning": "report envelope remains version 1; Codec Boundary adds inner report fields"
    }))
}

fn hash_pixels(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

fn write_json(path: &Path, value: &serde_json::Value) -> Result<(), Box<dyn std::error::Error>> {
    fs::write(path, serde_json::to_string_pretty(value)? + "\n")?;
    Ok(())
}
