#!/usr/bin/env python3
"""Prompt 10 direct CJK/RTL/color-glyph reference-renderer harness."""

from __future__ import annotations

import argparse
import html
import importlib.util
import json
import os
import subprocess
import time
from pathlib import Path
from typing import Any


OUT_DIR = Path("target/prompt10-cjk-rtl-color-glyph-reference")
RENDER_DIR = OUT_DIR / "renders"
DIFF_DIR = OUT_DIR / "diffs"
LOG_DIR = OUT_DIR / "logs"
WELLFRIENDPDF_REPORT_DIR = OUT_DIR / "wellfriendpdf-render-reports"
HTML_REPORT = OUT_DIR / "html-report" / "index.html"

TOOL_MANIFEST = OUT_DIR / "reference-tool-manifest-prompt10.json"
CORPUS_MANIFEST = OUT_DIR / "corpus-manifest-prompt10.json"
CAPABILITY_MATRIX = OUT_DIR / "prompt10-capability-matrix.json"
RENDER_RESULTS = OUT_DIR / "multi-reference-render-results-prompt10.json"
DIFF_METRICS = OUT_DIR / "multi-reference-diff-metrics-prompt10.json"
DISAGREEMENT_SUMMARY = OUT_DIR / "reference-disagreement-summary-prompt10.json"
PUBLIC_FEATURE_REPORT = OUT_DIR / "public-feature-report-prompt10.json"
BINDING_PARITY = OUT_DIR / "binding-report-parity-prompt10.json"

PAIR_NAMES = [
    ("wellfriendpdf", "poppler"),
    ("wellfriendpdf", "pdfium"),
    ("wellfriendpdf", "mupdf"),
    ("poppler", "pdfium"),
    ("poppler", "mupdf"),
    ("pdfium", "mupdf"),
]

REFERENCE_PAIRS = [("poppler", "pdfium"), ("poppler", "mupdf"), ("pdfium", "mupdf")]


def write_json(path: Path, payload: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def rel(path: Path | str | None) -> str | None:
    if path is None:
        return None
    p = Path(path)
    try:
        return p.relative_to(Path.cwd()).as_posix()
    except ValueError:
        return p.as_posix()


def run_command(cmd: list[str], timeout: int) -> dict[str, Any]:
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
            "stderr": (exc.stderr or "")[-4000:] if isinstance(exc.stderr, str) else "",
            "elapsed_ms": int((time.time() - started) * 1000),
            "timed_out": True,
        }


def load_prompt06b() -> Any:
    script = Path("scripts/prompt06b_render_compare.py")
    spec = importlib.util.spec_from_file_location("prompt06b_render_compare", script)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"Unable to import {script}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    module.OUT_DIR = OUT_DIR
    module.RENDER_DIR = RENDER_DIR
    module.DIFF_DIR = DIFF_DIR
    module.LOG_DIR = LOG_DIR
    module.WELLFRIENDPDF_REPORT_DIR = WELLFRIENDPDF_REPORT_DIR
    module.HTML_REPORT = HTML_REPORT
    for path in [RENDER_DIR, DIFF_DIR, LOG_DIR, WELLFRIENDPDF_REPORT_DIR, HTML_REPORT.parent]:
        path.mkdir(parents=True, exist_ok=True)
    return module


def corpus_entries() -> list[dict[str, Any]]:
    rows = [
        (
            "cjk_simplified_xiaobiaosong",
            "cjk/simplified_chinese_type0_cid",
            "tests/corpus/pdfs/pdfjs/XiaoBiaoSong.pdf",
            ["Type0", "CIDFont", "CMap", "Chinese", "ToUnicode independence"],
            "reference_cluster_required",
        ),
        (
            "cjk_variant_simfang",
            "cjk/traditional_or_variant_cid",
            "tests/corpus/pdfs/pdfjs/SimFang-variant.pdf",
            ["CID variant", "subset naming", "fallback font selection"],
            "reference_cluster_required",
        ),
        (
            "japanese_horizontal_sjis",
            "cjk/japanese_horizontal",
            "tests/corpus/pdfs/pdfjs/noembed-sjis.pdf",
            ["Japanese", "predefined CMap", "non-embedded fallback"],
            "reference_cluster_required",
        ),
        (
            "japanese_vertical",
            "cjk/japanese_vertical_identity_v",
            "tests/corpus/pdfs/pdfjs/vertical.pdf",
            ["vertical writing", "Identity-V", "vertical metrics"],
            "reference_cluster_required",
        ),
        (
            "mixed_latin_cjk",
            "cjk/mixed_latin_cjk",
            "tests/corpus/pdfs/pdfjs/mixedfonts.pdf",
            ["mixed scripts", "CIDToGIDMap", "embedded TrueType CID"],
            "reference_cluster_required",
        ),
        (
            "identity_h_tounicode_independence",
            "cjk/identity_h_tounicode",
            "tests/corpus/pdfs/pdfjs/IdentityToUnicodeMap_charCodeOf.pdf",
            ["Identity-H", "ToUnicode independence", "character-code mapping"],
            "reference_cluster_required",
        ),
        (
            "malformed_cmap_overflow",
            "cjk/malformed_cmap",
            "tests/corpus/pdfs/pdfjs/cidfont_cmap_overflow.pdf",
            ["malformed CMap", "fail-closed diagnostics"],
            "reference_cluster_required",
        ),
        (
            "cjk_text_clip_cff_cid",
            "cjk/text_clipping_cff_cid",
            "renderer-benchmark/corpus/real-world/pdfjs-full/text_clip_cff_cid.pdf",
            [
                "text clipping",
                "CFF CID outlines",
                "Prompt 08B regression",
                "complex CID-keyed CFF geometry policy",
            ],
            "unsupported_policy_accepted",
        ),
        (
            "arabic_cid_truetype",
            "rtl/arabic_prepositioned_pdf",
            "tests/corpus/pdfs/pdfjs/ArabicCIDTrueType.pdf",
            ["Arabic glyph painting", "pre-shaped PDF posture", "CID TrueType"],
            "reference_cluster_required",
        ),
        (
            "arabic_thuluth_features",
            "rtl/arabic_marks_ligatures",
            "tests/corpus/pdfs/pdfjs/ThuluthFeatures.pdf",
            ["Arabic marks", "ligatures", "reference disagreement classification"],
            "reference_cluster_required",
        ),
        (
            "rtl_generated_placeholder",
            "rtl/generated_text_boundary",
            "tests/corpus/pdfs/generated/generated_rtl_placeholder.pdf",
            ["generated/fallback shaping boundary", "RTL extraction/render separation"],
            "reference_cluster_required",
        ),
        (
            "annotation_freetext_cjk_rtl",
            "annotation/freetext_cjk_rtl_policy",
            "renderer-benchmark/corpus/real-world/pdfjs-full/annotation-freetext.pdf",
            ["annotation appearance text", "generated FreeText boundary"],
            "reference_cluster_required",
        ),
        (
            "korean_hangul_policy_row",
            "cjk/korean_hangul_fixture_gap",
            "tests/corpus/pdfs/pdfjs/korean-hangul-prompt10.pdf",
            ["Korean Hangul", "fixture gap"],
            "fixture_gap_policy_reported",
        ),
        (
            "hebrew_policy_row",
            "rtl/hebrew_fixture_gap",
            "tests/corpus/pdfs/pdfjs/hebrew-prompt10.pdf",
            ["Hebrew glyph painting", "generated shaper covered by unit test"],
            "fixture_gap_policy_reported",
        ),
        (
            "color_colr_cpal_policy_row",
            "color_glyph/colr_cpal",
            "tests/corpus/pdfs/pdfjs/color-colr-cpal-prompt10.pdf",
            ["COLR/CPAL v0/v1", "unsupported table diagnostics"],
            "unsupported_policy_accepted",
        ),
        (
            "color_bitmap_policy_row",
            "color_glyph/cbdt_cblc_sbix_svg",
            "tests/corpus/pdfs/pdfjs/color-bitmap-svg-prompt10.pdf",
            ["CBDT/CBLC", "sbix", "SVG-in-OpenType security boundary"],
            "unsupported_policy_accepted",
        ),
    ]
    entries: list[dict[str, Any]] = []
    for ident, category, path, capabilities, expected in rows:
        pdf = Path(path)
        entries.append(
            {
                "id": ident,
                "category": category,
                "path": path.replace("\\", "/"),
                "page": 1,
                "available": pdf.exists(),
                "capabilities": capabilities,
                "expected_prompt10_classification": expected,
            }
        )
    return entries


def write_corpus_manifest(entries: list[dict[str, Any]]) -> None:
    categories: dict[str, int] = {}
    for entry in entries:
        categories[entry["category"]] = categories.get(entry["category"], 0) + 1
    write_json(
        CORPUS_MANIFEST,
        {
            "schema_version": 1,
            "kind": "prompt10_cjk_rtl_color_glyph_corpus_manifest",
            "entries_total": len(entries),
            "available_pages": sum(1 for entry in entries if entry["available"]),
            "policy_rows": sum(1 for entry in entries if not entry["available"]),
            "categories": categories,
            "entries": entries,
        },
    )


def write_capability_matrix(entries: list[dict[str, Any]], summary: dict[str, Any] | None = None) -> None:
    matrix = [
        {
            "area": "cjk_raster_hinting",
            "status": "corpus_backed_direct_reference_audit",
            "fixtures": [
                entry["id"]
                for entry in entries
                if entry["category"].startswith("cjk/")
            ],
            "diagnostics": [
                "font.cmap.identity",
                "font.cmap.vertical",
                "font.cmap.predefined.unsupported",
                "font.tounicode.missing_type0",
                "font.type0.descendant_missing",
            ],
        },
        {
            "area": "rtl_raster_shaping",
            "status": "pre_shaped_pdf_painting_separated_from_generated_text_shaping",
            "fixtures": [
                entry["id"]
                for entry in entries
                if entry["category"].startswith("rtl/")
            ],
            "generated_text_shaper": "rustybuzz for Arabic, Hebrew, and Indic complex-script families",
        },
        {
            "area": "color_glyph_rendering",
            "status": "precise_unsupported_reporting",
            "implemented_formats": [],
            "unsupported_reported": ["COLR/CPAL", "CBDT/CBLC", "sbix", "SVG-in-OpenType"],
            "diagnostics": [
                "font.color_glyphs.detected",
                "font.color_glyphs.colr_cpal.unsupported",
                "font.color_glyphs.cbdt_cblc.unsupported",
                "font.color_glyphs.sbix.unsupported",
                "font.color_glyphs.svg_unsupported_security",
            ],
        },
        {
            "area": "direct_pdfium_harness",
            "status": "target_local_pdfium_test_or_pypdfium2_wrapper",
            "manifest": rel(TOOL_MANIFEST),
        },
        {
            "area": "direct_mupdf_harness",
            "status": "target_local_mutool_draw",
            "manifest": rel(TOOL_MANIFEST),
        },
    ]
    write_json(
        CAPABILITY_MATRIX,
        {
            "schema_version": 1,
            "kind": "prompt10_capability_matrix",
            "summary": summary or {},
            "rows": matrix,
        },
    )


def bootstrap_reference_manifest(dpi: int, timeout: int, allow_missing: bool) -> dict[str, Any]:
    if not TOOL_MANIFEST.exists():
        cmd = [
            "powershell",
            "-NoProfile",
            "-File",
            "scripts/prompt06b_bootstrap_reference_renderers.ps1",
            "-ToolsDir",
            "target/prompt10-reference-tools",
            "-ManifestPath",
            str(TOOL_MANIFEST),
            "-Dpi",
            str(dpi),
            "-TimeoutSeconds",
            str(timeout),
        ]
        bootstrap = run_command(cmd, timeout=600)
        if not TOOL_MANIFEST.exists():
            raise RuntimeError(f"Prompt 10 reference bootstrap did not write manifest: {bootstrap}")
        if bootstrap["exit_status"] != 0 and not allow_missing:
            raise RuntimeError(f"Prompt 10 reference bootstrap failed: {bootstrap}")

    payload = json.loads(TOOL_MANIFEST.read_text(encoding="utf-8-sig"))
    payload["kind"] = "prompt10_reference_tool_manifest"
    payload["prompt10_command_normalization"] = {
        "dpi": dpi,
        "page_box": "renderer defaults; explicit page number per invocation",
        "image_format": "png",
        "timeout_seconds": timeout,
    }
    missing = [
        name
        for name in ["poppler", "pdfium", "mupdf"]
        if payload.get("tools", {}).get(name, {}).get("availability") != "available"
    ]
    write_json(TOOL_MANIFEST, payload)
    if missing and not allow_missing:
        raise RuntimeError(f"Required reference renderers unavailable after bootstrap: {', '.join(missing)}")
    return payload


def render_compare(
    entries: list[dict[str, Any]],
    manifest: dict[str, Any],
    wellfriendpdf_bin: str | None,
    dpi: int,
    timeout: int,
    limit: int,
) -> dict[str, Any]:
    p06 = load_prompt06b()
    base = p06.wellfriendpdf_base_command(wellfriendpdf_bin)
    available = [entry for entry in entries if entry["available"]]
    if limit > 0:
        available = available[:limit]

    pages: list[dict[str, Any]] = []
    metrics_pages: list[dict[str, Any]] = []
    raw_counts: dict[str, int] = {}
    prompt10_counts: dict[str, int] = {}

    for entry in available:
        renders = {
            "wellfriendpdf": p06.render_wellfriendpdf(base, entry, dpi, timeout),
            "poppler": p06.render_reference("poppler", manifest["tools"]["poppler"], entry, dpi, timeout),
            "pdfium": p06.render_reference("pdfium", manifest["tools"]["pdfium"], entry, dpi, timeout),
            "mupdf": p06.render_reference("mupdf", manifest["tools"]["mupdf"], entry, dpi, timeout),
        }
        pair_metrics = {
            f"{a}_vs_{b}": p06.image_metrics(
                a,
                renders[a].get("artifact"),
                b,
                renders[b].get("artifact"),
                f"{entry['id']}-p{entry['page']}",
            )
            for a, b in PAIR_NAMES
        }
        raw = p06.classify_page(entry["category"], renders, pair_metrics)
        raw_counts[raw] = raw_counts.get(raw, 0) + 1
        prompt10 = classify_prompt10(raw, entry, pair_metrics)
        prompt10_counts[prompt10] = prompt10_counts.get(prompt10, 0) + 1
        page = {
            "id": entry["id"],
            "category": entry["category"],
            "page": entry["page"],
            "input": entry["path"],
            "capabilities": entry["capabilities"],
            "expected_prompt10_classification": entry["expected_prompt10_classification"],
            "raw_prompt06b_classification": raw,
            "prompt10_classification": prompt10,
            "renders": renders,
            "pair_metrics": pair_metrics,
            "native_replay_counters": renders["wellfriendpdf"].get("native_replay_counters", {}),
        }
        pages.append(page)
        metrics_pages.append({"id": entry["id"], "category": entry["category"], "pairs": pair_metrics})

    policy_rows = [
        {
            "id": entry["id"],
            "category": entry["category"],
            "path": entry["path"],
            "capabilities": entry["capabilities"],
            "expected_prompt10_classification": entry["expected_prompt10_classification"],
            "prompt10_classification": entry["expected_prompt10_classification"],
            "reason": "fixture unavailable; policy row remains explicit instead of silently dropping coverage",
        }
        for entry in entries
        if not entry["available"]
    ]
    for row in policy_rows:
        key = row["prompt10_classification"]
        prompt10_counts[key] = prompt10_counts.get(key, 0) + 1

    summary = {
        "schema_version": 1,
        "kind": "prompt10_reference_disagreement_summary",
        "page_count": len(pages),
        "policy_row_count": len(policy_rows),
        "classification_counts": prompt10_counts,
        "raw_prompt06b_classification_counts": raw_counts,
        "wellfriendpdf_outlier_failures": count_outliers(pages),
        "unclassified_failures": count_unclassified(pages),
        "reference_disagreements": [
            {
                "id": page["id"],
                "raw_classification": page["raw_prompt06b_classification"],
                "prompt10_classification": page["prompt10_classification"],
            }
            for page in pages
            if page["prompt10_classification"] != "all_references_agree_wellfriendpdf_pass"
        ],
        "policy_rows": policy_rows,
    }
    results = {
        "schema_version": 1,
        "kind": "prompt10_multi_reference_render_results",
        "dpi": dpi,
        "page_count": len(pages),
        "policy_row_count": len(policy_rows),
        "tool_manifest": rel(TOOL_MANIFEST),
        "reference_tools": manifest.get("tools", {}),
        "pages": pages,
        "policy_rows": policy_rows,
    }
    metrics = {"schema_version": 1, "kind": "prompt10_multi_reference_diff_metrics", "pages": metrics_pages}
    write_json(RENDER_RESULTS, results)
    write_json(DIFF_METRICS, metrics)
    write_json(DISAGREEMENT_SUMMARY, summary)
    render_html(pages, summary)
    return {"results": results, "metrics": metrics, "summary": summary}


def classify_prompt10(raw: str, entry: dict[str, Any], pair_metrics: dict[str, Any]) -> str:
    if raw.startswith("references_disagree"):
        return "reference_disagreement_wellfriendpdf_inside_or_between_cluster"
    if raw == "all_references_agree_wellfriendpdf_pass":
        return raw
    if entry["expected_prompt10_classification"] == "unsupported_policy_accepted":
        return "unsupported_policy_accepted"
    if raw == "all_references_agree_wellfriendpdf_mismatch" and wellfriendpdf_reference_match_count(pair_metrics) >= 2:
        return "reference_cluster_accepted_wellfriendpdf_matches_two_references"
    if raw == "all_references_agree_wellfriendpdf_mismatch" and raster_threshold_accepted(entry, pair_metrics):
        return "cjk_rtl_raster_threshold_accepted"
    if entry["expected_prompt10_classification"] in {
        "unsupported_policy_accepted",
        "fixture_gap_policy_reported",
    }:
        return entry["expected_prompt10_classification"]
    return raw


def wellfriendpdf_reference_match_count(pair_metrics: dict[str, Any]) -> int:
    return sum(
        1
        for pair in ["wellfriendpdf_vs_poppler", "wellfriendpdf_vs_pdfium", "wellfriendpdf_vs_mupdf"]
        if pair_metrics[pair].get("threshold_pass")
    )


def raster_threshold_accepted(entry: dict[str, Any], pair_metrics: dict[str, Any]) -> bool:
    if not (entry["category"].startswith("cjk/") or entry["category"].startswith("rtl/")):
        return False
    if "text_clipping" in entry["category"]:
        return False
    wellfriendpdf_pairs = [pair_metrics[pair] for pair in ["wellfriendpdf_vs_poppler", "wellfriendpdf_vs_pdfium", "wellfriendpdf_vs_mupdf"]]
    if any(pair.get("status") != "computed" for pair in wellfriendpdf_pairs):
        return False
    max_mean = max(float(pair.get("mean_abs_error", 999.0)) for pair in wellfriendpdf_pairs)
    max_changed8 = max(float(pair.get("changed_pixel_threshold8_percentage", 1.0)) for pair in wellfriendpdf_pairs)
    return max_mean <= 6.0 and max_changed8 <= 0.07


def count_outliers(pages: list[dict[str, Any]]) -> int:
    return sum(
        1
        for page in pages
        if page["prompt10_classification"]
        in {"all_references_agree_wellfriendpdf_mismatch", "wellfriendpdf_render_failure", "dimension_mismatch"}
    )


def count_unclassified(pages: list[dict[str, Any]]) -> int:
    return sum(
        1
        for page in pages
        if page["prompt10_classification"] in {"needs_manual_review", "reference_tool_failure"}
    )


def render_html(pages: list[dict[str, Any]], summary: dict[str, Any]) -> None:
    rows = []
    for page in pages:
        pairs = page["pair_metrics"]
        rows.append(
            "<tr>"
            f"<td>{html.escape(page['id'])}</td>"
            f"<td>{html.escape(page['category'])}</td>"
            f"<td>{html.escape(page['prompt10_classification'])}</td>"
            f"<td>{html.escape(page['raw_prompt06b_classification'])}</td>"
            f"<td>{html.escape(page['renders']['wellfriendpdf']['status'])}</td>"
            f"<td>{html.escape(page['renders']['poppler']['status'])}</td>"
            f"<td>{html.escape(page['renders']['pdfium']['status'])}</td>"
            f"<td>{html.escape(page['renders']['mupdf']['status'])}</td>"
            f"<td>{pairs['wellfriendpdf_vs_poppler'].get('changed_pixel_threshold8_percentage', '')}</td>"
            f"<td>{pairs['wellfriendpdf_vs_pdfium'].get('changed_pixel_threshold8_percentage', '')}</td>"
            f"<td>{pairs['wellfriendpdf_vs_mupdf'].get('changed_pixel_threshold8_percentage', '')}</td>"
            "</tr>"
        )
    policy_rows = [
        "<tr>"
        f"<td>{html.escape(row['id'])}</td>"
        f"<td>{html.escape(row['category'])}</td>"
        f"<td>{html.escape(row['prompt10_classification'])}</td>"
        f"<td colspan='8'>{html.escape(row['reason'])}</td>"
        "</tr>"
        for row in summary["policy_rows"]
    ]
    HTML_REPORT.parent.mkdir(parents=True, exist_ok=True)
    HTML_REPORT.write_text(
        "<!doctype html><meta charset='utf-8'>"
        "<title>Prompt 10 CJK/RTL/Color Glyph Reference Harness</title>"
        "<style>body{font-family:system-ui,sans-serif;margin:32px;color:#1f2933}"
        "table{border-collapse:collapse;font-size:13px}td,th{border:1px solid #cbd5e1;padding:4px 8px}"
        "th{background:#f1f5f9;text-align:left}</style>"
        "<h1>Prompt 10 CJK/RTL/Color Glyph Reference Harness</h1>"
        f"<p>Rendered pages: {summary['page_count']}. Policy rows: {summary['policy_row_count']}. "
        f"Wellfriend outliers: {summary['wellfriendpdf_outlier_failures']}. "
        f"Unclassified: {summary['unclassified_failures']}.</p>"
        "<h2>Classification Counts</h2><pre>"
        f"{html.escape(json.dumps(summary['classification_counts'], indent=2, sort_keys=True))}</pre>"
        "<h2>Pages</h2><table><tr><th>Fixture</th><th>Category</th><th>Prompt 10</th>"
        "<th>Raw</th><th>Wellfriend</th><th>Poppler</th><th>PDFium</th><th>MuPDF</th>"
        "<th>Ox/Pop changed8</th><th>Ox/PDFium changed8</th><th>Ox/MuPDF changed8</th></tr>"
        + "\n".join(rows + policy_rows)
        + "</table>",
        encoding="utf-8",
    )


def run_feature_report(timeout: int) -> dict[str, Any]:
    cmd = [
        "cargo",
        "run",
        "-p",
        "wellfriendpdf-cli",
        "--quiet",
        "--",
        "feature-report",
        "--pretty",
        "--output",
        str(PUBLIC_FEATURE_REPORT),
    ]
    result = run_command(cmd, timeout=timeout)
    has_prompt10 = False
    if PUBLIC_FEATURE_REPORT.exists():
        try:
            payload = json.loads(PUBLIC_FEATURE_REPORT.read_text(encoding="utf-8"))
            has_prompt10 = "prompt10_cjk_rtl_color_glyph_reference_harness" in payload.get("report", {})
        except json.JSONDecodeError:
            has_prompt10 = False
    return {
        "status": "passed" if result["exit_status"] == 0 and has_prompt10 else "failed",
        "has_prompt10_section": has_prompt10,
        "artifact": rel(PUBLIC_FEATURE_REPORT) if PUBLIC_FEATURE_REPORT.exists() else None,
        "command": result,
    }


def write_binding_parity(feature_report: dict[str, Any] | None) -> None:
    write_json(
        BINDING_PARITY,
        {
            "schema_version": 1,
            "kind": "prompt10_binding_report_parity",
            "shared_report_surface": "wellfriendpdf_engine::sdk::feature_report_json",
            "feature_report": feature_report or {"status": "skipped"},
            "bindings": {
                "rust_sdk": "shared facade",
                "cli": "wellfriendpdf feature-report",
                "python": "wellfriendpdf.feature_report()",
                "c_abi": "wellfriendpdf_feature_report_json",
                "wasm": "feature_report_json",
                "dotnet": "WellfriendDocument.FeatureReportJson",
                "java_maven": "WellfriendPdf.featureReportJson",
                "java_gradle": "WellfriendPdf.featureReportJson",
            },
        },
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--wellfriendpdf-bin")
    parser.add_argument("--dpi", type=int, default=72)
    parser.add_argument("--timeout", type=int, default=120)
    parser.add_argument("--limit", type=int, default=0, help="Limit rendered available corpus pages; 0 renders all.")
    parser.add_argument("--skip-render", action="store_true")
    parser.add_argument("--skip-feature-report", action="store_true")
    parser.add_argument("--allow-missing-reference-tools", action="store_true")
    args = parser.parse_args()

    OUT_DIR.mkdir(parents=True, exist_ok=True)
    entries = corpus_entries()
    write_corpus_manifest(entries)

    render_payload: dict[str, Any] | None = None
    if not args.skip_render:
        manifest = bootstrap_reference_manifest(args.dpi, args.timeout, args.allow_missing_reference_tools)
        render_payload = render_compare(entries, manifest, args.wellfriendpdf_bin, args.dpi, args.timeout, args.limit)

    feature_payload = None if args.skip_feature_report else run_feature_report(args.timeout)
    write_binding_parity(feature_payload)
    write_capability_matrix(entries, render_payload["summary"] if render_payload else None)

    summary = render_payload["summary"] if render_payload else {
        "wellfriendpdf_outlier_failures": None,
        "unclassified_failures": None,
        "classification_counts": {},
    }
    status = "passed"
    if render_payload and (summary["wellfriendpdf_outlier_failures"] or summary["unclassified_failures"]):
        status = "failed"
    if feature_payload and feature_payload["status"] != "passed":
        status = "failed"

    print(
        json.dumps(
            {
                "status": status,
                "available_pages": sum(1 for entry in entries if entry["available"]),
                "policy_rows": sum(1 for entry in entries if not entry["available"]),
                "wellfriendpdf_outlier_failures": summary["wellfriendpdf_outlier_failures"],
                "unclassified_failures": summary["unclassified_failures"],
                "artifacts": {
                    "corpus": rel(CORPUS_MANIFEST),
                    "capability_matrix": rel(CAPABILITY_MATRIX),
                    "render_results": rel(RENDER_RESULTS) if RENDER_RESULTS.exists() else None,
                    "diff_metrics": rel(DIFF_METRICS) if DIFF_METRICS.exists() else None,
                    "summary": rel(DISAGREEMENT_SUMMARY) if DISAGREEMENT_SUMMARY.exists() else None,
                    "html": rel(HTML_REPORT) if HTML_REPORT.exists() else None,
                    "feature_report": rel(PUBLIC_FEATURE_REPORT) if PUBLIC_FEATURE_REPORT.exists() else None,
                    "binding_parity": rel(BINDING_PARITY),
                },
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0 if status == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
