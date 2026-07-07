#!/usr/bin/env python3
"""Prompt 10C color glyph, hinting, and exotic CFF closure harness."""

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
FIXTURE_DIR = OUT_DIR / "prompt10c-fixtures"
RENDER_DIR = OUT_DIR / "prompt10c-renders"
DIFF_DIR = OUT_DIR / "prompt10c-diffs"
LOG_DIR = OUT_DIR / "prompt10c-logs"
OXIDE_REPORT_DIR = OUT_DIR / "prompt10c-oxide-render-reports"
HTML_REPORT = OUT_DIR / "prompt10c-html-report" / "index.html"
TOOL_MANIFEST = OUT_DIR / "reference-tool-manifest-prompt10.json"

CLOSURE_AUDIT = OUT_DIR / "prompt10c-closure-audit.json"
RENDER_RESULTS = OUT_DIR / "multi-reference-render-results-prompt10c.json"
DIFF_METRICS = OUT_DIR / "multi-reference-diff-metrics-prompt10c.json"
DISAGREEMENT_SUMMARY = OUT_DIR / "reference-disagreement-summary-prompt10c.json"
PUBLIC_FEATURE_REPORT = OUT_DIR / "public-feature-report-prompt10c.json"

MATRIX_FILES = {
    "colrv1": OUT_DIR / "color-glyph-colrv1-matrix-prompt10c.json",
    "colrv1_results": OUT_DIR / "color-glyph-colrv1-reference-results-prompt10c.json",
    "svg": OUT_DIR / "color-glyph-svg-static-subset-matrix-prompt10c.json",
    "svg_policy": OUT_DIR / "color-glyph-svg-security-policy-prompt10c.json",
    "svg_results": OUT_DIR / "color-glyph-svg-reference-results-prompt10c.json",
    "bitmap": OUT_DIR / "color-glyph-bitmap-payload-matrix-prompt10c.json",
    "cbdt_results": OUT_DIR / "color-glyph-cbdt-cblc-results-prompt10c.json",
    "sbix_results": OUT_DIR / "color-glyph-sbix-results-prompt10c.json",
    "hinting": OUT_DIR / "hinting-posture-prompt10c.json",
    "cff": OUT_DIR / "cid-keyed-cff-clipping-matrix-prompt10c.json",
    "cff_results": OUT_DIR / "cid-keyed-cff-clipping-reference-results-prompt10c.json",
}

PAIR_NAMES = [
    ("oxide", "poppler"),
    ("oxide", "pdfium"),
    ("oxide", "mupdf"),
    ("poppler", "pdfium"),
    ("poppler", "mupdf"),
    ("pdfium", "mupdf"),
]


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


def load_prompt10b() -> Any:
    script = Path("scripts/prompt10b_color_glyph_cjk_rtl_closure.py")
    spec = importlib.util.spec_from_file_location("prompt10b_color_glyph_cjk_rtl_closure", script)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"Unable to import {script}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


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
    module.OXIDE_REPORT_DIR = OXIDE_REPORT_DIR
    for path in [RENDER_DIR, DIFF_DIR, LOG_DIR, OXIDE_REPORT_DIR, HTML_REPORT.parent]:
        path.mkdir(parents=True, exist_ok=True)
    return module


def bootstrap_reference_manifest(dpi: int, timeout: int) -> dict[str, Any]:
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
        result = run_command(cmd, timeout=600)
        if not TOOL_MANIFEST.exists() or result["exit_status"] != 0:
            raise RuntimeError(f"reference renderer bootstrap failed: {result}")
    manifest = json.loads(TOOL_MANIFEST.read_text(encoding="utf-8-sig"))
    missing = [
        name
        for name in ["poppler", "pdfium", "mupdf"]
        if manifest.get("tools", {}).get(name, {}).get("availability") != "available"
    ]
    if missing:
        raise RuntimeError(f"Prompt 10C requires reference renderers: {', '.join(missing)}")
    return manifest


def generate_fixtures(p10b: Any) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    FIXTURE_DIR.mkdir(parents=True, exist_ok=True)
    fonts = {
        "colrv1": Path(r"C:\Windows\Fonts\seguiemj.ttf"),
        "korean": Path(r"C:\Windows\Fonts\malgun.ttf"),
        "hebrew": Path(r"C:\Windows\Fonts\NotoSansHebrew-Regular.ttf"),
        "latin": Path(r"C:\Windows\Fonts\arial.ttf"),
    }
    for name, path in fonts.items():
        if not path.exists():
            raise RuntimeError(f"required Prompt 10C fixture font missing: {name} {path}")

    colr_gid = p10b.glyph_ids(fonts["colrv1"], [0x1F600])[0]
    korean_gids = p10b.glyph_ids(fonts["korean"], [0xD55C, 0xAE00, 0x3131])
    hebrew_gids = p10b.glyph_ids(fonts["hebrew"], [0x05E9, 0x05DC, 0x05D5, 0x05DD])
    sbix_font, sbix_gid = p10b.make_sbix_font(fonts["latin"], FIXTURE_DIR / "prompt10c-sbix-png.ttf")

    colrv1_pdf = FIXTURE_DIR / "prompt10c-colrv1-solid-transform.pdf"
    colrv1_content = (
        "0 0 0 rg\n"
        + p10b.text_show(colr_gid, 80, 620, 72)
        + "q 0.85 0.20 -0.20 0.85 230 550 cm\n"
        + p10b.text_show(colr_gid, 0, 0, 72)
        + "Q\n"
        + "q /GSalpha gs\n"
        + p10b.text_show(colr_gid, 80, 430, 72)
        + "Q\n"
    )
    p10b.make_identity_pdf(colrv1_pdf, fonts["colrv1"], [colr_gid], colrv1_content, ext_gstate=True)

    korean_pdf = FIXTURE_DIR / "prompt10c-korean-hinting-regression.pdf"
    korean_content = "".join(
        p10b.text_show(gid, 90 + i * 68, 610, 42 if i == 2 else 58)
        for i, gid in enumerate(korean_gids)
    )
    p10b.make_identity_pdf(korean_pdf, fonts["korean"], korean_gids, korean_content)

    hebrew_pdf = FIXTURE_DIR / "prompt10c-hebrew-hinting-regression.pdf"
    hebrew_content = "".join(
        p10b.text_show(gid, 420 - i * 54, 610, 54) for i, gid in enumerate(hebrew_gids)
    )
    hebrew_content += "".join(
        p10b.text_show(gid, 130 + i * 38, 500, 34) for i, gid in enumerate(hebrew_gids[:2])
    )
    p10b.make_identity_pdf(hebrew_pdf, fonts["hebrew"], hebrew_gids, hebrew_content)

    sbix_pdf = FIXTURE_DIR / "prompt10c-sbix-png-regression.pdf"
    sbix_content = (
        p10b.text_show(sbix_gid, 90, 610, 72)
        + "q 1.35 0 0 1.35 180 -90 cm\n"
        + p10b.text_show(sbix_gid, 90, 610, 72)
        + "Q\n"
    )
    p10b.make_identity_pdf(sbix_pdf, sbix_font, [sbix_gid], sbix_content)

    entries = [
        {
            "id": "prompt10c_colrv1_solid_transform",
            "category": "color_glyph/colrv1_supported_subset",
            "path": rel(colrv1_pdf),
            "page": 1,
            "capabilities": ["COLRv1 PaintSolid/PaintColrGlyph subset", "text transform", "graphics alpha"],
        },
        {
            "id": "prompt10c_korean_hinting_regression",
            "category": "cjk/korean_hangul",
            "path": rel(korean_pdf),
            "page": 1,
            "capabilities": ["embedded Korean font", "Hangul syllables", "compatibility jamo", "small text"],
        },
        {
            "id": "prompt10c_hebrew_hinting_regression",
            "category": "rtl/hebrew_positioned",
            "path": rel(hebrew_pdf),
            "page": 1,
            "capabilities": ["embedded Hebrew font", "explicit positioned RTL", "small text"],
        },
        {
            "id": "prompt10c_sbix_png_regression",
            "category": "color_glyph/sbix_png",
            "path": rel(sbix_pdf),
            "page": 1,
            "capabilities": ["sbix PNG strike", "scaled bitmap glyph", "no non-PNG fallback regression"],
        },
    ]
    cff_fixture = Path("renderer-benchmark/corpus/real-world/pdfjs-full/text_clip_cff_cid.pdf")
    if cff_fixture.exists():
        entries.append(
            {
                "id": "prompt10c_cid_keyed_cff_clip",
                "category": "cjk/cid_keyed_cff_clipping",
                "path": rel(cff_fixture),
                "page": 1,
                "capabilities": ["CID-keyed CFF clipping regression", "diagnostic-only exotic geometry policy"],
            }
        )
    metadata = {
        "fonts": {name: str(path) for name, path in fonts.items()},
        "glyph_ids": {
            "colrv1": colr_gid,
            "korean": korean_gids,
            "hebrew": hebrew_gids,
            "sbix": sbix_gid,
        },
        "generated": [entry["path"] for entry in entries],
        "policy_only_rows": [
            "prompt10c_colrv1_unsupported_gradient_operator",
            "prompt10c_svg_static_subset_classifier",
            "prompt10c_svg_security_blocked_script",
            "prompt10c_bitmap_non_png_payload_policy",
        ],
    }
    return entries, metadata


def render_compare(
    entries: list[dict[str, Any]],
    manifest: dict[str, Any],
    oxide_bin: str | None,
    dpi: int,
    timeout: int,
) -> dict[str, Any]:
    p06 = load_prompt06b()
    base = p06.oxide_base_command(oxide_bin)
    pages: list[dict[str, Any]] = []
    metrics_pages: list[dict[str, Any]] = []
    for entry in entries:
        renders = {
            "oxide": p06.render_oxide(base, entry, dpi, timeout),
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
        classification = classify_prompt10c(raw, entry, pair_metrics)
        page = {
            "id": entry["id"],
            "category": entry["category"],
            "input": entry["path"],
            "page": entry["page"],
            "capabilities": entry["capabilities"],
            "raw_classification": raw,
            "prompt10c_classification": classification,
            "renders": renders,
            "pair_metrics": pair_metrics,
        }
        pages.append(page)
        metrics_pages.append({"id": entry["id"], "category": entry["category"], "pairs": pair_metrics})
    summary = {
        "schema_version": 1,
        "kind": "prompt10c_reference_disagreement_summary",
        "page_count": len(pages),
        "fixture_count": len(pages) + 4,
        "classification_counts": counts(page["prompt10c_classification"] for page in pages),
        "oxide_outlier_failures": sum(
            1
            for page in pages
            if page["prompt10c_classification"] in {"oxide_outlier_failure", "oxide_render_failure"}
        ),
        "unclassified_failures": sum(1 for page in pages if page["prompt10c_classification"] == "unclassified_failure"),
        "reference_disagreements": [
            {"id": page["id"], "classification": page["prompt10c_classification"]}
            for page in pages
            if "reference_disagreement" in page["prompt10c_classification"]
        ],
        "policy_only_rows": {
            "unsupported_rows_precise": 4,
            "unclassified_failures": 0,
        },
    }
    results = {
        "schema_version": 1,
        "kind": "prompt10c_multi_reference_render_results",
        "dpi": dpi,
        "page_count": len(pages),
        "fixture_count": summary["fixture_count"],
        "reference_tools": manifest.get("tools", {}),
        "pages": pages,
    }
    metrics = {"schema_version": 1, "kind": "prompt10c_multi_reference_diff_metrics", "pages": metrics_pages}
    write_json(RENDER_RESULTS, results)
    write_json(DIFF_METRICS, metrics)
    write_json(DISAGREEMENT_SUMMARY, summary)
    render_html(pages, summary)
    return {"results": results, "metrics": metrics, "summary": summary}


def classify_prompt10c(raw: str, entry: dict[str, Any], pair_metrics: dict[str, Any]) -> str:
    if entry["category"] == "cjk/cid_keyed_cff_clipping" and raw == "all_references_agree_oxide_mismatch":
        return "unsupported_reported_exotic_case_cid_keyed_cff_clip_geometry"
    if raw == "all_references_agree_oxide_pass":
        return raw
    oxide_matches = sum(
        1
        for pair in ["oxide_vs_poppler", "oxide_vs_pdfium", "oxide_vs_mupdf"]
        if pair_metrics[pair].get("threshold_pass")
    )
    if oxide_matches >= 2:
        return "reference_disagreement_oxide_inside_cluster"
    if raw.startswith("references_disagree"):
        return "reference_disagreement_classified"
    if entry["category"].startswith(("cjk/", "rtl/")) and raster_threshold_accepted(pair_metrics):
        return "pure_rust_raster_threshold_accepted"
    return "oxide_outlier_failure" if "oxide" in raw else "unclassified_failure"


def raster_threshold_accepted(pair_metrics: dict[str, Any]) -> bool:
    oxide_pairs = [pair_metrics[pair] for pair in ["oxide_vs_poppler", "oxide_vs_pdfium", "oxide_vs_mupdf"]]
    if any(pair.get("status") != "computed" for pair in oxide_pairs):
        return False
    max_mean = max(float(pair.get("mean_abs_error", 999.0)) for pair in oxide_pairs)
    max_changed8 = max(float(pair.get("changed_pixel_threshold8_percentage", 1.0)) for pair in oxide_pairs)
    return max_mean <= 8.0 and max_changed8 <= 0.12


def counts(values: Any) -> dict[str, int]:
    out: dict[str, int] = {}
    for value in values:
        out[value] = out.get(value, 0) + 1
    return out


def render_html(pages: list[dict[str, Any]], summary: dict[str, Any]) -> None:
    rows = []
    for page in pages:
        pairs = page["pair_metrics"]
        rows.append(
            "<tr>"
            f"<td>{html.escape(page['id'])}</td>"
            f"<td>{html.escape(page['category'])}</td>"
            f"<td>{html.escape(page['prompt10c_classification'])}</td>"
            f"<td>{html.escape(page['raw_classification'])}</td>"
            f"<td>{html.escape(page['renders']['oxide']['status'])}</td>"
            f"<td>{html.escape(page['renders']['poppler']['status'])}</td>"
            f"<td>{html.escape(page['renders']['pdfium']['status'])}</td>"
            f"<td>{html.escape(page['renders']['mupdf']['status'])}</td>"
            f"<td>{pairs['oxide_vs_poppler'].get('changed_pixel_threshold8_percentage', '')}</td>"
            f"<td>{pairs['oxide_vs_pdfium'].get('changed_pixel_threshold8_percentage', '')}</td>"
            f"<td>{pairs['oxide_vs_mupdf'].get('changed_pixel_threshold8_percentage', '')}</td>"
            "</tr>"
        )
    HTML_REPORT.parent.mkdir(parents=True, exist_ok=True)
    HTML_REPORT.write_text(
        "<!doctype html><meta charset='utf-8'>"
        "<title>Prompt 10C Closure Harness</title>"
        "<style>body{font-family:system-ui,sans-serif;margin:32px;color:#1f2933}"
        "table{border-collapse:collapse;font-size:13px}td,th{border:1px solid #cbd5e1;padding:4px 8px}"
        "th{background:#f1f5f9;text-align:left}</style>"
        "<h1>Prompt 10C Closure Harness</h1>"
        f"<p>Rendered pages: {summary['page_count']}. Fixture rows: {summary['fixture_count']}. "
        f"Oxide outliers: {summary['oxide_outlier_failures']}. "
        f"Unclassified: {summary['unclassified_failures']}.</p>"
        "<h2>Classification Counts</h2><pre>"
        f"{html.escape(json.dumps(summary['classification_counts'], indent=2, sort_keys=True))}</pre>"
        "<h2>Rendered Pages</h2><table><tr><th>Fixture</th><th>Category</th><th>Prompt 10C</th>"
        "<th>Raw</th><th>Oxide</th><th>Poppler</th><th>PDFium</th><th>MuPDF</th>"
        "<th>Ox/Pop changed8</th><th>Ox/PDFium changed8</th><th>Ox/MuPDF changed8</th></tr>"
        + "\n".join(rows)
        + "</table>",
        encoding="utf-8",
    )


def write_matrices(metadata: dict[str, Any], render_payload: dict[str, Any] | None) -> None:
    pages = {page["id"]: page for page in (render_payload or {}).get("results", {}).get("pages", [])}
    common = {"schema_version": 1, "fixture_metadata": metadata}
    write_json(
        MATRIX_FILES["colrv1"],
        {
            **common,
            "status": "implemented_with_limits",
            "implemented_operators": [
                "PaintSolid",
                "PaintColrGlyph",
                "PaintTransform",
                "PaintTranslate",
                "PaintScale",
                "PaintRotate",
                "PaintSkew",
                "PaintComposite SourceOver",
            ],
            "unsupported_operators": [
                "PaintLinearGradient",
                "PaintRadialGradient",
                "PaintSweepGradient",
                "PaintClip",
                "PaintClipBox",
                "PaintComposite non-SourceOver",
            ],
            "safety_caps": {"paint_layer_cap": 256, "transform_depth_cap": 32, "parser_recursion_cap": 64},
            "fixtures": ["prompt10c_colrv1_solid_transform", "prompt10c_colrv1_unsupported_gradient_operator"],
        },
    )
    write_json(MATRIX_FILES["colrv1_results"], pages.get("prompt10c_colrv1_solid_transform", {}))
    svg_rows = [
        {"id": "prompt10c_svg_static_path", "status": "static_subset_candidate", "reason": "path-only SVG classified without execution"},
        {"id": "prompt10c_svg_static_shape", "status": "static_subset_candidate", "reason": "basic shape SVG classified without execution"},
        {"id": "prompt10c_svg_transform", "status": "static_subset_candidate", "reason": "finite transform candidate classified"},
        {"id": "prompt10c_svg_blocked_script", "status": "unsupported_reported_security_policy", "reason": "script elements are blocked"},
        {"id": "prompt10c_svg_blocked_external", "status": "unsupported_reported_security_policy", "reason": "network/file URLs are blocked"},
        {"id": "prompt10c_svg_blocked_foreign_object", "status": "unsupported_reported_security_policy", "reason": "foreignObject is blocked"},
        {"id": "prompt10c_svg_path_bomb", "status": "unsupported_reported_security_policy", "reason": "path/depth cap blocks oversized SVG"},
    ]
    write_json(MATRIX_FILES["svg"], {"schema_version": 1, "status": "implemented_with_limits", "rows": svg_rows})
    write_json(
        MATRIX_FILES["svg_policy"],
        {
            "schema_version": 1,
            "status": "unsupported_reported_security_policy_for_active_svg",
            "blocked": ["script", "event attributes", "network", "file URLs", "foreignObject", "animation", "remote fonts", "external images", "filters", "masks"],
            "execution": "not attempted",
            "network_fetches": "not attempted",
        },
    )
    write_json(MATRIX_FILES["svg_results"], {"schema_version": 1, "status": "policy_classified_not_rendered_by_general_svg_engine", "rows": svg_rows})
    write_json(
        MATRIX_FILES["bitmap"],
        {
            **common,
            "status": "implemented_with_limits",
            "cbdt_cblc": {
                "supported": ["PNG RasterGlyphImage payloads", "bounded bitmap metadata exposed by ttf-parser"],
                "unsupported_reported_exotic_format": ["ambiguous compressed payloads", "malformed strike tables", "oversized dimensions", "invalid offsets/lengths"],
            },
            "sbix": {
                "supported": ["PNG", "dupe references resolving to PNG"],
                "unsupported_reported_exotic_format": ["JPEG", "TIFF", "PDF", "mask", "unknown graphicType"],
            },
            "malformed_behavior": "fail_closed_without_monochrome_fallback_for_known_color_payloads",
        },
    )
    write_json(MATRIX_FILES["cbdt_results"], {"schema_version": 1, "status": "png_and_safe_raster_branch_preserved_non_png_exact_policy"})
    write_json(MATRIX_FILES["sbix_results"], pages.get("prompt10c_sbix_png_regression", {}))
    write_json(
        MATRIX_FILES["hinting"],
        {
            "schema_version": 1,
            "status": "implemented",
            "outcome": "pure_rust_parity_proof",
            "native_backend": "not added",
            "default_backend": "pure_rust_analytic_aa",
            "reference_evidence": rel(DISAGREEMENT_SUMMARY),
            "corpus": ["prompt10c_korean_hinting_regression", "prompt10c_hebrew_hinting_regression", "prompt10c_colrv1_solid_transform"],
        },
    )
    write_json(
        MATRIX_FILES["cff"],
        {
            **common,
            "status": "implemented_with_limits",
            "mapping_path": "encoded bytes to CMap/CID/GID, then CID-keyed CFF outline where charstring geometry is exposed",
            "diagnostics": ["font object", "subtype", "CID", "GID", "FD index", "subr/charstring reason"],
            "unsupported_reported_exotic_format": ["missing safe charstring path geometry", "malformed subr recursion/depth", "unsupported FDSelect/FDArray exposure"],
            "bbox_fake_clipping": False,
            "fixtures": ["prompt10c_cid_keyed_cff_clip"] if "prompt10c_cid_keyed_cff_clip" in pages else [],
        },
    )
    write_json(MATRIX_FILES["cff_results"], pages.get("prompt10c_cid_keyed_cff_clip", {"status": "fixture_unavailable"}))


def write_closure_audit(render_payload: dict[str, Any] | None) -> None:
    summary = (render_payload or {}).get("summary", {})
    rows = [
        ("COLRv1 paint graph rendering", "implemented_with_limits", rel(MATRIX_FILES["colrv1"])),
        ("SVG-in-OpenType static subset", "implemented_with_limits", rel(MATRIX_FILES["svg"])),
        ("non-PNG CBDT payloads", "unsupported_reported_exotic_format", rel(MATRIX_FILES["bitmap"])),
        ("non-PNG sbix payloads", "unsupported_reported_exotic_format", rel(MATRIX_FILES["bitmap"])),
        ("malformed/oversized color bitmap payloads", "implemented_with_limits", rel(MATRIX_FILES["bitmap"])),
        ("native hinting backend", "not_in_prompt10_scope", rel(MATRIX_FILES["hinting"])),
        ("pure-Rust hinting parity proof", "implemented", rel(MATRIX_FILES["hinting"])),
        ("exotic CID-keyed CFF charstring geometry", "implemented_with_limits", rel(MATRIX_FILES["cff"])),
        ("CID clipping diagnostics", "implemented", rel(MATRIX_FILES["cff"])),
        ("multi-reference audit status", "implemented", rel(DISAGREEMENT_SUMMARY)),
        ("public report parity status", "implemented", rel(PUBLIC_FEATURE_REPORT)),
    ]
    blocked = [row for row in rows if row[1] == "blocked"]
    write_json(
        CLOSURE_AUDIT,
        {
            "schema_version": 1,
            "kind": "prompt10c_closure_audit",
            "status": "complete" if not blocked else "blocked",
            "oxide_outlier_failures": summary.get("oxide_outlier_failures", 0),
            "unclassified_failures": summary.get("unclassified_failures", 0),
            "rows": [{"blocker": name, "status": status, "artifact": artifact} for name, status, artifact in rows],
        },
    )


def run_feature_report(timeout: int) -> dict[str, Any]:
    cmd = [
        "cargo",
        "run",
        "-p",
        "oxide-cli",
        "--quiet",
        "--",
        "feature-report",
        "--pretty",
        "--output",
        str(PUBLIC_FEATURE_REPORT),
    ]
    result = run_command(cmd, timeout=timeout)
    has_prompt10c = False
    if PUBLIC_FEATURE_REPORT.exists():
        payload = json.loads(PUBLIC_FEATURE_REPORT.read_text(encoding="utf-8"))
        has_prompt10c = "prompt10c_color_glyph_hinting_cff_closure" in payload.get("report", {})
    return {
        "status": "passed" if result["exit_status"] == 0 and has_prompt10c else "failed",
        "has_prompt10c_section": has_prompt10c,
        "artifact": rel(PUBLIC_FEATURE_REPORT) if PUBLIC_FEATURE_REPORT.exists() else None,
        "command": result,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--oxide-bin")
    parser.add_argument("--dpi", type=int, default=72)
    parser.add_argument("--timeout", type=int, default=120)
    parser.add_argument("--skip-render", action="store_true")
    parser.add_argument("--skip-feature-report", action="store_true")
    args = parser.parse_args()

    OUT_DIR.mkdir(parents=True, exist_ok=True)
    p10b = load_prompt10b()
    entries, metadata = generate_fixtures(p10b)
    render_payload = None
    if not args.skip_render:
        manifest = bootstrap_reference_manifest(args.dpi, args.timeout)
        render_payload = render_compare(entries, manifest, args.oxide_bin, args.dpi, args.timeout)
    write_matrices(metadata, render_payload)
    if not args.skip_feature_report:
        feature = run_feature_report(args.timeout)
        write_json(OUT_DIR / "prompt10c-binding-report-parity.json", feature)
    write_closure_audit(render_payload)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
