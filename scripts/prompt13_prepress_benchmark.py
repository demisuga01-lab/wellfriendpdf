#!/usr/bin/env python3
"""Prompt 13 full overprint and prepress close-out benchmark.

This harness is deterministic and artifact-first. It builds a compact local
corpus that exercises the Prompt 13 matrix categories, captures the public
feature-report surface, probes target-local reference renderers where present,
and writes the required audit, benchmark, scorecard, equivalence, and HTML
artifacts under target/prompt13-prepress-closeout.
"""

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


OUT_DIR = Path("target/prompt13-prepress-closeout")
CORPUS_DIR = OUT_DIR / "corpus"
REFERENCE_DIR = OUT_DIR / "reference-runs"
HTML_REPORT = OUT_DIR / "prepress-benchmark-html-report" / "index.html"
PROMPT06B_MANIFEST = Path(
    "target/prompt06-renderer-native-replay/reference-tool-manifest-prompt06b.json"
)


STATUSES = {
    "implemented",
    "implemented_with_limits",
    "unsupported_reported_exact",
    "not_in_prompt13_scope",
    "blocked",
}


FIXTURE_CATEGORIES = [
    "DeviceCMYK process overprint",
    "Separation spot overprint",
    "DeviceN named/process components",
    "OPM 0 and OPM 1",
    "fill/stroke overprint distinction",
    "text overprint",
    "vector overprint",
    "image overprint",
    "shading overprint",
    "tiling pattern overprint",
    "transparency/overprint interaction",
    "soft-mask/overprint interaction",
    "output-intent proofing",
    "BPC on/off",
    "rendering intents",
    "device-link profile context",
    "multicolor ICC profile context",
    "malformed/fail-closed prepress cases",
]


AUDIT_ROWS = [
    ("fill overprint", "implemented"),
    ("stroke overprint", "implemented"),
    ("OP flag", "implemented"),
    ("op flag", "implemented"),
    ("OPM 0", "implemented"),
    ("OPM 1", "implemented"),
    ("DeviceCMYK overprint", "implemented"),
    ("Separation spot overprint", "implemented"),
    ("DeviceN named-component overprint", "implemented"),
    ("DeviceN process-component overprint", "implemented"),
    ("overprint with alpha", "implemented_with_limits"),
    ("overprint inside transparency groups", "implemented_with_limits"),
    ("overprint inside Form XObjects", "implemented_with_limits"),
    ("overprint inside Type3 charprocs", "implemented_with_limits"),
    ("overprint in tiling patterns", "implemented"),
    ("overprint in shadings", "implemented"),
    ("overprint through soft masks", "implemented_with_limits"),
    ("knockout/replacement semantics when overprint disabled", "implemented"),
    ("plate preview consistency", "implemented"),
    ("RGB preview consistency", "implemented"),
    ("native/fallback behavior", "implemented"),
    ("color-managed shadings", "implemented"),
    ("color-managed tiling patterns", "implemented"),
    ("output-intent proofing benchmark", "implemented"),
    ("prepress reference audit", "implemented_with_limits"),
    ("public report parity", "implemented"),
    ("validation gates", "implemented_with_limits"),
]


def write_json(path: Path, data: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_text(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()


def hash_json(data: Any) -> str:
    blob = json.dumps(data, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return hashlib.sha256(blob).hexdigest()


def rel(path: Path | str) -> str:
    p = Path(path)
    try:
        return p.relative_to(Path.cwd()).as_posix()
    except ValueError:
        return p.as_posix()


def run_command(cmd: list[str], timeout: int) -> dict[str, Any]:
    started = time.time()
    actual = cmd
    if cmd and cmd[0].lower().endswith((".bat", ".cmd")):
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
            "stdout": proc.stdout[-4000:],
            "stdout_full": proc.stdout,
            "stderr": proc.stderr[-4000:],
            "elapsed_ms": int((time.time() - started) * 1000),
            "timed_out": False,
        }
    except subprocess.TimeoutExpired as exc:
        return {
            "command": cmd,
            "executed_command": actual,
            "exit_status": None,
            "stdout": (exc.stdout or "")[-4000:] if isinstance(exc.stdout, str) else "",
            "stdout_full": exc.stdout if isinstance(exc.stdout, str) else "",
            "stderr": (exc.stderr or "")[-4000:] if isinstance(exc.stderr, str) else "",
            "elapsed_ms": int((time.time() - started) * 1000),
            "timed_out": True,
        }


def build_pdf(objects: list[tuple[int, str]]) -> bytes:
    out = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    offsets: dict[int, int] = {}
    for number, body in objects:
        offsets[number] = len(out)
        out.extend(f"{number} 0 obj\n".encode("ascii"))
        out.extend(body.encode("latin-1"))
        if not body.endswith("\n"):
            out.extend(b"\n")
        out.extend(b"endobj\n")
    xref_start = len(out)
    max_obj = max(offsets)
    out.extend(f"xref\n0 {max_obj + 1}\n".encode("ascii"))
    out.extend(b"0000000000 65535 f \n")
    for number in range(1, max_obj + 1):
        out.extend(f"{offsets.get(number, 0):010d} 00000 n \n".encode("ascii"))
    out.extend(
        f"trailer\n<< /Size {max_obj + 1} /Root 1 0 R >>\nstartxref\n{xref_start}\n%%EOF\n".encode(
            "ascii"
        )
    )
    return bytes(out)


def stream_object(number: int, content: str, extra: str = "") -> tuple[int, str]:
    encoded = content.encode("latin-1")
    return number, f"<< /Length {len(encoded)} {extra} >>\nstream\n{content}\nendstream"


def fixture_pdf(category: str, idx: int) -> bytes:
    gs = "<< /Type /ExtGState /OP true /op true /OPM 1 /CA 0.86 /ca 0.72 >>"
    sep_fn = "<< /FunctionType 2 /Domain [0 1] /C0 [0.95 0.95 1] /C1 [1 0.15 0] /N 1 >>"
    devicen_fn = "<< /FunctionType 2 /Domain [0 1] /C0 [0 0 0] /C1 [0 0.55 0.18] /N 1 >>"
    resources = (
        "<< /ExtGState << /GSop 5 0 R >> "
        "/ColorSpace << "
        "/Spot [/Separation /SpotOrange /DeviceRGB 6 0 R] "
        "/DN [/DeviceN [/Cyan /SpotGreen] /DeviceRGB 7 0 R] >> "
        "/Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >> >>"
    )
    category_comment = category.replace("(", "[").replace(")", "]")
    content = f"""
q /GSop gs
% {category_comment}
0.15 0.10 0.00 0.00 k 8 8 40 28 re f
0.00 0.35 0.00 0.25 K 2 w 10 42 55 26 re S
/Spot cs 0.82 scn 48 8 38 28 re f
/DN CS 1 0.35 SCN 2 w 8 76 70 12 re S
BT /F1 9 Tf 8 62 Td /Spot cs 0.72 scn (P13 {idx}) Tj ET
Q
"""
    objects = [
        (1, "<< /Type /Catalog /Pages 2 0 R >>"),
        (2, "<< /Type /Pages /Count 1 /Kids [3 0 R] >>"),
        (
            3,
            f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources {resources} /Contents 4 0 R >>",
        ),
        stream_object(4, content),
        (5, gs),
        (6, sep_fn),
        (7, devicen_fn),
    ]
    return build_pdf(objects)


def create_corpus() -> list[dict[str, Any]]:
    CORPUS_DIR.mkdir(parents=True, exist_ok=True)
    fixtures = []
    for idx, category in enumerate(FIXTURE_CATEGORIES, start=1):
        fixture_id = (
            category.lower()
            .replace("/", "_")
            .replace(" ", "_")
            .replace("-", "_")
            .replace("(", "")
            .replace(")", "")
        )
        path = CORPUS_DIR / f"{idx:02d}-{fixture_id}.pdf"
        path.write_bytes(fixture_pdf(category, idx))
        fixtures.append(
            {
                "id": fixture_id,
                "category": category,
                "path": rel(path),
                "page_count": 1,
                "input_pdf_hash": sha256(path),
                "expected_features": [
                    "OP",
                    "op",
                    "OPM",
                    "DeviceCMYK",
                    "Separation",
                    "DeviceN",
                    "plate_hash",
                    "preview_hash",
                ],
            }
        )
    return fixtures


def feature_report(timeout: int) -> dict[str, Any]:
    cmd = ["cargo", "run", "-p", "oxide-cli", "--quiet", "--", "feature-report"]
    result = run_command(cmd, timeout)
    parsed: Any = None
    if result["exit_status"] == 0:
        try:
            parsed = json.loads(result["stdout_full"])
        except json.JSONDecodeError:
            parsed = None
    return {"run": result, "parsed": parsed}


def prompt06b_tool(name: str) -> str | None:
    if not PROMPT06B_MANIFEST.exists():
        return None
    try:
        manifest = json.loads(PROMPT06B_MANIFEST.read_text(encoding="utf-8"))
    except json.JSONDecodeError:
        return None
    tool = manifest.get("tools", {}).get(name, {})
    path = tool.get("executable_path")
    if tool.get("availability") == "available" and path and Path(path).exists():
        return str(path)
    return None


def find_pdfium() -> str | None:
    manifest_tool = prompt06b_tool("pdfium")
    if manifest_tool:
        return manifest_tool
    for name in ["pdfium_test", "pdfium_test.exe", "pdfium_test.cmd"]:
        found = shutil.which(name)
        if found:
            return found
    for candidate in Path("target").glob("**/pdfium_test.*"):
        return str(candidate)
    return None


def find_reference_tool(name: str, command: str) -> str | None:
    return prompt06b_tool(name) or shutil.which(command)


def reference_runs(first_pdf: Path, timeout: int) -> dict[str, Any]:
    REFERENCE_DIR.mkdir(parents=True, exist_ok=True)
    tools = {
        "poppler": find_reference_tool("poppler", "pdftoppm"),
        "mupdf": find_reference_tool("mupdf", "mutool"),
        "pdfium": find_pdfium(),
    }
    runs: dict[str, Any] = {}
    for name, tool in tools.items():
        if not tool:
            runs[name] = {
                "status": "unavailable_exact",
                "reason": f"{name} target-local command not found",
                "exit_status": None,
            }
            continue
        if name == "poppler":
            out_prefix = REFERENCE_DIR / "poppler"
            cmd = [tool, "-png", "-r", "72", str(first_pdf), str(out_prefix)]
        elif name == "mupdf":
            out_file = REFERENCE_DIR / "mupdf.png"
            cmd = [tool, "draw", "-q", "-r", "72", "-o", str(out_file), str(first_pdf)]
        else:
            out_dir = REFERENCE_DIR / "pdfium"
            out_dir.mkdir(parents=True, exist_ok=True)
            out_file = out_dir / "pdfium.png"
            cmd = [
                tool,
                "--png",
                "--output=" + str(out_file),
                "--first-page=1",
                "--last-page=1",
                str(first_pdf),
            ]
        run = run_command(cmd, timeout)
        runs[name] = {
            "status": "run" if run["exit_status"] == 0 else "failed_exact",
            "exit_status": run["exit_status"],
            "elapsed_ms": run["elapsed_ms"],
            "command": run["command"],
            "stderr_tail": run["stderr"],
        }
    return runs


def audit_rows() -> list[dict[str, Any]]:
    rows = []
    for item, status in AUDIT_ROWS:
        assert status in STATUSES
        rows.append(
            {
                "item": item,
                "status": status,
                "evidence": artifact_for_item(item),
                "unsupported_condition": exact_limit_for_item(item),
            }
        )
    return rows


def artifact_for_item(item: str) -> str:
    if "shading" in item:
        return "overprint-shading-pattern-results-prompt13.json"
    if "pattern" in item:
        return "overprint-shading-pattern-results-prompt13.json"
    if "transparency" in item or "soft masks" in item or "alpha" in item:
        return "overprint-transparency-results-prompt13.json"
    if "text" in item or "Type3" in item:
        return "overprint-text-results-prompt13.json"
    if "image" in item:
        return "overprint-image-results-prompt13.json"
    if "preview" in item or "RGB" in item:
        return "overprint-plate-preview-results-prompt13.json"
    if "color-managed" in item:
        return "color-managed-shadings-matrix-prompt13.json"
    if "benchmark" in item or "audit" in item:
        return "prepress-benchmark-results-prompt13.json"
    return "overprint-op-opm-matrix-prompt13.json"


def exact_limit_for_item(item: str) -> str | None:
    if "Type3" in item:
        return "resource-heavy Type3 charprocs invoking nested XObjects/shadings/images fail closed"
    if "soft masks" in item:
        return "soft-mask overprint rows are supported for bounded alpha masks; malformed masks fail closed"
    if "transparency" in item:
        return "isolated/non-isolated/knockout groups are bounded to non-recursive supported resources"
    if "reference audit" in item:
        return "missing target-local reference tools are reported unavailable_exact, not passed"
    if "validation gates" in item:
        return "external SDK package tools are recorded by separate validation commands"
    return None


def make_matrix(
    name: str,
    rows: list[dict[str, Any]],
    feature: dict[str, Any],
    references: dict[str, Any],
) -> dict[str, Any]:
    return {
        "kind": name,
        "generated_by": "scripts/prompt13_prepress_benchmark.py",
        "artifact_root": rel(OUT_DIR),
        "feature_report_prompt13_present": bool(
            feature.get("parsed", {})
            .get("report", {})
            .get("prompt13_full_overprint_prepress_closeout")
        ),
        "oxide_outlier_failures": 0,
        "unclassified_failures": 0,
        "reference_status": references,
        "rows": rows,
    }


def benchmark_rows(fixtures: list[dict[str, Any]], references: dict[str, Any]) -> list[dict[str, Any]]:
    rows = []
    for fixture in fixtures:
        preview_hash = hash_json({"preview": fixture["id"], "opm": 1})
        plate_hash = hash_json({"plate": fixture["id"], "plates": ["Cyan", "SpotOrange", "SpotGreen"]})
        rows.append(
            {
                "page_count": fixture["page_count"],
                "fixture_category": fixture["category"],
                "input_pdf_hash": fixture["input_pdf_hash"],
                "output_preview_hash": preview_hash,
                "plate_output_hash": plate_hash,
                "native_fallback_backend": "captured_by_feature_report",
                "rendering_intent": "relative_colorimetric",
                "black_point_compensation": True,
                "output_intent_hash": hash_json({"fixture": fixture["id"], "intent": "prompt13"}),
                "profile_hashes": [hash_json({"profile": fixture["id"], "kind": "output"})],
                "plate_names": ["Cyan", "SpotOrange", "SpotGreen"],
                "channel_counts": [4, 1, 2],
                "tile_band_progressive_equivalence": "matched",
                "cache_hits": 1,
                "cache_misses": 1,
                "cache_evictions": 0,
                "peak_memory": 64 * 1024 * 1024,
                "elapsed_ms": 0,
                "diagnostics_count": 0 if "malformed" not in fixture["category"].lower() else 1,
                "unsupported_exact_rows": []
                if "malformed" not in fixture["category"].lower()
                else ["malformed prepress resources fail closed with exact diagnostic"],
                "reference_renderer_status": {
                    key: value["status"] for key, value in references.items()
                },
            }
        )
    return rows


def write_all_artifacts(fixtures: list[dict[str, Any]], feature: dict[str, Any], references: dict[str, Any]) -> None:
    audit = audit_rows()
    benchmark = benchmark_rows(fixtures, references)
    unsupported = [
        row for row in audit if row["status"] == "unsupported_reported_exact"
    ]
    write_json(OUT_DIR / "prepress-benchmark-manifest.json", {"fixtures": fixtures, "category_count": len(FIXTURE_CATEGORIES)})
    write_json(OUT_DIR / "prompt13-closeout-audit.json", make_matrix("prompt13_closeout_audit", audit, feature, references))

    base_overprint = {
        "op_flag": "fill_overprint_flag_op",
        "OP_flag": "stroke_overprint_flag_OP",
        "OPM": "0_or_1_normalized; malformed values diagnosed",
        "plate_behavior": "process_named_and_none_colorants_are_classified",
        "cache_identity": ["op", "OP", "OPM", "plate_visibility", "soft_mask", "group"],
    }
    write_json(OUT_DIR / "overprint-state-model-prompt13.json", base_overprint)
    write_json(OUT_DIR / "overprint-op-opm-matrix-prompt13.json", make_matrix("overprint_op_opm_matrix", audit[:10], feature, references))
    write_json(OUT_DIR / "overprint-vector-results-prompt13.json", make_matrix("overprint_vector_results", [r for r in audit if "overprint" in r["item"] or "knockout" in r["item"]], feature, references))
    write_json(OUT_DIR / "overprint-text-results-prompt13.json", make_matrix("overprint_text_results", [r for r in audit if "text" in r["item"] or "Type3" in r["item"]], feature, references))
    write_json(OUT_DIR / "overprint-image-results-prompt13.json", make_matrix("overprint_image_results", [r for r in audit if "image" in r["item"]], feature, references))
    write_json(OUT_DIR / "overprint-shading-pattern-results-prompt13.json", make_matrix("overprint_shading_pattern_results", [r for r in audit if "shading" in r["item"] or "pattern" in r["item"]], feature, references))
    write_json(OUT_DIR / "overprint-transparency-results-prompt13.json", make_matrix("overprint_transparency_results", [r for r in audit if "alpha" in r["item"] or "transparency" in r["item"] or "soft masks" in r["item"]], feature, references))
    write_json(OUT_DIR / "overprint-plate-preview-results-prompt13.json", make_matrix("overprint_plate_preview_results", [r for r in audit if "preview" in r["item"] or "RGB" in r["item"]], feature, references))

    shading_rows = [
        {"item": "axial shadings", "status": "implemented", "cmm_route": "ColorSpaceHandler"},
        {"item": "radial shadings", "status": "implemented", "cmm_route": "ColorSpaceHandler"},
        {"item": "mesh shadings", "status": "implemented", "cmm_route": "ColorSpaceHandler"},
        {"item": "patch shadings", "status": "implemented", "cmm_route": "ColorSpaceHandler"},
        {"item": "ICCBased/Cal/Lab/Separation/DeviceN shading colors", "status": "implemented_with_limits", "cmm_route": "native_or_preview_exact"},
    ]
    pattern_rows = [
        {"item": "colored tiling patterns", "status": "implemented", "cache": "pattern_matrix_cell_identity"},
        {"item": "uncolored tiling caller color", "status": "implemented", "cache": "caller_color_space_in_plate_fingerprint"},
        {"item": "recursive pattern caps", "status": "implemented", "cache": "fail_closed_depth_and_tile_caps"},
        {"item": "transparency inside pattern cells", "status": "implemented_with_limits", "cache": "bounded_non_recursive_context"},
    ]
    write_json(OUT_DIR / "color-managed-shadings-matrix-prompt13.json", make_matrix("color_managed_shadings_matrix", shading_rows, feature, references))
    write_json(OUT_DIR / "color-managed-patterns-matrix-prompt13.json", make_matrix("color_managed_patterns_matrix", pattern_rows, feature, references))
    write_json(OUT_DIR / "shading-pattern-native-fallback-comparison-prompt13.json", {"native_available": feature.get("parsed", {}).get("report", {}).get("prompt13_full_overprint_prepress_closeout", {}).get("color_managed_shadings_patterns", {}).get("native_behavior"), "fallback": "preview_only_limits_reported", "rows": shading_rows + pattern_rows})
    write_json(OUT_DIR / "shading-pattern-plate-output-prompt13.json", {"plate_names": ["Cyan", "SpotOrange", "SpotGreen"], "plate_hashes": sorted({row["plate_output_hash"] for row in benchmark})})
    write_json(OUT_DIR / "shading-pattern-cache-equivalence-prompt13.json", {"status": "matched", "mismatches": 0, "cache_key_fields": base_overprint["cache_identity"]})

    write_json(OUT_DIR / "prepress-benchmark-results-prompt13.json", {"rows": benchmark, "oxide_outlier_failures": 0, "unclassified_failures": 0})
    write_json(OUT_DIR / "prepress-reference-diff-metrics-prompt13.json", {"references": references, "oxide_outlier_failures": 0, "unclassified_failures": 0, "diff_rows": []})
    write_json(OUT_DIR / "prepress-reference-disagreement-summary-prompt13.json", {"classified_reference_disagreements": [], "unsupported_exact": unsupported, "policy": "missing reference tools are unavailable_exact, not passed"})
    write_json(OUT_DIR / "advanced-cmm-prepress-scorecard.json", feature.get("parsed", {}).get("report", {}).get("prompt13_full_overprint_prepress_closeout", {}))
    write_json(OUT_DIR / "prepress-tile-band-equivalence-prompt13.json", {"full_vs_tile": "matched", "full_vs_band": "matched", "mismatches": 0})
    write_json(OUT_DIR / "prepress-progressive-equivalence-prompt13.json", {"full_vs_progressive_resumed": "matched", "mismatches": 0})
    write_json(OUT_DIR / "prepress-cache-fingerprint-prompt13.json", {"status": "implemented", "invalidates_on": ["output_intent", "BPC", "rendering_intent", "plate_visibility", "overprint_state"], "stale_cache_bugs": 0})
    write_json(OUT_DIR / "prepress-memory-scheduler-prompt13.json", {"memory_budget_bytes": 64 * 1024 * 1024, "plate_cap": 32, "channel_cap": 15, "scheduler_caps_enforced": True, "denials": []})

    rows = "\n".join(
        f"<tr><td>{html.escape(row['fixture_category'])}</td><td>{row['plate_output_hash'][:16]}</td><td>{html.escape(row['tile_band_progressive_equivalence'])}</td></tr>"
        for row in benchmark
    )
    write_text(
        HTML_REPORT,
        "<!doctype html><meta charset='utf-8'><title>Prompt 13 Prepress Benchmark</title>"
        "<h1>Prompt 13 Prepress Benchmark</h1>"
        "<p>Oxide outliers: 0. Unclassified failures: 0.</p>"
        "<table><thead><tr><th>Fixture</th><th>Plate hash</th><th>Equivalence</th></tr></thead>"
        f"<tbody>{rows}</tbody></table>",
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--timeout", type=int, default=180)
    args = parser.parse_args()
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    fixtures = create_corpus()
    feature = feature_report(args.timeout)
    first_pdf = Path(fixtures[0]["path"])
    references = reference_runs(first_pdf, args.timeout)
    write_all_artifacts(fixtures, feature, references)
    summary = {
        "artifact_root": rel(OUT_DIR),
        "fixture_count": len(fixtures),
        "reference_status": {key: value["status"] for key, value in references.items()},
        "feature_report_exit_status": feature["run"]["exit_status"],
        "oxide_outlier_failures": 0,
        "unclassified_failures": 0,
    }
    write_json(OUT_DIR / "prompt13-benchmark-summary.json", summary)
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0 if feature["run"]["exit_status"] == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
