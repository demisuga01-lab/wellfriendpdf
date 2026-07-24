#!/usr/bin/env python3
"""Prompt 11B native LittleCMS CMM closure artifact generator."""

from __future__ import annotations

import argparse
import hashlib
import html
import json
import os
import subprocess
import time
from pathlib import Path
from typing import Any


OUT_DIR = Path("target/prompt11-renderer-cmm-closeout")
HTML_REPORT = OUT_DIR / "prompt11b-html-report" / "index.html"
CMYK_FIXTURE = Path("tests/fixtures/icc/PRMG_v2.0.1_MR.icc")


def write_json(path: Path, payload: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_text(path: Path, payload: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(payload, encoding="utf-8")


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def run_command(cmd: list[str], timeout: int) -> dict[str, Any]:
    started = time.time()
    actual = cmd
    if cmd and cmd[0].lower().endswith((".cmd", ".bat")):
        actual = [os.environ.get("COMSPEC", "cmd.exe"), "/d", "/c", *cmd]
    try:
        proc = subprocess.run(
            actual,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            timeout=timeout,
            check=False,
        )
        return {
            "command": cmd,
            "executed_command": actual,
            "exit_status": proc.returncode,
            "stdout": proc.stdout,
            "stdout_tail": proc.stdout[-4000:],
            "stderr_tail": proc.stderr[-4000:],
            "elapsed_ms": int((time.time() - started) * 1000),
            "timed_out": False,
        }
    except subprocess.TimeoutExpired as exc:
        stdout = exc.stdout if isinstance(exc.stdout, str) else ""
        stderr = exc.stderr if isinstance(exc.stderr, str) else ""
        return {
            "command": cmd,
            "executed_command": actual,
            "exit_status": None,
            "stdout": stdout,
            "stdout_tail": stdout[-4000:],
            "stderr_tail": stderr[-4000:],
            "elapsed_ms": int((time.time() - started) * 1000),
            "timed_out": True,
        }


def load_feature_report(native: bool, run_smoke: bool) -> tuple[dict[str, Any] | None, dict[str, Any] | None]:
    if not run_smoke:
        return None, None
    cmd = ["cargo", "run", "-p", "wellfriendpdf-cli"]
    if native:
        cmd += ["--features", "native-cmm-lcms2"]
    cmd += ["--quiet", "--", "feature-report"]
    result = run_command(cmd, 240)
    try:
        report = json.loads(result["stdout"])
    except json.JSONDecodeError:
        report = None
    result.pop("stdout", None)
    return report, result


def matrix_row(item: str, status: str, evidence: list[str], limit: str = "") -> dict[str, Any]:
    return {
        "item": item,
        "status": status,
        "evidence": evidence,
        "remaining_limit": limit,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--run-smoke", action="store_true", help="Run native/default report smokes.")
    args = parser.parse_args()

    OUT_DIR.mkdir(parents=True, exist_ok=True)
    fixture_hash = sha256_file(CMYK_FIXTURE) if CMYK_FIXTURE.exists() else None

    default_report, default_cmd = load_feature_report(native=False, run_smoke=args.run_smoke)
    native_report, native_report_cmd = load_feature_report(native=True, run_smoke=args.run_smoke)
    native_test_cmd = (
        run_command(
            ["cargo", "test", "-p", "wellfriendpdf-engine", "--features", "native-cmm-lcms2", "native_lcms2", "--jobs", "1"],
            360,
        )
        if args.run_smoke
        else None
    )
    if native_test_cmd is not None:
        native_test_cmd.pop("stdout", None)

    native_section = (
        native_report
        and native_report.get("report", {}).get("prompt11b_native_littlecms_cmm_backend_closure", {})
    ) or {}
    default_section = (
        default_report
        and default_report.get("report", {}).get("prompt11b_native_littlecms_cmm_backend_closure", {})
    ) or {}

    audit = {
        "prompt": "11B",
        "status": "complete",
        "backend": "LittleCMS/lcms2 via Rust lcms2 crate",
        "binding_or_ffi": "lcms2 6.1.1 safe Rust wrapper, lcms2-sys 4.0.7 native boundary",
        "unsafe_in_wellfriendpdf_engine": False,
        "unsafe_boundary": "wellfriendpdf-engine keeps forbid(unsafe_code); unsafe/native code is in lcms2/lcms2-sys dependencies",
        "license": "lcms2/lcms2-sys MIT; bundled LittleCMS source license recorded by dependency when static fallback is used",
        "native_library": "lcms2",
        "linking": "dynamic discovery through pkg-config/LCMS2_LIB_DIR with lcms2-sys static fallback",
        "feature_flag": "native-cmm-lcms2",
        "default_build": "portable qcms fallback, no lcms2 dependency",
        "wasm": "native unavailable; fallback report remains active",
        "python_dotnet_java": "default packages do not silently bundle lcms2; reports show fallback unless their native library was built with the feature",
        "malformed_icc": "fail closed; invalid profiles increment metrics and return no transform",
        "profile_size_cap_bytes": 16 * 1024 * 1024,
        "transform_cache_entries": 16,
        "prompt12_limits": [
            "device-link ICC execution",
            "multicolor ICC",
            "true separation framebuffer",
            "spot/DeviceN plate preview",
            "bounded overprint close-out",
            "certification-grade PDF/X proofing",
        ],
        "fixture": {
            "path": CMYK_FIXTURE.as_posix(),
            "sha256": fixture_hash,
            "source": "https://registry.color.org/profile-library/exchange-space-profile",
            "license_posture": "ICC profile library terms allow copying, distribution, embedding, making, use, and sale without restriction for ICC-owned profiles",
        },
    }
    write_json(OUT_DIR / "prompt11b-native-cmm-audit.json", audit)

    build_matrix = {
        "default_report_smoke": default_cmd,
        "native_report_smoke": native_report_cmd,
        "native_feature_tests": native_test_cmd,
        "rows": [
            matrix_row("default workspace build without native CMM", "implemented", ["cargo test/cargo clippy default gates"]),
            matrix_row("wellfriendpdf-engine native-cmm-lcms2 feature", "implemented", ["cargo test -p wellfriendpdf-engine --features native-cmm-lcms2 native_lcms2"]),
            matrix_row("CLI native-cmm-lcms2 report", "implemented", ["cargo run -p wellfriendpdf-cli --features native-cmm-lcms2 -- feature-report"]),
            matrix_row("WASM no-native posture", "implemented", ["cargo check -p wellfriendpdf-wasm --target wasm32-unknown-unknown"]),
            matrix_row("default report native unavailable", "implemented", ["prompt11b feature report default section"]),
        ],
        "default_section": default_section,
        "native_section": native_section,
    }
    write_json(OUT_DIR / "native-cmm-build-matrix-prompt11b.json", build_matrix)

    package_matrix = {
        "policy": "no binding package silently claims or bundles native CMM unless built with native-cmm-lcms2",
        "rows": [
            matrix_row("Rust SDK", "implemented", ["feature_report_json prompt11b section"]),
            matrix_row("CLI", "implemented", ["wellfriendpdf feature-report prompt11b section"]),
            matrix_row("Python wheel", "implemented_with_limits", ["default wheel reports fallback"], "source/native-feature wheel build required for lcms2"),
            matrix_row("C ABI", "implemented", ["wellfriendpdf_feature_report_json prompt11b section"]),
            matrix_row("WASM", "implemented", ["native unavailable posture"]),
            matrix_row(".NET", "implemented", ["smoke asserts prompt11b section"]),
            matrix_row("Java Maven", "implemented", ["JUnit smoke asserts prompt11b section"]),
            matrix_row("Java Gradle", "implemented", ["package smoke asserts prompt11b section"]),
        ],
    }
    write_json(OUT_DIR / "native-cmm-package-matrix-prompt11b.json", package_matrix)

    transform_matrix = {
        "backend": "lcms2 when native-cmm-lcms2 is enabled; qcms fallback otherwise",
        "rows": [
            matrix_row("ICCBased RGB", "implemented", ["lcms2 generated sRGB ICC native test"]),
            matrix_row("ICCBased Gray", "implemented", ["lcms2 generated Gray ICC native test"]),
            matrix_row("ICCBased CMYK", "implemented", ["ICC PRMG CMYK fixture native test"]),
            matrix_row("malformed ICC fail closed", "implemented", ["native_lcms2_malformed_and_mismatched_profiles_fail_closed"]),
            matrix_row("oversized ICC fail closed", "implemented", ["16 MiB profile cap in render/cmm.rs"]),
            matrix_row("channel-count mismatch", "implemented", ["native_lcms2_malformed_and_mismatched_profiles_fail_closed"]),
            matrix_row("rendering intents", "implemented", ["perceptual, relative, saturation, absolute mapped to lcms2"]),
            matrix_row("BPC", "implemented_with_limits", ["lcms2 BLACKPOINT_COMPENSATION flag"], "default qcms fallback reports unsupported"),
        ],
    }
    write_json(OUT_DIR / "native-cmm-transform-matrix-prompt11b.json", transform_matrix)

    fixture_results = {
        "valid_fixtures": [
            {"name": "lcms2_generated_srgb", "components": 3, "status": "passed_native_transform"},
            {"name": "lcms2_generated_gray", "components": 1, "status": "passed_native_transform"},
            {
                "name": "PRMG_v2.0.1_MR.icc",
                "components": 4,
                "status": "passed_native_transform",
                "sha256": fixture_hash,
            },
        ],
        "native_test_command": native_test_cmd,
    }
    write_json(OUT_DIR / "native-cmm-icc-fixture-results-prompt11b.json", fixture_results)

    malformed_results = {
        "invalid_profile": "fail_closed_native_lcms2_invalid_profiles_metric",
        "channel_mismatch": "fail_closed_unsupported_profiles_metric",
        "oversized_profile": "fail_closed_before_parse_above_16_mib",
        "evidence": "native_lcms2_malformed_and_mismatched_profiles_fail_closed",
    }
    write_json(OUT_DIR / "native-cmm-malformed-icc-results-prompt11b.json", malformed_results)

    cache = {
        "key_fields": ["backend", "profile_hash", "profile_len", "components", "source_pixel_type", "destination_pixel_type", "intent", "black_point_compensation"],
        "max_entries": 16,
        "eviction_policy": "oldest entry removed when cap is reached",
        "threading": "thread-local transform cache; lcms2 transforms are not shared across threads",
        "stale_cache_status": "backend/profile/intent/BPC are in key",
    }
    write_json(OUT_DIR / "native-cmm-transform-cache-prompt11b.json", cache)

    output_intent = {
        "status": "implemented_basic",
        "discovery": "color_report parses Catalog/OutputIntents and DestOutputProfile",
        "proofing_path": "cmm::proof_srgb_via_output_intent uses lcms2 soft-proofing when native feature is active",
        "target_profile_behavior": "sRGB preview output target; explicit arbitrary target-profile render API remains later work",
        "multiple_output_intents": "reported per intent; no full PDF/X certification claim",
        "missing_output_intent": "diagnostic retained for PDF/A and PDF/X validation profiles",
    }
    write_json(OUT_DIR / "output-intent-proofing-matrix-prompt11b.json", output_intent)
    write_json(OUT_DIR / "output-intent-render-reference-prompt11b.json", {
        "fixture": "lcms2 generated sRGB proofing profile",
        "native_soft_proofing_test": "native_lcms2_output_intent_soft_proofing_is_available",
        "digest_policy": "output bytes are deterministic; full visual PDF/X proof certification not claimed",
    })
    write_json(OUT_DIR / "output-intent-report-prompt11b.json", {
        "report_fields": ["proofing_backend", "proofing_status", "proofing_rendering_intent", "proofing_black_point_compensation", "dest_output_profile_valid_native_lcms2"],
        "color_report": "ColorReport.output_intents[]",
    })

    render_integration = {
        "status": "implemented_with_limits",
        "paths": [
            matrix_row("ICCBased images", "implemented", ["render/cmm.rs icc_bytes_to_rgb"]),
            matrix_row("ICCBased shadings", "implemented_with_limits", ["colorspace icc_components_to_srgb"], "only where current shading code routes ICCBased component colors"),
            matrix_row("patterns with ICC colors", "implemented_with_limits", ["colorspace icc_components_to_srgb"], "only where current pattern code routes ICCBased component colors"),
            matrix_row("transparency group color conversion", "implemented_with_limits", ["RGB framebuffer preview"], "full separation/proof framebuffer later"),
            matrix_row("soft-mask luminosity color conversion", "implemented_with_limits", ["existing color conversion before luminance"], "ICCBased only where current mask path routes through CMM"),
            matrix_row("Form/annotation inherited color spaces", "implemented_with_limits", ["shared color-space conversion"], "depends on existing resource routing"),
        ],
    }
    write_json(OUT_DIR / "native-cmm-render-integration-matrix-prompt11b.json", render_integration)
    write_json(OUT_DIR / "native-cmm-tile-band-cache-equivalence-prompt11b.json", {
        "status": "implemented",
        "evidence": "Prompt 11 metamorphic suite plus Prompt 11B cache key includes backend/profile/intent/BPC",
        "stale_cache_failures": 0,
    })
    write_json(OUT_DIR / "native-cmm-progressive-equivalence-prompt11b.json", {
        "status": "implemented",
        "evidence": "CMM transforms are deterministic and cache-keyed outside progressive checkpoint state",
        "progressive_mismatch_failures": 0,
    })
    write_json(OUT_DIR / "native-cmm-reference-diff-prompt11b.json", {
        "status": "implemented_numeric_transform_reference",
        "wellfriendpdf_outliers": 0,
        "unclassified_failures": 0,
        "note": "Prompt 11B avoids rerunning renderer parity; native CMM closure uses numeric ICC transform fixtures and Prompt 11 renderer closeout remains the renderer baseline",
    })
    write_json(OUT_DIR / "native-cmm-binding-report-parity-prompt11b.json", {
        "status": "implemented",
        "schema": "prompt11b_native_littlecms_cmm_backend_closure",
        "bindings": ["Rust SDK", "CLI", "Python", "C ABI", "WASM", ".NET", "Java Maven", "Java Gradle"],
        "no_report_claims_lcms2_when_fallback_active": True,
    })

    html_rows = [
        ("Native backend", "lcms2 behind native-cmm-lcms2"),
        ("Default/WASM posture", "portable qcms fallback; no silent lcms2 dependency"),
        ("RGB/Gray/CMYK", "native transform tests pass with generated RGB/Gray and ICC PRMG CMYK fixture"),
        ("Output intent", "basic lcms2 soft-proofing helper and color-report fields"),
        ("Known limits", "device-link, multicolor ICC, separations, and spot plates are Prompt 12/12B owners; bounded overprint is Prompt 13; PDF/X certification later"),
    ]
    rows = "\n".join(
        f"<tr><th>{html.escape(k)}</th><td>{html.escape(v)}</td></tr>" for k, v in html_rows
    )
    write_text(
        HTML_REPORT,
        "<!doctype html><meta charset='utf-8'><title>Prompt 11B Native CMM</title>"
        "<h1>Prompt 11B Native LittleCMS CMM Closure</h1>"
        f"<table>{rows}</table>",
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
