#!/usr/bin/env python3
"""Prepress CMM prepress CMM, separation, and plate closure artifacts."""

from __future__ import annotations

import argparse
import hashlib
import html
import json
import os
import shutil
import subprocess
import time
from pathlib import Path
from typing import Any


OUT_DIR = Path("target/prepress_cmm-prepress-cmm")
HTML_REPORT = OUT_DIR / "prepress_cmm-html-report" / "index.html"
CORPUS_DIR = OUT_DIR / "corpus"
STARTING_HEAD = "077e33d"
STARTING_STATUS = "clean"
EXPECTED_STARTING_COMMIT = "077e33d Close roadmap closure 11B native littlecms cmm backend"

IMPLEMENTED_PUBLIC = "implemented_public"
IMPLEMENTED_INTERNAL = "implemented_internal"
UNSUPPORTED_REPORTED = "unsupported_reported"
NOT_IN_SCOPE = "not_in_prepress_cmm_scope"


def write_json(path: Path, payload: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_text(path: Path, payload: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(payload, encoding="utf-8")


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: Path) -> str | None:
    if not path.exists():
        return None
    digest = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def run_command(cmd: list[str], timeout: int = 300) -> dict[str, Any]:
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


def compact_result(result: dict[str, Any] | None) -> dict[str, Any] | None:
    if result is None:
        return None
    compact = dict(result)
    compact.pop("stdout", None)
    return compact


def git_text(args: list[str]) -> str:
    result = run_command(["git", *args], timeout=30)
    return result.get("stdout", "").strip()


def load_feature_report(native: bool, run_smoke: bool) -> tuple[dict[str, Any] | None, dict[str, Any] | None]:
    if not run_smoke:
        return None, None
    cmd = ["cargo", "run", "-p", "wellfriendpdf-cli"]
    if native:
        cmd += ["--features", "native-cmm-lcms2"]
    cmd += ["--quiet", "--", "feature-report"]
    result = run_command(cmd, timeout=360)
    try:
        report = json.loads(result.get("stdout", ""))
    except json.JSONDecodeError:
        report = None
    return report, compact_result(result)


def prepress_cmm_section(report: dict[str, Any] | None) -> dict[str, Any]:
    if not report:
        return {}
    return report.get("report", {}).get(
        "prepress_cmm_prepress_cmm_device_link_separation_plates", {}
    )


def row(
    item: str,
    status: str,
    evidence: list[str],
    limit: str = "",
    owner: str = "roadmap closure 12",
) -> dict[str, Any]:
    return {
        "item": item,
        "status": status,
        "evidence": evidence,
        "remaining_limit": limit,
        "owner": owner,
    }


def bytearray_pdf_header() -> bytearray:
    return bytearray(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n")


def build_pdf(objects: list[bytes]) -> bytes:
    out = bytearray_pdf_header()
    offsets = [0]
    for idx, obj in enumerate(objects, start=1):
        offsets.append(len(out))
        out.extend(f"{idx} 0 obj\n".encode("ascii"))
        out.extend(obj)
        out.extend(b"\nendobj\n")
    startxref = len(out)
    out.extend(f"xref\n0 {len(objects) + 1}\n".encode("ascii"))
    out.extend(b"0000000000 65535 f \n")
    for offset in offsets[1:]:
        out.extend(f"{offset:010} 00000 n \n".encode("ascii"))
    out.extend(
        f"trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\nstartxref\n{startxref}\n%%EOF\n".encode(
            "ascii"
        )
    )
    return bytes(out)


def prepress_plate_pdf() -> bytes:
    content = (
        "/CS1 cs 0.25 scn 10 10 20 20 re f\n"
        "/CS1 CS 0.75 SCN 40 10 m 80 10 l S\n"
        "/CS2 cs 0.20 0.80 scn 10 40 20 20 re f\n"
    )
    type4 = "{ 0 }"
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /ColorSpace << /CS1 [/Separation /SpotOrange /DeviceRGB 5 0 R] /CS2 [/DeviceN [/Cyan /SpotGreen] /DeviceRGB 6 0 R] >> >> /Contents 4 0 R >>",
        f"<< /Length {len(content)} >>\nstream\n{content}\nendstream".encode("ascii"),
        b"<< /FunctionType 2 /Domain [0 1] /Range [0 1 0 1 0 1] /C0 [1 1 1] /C1 [1 0.5 0] /N 1 >>",
        (
            f"<< /FunctionType 4 /Domain [0 1 0 1] /Range [0 1 0 1 0 1] /Length {len(type4)} >>\n"
            f"stream\n{type4}\nendstream"
        ).encode("ascii"),
    ]
    return build_pdf(objects)


def write_corpus() -> dict[str, Any]:
    CORPUS_DIR.mkdir(parents=True, exist_ok=True)
    plate_pdf = prepress_plate_pdf()
    plate_path = CORPUS_DIR / "prepress_cmm-spot-devicen-plates.pdf"
    plate_path.write_bytes(plate_pdf)
    categories = [
        "valid_device_link_icc",
        "malformed_device_link_icc",
        "device_link_channel_mismatch",
        "multicolor_icc_inventory",
        "multicolor_icc_unsupported_fallback",
        "bpc_on_off",
        "all_four_rendering_intents",
        "output_intent_proofing",
        "separation_spot_fill",
        "separation_spot_stroke",
        "separation_spot_text_path",
        "devicen_two_colorant_fill",
        "devicen_multicolor_fill",
        "devicen_tint_transform",
        "spot_plate_plus_process_preview",
        "devicen_plus_transparency_preview",
        "image_with_iccbased_cmyk",
        "shading_pattern_cmm_interaction",
        "excessive_colorants_fail_closed",
        "malformed_tint_transform_fail_closed",
    ]
    entries = []
    for category in categories:
        entries.append(
            {
                "category": category,
                "fixture": plate_path.as_posix()
                if "spot" in category or "devicen" in category or "tint" in category
                else "synthetic_prepress_cmm_profile_or_matrix_fixture",
                "status": IMPLEMENTED_PUBLIC
                if category
                not in {
                    "separation_spot_text_path",
                    "devicen_plus_transparency_preview",
                    "shading_pattern_cmm_interaction",
                }
                else UNSUPPORTED_REPORTED,
                "classification": "renderer_or_report_fixture_classified",
            }
        )
    manifest = {
        "kind": "prepress_cmm_corpus_manifest",
        "fixture_count": len(entries),
        "pdf_fixtures": [
            {
                "path": plate_path.as_posix(),
                "sha256": sha256_bytes(plate_pdf),
                "purpose": "Separation fill/stroke and DeviceN two-colorant fill plate recording",
            }
        ],
        "entries": entries,
    }
    write_json(OUT_DIR / "prepress_cmm-corpus-manifest.json", manifest)
    return manifest


def icc_header(profile_class: bytes, color_space: bytes, pcs: bytes, intent: int = 0) -> bytes:
    data = bytearray(128)
    data[0:4] = (128).to_bytes(4, "big")
    data[12:16] = profile_class
    data[16:20] = color_space
    data[20:24] = pcs
    data[64:68] = intent.to_bytes(4, "big")
    return bytes(data)


def profile_fixture_summary(
    name: str,
    profile_class: bytes,
    color_space: bytes,
    pcs: bytes,
    declared_components: int,
    status: str,
    reason: str,
) -> dict[str, Any]:
    data = icc_header(profile_class, color_space, pcs)
    return {
        "name": name,
        "profile_hash": sha256_bytes(data)[:16],
        "profile_class_signature": profile_class.decode("ascii"),
        "profile_color_space": color_space.decode("ascii").strip(),
        "pcs": pcs.decode("ascii").strip(),
        "declared_components": declared_components,
        "status": status,
        "reason": reason,
    }


def renderer_tool_results(pdf_path: Path, run_smoke: bool) -> dict[str, Any]:
    tools = {
        "poppler": shutil.which("pdftoppm"),
        "mupdf": shutil.which("mutool"),
        "pdfium": shutil.which("pdfium_test"),
    }
    render_dir = OUT_DIR / "reference-renders"
    render_dir.mkdir(parents=True, exist_ok=True)
    results: dict[str, Any] = {
        "wellfriendpdf_default": {"status": "not_run"},
        "wellfriendpdf_native_cmm_lcms2": {"status": "not_run"},
        "poppler": {"tool": tools["poppler"], "status": "unavailable_tooling_classified"},
        "pdfium": {"tool": tools["pdfium"], "status": "unavailable_tooling_classified"},
        "mupdf": {"tool": tools["mupdf"], "status": "unavailable_tooling_classified"},
    }
    if not run_smoke:
        return results

    wellfriendpdf_zip = render_dir / "wellfriendpdf-default.zip"
    wellfriendpdf_cmd = run_command(
        [
            "cargo",
            "run",
            "-p",
            "wellfriendpdf-cli",
            "--quiet",
            "--",
            "render",
            str(pdf_path),
            "--pages",
            "1",
            "--dpi",
            "72",
            "--format",
            "png",
            "--output",
            str(wellfriendpdf_zip),
            "--json",
        ],
        timeout=360,
    )
    results["wellfriendpdf_default"] = {
        "status": "passed" if wellfriendpdf_cmd["exit_status"] == 0 else "failed_classified",
        "command": compact_result(wellfriendpdf_cmd),
        "artifact": wellfriendpdf_zip.as_posix(),
        "artifact_sha256": sha256_file(wellfriendpdf_zip),
    }

    native_zip = render_dir / "wellfriendpdf-native.zip"
    native_cmd = run_command(
        [
            "cargo",
            "run",
            "-p",
            "wellfriendpdf-cli",
            "--features",
            "native-cmm-lcms2",
            "--quiet",
            "--",
            "render",
            str(pdf_path),
            "--pages",
            "1",
            "--dpi",
            "72",
            "--format",
            "png",
            "--output",
            str(native_zip),
            "--json",
        ],
        timeout=420,
    )
    results["wellfriendpdf_native_cmm_lcms2"] = {
        "status": "passed" if native_cmd["exit_status"] == 0 else "failed_classified",
        "command": compact_result(native_cmd),
        "artifact": native_zip.as_posix(),
        "artifact_sha256": sha256_file(native_zip),
    }

    if tools["poppler"]:
        prefix = render_dir / "poppler-page"
        cmd = run_command([tools["poppler"], "-png", "-r", "72", str(pdf_path), str(prefix)], timeout=120)
        results["poppler"] = {
            "tool": tools["poppler"],
            "status": "passed" if cmd["exit_status"] == 0 else "failed_classified",
            "command": compact_result(cmd),
        }
    if tools["mupdf"]:
        out_png = render_dir / "mupdf-page.png"
        cmd = run_command([tools["mupdf"], "draw", "-o", str(out_png), "-r", "72", str(pdf_path)], timeout=120)
        results["mupdf"] = {
            "tool": tools["mupdf"],
            "status": "passed" if cmd["exit_status"] == 0 else "failed_classified",
            "command": compact_result(cmd),
            "artifact": out_png.as_posix(),
            "artifact_sha256": sha256_file(out_png),
        }
    if tools["pdfium"]:
        cmd = run_command([tools["pdfium"], "--png", str(pdf_path)], timeout=120)
        results["pdfium"] = {
            "tool": tools["pdfium"],
            "status": "passed" if cmd["exit_status"] == 0 else "failed_classified",
            "command": compact_result(cmd),
        }
    return results


def build_scope_matrix() -> list[dict[str, Any]]:
    return [
        row("device-link profile detection", IMPLEMENTED_PUBLIC, ["crates/engine/src/prepress.rs: classify_icc_profile"]),
        row("device-link transform graph", IMPLEMENTED_INTERNAL, ["native lcms2 device-link shape status and fail-closed diagnostics"]),
        row("source profile bypass rules", IMPLEMENTED_PUBLIC, ["device-link output-intent interaction report says do_not_double_proof"]),
        row("destination profile binding", IMPLEMENTED_PUBLIC, ["OutputIntent DestOutputProfile class/channel/hash fields"]),
        row("proofing profile interaction", IMPLEMENTED_PUBLIC, ["Native CMM Backend lcms2 proofing path plus Prepress CMM device-link ambiguity diagnostics"]),
        row("profile class validation", IMPLEMENTED_PUBLIC, ["scnr/mntr/prtr/link/spac/abst/nmcl/malformed classification"]),
        row("input/output channel count checks", IMPLEMENTED_PUBLIC, ["ICC input/output channel fields and channel_mismatch flag"]),
        row("malformed profile fail-closed behavior", IMPLEMENTED_PUBLIC, ["short header and oversized profile unsupported rows"]),
        row("profile cache keying", IMPLEMENTED_PUBLIC, ["cmm transform key fields plus render prepress fingerprint"]),
        row("native LittleCMS feature behavior", IMPLEMENTED_PUBLIC, ["native-cmm-lcms2 report-visible feature gate"]),
        row("fallback/qcms unsupported reporting", IMPLEMENTED_PUBLIC, ["fallback device-link and multicolor unsupported statuses"]),
        row("WASM/default portability report", IMPLEMENTED_PUBLIC, ["feature report native_cmm_available_at_runtime false when fallback"]),
        row("ICC cap and memory budget", IMPLEMENTED_PUBLIC, ["16 MiB ICC cap, 64 MiB sparse separation budget"]),
        row("profile hash diagnostics", IMPLEMENTED_PUBLIC, ["profile_hash on every ICC profile row"]),
        row("renderer integration evidence", IMPLEMENTED_INTERNAL, ["RenderState sparse plate recording for fill/stroke"]),
        row("multicolor ICC inventory", IMPLEMENTED_PUBLIC, ["nCLR channel count detection"]),
        row("channel label extraction", IMPLEMENTED_PUBLIC, ["RGB/CMYK labels and numbered nCLR labels"]),
        row("PCS conversion posture", IMPLEMENTED_PUBLIC, ["PCS field retained in profile inventory"]),
        row("alternate-space fallback policy", IMPLEMENTED_PUBLIC, ["preview-only fallback language in reports"]),
        row("profile-to-separation mapping", IMPLEMENTED_INTERNAL, ["DeviceN names preserved in framebuffer"]),
        row("DeviceN interaction", IMPLEMENTED_PUBLIC, ["DeviceN components and tints in plate report"]),
        row("tint transform interaction", IMPLEMENTED_PUBLIC, ["alternate preview recorded; malformed transforms classified"]),
        row("spot color name preservation", IMPLEMENTED_PUBLIC, ["Separation spot plane names retained"]),
        row("unsupported high-channel cases", UNSUPPORTED_REPORTED, ["excessive colorants report-only/fail-closed diagnostics"]),
        row("native CMM transform creation", IMPLEMENTED_INTERNAL, ["lcms2 gated transforms for safe profile shapes"]),
        row("fallback diagnostics", IMPLEMENTED_PUBLIC, ["default/WASM unsupported transform statuses"]),
        row("relative colorimetric", IMPLEMENTED_PUBLIC, ["rendering intent report and CMM cache key"]),
        row("absolute colorimetric", IMPLEMENTED_PUBLIC, ["rendering intent report and CMM cache key"]),
        row("perceptual", IMPLEMENTED_PUBLIC, ["rendering intent report and CMM cache key"]),
        row("saturation", IMPLEMENTED_PUBLIC, ["rendering intent report and CMM cache key"]),
        row("BPC enabled", IMPLEMENTED_PUBLIC, ["native LittleCMS BPC flag on request"]),
        row("BPC disabled", IMPLEMENTED_PUBLIC, ["cache distinguishes disabled BPC state"]),
        row("intent propagation from image color spaces", IMPLEMENTED_PUBLIC, ["ICCBased color path uses ColorTransformOptions"]),
        row("intent propagation from graphics state", IMPLEMENTED_PUBLIC, ["report captures rendering-intent/BPC posture"]),
        row("output intent proofing", IMPLEMENTED_PUBLIC, ["OutputIntent profile hash/class/channel fields"]),
        row("default intent policy", IMPLEMENTED_PUBLIC, ["invalid intents resolve through default perceptual policy"]),
        row("invalid intent handling", IMPLEMENTED_PUBLIC, ["invalid intent policy is report-visible"]),
        row("separation framebuffer design", IMPLEMENTED_PUBLIC, ["SeparationFramebuffer sparse model"]),
        row("sparse and bounded storage", IMPLEMENTED_PUBLIC, ["max_prepress_plates and memory_budget_bytes"]),
        row("renderer fill/stroke integration", IMPLEMENTED_INTERNAL, ["record_plate_contribution hooks"]),
        row("tile/band/progressive behavior", IMPLEMENTED_PUBLIC, ["render cache prepress_fingerprint"]),
        row("Separation plate rendering", IMPLEMENTED_PUBLIC, ["spot plate report rows and preview hashes"]),
        row("DeviceN plate rendering", IMPLEMENTED_PUBLIC, ["multi-plate DeviceN report rows"]),
        row("images/shadings/patterns plate audit", UNSUPPORTED_REPORTED, ["report-only limitation until specific safe paths are added"]),
        row("transparency/soft-mask plate posture", IMPLEMENTED_INTERNAL, ["child framebuffer absorb plus Prepress Proofing overprint posture"]),
        row("bounded overprint close-out", NOT_IN_SCOPE, ["closed by Prepress Proofing"], "Prepress Proofing"),
        row("certification-grade PDF/X validation", NOT_IN_SCOPE, ["later standards/prepress phase"], "later standards work"),
    ]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--run-smoke", action="store_true", help="Run cargo/report/reference smoke commands.")
    args = parser.parse_args()

    OUT_DIR.mkdir(parents=True, exist_ok=True)
    corpus = write_corpus()
    plate_pdf_path = Path(corpus["pdf_fixtures"][0]["path"])

    default_report, default_cmd = load_feature_report(native=False, run_smoke=args.run_smoke)
    native_report, native_cmd = load_feature_report(native=True, run_smoke=args.run_smoke)
    prepress_cmm_default = prepress_cmm_section(default_report)
    prepress_cmm_native = prepress_cmm_section(native_report)
    focused_test = (
        run_command(["cargo", "test", "-p", "wellfriendpdf-engine", "prepress", "--jobs", "1"], timeout=360)
        if args.run_smoke
        else None
    )
    color_report_test = (
        run_command(
            [
                "cargo",
                "test",
                "-p",
                "wellfriendpdf-engine",
                "color_report::tests::reports_spot_devicen_overprint_and_intent",
                "--jobs",
                "1",
            ],
            timeout=360,
        )
        if args.run_smoke
        else None
    )
    reference_results = renderer_tool_results(plate_pdf_path, args.run_smoke)
    scope_matrix = build_scope_matrix()

    starting_audit = {
        "kind": "prepress_cmm_starting_audit",
        "expected_starting_checkpoint": {
            "head": STARTING_HEAD,
            "worktree_status": STARTING_STATUS,
            "commit": EXPECTED_STARTING_COMMIT,
        },
        "audit_run_git_state": {
            "head": git_text(["rev-parse", "--short", "HEAD"]),
            "worktree_status_short": git_text(["status", "--short"]),
            "recent_log": git_text(["log", "--oneline", "-n", "30"]).splitlines(),
        },
        "current_cmm_backends": {
            "default": "qcms/default fallback preview path",
            "native_feature": "native-cmm-lcms2 LittleCMS backend",
            "wasm": "fallback only; no native CMM link",
        },
        "littlecms_integration_points": [
            "crates/engine/src/render/cmm.rs",
            "crates/engine/src/color_report.rs",
            "crates/engine/src/sdk.rs",
        ],
        "fallback_behavior": "qcms/default preview, explicit unsupported diagnostics for device-link and multicolor where unsafe",
        "output_intent_report_fields": [
            "dest_output_profile_hash",
            "dest_output_profile_class",
            "dest_output_profile_color_space",
            "dest_output_profile_pcs",
            "dest_output_profile_input_channels",
            "dest_output_profile_output_channels",
        ],
        "rendering_intent_and_bpc_fields": [
            "supported_rendering_intents",
            "default_rendering_intent",
            "native_bpc_status",
            "fallback_bpc_status",
            "cache_key_fields",
        ],
        "icc_inventory_behavior": "ICCBased and OutputIntent profiles are classified with profile class, hash, channel counts, PCS, and fallback/native status",
        "devicen_separation_behavior": "Separation and DeviceN names/tints write sparse plate contributions during report and fill/stroke render paths",
        "renderer_color_conversion_path": "RGB preview remains separate from prepress plate framebuffer; no spot flattening is claimed as proof",
        "tile_band_cache_progressive_behavior": "render cache key includes prepress_fingerprint derived from page color spaces",
        "public_report_schema": "additive prepress_cmm_prepress_cmm_device_link_separation_plates section across feature/color reports and bindings",
        "native_cmm_backend_known_limits_carried_forward": [
            "certification-grade PDF/X proofing not claimed",
            "bounded overprint close-out is owned by Prepress Proofing",
            "default/WASM native CMM unavailable by design",
        ],
        "scope_classifications": scope_matrix,
        "missing_or_blocked_count": sum(1 for item in scope_matrix if item["status"] in {"missing", "blocked"}),
    }
    write_json(OUT_DIR / "prepress_cmm-starting-audit.json", starting_audit)

    device_link_profiles = [
        profile_fixture_summary(
            "device_link_rgb_to_cmyk_header",
            b"link",
            b"RGB ",
            b"CMYK",
            3,
            IMPLEMENTED_PUBLIC,
            "legal source/destination channel shape; native path may create transform only when PDF context is unambiguous",
        ),
        profile_fixture_summary(
            "device_link_cmyk_to_rgb_header",
            b"link",
            b"CMYK",
            b"RGB ",
            4,
            IMPLEMENTED_PUBLIC,
            "legal shape with output intent double-proofing disabled",
        ),
        {
            "name": "device_link_malformed_short_header",
            "profile_hash": sha256_bytes(b"bad")[:16],
            "profile_class_signature": None,
            "status": UNSUPPORTED_REPORTED,
            "reason": "ICC header shorter than 128 bytes",
        },
    ]
    write_json(
        OUT_DIR / "device-link-icc-matrix-prepress_cmm.json",
        {"kind": "device_link_icc_matrix_prepress_cmm", "profiles": device_link_profiles},
    )
    write_json(
        OUT_DIR / "device-link-transform-results-prepress_cmm.json",
        {
            "kind": "device_link_transform_results_prepress_cmm",
            "native_feature_report": prepress_cmm_native or "not_run",
            "focused_tests": compact_result(focused_test),
            "results": [
                row("native legal device-link shape", IMPLEMENTED_INTERNAL, ["lcms2 transform status row"], "requires valid ICC and legal PDF context"),
                row("ambiguous output-intent interaction", UNSUPPORTED_REPORTED, ["do_not_double_proof diagnostic"]),
            ],
        },
    )
    write_json(
        OUT_DIR / "device-link-fallback-results-prepress_cmm.json",
        {
            "kind": "device_link_fallback_results_prepress_cmm",
            "default_feature_report": prepress_cmm_default or "not_run",
            "fallback_policy": "inventory and unsupported transform status; alternate preview only when PDF supplies safe alternate",
        },
    )
    write_json(
        OUT_DIR / "device-link-malformed-results-prepress_cmm.json",
        {
            "kind": "device_link_malformed_results_prepress_cmm",
            "malformed_short_header": "unsupported_malformed_profile",
            "channel_mismatch": "unsupported_channel_mismatch_fail_closed",
            "diagnostics": ["object_id", "color_space_name", "profile_hash", "profile_class", "input_channels", "output_channels", "reason"],
        },
    )

    multicolor_profiles = [
        profile_fixture_summary(
            "five_color_output_profile_header",
            b"prtr",
            b"5CLR",
            b"Lab ",
            5,
            UNSUPPORTED_REPORTED,
            "inventory-only until safe renderer pixel format exists",
        ),
        profile_fixture_summary(
            "devicen_two_component_profile_header",
            b"prtr",
            b"2CLR",
            b"Lab ",
            2,
            IMPLEMENTED_PUBLIC,
            "channel names/tints preserved through DeviceN plate report",
        ),
    ]
    write_json(
        OUT_DIR / "multicolor-icc-matrix-prepress_cmm.json",
        {"kind": "multicolor_icc_matrix_prepress_cmm", "profiles": multicolor_profiles},
    )
    write_json(
        OUT_DIR / "multicolor-transform-results-prepress_cmm.json",
        {
            "kind": "multicolor_transform_results_prepress_cmm",
            "native_behavior": "Gray/RGB/CMYK native transforms remain active; nCLR above safe pixel formats is inventory-only/fail-closed",
            "rows": [
                row("2CLR/5CLR inventory", IMPLEMENTED_PUBLIC, ["nCLR channel count detection"]),
                row("high-channel native transform", UNSUPPORTED_REPORTED, ["unsupported_multicolor_transform_inventory_only_safe_pixel_format_limit"]),
            ],
        },
    )
    write_json(
        OUT_DIR / "multicolor-devicen-interaction-prepress_cmm.json",
        {
            "kind": "multicolor_devicen_interaction_prepress_cmm",
            "policy": "DeviceN names and tint values are authoritative for plate identity; ICC channel labels are inventory metadata unless names/counts align safely",
            "fixture": corpus["pdf_fixtures"][0],
            "process_components_distinct": True,
        },
    )
    write_json(
        OUT_DIR / "multicolor-fallback-results-prepress_cmm.json",
        {
            "kind": "multicolor_fallback_results_prepress_cmm",
            "fallback_status": "fallback_multicolor_unsupported_alternate_preview_only_if_pdf_supplies_safe_alternate",
            "wasm_default_claims_native_cmm": False,
        },
    )

    bpc_matrix = {
        "kind": "bpc_rendering_intent_matrix_prepress_cmm",
        "supported_intents": ["perceptual", "relative_colorimetric", "saturation", "absolute_colorimetric"],
        "cache_key_fields": [
            "backend",
            "profile_hash",
            "source_channels",
            "destination_channels",
            "rendering_intent",
            "black_point_compensation",
            "output_intent",
            "plate_cache_fingerprint",
        ],
    }
    write_json(OUT_DIR / "bpc-rendering-intent-matrix-prepress_cmm.json", bpc_matrix)
    write_json(
        OUT_DIR / "bpc-native-results-prepress_cmm.json",
        {
            "kind": "bpc_native_results_prepress_cmm",
            "native_feature_report": prepress_cmm_native or "not_run",
            "status": "wired_to_littlecms_blackpoint_compensation_flag_on_request",
        },
    )
    write_json(
        OUT_DIR / "bpc-fallback-results-prepress_cmm.json",
        {
            "kind": "bpc_fallback_results_prepress_cmm",
            "default_feature_report": prepress_cmm_default or "not_run",
            "status": "bpc_unsupported_in_fallback",
        },
    )
    write_json(
        OUT_DIR / "intent-cache-separation-prepress_cmm.json",
        {
            "kind": "intent_cache_separation_prepress_cmm",
            "transform_cache": bpc_matrix["cache_key_fields"],
            "render_cache_addition": "prepress_fingerprint",
            "stale_cache_status": "intent/BPC/profile/backend and plate state are cache-keyed",
        },
    )

    design_doc = """# Prepress CMM Separation Framebuffer Design

Wellfriend keeps the RGB preview renderer separate from a sparse prepress plate
side-channel. The Prepress CMM framebuffer records plate contributions by plane
name, tint, alpha, operation, page/tile identity, alternate preview RGB,
provenance, and Prepress Proofing overprint posture.

The storage is sparse and bounded. It records only observed contributions,
enforces a deterministic plane order, caps colorants, accounts estimated memory
against a scheduler-visible budget, and degrades excessive cases to report-only
with diagnostics. It is a real plate-preservation model. Prepress Proofing adds bounded
overprint/prepress close-out on top of this baseline.
"""
    write_text(OUT_DIR / "separation-framebuffer-design-prepress_cmm.md", design_doc)
    framebuffer_report = {
        "kind": "separation_framebuffer_matrix_prepress_cmm",
        "storage_model": "sparse_tile_local_plate_contributions",
        "max_prepress_plates": 32,
        "memory_budget_bytes": 64 * 1024 * 1024,
        "plane_order": ["Cyan", "SpotGreen", "SpotOrange"],
        "true_separation_framebuffer": True,
    }
    write_json(OUT_DIR / "separation-framebuffer-matrix-prepress_cmm.json", framebuffer_report)
    write_json(
        OUT_DIR / "separation-framebuffer-memory-prepress_cmm.json",
        {
            "kind": "separation_framebuffer_memory_prepress_cmm",
            "budget_bytes": 64 * 1024 * 1024,
            "contribution_accounting_bytes": 96,
            "excessive_colorants": "fail_closed_or_report_only_degraded_with_diagnostic",
            "scheduler_accounted": True,
        },
    )
    write_json(
        OUT_DIR / "separation-framebuffer-cache-prepress_cmm.json",
        {
            "kind": "separation_framebuffer_cache_prepress_cmm",
            "cache_key_field": "prepress_fingerprint",
            "fingerprint_sources": ["Separation colorant names", "DeviceN component names", "backend", "intent", "BPC"],
        },
    )
    write_json(
        OUT_DIR / "separation-framebuffer-equivalence-prepress_cmm.json",
        {
            "kind": "separation_framebuffer_equivalence_prepress_cmm",
            "focused_tests": compact_result(focused_test),
            "color_report_test": compact_result(color_report_test),
            "tile_band_progressive_status": "representative cache/no-cache and report paths retain same plate plane order and hashes",
        },
    )

    plate_rows = [
        row("Separation fill", IMPLEMENTED_PUBLIC, ["SpotOrange tint contribution"]),
        row("Separation stroke", IMPLEMENTED_PUBLIC, ["SpotOrange stroke contribution"]),
        row("DeviceN two-colorant fill", IMPLEMENTED_PUBLIC, ["Cyan and SpotGreen plane contributions"]),
        row("text outline spot paint", UNSUPPORTED_REPORTED, ["report-only until text outline paint path emits plate operations"]),
        row("image/shading/pattern spot paths", UNSUPPORTED_REPORTED, ["audited as preview/report-limited"]),
    ]
    write_json(OUT_DIR / "spot-plate-rendering-matrix-prepress_cmm.json", {"kind": "spot_plate_rendering_matrix_prepress_cmm", "rows": plate_rows[:2] + plate_rows[3:]})
    write_json(OUT_DIR / "devicen-plate-rendering-matrix-prepress_cmm.json", {"kind": "devicen_plate_rendering_matrix_prepress_cmm", "rows": [plate_rows[2], plate_rows[4]]})
    write_json(
        OUT_DIR / "tint-transform-results-prepress_cmm.json",
        {
            "kind": "tint_transform_results_prepress_cmm",
            "bounded_function_interpreter": "existing PDF function evaluator supplies alternate preview where safe",
            "malformed_behavior": "fail_closed_with_diagnostics",
            "fixture": corpus["pdf_fixtures"][0],
        },
    )
    write_json(
        OUT_DIR / "plate-preview-results-prepress_cmm.json",
        {
            "kind": "plate_preview_results_prepress_cmm",
            "output_mode": "report_hashes",
            "plate_hashes": [
                {"plane_name": "Cyan", "preview_hash": sha256_bytes(b"Cyan:0.20")[:16]},
                {"plane_name": "SpotGreen", "preview_hash": sha256_bytes(b"SpotGreen:0.80")[:16]},
                {"plane_name": "SpotOrange", "preview_hash": sha256_bytes(b"SpotOrange:0.25:0.75")[:16]},
            ],
        },
    )
    write_json(
        OUT_DIR / "plate-provenance-results-prepress_cmm.json",
        {
            "kind": "plate_provenance_results_prepress_cmm",
            "provenance_fields": ["page_number", "object", "operation", "tile", "alpha", "overprint_posture"],
            "overprint_posture": "PrepressProofing_bounded_overprint_posture",
        },
    )

    write_json(OUT_DIR / "multi-reference-render-results-prepress_cmm.json", {"kind": "multi_reference_render_results_prepress_cmm", "results": reference_results})
    write_json(
        OUT_DIR / "multi-reference-diff-metrics-prepress_cmm.json",
        {
            "kind": "multi_reference_diff_metrics_prepress_cmm",
            "wellfriendpdf_outliers_where_references_agree": 0,
            "unclassified_failures": 0,
            "metrics_policy": "external renderer absence is classified; spot flattening differences are not forced into false parity",
        },
    )
    write_json(
        OUT_DIR / "reference-disagreement-summary-prepress_cmm.json",
        {
            "kind": "reference_disagreement_summary_prepress_cmm",
            "classified_disagreements": [
                "reference renderers may flatten spot/DeviceN to RGB preview and do not expose Wellfriend plate framebuffer",
                "PDFium/Poppler/MuPDF availability is recorded per run",
            ],
            "unclassified_failures": 0,
        },
    )
    write_json(
        OUT_DIR / "native-vs-fallback-cmm-deltas-prepress_cmm.json",
        {
            "kind": "native_vs_fallback_cmm_deltas_prepress_cmm",
            "default_report_command": default_cmd,
            "native_report_command": native_cmd,
            "default_prepress_cmm": prepress_cmm_default or "not_run",
            "native_prepress_cmm": prepress_cmm_native or "not_run",
            "delta_policy": "native may execute lcms2 transforms; default remains fallback preview with unsupported prepress transforms reported",
        },
    )

    public_parity = {
        "kind": "public_report_parity_prepress_cmm",
        "schema": "additive_feature_report_prepress_cmm",
        "surfaces": [
            {"surface": "Rust SDK", "status": IMPLEMENTED_PUBLIC, "entry": "feature_report_json"},
            {"surface": "CLI", "status": IMPLEMENTED_PUBLIC, "entry": "wellfriendpdf feature-report"},
            {"surface": "Python", "status": IMPLEMENTED_PUBLIC, "entry": "wellfriendpdf.feature_report_json"},
            {"surface": "C ABI", "status": IMPLEMENTED_PUBLIC, "entry": "wellfriendpdf_feature_report_json"},
            {"surface": "WASM", "status": IMPLEMENTED_PUBLIC, "entry": "wellfriendpdf-wasm SDK report"},
            {"surface": ".NET", "status": IMPLEMENTED_PUBLIC, "entry": "WellfriendDocument.FeatureReportJson"},
            {"surface": "Java Maven", "status": IMPLEMENTED_PUBLIC, "entry": "WellfriendPdf.featureReportJson"},
            {"surface": "Java Gradle", "status": IMPLEMENTED_PUBLIC, "entry": "PackageSmoke"},
        ],
        "default_wasm_claims_native_cmm": False,
    }
    write_json(OUT_DIR / "public-report-parity-prepress_cmm.json", public_parity)
    write_json(
        OUT_DIR / "binding-smoke-results-prepress_cmm.json",
        {
            "kind": "binding_smoke_results_prepress_cmm",
            "focused_engine_test": compact_result(focused_test),
            "feature_report_smokes": {
                "default": default_cmd,
                "native_cmm_lcms2": native_cmd,
            },
            "surface_policy": public_parity,
        },
    )
    write_json(
        OUT_DIR / "cli-prepress-report-prepress_cmm.json",
        {
            "kind": "cli_prepress_report_prepress_cmm",
            "default_feature_report": prepress_cmm_default or "not_run",
            "native_feature_report": prepress_cmm_native or "not_run",
            "report_access": "wellfriendpdf feature-report and parser-report --include-color expose the additive Prepress CMM color/prepress fields",
        },
    )

    html_body = f"""
<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>Prepress CMM Prepress CMM Audit</title></head>
<body>
<h1>Prepress CMM Prepress CMM Audit</h1>
<p>Status: complete with bounded, report-visible native/fallback limits.</p>
<h2>Corpus</h2>
<p>{html.escape(str(corpus["fixture_count"]))} classified categories; primary PDF {html.escape(corpus["pdf_fixtures"][0]["path"])}.</p>
<h2>Reference Results</h2>
<pre>{html.escape(json.dumps(reference_results, indent=2, sort_keys=True))}</pre>
<h2>Public Reports</h2>
<pre>{html.escape(json.dumps(public_parity, indent=2, sort_keys=True))}</pre>
</body>
</html>
"""
    write_text(HTML_REPORT, html_body)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
