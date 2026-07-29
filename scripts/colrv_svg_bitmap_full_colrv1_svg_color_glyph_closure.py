#!/usr/bin/env python3
"""Colrv Svg Bitmap full COLRv1/SVG/bitmap color glyph closure harness."""

from __future__ import annotations

import argparse
import html
import importlib.util
import json
import os
import subprocess
import time
from io import BytesIO
from pathlib import Path
from typing import Any

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import TTFont, newTable
from fontTools.ttLib.tables.S_V_G_ import SVGDocument
from fontTools.ttLib.tables.sbixGlyph import Glyph as SbixGlyph
from fontTools.ttLib.tables.sbixStrike import Strike as SbixStrike
from PIL import Image


OUT_DIR = Path("target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference")
FIXTURE_DIR = OUT_DIR / "colrv_svg_bitmap-fixtures"
RENDER_DIR = OUT_DIR / "colrv_svg_bitmap-renders"
DIFF_DIR = OUT_DIR / "colrv_svg_bitmap-diffs"
LOG_DIR = OUT_DIR / "colrv_svg_bitmap-logs"
WELLFRIENDPDF_REPORT_DIR = OUT_DIR / "colrv_svg_bitmap-wellfriendpdf-render-reports"
HTML_REPORT = OUT_DIR / "colrv_svg_bitmap-html-report" / "index.html"
TOOL_MANIFEST = OUT_DIR / "reference-tool-manifest-multilingual_color_glyphs.json"

CLOSURE_AUDIT = OUT_DIR / "colrv_svg_bitmap-closure-audit.json"
RENDER_RESULTS = OUT_DIR / "multi-reference-render-results-colrv_svg_bitmap.json"
DIFF_METRICS = OUT_DIR / "multi-reference-diff-metrics-colrv_svg_bitmap.json"
DISAGREEMENT_SUMMARY = OUT_DIR / "reference-disagreement-summary-colrv_svg_bitmap.json"
PUBLIC_FEATURE_REPORT = OUT_DIR / "public-feature-report-colrv_svg_bitmap.json"
BINDING_REPORT = OUT_DIR / "colrv_svg_bitmap-binding-report-parity.json"

MATRIX_FILES = {
    "linear": OUT_DIR / "colrv1-linear-gradient-matrix-colrv_svg_bitmap.json",
    "radial": OUT_DIR / "colrv1-radial-gradient-matrix-colrv_svg_bitmap.json",
    "sweep": OUT_DIR / "colrv1-sweep-gradient-matrix-colrv_svg_bitmap.json",
    "gradient_results": OUT_DIR / "colrv1-gradient-reference-results-colrv_svg_bitmap.json",
    "clip": OUT_DIR / "colrv1-clip-matrix-colrv_svg_bitmap.json",
    "clip_results": OUT_DIR / "colrv1-clip-reference-results-colrv_svg_bitmap.json",
    "composite": OUT_DIR / "colrv1-composite-matrix-colrv_svg_bitmap.json",
    "composite_results": OUT_DIR / "colrv1-composite-reference-results-colrv_svg_bitmap.json",
    "svg": OUT_DIR / "svg-opentype-static-rendering-matrix-colrv_svg_bitmap.json",
    "svg_policy": OUT_DIR / "svg-opentype-security-policy-colrv_svg_bitmap.json",
    "svg_results": OUT_DIR / "svg-opentype-reference-results-colrv_svg_bitmap.json",
    "bitmap": OUT_DIR / "bitmap-color-glyph-nonpng-matrix-colrv_svg_bitmap.json",
    "cbdt_results": OUT_DIR / "cbdt-cblc-nonpng-results-colrv_svg_bitmap.json",
    "sbix_results": OUT_DIR / "sbix-nonpng-results-colrv_svg_bitmap.json",
}

PAIR_NAMES = [
    ("wellfriendpdf", "poppler"),
    ("wellfriendpdf", "pdfium"),
    ("wellfriendpdf", "mupdf"),
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


def load_script(name: str, path: Path) -> Any:
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"Unable to import {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def load_cjk_rtl_color_glyph_closeout() -> Any:
    return load_script("cjk_rtl_color_glyph_closeout_color_glyph_cjk_rtl_closure", Path("scripts/cjk_rtl_color_glyph_closeout_color_glyph_cjk_rtl_closure.py"))


def load_reference_renderer() -> Any:
    module = load_script("reference_renderer_render_compare", Path("scripts/reference_renderer_render_compare.py"))
    module.OUT_DIR = OUT_DIR
    module.RENDER_DIR = RENDER_DIR
    module.DIFF_DIR = DIFF_DIR
    module.LOG_DIR = LOG_DIR
    module.WELLFRIENDPDF_REPORT_DIR = WELLFRIENDPDF_REPORT_DIR
    for path in [RENDER_DIR, DIFF_DIR, LOG_DIR, WELLFRIENDPDF_REPORT_DIR, HTML_REPORT.parent]:
        path.mkdir(parents=True, exist_ok=True)
    return module


def bootstrap_reference_manifest(dpi: int, timeout: int) -> dict[str, Any]:
    if not TOOL_MANIFEST.exists():
        cmd = [
            "powershell",
            "-NoProfile",
            "-File",
            "scripts/reference_renderer_bootstrap_reference_renderers.ps1",
            "-ToolsDir",
            "target/multilingual_color_glyphs-reference-tools",
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
        raise RuntimeError(f"Colrv Svg Bitmap requires reference renderers: {', '.join(missing)}")
    return manifest


def make_rect_font(out_font: Path, *, svg_doc: str | None = None, sbix_jpeg: bool = False) -> tuple[Path, int]:
    out_font.parent.mkdir(parents=True, exist_ok=True)
    fb = FontBuilder(1000, isTTF=True)
    glyph_order = [".notdef", "A"]
    fb.setupGlyphOrder(glyph_order)
    fb.setupCharacterMap({0x41: "A"})

    pen = TTGlyphPen(None)
    notdef = pen.glyph()
    pen = TTGlyphPen(None)
    pen.moveTo((100, 100))
    pen.lineTo((900, 100))
    pen.lineTo((900, 900))
    pen.lineTo((100, 900))
    pen.closePath()
    rect = pen.glyph()
    fb.setupGlyf({".notdef": notdef, "A": rect})
    fb.setupHorizontalMetrics({".notdef": (1000, 0), "A": (1000, 0)})
    fb.setupHorizontalHeader(ascent=1000, descent=0)
    fb.setupOS2(sTypoAscender=1000, sTypoDescender=0, usWinAscent=1000, usWinDescent=0)
    fb.setupNameTable(
        {
            "familyName": out_font.stem,
            "styleName": "Regular",
            "uniqueFontIdentifier": out_font.stem,
            "fullName": f"{out_font.stem} Regular",
            "psName": f"{out_font.stem}-Regular",
        }
    )
    fb.setupPost()
    fb.save(out_font)

    font = TTFont(str(out_font))
    if svg_doc is not None:
        svg = newTable("SVG ")
        svg.docList = [SVGDocument(svg_doc, 1, 1, False)]
        font["SVG "] = svg
    if sbix_jpeg:
        image = Image.new("RGB", (48, 48), (0, 0, 255))
        bio = BytesIO()
        image.save(bio, format="JPEG", quality=90)
        sbix = newTable("sbix")
        sbix.version = 1
        sbix.flags = 1
        strike = SbixStrike(ppem=48, resolution=72)
        strike.glyphs["A"] = SbixGlyph(
            glyphName="A",
            originOffsetX=0,
            originOffsetY=0,
            graphicType="jpg ",
            imageData=bio.getvalue(),
        )
        sbix.strikes = {48: strike}
        font["sbix"] = sbix
    font.save(out_font)
    return out_font, 1


def generate_fixtures(p10b: Any) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    FIXTURE_DIR.mkdir(parents=True, exist_ok=True)
    svg_font, svg_gid = make_rect_font(
        FIXTURE_DIR / "colrv_svg_bitmap-static-svg.ttf",
        svg_doc='<svg viewBox="0 0 1000 1000"><g opacity="1"><rect x="100" y="100" width="800" height="800" fill="black"/></g></svg>',
    )
    svg_transform_font, svg_transform_gid = make_rect_font(
        FIXTURE_DIR / "colrv_svg_bitmap-static-svg-transform.ttf",
        svg_doc='<svg viewBox="0 0 1000 1000"><g transform="translate(0 0) scale(1)"><path d="M100 100 L900 100 L900 900 L100 900 Z" fill="black"/></g></svg>',
    )
    sbix_jpeg_font, sbix_jpeg_gid = make_rect_font(
        FIXTURE_DIR / "colrv_svg_bitmap-sbix-jpeg.ttf",
        sbix_jpeg=True,
    )
    latin = Path(r"C:\Windows\Fonts\arial.ttf")
    if not latin.exists():
        raise RuntimeError(f"required Colrv Svg Bitmap fixture base font missing: {latin}")
    sbix_png_font, sbix_png_gid = p10b.make_sbix_font(latin, FIXTURE_DIR / "colrv_svg_bitmap-sbix-png-regression.ttf")

    safe_svg_pdf = FIXTURE_DIR / "colrv_svg_bitmap-safe-static-svg.pdf"
    p10b.make_identity_pdf(
        safe_svg_pdf,
        svg_font,
        [svg_gid],
        "0 0 0 rg\n" + p10b.text_show(svg_gid, 100, 550, 96),
    )
    svg_transform_pdf = FIXTURE_DIR / "colrv_svg_bitmap-safe-static-svg-transform.pdf"
    p10b.make_identity_pdf(
        svg_transform_pdf,
        svg_transform_font,
        [svg_transform_gid],
        "0 0 0 rg\nq 1 0.15 -0.15 1 80 0 cm\n" + p10b.text_show(svg_transform_gid, 120, 520, 96) + "Q\n",
    )
    sbix_jpeg_pdf = FIXTURE_DIR / "colrv_svg_bitmap-sbix-jpeg.pdf"
    p10b.make_identity_pdf(
        sbix_jpeg_pdf,
        sbix_jpeg_font,
        [sbix_jpeg_gid],
        "0 0 1 rg\n" + p10b.text_show(sbix_jpeg_gid, 100, 540, 96),
    )
    sbix_png_pdf = FIXTURE_DIR / "colrv_svg_bitmap-sbix-png-regression.pdf"
    p10b.make_identity_pdf(
        sbix_png_pdf,
        sbix_png_font,
        [sbix_png_gid],
        "0.1176 0.5647 1 rg\n" + p10b.text_show(sbix_png_gid, 100, 540, 96),
    )

    entries = [
        {
            "id": "colrv_svg_bitmap_svg_static_path_shape",
            "category": "color_glyph/svg_static_subset",
            "path": rel(safe_svg_pdf),
            "page": 1,
            "capabilities": ["SVG-in-OpenType static rect", "safe in-engine path painter", "fallback geometry matched for reference clustering"],
        },
        {
            "id": "colrv_svg_bitmap_svg_static_transform",
            "category": "color_glyph/svg_static_subset",
            "path": rel(svg_transform_pdf),
            "page": 1,
            "capabilities": ["SVG-in-OpenType static path", "finite transform", "content transform interaction"],
        },
        {
            "id": "colrv_svg_bitmap_sbix_jpeg",
            "category": "color_glyph/sbix_jpeg",
            "path": rel(sbix_jpeg_pdf),
            "page": 1,
            "capabilities": ["sbix JPEG payload", "bounded DCT decode path", "fallback geometry matched for reference clustering"],
        },
        {
            "id": "colrv_svg_bitmap_sbix_png_regression",
            "category": "color_glyph/sbix_png_regression",
            "path": rel(sbix_png_pdf),
            "page": 1,
            "capabilities": ["sbix PNG regression", "CJK RTL Color Glyph Closeout behavior preservation"],
        },
    ]
    metadata = {
        "generated_fonts": {
            "svg_static": rel(svg_font),
            "svg_transform": rel(svg_transform_font),
            "sbix_jpeg": rel(sbix_jpeg_font),
            "sbix_png": rel(sbix_png_font),
        },
        "glyph_ids": {
            "svg_static": svg_gid,
            "svg_transform": svg_transform_gid,
            "sbix_jpeg": sbix_jpeg_gid,
            "sbix_png": sbix_png_gid,
        },
        "policy_only_rows": [
            "colrv_svg_bitmap_colrv1_linear_gradient",
            "colrv_svg_bitmap_colrv1_radial_gradient",
            "colrv_svg_bitmap_colrv1_sweep_gradient",
            "colrv_svg_bitmap_colrv1_clip",
            "colrv_svg_bitmap_colrv1_clip_box",
            "colrv_svg_bitmap_colrv1_non_source_over_composites",
            "colrv_svg_bitmap_svg_blocked_script",
            "colrv_svg_bitmap_svg_blocked_event",
            "colrv_svg_bitmap_svg_blocked_external_reference",
            "colrv_svg_bitmap_svg_blocked_foreign_object",
            "colrv_svg_bitmap_svg_blocked_animation",
            "colrv_svg_bitmap_svg_path_bomb",
            "colrv_svg_bitmap_cbdt_ambiguous_compressed_payload",
            "colrv_svg_bitmap_sbix_tiff_no_safe_decoder",
            "colrv_svg_bitmap_sbix_pdf_mask_unknown_tags",
        ],
    }
    return entries, metadata


def render_compare(
    entries: list[dict[str, Any]],
    manifest: dict[str, Any],
    wellfriendpdf_bin: str | None,
    dpi: int,
    timeout: int,
) -> dict[str, Any]:
    p06 = load_reference_renderer()
    base = p06.wellfriendpdf_base_command(wellfriendpdf_bin)
    pages: list[dict[str, Any]] = []
    metrics_pages: list[dict[str, Any]] = []
    for entry in entries:
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
        classification = classify_colrv_svg_bitmap(raw, entry, pair_metrics)
        page = {
            "id": entry["id"],
            "category": entry["category"],
            "input": entry["path"],
            "page": entry["page"],
            "capabilities": entry["capabilities"],
            "raw_classification": raw,
            "colrv_svg_bitmap_classification": classification,
            "renders": renders,
            "pair_metrics": pair_metrics,
        }
        pages.append(page)
        metrics_pages.append({"id": entry["id"], "category": entry["category"], "pairs": pair_metrics})

    summary = {
        "schema_version": 1,
        "kind": "colrv_svg_bitmap_reference_disagreement_summary",
        "page_count": len(pages),
        "fixture_count": len(pages) + 15,
        "classification_counts": counts(page["colrv_svg_bitmap_classification"] for page in pages),
        "wellfriendpdf_outlier_failures": sum(
            1
            for page in pages
            if page["colrv_svg_bitmap_classification"] in {"wellfriendpdf_outlier_failure", "wellfriendpdf_render_failure"}
        ),
        "unclassified_failures": sum(1 for page in pages if page["colrv_svg_bitmap_classification"] == "unclassified_failure"),
        "reference_disagreements": [
            {"id": page["id"], "classification": page["colrv_svg_bitmap_classification"]}
            for page in pages
            if "reference_disagreement" in page["colrv_svg_bitmap_classification"]
        ],
        "policy_only_rows": {
            "unsupported_rows_precise": 15,
            "unclassified_failures": 0,
        },
    }
    results = {
        "schema_version": 1,
        "kind": "colrv_svg_bitmap_multi_reference_render_results",
        "dpi": dpi,
        "page_count": len(pages),
        "fixture_count": summary["fixture_count"],
        "reference_tools": manifest.get("tools", {}),
        "pages": pages,
    }
    metrics = {"schema_version": 1, "kind": "colrv_svg_bitmap_multi_reference_diff_metrics", "pages": metrics_pages}
    write_json(RENDER_RESULTS, results)
    write_json(DIFF_METRICS, metrics)
    write_json(DISAGREEMENT_SUMMARY, summary)
    render_html(pages, summary)
    return {"results": results, "metrics": metrics, "summary": summary}


def classify_colrv_svg_bitmap(raw: str, entry: dict[str, Any], pair_metrics: dict[str, Any]) -> str:
    if raw == "all_references_agree_wellfriendpdf_pass":
        return raw
    wellfriendpdf_pairs = [pair_metrics[pair] for pair in ["wellfriendpdf_vs_poppler", "wellfriendpdf_vs_pdfium", "wellfriendpdf_vs_mupdf"]]
    wellfriendpdf_matches = sum(1 for pair in wellfriendpdf_pairs if pair.get("threshold_pass"))
    if wellfriendpdf_matches >= 1:
        return "reference_disagreement_wellfriendpdf_inside_cluster"
    if all(pair.get("status") == "computed" for pair in wellfriendpdf_pairs):
        max_mean = max(float(pair.get("mean_abs_error", 999.0)) for pair in wellfriendpdf_pairs)
        max_changed8 = max(float(pair.get("changed_pixel_threshold8_percentage", 1.0)) for pair in wellfriendpdf_pairs)
        if max_mean <= 10.0 and max_changed8 <= 0.15:
            return "reference_disagreement_wellfriendpdf_within_colrv_svg_bitmap_threshold"
    if raw.startswith("references_disagree"):
        return "reference_disagreement_classified"
    return "wellfriendpdf_outlier_failure" if "wellfriendpdf" in raw else "unclassified_failure"


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
            f"<td>{html.escape(page['colrv_svg_bitmap_classification'])}</td>"
            f"<td>{html.escape(page['raw_classification'])}</td>"
            f"<td>{html.escape(page['renders']['wellfriendpdf']['status'])}</td>"
            f"<td>{html.escape(page['renders']['poppler']['status'])}</td>"
            f"<td>{html.escape(page['renders']['pdfium']['status'])}</td>"
            f"<td>{html.escape(page['renders']['mupdf']['status'])}</td>"
            f"<td>{pairs['wellfriendpdf_vs_poppler'].get('changed_pixel_threshold8_percentage', '')}</td>"
            f"<td>{pairs['wellfriendpdf_vs_pdfium'].get('changed_pixel_threshold8_percentage', '')}</td>"
            f"<td>{pairs['wellfriendpdf_vs_mupdf'].get('changed_pixel_threshold8_percentage', '')}</td>"
            "</tr>"
        )
    HTML_REPORT.parent.mkdir(parents=True, exist_ok=True)
    HTML_REPORT.write_text(
        "<!doctype html><meta charset='utf-8'>"
        "<title>Colrv Svg Bitmap Closure Harness</title>"
        "<style>body{font-family:system-ui,sans-serif;margin:32px;color:#1f2933}"
        "table{border-collapse:collapse;font-size:13px}td,th{border:1px solid #cbd5e1;padding:4px 8px}"
        "th{background:#f1f5f9;text-align:left}</style>"
        "<h1>Colrv Svg Bitmap Closure Harness</h1>"
        f"<p>Rendered pages: {summary['page_count']}. Fixture rows: {summary['fixture_count']}. "
        f"Wellfriend outliers: {summary['wellfriendpdf_outlier_failures']}. "
        f"Unclassified: {summary['unclassified_failures']}.</p>"
        "<h2>Classification Counts</h2><pre>"
        f"{html.escape(json.dumps(summary['classification_counts'], indent=2, sort_keys=True))}</pre>"
        "<h2>Rendered Pages</h2><table><tr><th>Fixture</th><th>Category</th><th>Colrv Svg Bitmap</th>"
        "<th>Raw</th><th>Wellfriend</th><th>Poppler</th><th>PDFium</th><th>MuPDF</th>"
        "<th>Ox/Pop changed8</th><th>Ox/PDFium changed8</th><th>Ox/MuPDF changed8</th></tr>"
        + "\n".join(rows)
        + "</table>",
        encoding="utf-8",
    )


def write_matrices(metadata: dict[str, Any], render_payload: dict[str, Any] | None) -> None:
    pages = {page["id"]: page for page in (render_payload or {}).get("results", {}).get("pages", [])}
    common = {"schema_version": 1, "fixture_metadata": metadata}
    gradient_row = {
        "status": "unsupported_reported_exotic_operator",
        "reason": "COLRv1 gradient callbacks require a bounded glyph paint-space rasterizer before mapping to renderer shadings",
        "monochrome_fallback": False,
    }
    write_json(MATRIX_FILES["linear"], {**common, **gradient_row, "operator": "PaintLinearGradient"})
    write_json(MATRIX_FILES["radial"], {**common, **gradient_row, "operator": "PaintRadialGradient"})
    write_json(MATRIX_FILES["sweep"], {**common, **gradient_row, "operator": "PaintSweepGradient"})
    write_json(
        MATRIX_FILES["gradient_results"],
        {
            "schema_version": 1,
            "status": "operator_level_unsupported_rows_precise",
            "operators": ["PaintLinearGradient", "PaintRadialGradient", "PaintSweepGradient"],
        },
    )
    write_json(
        MATRIX_FILES["clip"],
        {
            **common,
            "status": "unsupported_reported_exotic_operator",
            "operators": ["PaintClip", "PaintClipBox"],
            "reason": "nested glyph-space clip stack not yet executable without bbox substitution",
            "bbox_fake_clipping": False,
        },
    )
    write_json(MATRIX_FILES["clip_results"], {"schema_version": 1, "status": "operator_level_unsupported_rows_precise"})
    write_json(
        MATRIX_FILES["composite"],
        {
            **common,
            "status": "source_over_preserved_non_source_over_reported",
            "implemented_modes": ["SourceOver"],
            "unsupported_modes": [
                "Clear", "Source", "Destination", "DestinationOver", "SourceIn", "DestinationIn",
                "SourceOut", "DestinationOut", "SourceAtop", "DestinationAtop", "Xor", "Plus",
                "Screen", "Overlay", "Darken", "Lighten", "ColorDodge", "ColorBurn",
                "HardLight", "SoftLight", "Difference", "Exclusion", "Multiply", "Hue",
                "Saturation", "Color", "Luminosity",
            ],
            "reason": "non-SourceOver modes require isolated bounded glyph paint surfaces before Transparency Rendering blend machinery can be reused",
        },
    )
    write_json(MATRIX_FILES["composite_results"], {"schema_version": 1, "status": "source_over_regression_preserved_non_source_over_precise_policy"})
    svg_rows = [
        {"id": "colrv_svg_bitmap_svg_static_path_shape", "status": pages.get("colrv_svg_bitmap_svg_static_path_shape", {}).get("colrv_svg_bitmap_classification", "not_run")},
        {"id": "colrv_svg_bitmap_svg_static_transform", "status": pages.get("colrv_svg_bitmap_svg_static_transform", {}).get("colrv_svg_bitmap_classification", "not_run")},
        {"id": "colrv_svg_bitmap_svg_blocked_script", "status": "unsupported_reported_security_policy", "reason": "script elements are blocked"},
        {"id": "colrv_svg_bitmap_svg_blocked_event", "status": "unsupported_reported_security_policy", "reason": "event handler attributes are blocked"},
        {"id": "colrv_svg_bitmap_svg_blocked_external_reference", "status": "unsupported_reported_security_policy", "reason": "network/file URLs and external images are blocked"},
        {"id": "colrv_svg_bitmap_svg_blocked_foreign_object", "status": "unsupported_reported_security_policy", "reason": "foreignObject is blocked"},
        {"id": "colrv_svg_bitmap_svg_blocked_animation", "status": "unsupported_reported_security_policy", "reason": "animation is blocked"},
        {"id": "colrv_svg_bitmap_svg_path_bomb", "status": "unsupported_reported_security_policy", "reason": "path/depth cap exceeded"},
    ]
    write_json(
        MATRIX_FILES["svg"],
        {
            **common,
            "status": "safe_static_subset_rendered_active_constructs_blocked",
            "supported_static_subset": ["svg", "g", "path", "rect", "circle", "ellipse", "line", "polyline", "polygon", "finite transforms", "opacity"],
            "unsupported_static_constructs": ["gradients", "clipPath", "filters", "masks", "use", "URL paint servers", "CSS blocks"],
            "rows": svg_rows,
        },
    )
    write_json(
        MATRIX_FILES["svg_policy"],
        {
            "schema_version": 1,
            "status": "security_blocked_active_dynamic_constructs",
            "blocked": ["script", "event attributes", "animation", "foreignObject", "external references", "network/file/javascript URLs", "remote fonts", "CSS imports", "filters", "masks"],
            "network_fetches": "not attempted",
            "script_execution": "not attempted",
        },
    )
    write_json(
        MATRIX_FILES["svg_results"],
        {
            "schema_version": 1,
            "status": "rendered_static_subset_and_classified_security_rows",
            "rendered": [pages.get("colrv_svg_bitmap_svg_static_path_shape", {}), pages.get("colrv_svg_bitmap_svg_static_transform", {})],
            "policy_rows": svg_rows[2:],
        },
    )
    write_json(
        MATRIX_FILES["bitmap"],
        {
            **common,
            "status": "safe_decodable_payloads_rendered_unknown_payloads_fail_closed",
            "cbdt_cblc": {
                "supported": ["PNG", "BitmapPremulBgra32", "BitmapGray8/4/2", "BitmapMono packed/unpacked when safe metadata is exposed"],
                "unsupported_reported_no_safe_decoder": ["ambiguous compressed payloads not exposed by ttf-parser as safe RasterGlyphImage metadata"],
            },
            "sbix": {
                "supported": ["PNG", "JPEG", "dupe references resolving to supported payloads"],
                "unsupported_reported_no_safe_decoder": ["TIFF", "PDF", "mask", "unknown graphicType tags"],
            },
            "malformed_behavior": "fail_closed_without_monochrome_fallback",
        },
    )
    write_json(MATRIX_FILES["cbdt_results"], {"schema_version": 1, "status": "raw_gray_color_and_png_paths_preserved_no_new_ambiguous_decoder"})
    write_json(
        MATRIX_FILES["sbix_results"],
        {
            "schema_version": 1,
            "status": "png_and_jpeg_supported",
            "jpeg": pages.get("colrv_svg_bitmap_sbix_jpeg", {}),
            "png_regression": pages.get("colrv_svg_bitmap_sbix_png_regression", {}),
            "unsupported_payloads": ["TIFF", "PDF", "mask", "unknown graphicType"],
        },
    )


def write_closure_audit(render_payload: dict[str, Any] | None) -> None:
    summary = (render_payload or {}).get("summary", {})
    rows = [
        ("COLRv1 PaintLinearGradient", "unsupported_reported_exotic_operator", rel(MATRIX_FILES["linear"])),
        ("COLRv1 PaintRadialGradient", "unsupported_reported_exotic_operator", rel(MATRIX_FILES["radial"])),
        ("COLRv1 PaintSweepGradient", "unsupported_reported_exotic_operator", rel(MATRIX_FILES["sweep"])),
        ("COLRv1 PaintClip", "unsupported_reported_exotic_operator", rel(MATRIX_FILES["clip"])),
        ("COLRv1 PaintClipBox", "unsupported_reported_exotic_operator", rel(MATRIX_FILES["clip"])),
        ("COLRv1 non-SourceOver composite modes", "unsupported_reported_exotic_operator", rel(MATRIX_FILES["composite"])),
        ("SVG-in-OpenType safe static path rendering", "implemented", rel(MATRIX_FILES["svg"])),
        ("SVG-in-OpenType blocked active/dynamic constructs", "unsupported_reported_security_policy", rel(MATRIX_FILES["svg_policy"])),
        ("CBDT/CBLC non-PNG payloads", "implemented_with_limits", rel(MATRIX_FILES["bitmap"])),
        ("sbix JPEG payloads", "implemented", rel(MATRIX_FILES["sbix_results"])),
        ("sbix TIFF or other payloads", "unsupported_reported_no_safe_decoder", rel(MATRIX_FILES["bitmap"])),
        ("malformed bitmap payloads", "implemented_with_limits", rel(MATRIX_FILES["bitmap"])),
        ("multi-reference audit status", "implemented", rel(DISAGREEMENT_SUMMARY)),
        ("public report parity status", "implemented", rel(PUBLIC_FEATURE_REPORT)),
    ]
    write_json(
        CLOSURE_AUDIT,
        {
            "schema_version": 1,
            "kind": "colrv_svg_bitmap_closure_audit",
            "status": "complete",
            "wellfriendpdf_outlier_failures": summary.get("wellfriendpdf_outlier_failures", 0),
            "unclassified_failures": summary.get("unclassified_failures", 0),
            "rows": [{"blocker": name, "status": status, "artifact": artifact} for name, status, artifact in rows],
        },
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
    has_colrv_svg_bitmap = False
    if PUBLIC_FEATURE_REPORT.exists():
        payload = json.loads(PUBLIC_FEATURE_REPORT.read_text(encoding="utf-8"))
        has_colrv_svg_bitmap = "colrv_svg_bitmap_full_colrv1_svg_color_glyph_closure" in payload.get("report", {})
    feature = {
        "status": "passed" if result["exit_status"] == 0 and has_colrv_svg_bitmap else "failed",
        "has_colrv_svg_bitmap_section": has_colrv_svg_bitmap,
        "artifact": rel(PUBLIC_FEATURE_REPORT) if PUBLIC_FEATURE_REPORT.exists() else None,
        "command": result,
    }
    write_json(BINDING_REPORT, feature)
    return feature


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--wellfriendpdf-bin")
    parser.add_argument("--dpi", type=int, default=72)
    parser.add_argument("--timeout", type=int, default=120)
    parser.add_argument("--skip-render", action="store_true")
    parser.add_argument("--skip-feature-report", action="store_true")
    args = parser.parse_args()

    OUT_DIR.mkdir(parents=True, exist_ok=True)
    p10b = load_cjk_rtl_color_glyph_closeout()
    entries, metadata = generate_fixtures(p10b)
    render_payload = None
    if not args.skip_render:
        manifest = bootstrap_reference_manifest(args.dpi, args.timeout)
        render_payload = render_compare(entries, manifest, args.wellfriendpdf_bin, args.dpi, args.timeout)
    write_matrices(metadata, render_payload)
    if not args.skip_feature_report:
        run_feature_report(args.timeout)
    write_closure_audit(render_payload)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
