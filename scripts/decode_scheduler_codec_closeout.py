#!/usr/bin/env python3
"""Generate Decode Scheduler codec scheduler/corpus/fuzz close-out artifacts."""

from __future__ import annotations

import argparse
import json
import sys
from collections import Counter
from pathlib import Path
from typing import Any


OUT_DIR = Path("target/decode_scheduler-codec-closeout")
INVENTORY = OUT_DIR / "decode-callsite-inventory.json"
MATRIX = OUT_DIR / "codec-coverage-matrix.json"
HOSTILE_RUN = OUT_DIR / "hostile-corpus-run.json"
FUZZ_INVENTORY = OUT_DIR / "fuzz-target-inventory.json"
FUZZ_SMOKE = OUT_DIR / "fuzz-smoke-report.json"
PERFORMANCE = OUT_DIR / "performance-report.json"
VERDICT = OUT_DIR / "closeout-verdict.json"


CALLSITES: list[dict[str, Any]] = [
    {
        "id": "raw_stream_decoding",
        "module": "crates/engine/src/filters.rs",
        "entry_points": ["decode_stream_lossless_with_limits", "decode_stream_with_limits"],
        "decode_behavior": "lossless stream bytes decoded under DecodeLimits",
        "status": "scheduler_covered",
        "artifact": "target/decode_scheduler-codec-closeout/codec-coverage-matrix.json",
        "test_or_smoke": "cargo test --workspace --jobs 1; hostile corpus runner",
    },
    {
        "id": "filter_chain_decoding",
        "module": "crates/engine/src/filters.rs",
        "entry_points": ["decode_stream_from_dict_with_limits", "apply_filter_bytes_with_limits"],
        "decode_behavior": "filter chains remain bounded by chain depth and output caps",
        "status": "scheduler_covered",
        "artifact": "target/decode_scheduler-codec-closeout/hostile-corpus-run.json",
        "test_or_smoke": "filter_chain_loops, unknown_filters, wrong_decodeparms fixtures",
    },
    {
        "id": "text_extraction_resource_decoding",
        "module": "crates/engine/src/engine.rs",
        "entry_points": ["ContentEngine::get_page_content"],
        "decode_behavior": "page content streams are admitted by scheduler before tokenization",
        "status": "scheduler_covered",
        "artifact": "target/decode_scheduler-codec-closeout/decode-callsite-inventory.json",
        "test_or_smoke": "cargo test --workspace --jobs 1",
    },
    {
        "id": "image_xobject_decoding",
        "module": "crates/engine/src/engine.rs; crates/engine/src/images/decoder.rs",
        "entry_points": ["ContentEngine::decode_image", "ImageDecoder::decode_with_limits"],
        "decode_behavior": "XObject image decode uses scheduler admission plus image/filter caps",
        "status": "scheduler_covered",
        "artifact": "target/decode_scheduler-codec-closeout/hostile-corpus-run.json",
        "test_or_smoke": "hostile image fixtures and cargo test",
    },
    {
        "id": "inline_image_decoding",
        "module": "crates/engine/src/engine.rs; crates/engine/src/images/decoder.rs",
        "entry_points": ["ContentEngine::decode_inline_image", "ImageDecoder::decode_inline_with_limits"],
        "decode_behavior": "inline image bytes are scheduler-admitted and limits-aware",
        "status": "scheduler_covered",
        "artifact": "target/decode_scheduler-codec-closeout/hostile-corpus-run.json",
        "test_or_smoke": "inline_image_eod_ambiguity fixture",
    },
    {
        "id": "soft_mask_decoding",
        "module": "crates/engine/src/render/page_renderer.rs",
        "entry_points": ["scheduled_load_smask"],
        "decode_behavior": "renderer soft mask path covered by Codec Boundary scheduler",
        "status": "already_covered",
        "artifact": "target/codec_boundary-codec-boundary-scheduler/renderer-scheduler-report.json",
        "test_or_smoke": "renderer Codec Boundary scheduler tests",
    },
    {
        "id": "stencil_mask_decoding",
        "module": "crates/engine/src/render/page_renderer.rs",
        "entry_points": ["scheduled_decode_image"],
        "decode_behavior": "renderer stencil/image-mask path covered by Codec Boundary scheduler",
        "status": "already_covered",
        "artifact": "target/codec_boundary-codec-boundary-scheduler/renderer-scheduler-report.json",
        "test_or_smoke": "renderer Codec Boundary scheduler tests",
    },
    {
        "id": "semantic_extraction_resource_decoding",
        "module": "crates/engine/src/parse.rs; crates/engine/src/semantic.rs",
        "entry_points": ["parse_document", "extract_semantic_document"],
        "decode_behavior": "semantic extraction uses ContentEngine text/image resource paths",
        "status": "scheduler_covered",
        "artifact": "target/decode_scheduler-codec-closeout/decode-callsite-inventory.json",
        "test_or_smoke": "cargo test --workspace --jobs 1",
    },
    {
        "id": "table_extraction_auxiliary_decoding",
        "module": "crates/engine/src/extract.rs; crates/engine/src/text.rs",
        "entry_points": ["extract_fields", "extract_text_semantic_model"],
        "decode_behavior": "table/field auxiliary extraction consumes scheduled page content decode",
        "status": "scheduler_covered",
        "artifact": "target/decode_scheduler-codec-closeout/decode-callsite-inventory.json",
        "test_or_smoke": "cargo test --workspace --jobs 1",
    },
    {
        "id": "font_stream_decoding",
        "module": "crates/engine/src/fonts",
        "entry_points": ["decode_stream_lossless_with_limits"],
        "decode_behavior": "font streams use central lossless stream scheduler and DecodeLimits",
        "status": "scheduler_covered",
        "artifact": "target/decode_scheduler-codec-closeout/codec-coverage-matrix.json",
        "test_or_smoke": "cargo test --workspace --jobs 1",
    },
    {
        "id": "cmap_stream_decoding",
        "module": "crates/engine/src/fonts/cmap.rs",
        "entry_points": ["decode_stream_lossless_with_limits"],
        "decode_behavior": "CMap streams use central lossless stream scheduler and DecodeLimits",
        "status": "scheduler_covered",
        "artifact": "target/decode_scheduler-codec-closeout/codec-coverage-matrix.json",
        "test_or_smoke": "cargo test --workspace --jobs 1",
    },
    {
        "id": "icc_profile_stream_decoding",
        "module": "crates/engine/src/color_report.rs; crates/engine/src/render/cmm.rs",
        "entry_points": ["decode_stream_lossless_with_limits"],
        "decode_behavior": "ICC profile streams use central lossless stream scheduler and DecodeLimits",
        "status": "scheduler_covered",
        "artifact": "target/decode_scheduler-codec-closeout/hostile-corpus-run.json",
        "test_or_smoke": "malformed_icc_profiles fixture",
    },
    {
        "id": "thumbnail_decoding",
        "module": "crates/engine/src/images",
        "entry_points": ["ContentEngine::decode_image"],
        "decode_behavior": "thumbnail/image decode goes through shared image decode scheduler when extracted",
        "status": "scheduler_covered",
        "artifact": "target/decode_scheduler-codec-closeout/codec-coverage-matrix.json",
        "test_or_smoke": "cargo test --workspace --jobs 1",
    },
    {
        "id": "embedded_file_stream_decoding",
        "module": "crates/engine/src/attachments.rs",
        "entry_points": ["extract_attachment_with_limits"],
        "decode_behavior": "attachment extraction uses scheduler admission and central stream caps",
        "status": "scheduler_covered",
        "artifact": "target/decode_scheduler-codec-closeout/hostile-corpus-run.json",
        "test_or_smoke": "embedded_file_bomb fixture",
    },
    {
        "id": "attachment_scan_decoding",
        "module": "crates/engine/src/attachments.rs",
        "entry_points": ["list_attachments"],
        "decode_behavior": "listing scans object dictionaries and names only; extraction is separately scheduled",
        "status": "metadata_only",
        "artifact": "target/decode_scheduler-codec-closeout/decode-callsite-inventory.json",
        "metadata_only_reason": "does not request embedded file stream bytes",
    },
    {
        "id": "sanitizer_active_content_scan_decoding",
        "module": "crates/engine/src/security.rs",
        "entry_points": ["scan_risky_content", "sanitize_pdf"],
        "decode_behavior": "active-content scan inspects dictionaries/actions without full stream decode",
        "status": "metadata_only",
        "artifact": "target/decode_scheduler-codec-closeout/decode-callsite-inventory.json",
        "metadata_only_reason": "active content evidence is stored in object dictionaries and names",
    },
    {
        "id": "redaction_verification_decoding",
        "module": "crates/engine/src/editing.rs",
        "entry_points": ["redaction verification report"],
        "decode_behavior": "verification uses scheduled text extraction path for page content",
        "status": "scheduler_covered",
        "artifact": "target/decode_scheduler-codec-closeout/codec-coverage-matrix.json",
        "test_or_smoke": "cargo test --workspace --jobs 1",
    },
    {
        "id": "conversion_image_decoding",
        "module": "crates/engine/src/engine.rs; crates/engine/src/office.rs",
        "entry_points": ["extract_image_bytes", "pdf_to_docx", "pdf_to_pptx"],
        "decode_behavior": "conversion image preparation uses ContentEngine image decode path",
        "status": "scheduler_covered",
        "artifact": "target/decode_scheduler-codec-closeout/codec-coverage-matrix.json",
        "test_or_smoke": "cargo test --workspace --jobs 1",
    },
    {
        "id": "office_export_image_preparation",
        "module": "crates/engine/src/office.rs",
        "entry_points": ["pdf_to_docx", "pdf_to_pptx"],
        "decode_behavior": "office export image preparation inherits scheduled ContentEngine image/text helpers",
        "status": "scheduler_covered",
        "artifact": "target/decode_scheduler-codec-closeout/codec-coverage-matrix.json",
        "test_or_smoke": "cargo test --workspace --jobs 1",
    },
    {
        "id": "ocr_prep_image_generation",
        "module": "crates/engine/src/parse.rs",
        "entry_points": ["ocr_page_blocks"],
        "decode_behavior": "OCR preparation renders a page image; renderer decode scheduler from Codec Boundary applies",
        "status": "already_covered",
        "artifact": "target/codec_boundary-codec-boundary-scheduler/renderer-scheduler-report.json",
        "test_or_smoke": "Codec Boundary renderer scheduler tests",
        "unsupported_note": "no OCR engine is claimed unless optional backend is compiled/configured",
    },
    {
        "id": "security_report_stream_inspection",
        "module": "crates/engine/src/security.rs",
        "entry_points": ["security_report"],
        "decode_behavior": "security report inspects encryption, signatures, actions, and dictionaries",
        "status": "metadata_only",
        "artifact": "target/decode_scheduler-codec-closeout/decode-callsite-inventory.json",
        "metadata_only_reason": "does not need full filtered stream bytes",
    },
    {
        "id": "standards_validation_stream_inspection",
        "module": "crates/engine/src/standards.rs",
        "entry_points": ["validate_standards_profile"],
        "decode_behavior": "standards profile validation is metadata/object-rule based in this phase",
        "status": "metadata_only",
        "artifact": "target/decode_scheduler-codec-closeout/decode-callsite-inventory.json",
        "metadata_only_reason": "does not trigger hostile stream decode",
    },
    {
        "id": "parser_report_generation",
        "module": "crates/engine/src/parser_report.rs",
        "entry_points": ["parser_report_bytes_with_options(include_decode=true)"],
        "decode_behavior": "report generation schedules every decoded stream probe and preserves object-id order",
        "status": "scheduler_covered",
        "artifact": "target/decode_scheduler-codec-closeout/hostile-corpus-run.json",
        "test_or_smoke": "hostile corpus runner",
    },
    {
        "id": "native_codec_backend_policy",
        "module": "crates/engine/src/codec_isolation.rs",
        "entry_points": ["select_codec_backend", "validate_codec_registry_policy"],
        "decode_behavior": "unsafe native backends remain denied by central policy",
        "status": "unsupported_reported",
        "artifact": "target/codec_boundary-codec-boundary-scheduler/native-codec-boundary-report.json",
        "owner_action": "Only revisit if a future audited native backend is added to the central registry.",
    },
]


def read_json(path: Path) -> dict[str, Any] | None:
    if not path.exists():
        return None
    return json.loads(path.read_text(encoding="utf-8"))


def write_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def build_inventory() -> dict[str, Any]:
    return {
        "schema_version": 1,
        "feature_area": "combined_decode_scheduler",
        "callsite_count": len(CALLSITES),
        "callsites": CALLSITES,
    }


def build_matrix(inventory: dict[str, Any]) -> dict[str, Any]:
    counts = Counter(item["status"] for item in inventory["callsites"])
    return {
        "schema_version": 1,
        "feature_area": "combined_decode_scheduler",
        "status_counts": dict(sorted(counts.items())),
        "coverage_percent": round(
            100.0
            * sum(
                counts.get(status, 0)
                for status in [
                    "scheduler_covered",
                    "metadata_only",
                    "already_covered",
                    "worker_isolated",
                    "in_process_rust_only",
                    "unsupported_reported",
                    "fail_closed",
                ]
            )
            / max(1, inventory["callsite_count"]),
            2,
        ),
        "rows": inventory["callsites"],
        "partial_blocked_missing": [
            item
            for item in inventory["callsites"]
            if item["status"] in {"partial", "blocked", "missing"}
        ],
    }


def build_performance(hostile: dict[str, Any] | None, fuzz: dict[str, Any] | None) -> dict[str, Any]:
    hostile_results = hostile.get("results", []) if hostile else []
    elapsed = [result.get("elapsed_ms", 0) for result in hostile_results]
    peak_reserved = [
        result.get("metrics", {}).get("scheduler_peak_reserved_bytes", 0)
        for result in hostile_results
    ]
    return {
        "schema_version": 1,
        "feature_area": "combined_decode_scheduler",
        "measurement_scope": "bounded local smoke over generated hostile corpus plus fuzz compile smoke",
        "hostile_corpus": {
            "fixture_count": hostile.get("fixture_count", 0) if hostile else 0,
            "pass_rate": hostile.get("pass_rate", 0.0) if hostile else 0.0,
            "max_elapsed_ms": max(elapsed) if elapsed else 0,
            "max_scheduler_peak_reserved_bytes": max(peak_reserved) if peak_reserved else 0,
        },
        "fuzz_compile": {
            "status": fuzz.get("compile_check", {}).get("status", "unavailable") if fuzz else "unavailable",
            "cargo_fuzz_available": fuzz.get("cargo_fuzz_available", False) if fuzz else False,
            "cargo_fuzz_reason": fuzz.get("cargo_fuzz_reason", "fuzz smoke report not generated") if fuzz else "fuzz smoke report not generated",
        },
        "throughput_benchmark": {
            "status": "smoke_measured",
            "basis": "hostile corpus per-fixture elapsed_ms",
        },
        "worker_overhead_benchmark": {
            "status": "not_remeasured_in_decode_scheduler",
            "basis": "Release Packaging release gate remains authoritative for subprocess worker overhead",
        },
    }


def build_verdict(
    matrix: dict[str, Any],
    hostile: dict[str, Any] | None,
    fuzz: dict[str, Any] | None,
    performance: dict[str, Any],
) -> dict[str, Any]:
    no_open_rows = not matrix["partial_blocked_missing"]
    hostile_ok = bool(hostile and hostile.get("failed", 1) == 0)
    fuzz_compile_ok = bool(fuzz and fuzz.get("compile_check", {}).get("status") == "pass")
    go = no_open_rows and hostile_ok and fuzz_compile_ok
    return {
        "schema_version": 1,
        "feature_area": "combined_decode_scheduler",
        "status": "go_for_native_renderer" if go else "partial",
        "release_grade_verdict": "ready_for_renderer_parity_phase_with_long_fuzz_release_debt" if go else "not_ready",
        "scheduler_coverage_percent": matrix["coverage_percent"],
        "unscheduled_decode_call_sites": matrix["partial_blocked_missing"],
        "hostile_corpus_pass_rate": hostile.get("pass_rate", 0.0) if hostile else 0.0,
        "fail_closed_evidence": {
            "hostile_corpus_run": str(HOSTILE_RUN.as_posix()),
            "structured_scheduler_denials": sum(
                result.get("metrics", {}).get("scheduler_budget_denials", 0)
                for result in (hostile.get("results", []) if hostile else [])
            ),
        },
        "worker_timeout_output_cap_evidence": {
            "worker_policy": "Release Packaging codec isolation worker timeout/output caps retained",
            "codec_boundary_native_boundary": "target/codec_boundary-codec-boundary-scheduler/native-codec-boundary-report.json",
        },
        "memory_performance_metrics": str(PERFORMANCE.as_posix()),
        "known_limits": [
            "Decode Scheduler smoke proves harnesses and bounded hostile corpus behavior; multi-day libFuzzer campaigns remain release hardening work.",
            "OCR preparation is scheduler-ready through renderer page-image generation, but OCR is not claimed without the optional backend.",
            "RLBox/WASM sandboxing remains hard-blocked; OS subprocess isolation is the practical native codec boundary.",
        ],
        "next_prompt_readiness": "roadmap closure 06 can begin" if go else "roadmap closure 06 should wait",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.parse_args()

    inv = build_inventory()
    matrix = build_matrix(inv)
    hostile = read_json(HOSTILE_RUN)
    fuzz = read_json(FUZZ_SMOKE)
    perf = build_performance(hostile, fuzz)
    verdict = build_verdict(matrix, hostile, fuzz, perf)

    write_json(INVENTORY, inv)
    write_json(MATRIX, matrix)
    write_json(PERFORMANCE, perf)
    write_json(VERDICT, verdict)
    print(
        json.dumps(
            {
                "inventory": str(INVENTORY),
                "matrix": str(MATRIX),
                "performance": str(PERFORMANCE),
                "verdict": str(VERDICT),
                "status": verdict["status"],
            }
        )
    )
    return 0 if verdict["status"] == "go_for_native_renderer" else 1


if __name__ == "__main__":
    sys.exit(main())
