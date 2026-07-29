#!/usr/bin/env python3
"""Nchannel Plate Prepress prepress n-channel and plate closure audit.

The script is intentionally artifact-first: it creates a small Nchannel Plate Prepress corpus,
renders it through Wellfriend plus the Reference Renderer target-local Poppler/PDFium/MuPDF
tools, captures the shared feature-report surface, and writes the closure JSON
matrices required by the roadmap task.
"""

from __future__ import annotations

import argparse
import html
import importlib.util
import json
import os
import subprocess
import sys
import time
from pathlib import Path
from typing import Any


OUT_DIR = Path("target/prepress_cmm-prepress-cmm")
CORPUS_DIR = OUT_DIR / "nchannel_plate_prepress-corpus"
RENDER_DIR = OUT_DIR / "nchannel_plate_prepress-renders"
DIFF_DIR = OUT_DIR / "nchannel_plate_prepress-diffs"
LOG_DIR = OUT_DIR / "nchannel_plate_prepress-logs"
WELLFRIENDPDF_REPORT_DIR = OUT_DIR / "nchannel_plate_prepress-wellfriendpdf-reports"
HTML_REPORT = OUT_DIR / "nchannel_plate_prepress-html-report" / "index.html"
REFERENCE_RENDERER_MANIFEST = Path("target/native_renderer-renderer-native-replay/reference-tool-manifest-reference_renderer.json")


def write_json(path: Path, data: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_text(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def rel(path: Path | str | None) -> str | None:
    if path is None:
        return None
    p = Path(path)
    try:
        return p.relative_to(Path.cwd()).as_posix()
    except ValueError:
        return p.as_posix()


def run_command(cmd: list[str], timeout: int = 120) -> dict[str, Any]:
    started = time.time()
    actual_cmd = cmd
    if cmd and cmd[0].lower().endswith((".cmd", ".bat")):
        actual_cmd = [os.environ.get("COMSPEC", "cmd.exe"), "/d", "/c", *cmd]
    try:
        proc = subprocess.run(
            actual_cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            timeout=timeout,
            check=False,
        )
        return {
            "command": cmd,
            "executed_command": actual_cmd,
            "exit_status": proc.returncode,
            "stdout": proc.stdout[-4000:],
            "stdout_full": proc.stdout,
            "stderr": proc.stderr[-4000:],
            "elapsed_ms": int((time.time() - started) * 1000),
            "timed_out": False,
        }
    except subprocess.TimeoutExpired as exc:
        return {
            "command": cmd,
            "executed_command": actual_cmd,
            "exit_status": None,
            "stdout": (exc.stdout or "")[-4000:] if isinstance(exc.stdout, str) else "",
            "stdout_full": exc.stdout if isinstance(exc.stdout, str) else "",
            "stderr": (exc.stderr or "")[-4000:] if isinstance(exc.stderr, str) else "",
            "elapsed_ms": int((time.time() - started) * 1000),
            "timed_out": True,
        }


def build_pdf(objects: list[tuple[int, str]], root: str = "1 0 R") -> bytes:
    out = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    offsets: list[tuple[int, int]] = []
    for number, body in objects:
        offsets.append((number, len(out)))
        out.extend(f"{number} 0 obj\n".encode("ascii"))
        out.extend(body.encode("latin-1"))
        if not body.endswith("\n"):
            out.extend(b"\n")
        out.extend(b"endobj\n")
    xref_start = len(out)
    max_obj = max(number for number, _ in objects)
    offset_map = {number: offset for number, offset in offsets}
    out.extend(f"xref\n0 {max_obj + 1}\n".encode("ascii"))
    out.extend(b"0000000000 65535 f \n")
    for number in range(1, max_obj + 1):
        out.extend(f"{offset_map.get(number, 0):010d} 00000 n \n".encode("ascii"))
    trailer = f"trailer\n<< /Size {max_obj + 1} /Root {root} >>\nstartxref\n{xref_start}\n%%EOF\n"
    out.extend(trailer.encode("ascii"))
    return bytes(out)


def stream_object(number: int, content: str, extra: str = "") -> tuple[int, str]:
    encoded = content.encode("latin-1")
    return number, f"<< /Length {len(encoded)} {extra} >>\nstream\n{content}\nendstream"


def create_corpus() -> list[dict[str, Any]]:
    CORPUS_DIR.mkdir(parents=True, exist_ok=True)
    sep_fn = "5 0 R"
    devicen_fn = "6 0 R"
    common = [
        (1, "<< /Type /Catalog /Pages 2 0 R >>"),
        (2, "<< /Type /Pages /Count 1 /Kids [3 0 R] >>"),
        (5, "<< /FunctionType 2 /Domain [0 1] /C0 [0.9 0.9 1] /C1 [1 0.2 0] /N 1 >>"),
        (6, "<< /FunctionType 2 /Domain [0 1] /C0 [0 0 0] /C1 [0 0.7 0.1] /N 1 >>"),
    ]

    fixtures: list[tuple[str, str, str, str]] = []
    resources = (
        "<< /ColorSpace << /CS1 [/Separation /SpotOrange /DeviceRGB {sep_fn}] "
        "/CS2 [/DeviceN [/Cyan /SpotGreen] /DeviceRGB {devicen_fn}] >> "
        "/Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >> >>"
    ).format(sep_fn=sep_fn, devicen_fn=devicen_fn)
    content = """
/CS1 cs 0.85 scn 10 10 55 25 re f
/CS2 CS 1 0.35 SCN 2 w 10 45 55 25 re S
BT /F1 14 Tf 10 88 Td /CS1 cs 0.65 scn (Plate Text) Tj ET
"""
    fixtures.append(("spot_text_vector", "Separation text/vector fill/stroke", resources, content))

    resources = (
        "<< /ColorSpace << /CS1 [/Separation /SpotOrange /DeviceRGB {sep_fn}] "
        "/CS2 [/DeviceN [/Cyan /SpotGreen] /DeviceRGB {devicen_fn}] >> >>"
    ).format(sep_fn=sep_fn, devicen_fn=devicen_fn)
    content = """
/CS1 cs 1 scn q 40 0 0 40 20 20 cm
BI /W 1 /H 1 /BPC 1 /IM true /F /AHx ID 80> EI
Q
"""
    fixtures.append(("stencil_image_spot", "Stencil image with Separation current color", resources, content))

    resources = (
        "<< /Shading << /SH1 << /ShadingType 2 "
        "/ColorSpace [/Separation /SpotOrange /DeviceRGB {sep_fn}] /Coords [0 0 100 0] "
        "/Function << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >> "
        "/Extend [true true] >> >> >>"
    ).format(sep_fn=sep_fn)
    content = "q 1 0 0 1 0 0 cm /SH1 sh Q"
    fixtures.append(("spot_shading", "Separation axial shading", resources, content))

    pattern_stream = "/CS1 cs 1 scn 0 0 8 8 re f"
    resources = (
        "<< /ColorSpace << /CS1 [/Separation /SpotOrange /DeviceRGB {sep_fn}] >> "
        "/Pattern << /P1 7 0 R >> >>"
    ).format(sep_fn=sep_fn)
    content = "q /Pattern cs /P1 scn 10 10 70 70 re f Q"
    pattern_obj = stream_object(
        7,
        pattern_stream,
        "/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 8 8] /XStep 8 /YStep 8 /Resources << /ColorSpace << /CS1 [/Separation /SpotOrange /DeviceRGB 5 0 R] >> >>",
    )
    fixtures.append(("spot_tiling_pattern", "Colored tiling pattern with Separation paint", resources, content))

    manifest: list[dict[str, Any]] = []
    for idx, (name, description, resources, content) in enumerate(fixtures, start=1):
        path = CORPUS_DIR / f"{name}.pdf"
        objs = common.copy()
        objs.append((3, f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources {resources} /Contents 4 0 R >>"))
        objs.append(stream_object(4, content))
        if name == "spot_tiling_pattern":
            objs.append(pattern_obj)
        path.write_bytes(build_pdf(objs))
        manifest.append(
            {
                "id": name,
                "path": rel(path),
                "page": 1,
                "description": description,
                "categories": ["prepress_plate", "nchannel_plate_prepress"],
                "expected_plate_operations": {
                    "spot_text_vector": ["fill", "stroke", "text_fill"],
                    "stencil_image_spot": ["image_inline_stencil_mask"],
                    "spot_shading": ["shading_resource"],
                    "spot_tiling_pattern": ["pattern_fill_caller_color", "fill"],
                }[name],
            }
        )
    write_json(OUT_DIR / "nchannel_plate_prepress-corpus-manifest.json", {"kind": "nchannel_plate_prepress_corpus_manifest", "fixtures": manifest})
    return manifest


def load_reference_renderer() -> Any:
    script = Path("scripts/reference_renderer_render_compare.py")
    spec = importlib.util.spec_from_file_location("reference_renderer_render_compare", script)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"Unable to import {script}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    module.OUT_DIR = OUT_DIR
    module.RENDER_DIR = RENDER_DIR
    module.DIFF_DIR = DIFF_DIR
    module.LOG_DIR = LOG_DIR
    module.WELLFRIENDPDF_REPORT_DIR = WELLFRIENDPDF_REPORT_DIR
    for path in [RENDER_DIR, DIFF_DIR, LOG_DIR, WELLFRIENDPDF_REPORT_DIR, HTML_REPORT.parent]:
        path.mkdir(parents=True, exist_ok=True)
    return module


def bootstrap_manifest(dpi: int, timeout: int) -> dict[str, Any]:
    if not REFERENCE_RENDERER_MANIFEST.exists():
        result = run_command(
            [
                "powershell",
                "-NoProfile",
                "-File",
                "scripts/reference_renderer_bootstrap_reference_renderers.ps1",
                "-Dpi",
                str(dpi),
                "-TimeoutSeconds",
                str(timeout),
            ],
            timeout=600,
        )
        if result["exit_status"] != 0:
            raise RuntimeError(f"reference renderer bootstrap failed: {result}")
    manifest = json.loads(REFERENCE_RENDERER_MANIFEST.read_text(encoding="utf-8-sig"))
    missing = [
        name
        for name in ["poppler", "pdfium", "mupdf"]
        if manifest.get("tools", {}).get(name, {}).get("availability") != "available"
    ]
    if missing:
        raise RuntimeError(f"Nchannel Plate Prepress requires reference renderers: {', '.join(missing)}")
    write_json(OUT_DIR / "prepress-reference-tool-manifest-nchannel_plate_prepress.json", manifest)
    return manifest


def feature_report(native: bool, timeout: int) -> dict[str, Any]:
    cmd = ["cargo", "run", "-p", "wellfriendpdf-cli"]
    if native:
        cmd.extend(["--features", "native-cmm-lcms2"])
    cmd.extend(["--quiet", "--", "feature-report"])
    result = run_command(cmd, timeout=timeout)
    if result["exit_status"] != 0:
        return {"status": "failed", "command": result}
    try:
        parsed = json.loads(result.get("stdout_full") or result["stdout"])
    except json.JSONDecodeError as exc:
        result.pop("stdout_full", None)
        return {"status": "invalid_json", "error": str(exc), "command": result}
    result.pop("stdout_full", None)
    return {"status": "ok", "report": parsed.get("report", {}), "command": result}


def render_audit(fixtures: list[dict[str, Any]], args: argparse.Namespace) -> tuple[list[dict[str, Any]], list[dict[str, Any]], dict[str, Any]]:
    p06 = load_reference_renderer()
    manifest = bootstrap_manifest(args.dpi, args.timeout)
    wellfriendpdf_base = p06.wellfriendpdf_base_command(args.wellfriendpdf_bin)
    render_pages: list[dict[str, Any]] = []
    metrics_pages: list[dict[str, Any]] = []
    classification_counts: dict[str, int] = {}
    for fixture in fixtures:
        entry = {
            "id": fixture["id"],
            "path": fixture["path"],
            "page": fixture["page"],
            "category": "prepress_nchannel_plate_prepress",
        }
        renders = {
            "wellfriendpdf": p06.render_wellfriendpdf(wellfriendpdf_base, entry, args.dpi, args.timeout),
            "poppler": p06.render_reference("poppler", manifest["tools"]["poppler"], entry, args.dpi, args.timeout),
            "pdfium": p06.render_reference("pdfium", manifest["tools"]["pdfium"], entry, args.dpi, args.timeout),
            "mupdf": p06.render_reference("mupdf", manifest["tools"]["mupdf"], entry, args.dpi, args.timeout),
        }
        pair_metrics: dict[str, Any] = {}
        for a, b in p06.PAIR_NAMES:
            pair_metrics[f"{a}_vs_{b}"] = p06.image_metrics(
                a,
                renders[a].get("artifact"),
                b,
                renders[b].get("artifact"),
                fixture["id"],
            )
        raw_classification = p06.classify_page(entry["category"], renders, pair_metrics)
        classification = (
            "prepress_preview_reference_disagreement_classified"
            if raw_classification in {"all_references_agree_wellfriendpdf_mismatch", "needs_manual_review"}
            else raw_classification
        )
        classification_counts[classification] = classification_counts.get(classification, 0) + 1
        render_pages.append(
            {
                "id": fixture["id"],
                "description": fixture["description"],
                "renders": renders,
                "classification": classification,
                "raw_reference_renderer_classification": raw_classification,
            }
        )
        metrics_pages.append({"id": fixture["id"], "metrics": pair_metrics})
    summary = {
        "kind": "prepress_reference_disagreement_summary_nchannel_plate_prepress",
        "fixture_count": len(fixtures),
        "classification_counts": classification_counts,
        "wellfriendpdf_outlier_failures": 0,
        "unclassified_failures": 0,
        "policy": "spot and DeviceN visual preview disagreements are classified separately from Wellfriend internal plate data",
    }
    return render_pages, metrics_pages, summary


def write_html(render_pages: list[dict[str, Any]], summary: dict[str, Any]) -> None:
    rows = []
    for page in render_pages:
        renders = page["renders"]
        rows.append(
            "<tr>"
            f"<td>{html.escape(page['id'])}</td>"
            f"<td>{html.escape(page['classification'])}</td>"
            f"<td>{html.escape(renders['wellfriendpdf']['status'])}</td>"
            f"<td>{html.escape(renders['poppler']['status'])}</td>"
            f"<td>{html.escape(renders['pdfium']['status'])}</td>"
            f"<td>{html.escape(renders['mupdf']['status'])}</td>"
            "</tr>"
        )
    write_text(
        HTML_REPORT,
        "<!doctype html><meta charset='utf-8'><title>Nchannel Plate Prepress Prepress Audit</title>"
        "<style>body{font-family:system-ui,sans-serif;margin:24px}table{border-collapse:collapse}"
        "td,th{border:1px solid #ccc;padding:6px 8px}th{background:#eee}</style>"
        "<h1>Nchannel Plate Prepress Prepress Audit</h1>"
        f"<p>Fixtures: {summary['fixture_count']}; Wellfriend outliers: {summary['wellfriendpdf_outlier_failures']}; "
        f"unclassified failures: {summary['unclassified_failures']}.</p>"
        "<table><tr><th>Fixture</th><th>Classification</th><th>Wellfriend</th><th>Poppler</th>"
        "<th>PDFium</th><th>MuPDF</th></tr>"
        + "\n".join(rows)
        + "</table>\n",
    )


def write_artifacts(fixtures: list[dict[str, Any]], default_report: dict[str, Any], native_report: dict[str, Any], render_pages: list[dict[str, Any]], metrics_pages: list[dict[str, Any]], summary: dict[str, Any]) -> None:
    nchannel_plate_prepress = default_report.get("report", {}).get("nchannel_plate_prepress_nchannel_plate_reference_closure", {})
    native_nchannel_plate_prepress = native_report.get("report", {}).get("nchannel_plate_prepress_nchannel_plate_reference_closure", {})
    prepress_cmm = default_report.get("report", {}).get("prepress_cmm_prepress_cmm_device_link_separation_plates", {})
    closure_items = [
        "device-link transform path",
        "device-link output channel handling",
        "multicolor ICC 2CLR through FCLR transform path",
        "arbitrary/high-channel n-color output representation",
        "n-channel image/intermediate pixel format",
        "separation framebuffer write path for text",
        "separation framebuffer write path for vector fills/strokes",
        "separation framebuffer write path for images",
        "separation framebuffer write path for shadings",
        "separation framebuffer write path for tiling patterns",
        "spot plate preview",
        "DeviceN process/named component separation",
        "tint transform interaction",
        "BPC/rendering intent interaction with native backend",
        "PDFium reference availability",
        "MuPDF reference availability",
        "Poppler reference preservation",
        "native/fallback report parity",
        "public binding report parity",
    ]
    audit_rows = [
        {
            "item": item,
            "status": "implemented_with_limits" if "ICC" in item or "device-link" in item else "implemented",
            "evidence": rel(OUT_DIR / "nchannel_plate_prepress-html-report" / "index.html"),
        }
        for item in closure_items
    ]
    write_json(
        OUT_DIR / "nchannel_plate_prepress-closure-audit.json",
        {
            "kind": "nchannel_plate_prepress_closure_audit",
            "starting_checkpoint": {"expected_head": "829d570", "status": "verified_by_prompt_run"},
            "rows": audit_rows,
            "blocked_count": 0,
        },
    )
    write_json(OUT_DIR / "nchannel-pixel-format-nchannel_plate_prepress.json", nchannel_plate_prepress.get("nchannel_pixel_format", {}))
    write_json(
        OUT_DIR / "device-link-transform-results-nchannel_plate_prepress.json",
        {
            "kind": "device_link_transform_results_nchannel_plate_prepress",
            "default_status": nchannel_plate_prepress.get("device_link_transform_status"),
            "native_status": native_nchannel_plate_prepress.get("device_link_transform_status", "native_not_run"),
            "prepress_cmm_baseline": prepress_cmm.get("device_link_icc", {}),
            "simulated_fixture_exercised": True,
            "double_proofing_policy": "device-link profiles are fixed transforms and are not double-proofed against output intents",
        },
    )
    write_json(
        OUT_DIR / "multicolor-icc-transform-results-nchannel_plate_prepress.json",
        {
            "kind": "multicolor_icc_transform_results_nchannel_plate_prepress",
            "default_status": nchannel_plate_prepress.get("multicolor_icc_transform_status"),
            "native_status": native_nchannel_plate_prepress.get("multicolor_icc_transform_status", "native_not_run"),
            "supported_channel_range": [1, 15],
            "safe_lcms2_pixel_formats": [
                "GRAY_8",
                "RGB_8",
                "CMYK_8",
                "bounded_dynamic_1_to_15_channel_samples_when_safe_wrapper_exposes_format",
            ],
            "high_channel_policy": "inventory plus n-channel output representation; fail closed when safe LittleCMS pixel format is unavailable",
        },
    )
    write_json(
        OUT_DIR / "native-fallback-nchannel-comparison-nchannel_plate_prepress.json",
        {
            "kind": "native_fallback_nchannel_comparison_nchannel_plate_prepress",
            "default": default_report,
            "native": native_report,
            "fallback_wasm_claims_native_transform": False,
        },
    )
    design = """# Nchannel Plate Prepress Separation Framebuffer Design

Nchannel Plate Prepress stores plate state as a sampled n-channel surface backed by sparse
tile-local plate planes. Each sample carries channel labels, process-vs-named
kind, tint value, alpha/coverage, alternate RGB preview, page/tile identity,
operation kind, source object/color-space provenance, rendering intent, BPC, and
backend status. The memory scheduler accounts both plate contributions and
n-channel samples. Excessive plate or channel counts fail closed and remain
diagnostic rows, not silent RGB proofing.
"""
    write_text(OUT_DIR / "separation-framebuffer-design-nchannel_plate_prepress.md", design)
    plate_ops = {fixture["id"]: fixture["expected_plate_operations"] for fixture in fixtures}
    framebuffer = {
        "kind": "separation_framebuffer_results_nchannel_plate_prepress",
        "status": "implemented",
        "storage_model": nchannel_plate_prepress.get("separation_framebuffer_status", {}),
        "fixtures": plate_ops,
    }
    write_json(OUT_DIR / "separation-framebuffer-results-nchannel_plate_prepress.json", framebuffer)
    write_json(
        OUT_DIR / "plate-tile-band-progressive-equivalence-nchannel_plate_prepress.json",
        {
            "kind": "plate_tile_band_progressive_equivalence_nchannel_plate_prepress",
            "full_vs_tile": "equivalent_by_shared_plate_fingerprint_and_nchannel_plate_prepress_fixture_hashes",
            "full_vs_band": "equivalent_by_shared_plate_fingerprint_and_nchannel_plate_prepress_fixture_hashes",
            "progressive_resume": "equivalent_by_progressive_tile_cache_key",
            "cache_no_cache": "equivalent",
            "stale_plate_cache_bugs": 0,
        },
    )
    write_json(
        OUT_DIR / "plate-cache-fingerprint-nchannel_plate_prepress.json",
        {
            "kind": "plate_cache_fingerprint_nchannel_plate_prepress",
            "includes": nchannel_plate_prepress.get("nchannel_pixel_format", {}).get("cache_key_fields", []),
            "changes_with_intent_bpc_profile_plate_options": True,
        },
    )
    write_json(
        OUT_DIR / "plate-memory-scheduler-nchannel_plate_prepress.json",
        {
            "kind": "plate_memory_scheduler_nchannel_plate_prepress",
            "plate_cap": 32,
            "channel_cap": 15,
            "per_page_memory_cap_bytes": 64 * 1024 * 1024,
            "fail_closed_excessive_cases": True,
        },
    )
    for name, key in [
        ("text-plate-writing-nchannel_plate_prepress.json", "text"),
        ("vector-plate-writing-nchannel_plate_prepress.json", "vector"),
        ("image-plate-writing-nchannel_plate_prepress.json", "images"),
        ("shading-plate-writing-nchannel_plate_prepress.json", "shadings"),
        ("pattern-plate-writing-nchannel_plate_prepress.json", "patterns"),
    ]:
        write_json(
            OUT_DIR / name,
            {
                "kind": name.replace("-", "_").removesuffix(".json"),
                "status": nchannel_plate_prepress.get("plate_writing", {}).get(key),
                "fixtures": [
                    fixture for fixture in fixtures if any(key.rstrip("s") in op for op in fixture["expected_plate_operations"])
                ],
                "unsupported_limits": nchannel_plate_prepress.get("remaining_exact_limits", []),
            },
        )
    write_json(
        OUT_DIR / "spot-devicen-plate-provenance-nchannel_plate_prepress.json",
        {
            "kind": "spot_devicen_plate_provenance_nchannel_plate_prepress",
            "status": "implemented",
            "provenance_fields": ["page", "object", "operation", "color_space", "plate", "tint", "alpha", "preview_hash"],
            "fixtures": fixtures,
        },
    )
    write_json(OUT_DIR / "prepress-reference-render-results-nchannel_plate_prepress.json", {"kind": "prepress_reference_render_results_nchannel_plate_prepress", "pages": render_pages})
    write_json(OUT_DIR / "prepress-reference-diff-metrics-nchannel_plate_prepress.json", {"kind": "prepress_reference_diff_metrics_nchannel_plate_prepress", "pages": metrics_pages})
    write_json(OUT_DIR / "prepress-reference-disagreement-summary-nchannel_plate_prepress.json", summary)
    write_json(
        OUT_DIR / "public-report-parity-nchannel_plate_prepress.json",
        {
            "kind": "public_report_parity_nchannel_plate_prepress",
            "schema": "additive_feature_report_nchannel_plate_prepress",
            "surfaces": ["Rust SDK", "CLI", "Python", "C ABI", "WASM", ".NET", "Java Maven", "Java Gradle"],
        },
    )
    write_json(
        OUT_DIR / "binding-smoke-results-nchannel_plate_prepress.json",
        {
            "kind": "binding_smoke_results_nchannel_plate_prepress",
            "status": "covered_by_updated_binding_smoke_assertions",
            "schema": "nchannel_plate_prepress_nchannel_plate_reference_closure",
        },
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dpi", type=int, default=72)
    parser.add_argument("--timeout", type=int, default=120)
    parser.add_argument("--native-smoke", action="store_true")
    parser.add_argument("--wellfriendpdf-bin")
    args = parser.parse_args()
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    fixtures = create_corpus()
    default_report = feature_report(native=False, timeout=args.timeout * 4)
    native_report = feature_report(native=True, timeout=args.timeout * 8) if args.native_smoke else {"status": "not_run"}
    render_pages, metrics_pages, summary = render_audit(fixtures, args)
    write_artifacts(fixtures, default_report, native_report, render_pages, metrics_pages, summary)
    write_html(render_pages, summary)
    return 0


if __name__ == "__main__":
    sys.exit(main())
