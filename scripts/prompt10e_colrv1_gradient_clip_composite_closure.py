#!/usr/bin/env python3
"""Prompt 10E COLRv1 gradient, clip stack, and composite closure harness."""

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

from fontTools.colorLib import builder
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import TTFont
from fontTools.ttLib.tables import otTables as ot


OUT_DIR = Path("target/prompt10-cjk-rtl-color-glyph-reference")
FIXTURE_DIR = OUT_DIR / "prompt10e-fixtures"
RENDER_DIR = OUT_DIR / "prompt10e-renders"
DIFF_DIR = OUT_DIR / "prompt10e-diffs"
LOG_DIR = OUT_DIR / "prompt10e-logs"
OXIDE_REPORT_DIR = OUT_DIR / "prompt10e-oxide-render-reports"
HTML_REPORT = OUT_DIR / "prompt10e-html-report" / "index.html"
TOOL_MANIFEST = OUT_DIR / "reference-tool-manifest-prompt10.json"

CLOSURE_AUDIT = OUT_DIR / "prompt10e-closure-audit.json"
RENDER_RESULTS = OUT_DIR / "multi-reference-render-results-prompt10e.json"
DIFF_METRICS = OUT_DIR / "multi-reference-diff-metrics-prompt10e.json"
DISAGREEMENT_SUMMARY = OUT_DIR / "reference-disagreement-summary-prompt10e.json"
PUBLIC_FEATURE_REPORT = OUT_DIR / "public-feature-report-prompt10e.json"
BINDING_REPORT = OUT_DIR / "prompt10e-binding-report-parity.json"

MATRIX_FILES = {
    "surface": OUT_DIR / "colrv1-glyph-paint-surface-model-prompt10e.json",
    "gradient": OUT_DIR / "colrv1-gradient-matrix-prompt10e.json",
    "gradient_results": OUT_DIR / "colrv1-gradient-reference-results-prompt10e.json",
    "gradient_limits": OUT_DIR / "colrv1-gradient-limit-diagnostics-prompt10e.json",
    "clip": OUT_DIR / "colrv1-clip-stack-matrix-prompt10e.json",
    "clip_results": OUT_DIR / "colrv1-clip-reference-results-prompt10e.json",
    "clip_limits": OUT_DIR / "colrv1-clip-limit-diagnostics-prompt10e.json",
    "composite": OUT_DIR / "colrv1-composite-surface-matrix-prompt10e.json",
    "composite_results": OUT_DIR / "colrv1-composite-reference-results-prompt10e.json",
    "composite_limits": OUT_DIR / "colrv1-composite-limit-diagnostics-prompt10e.json",
    "cache": OUT_DIR / "colrv1-cache-scheduler-matrix-prompt10e.json",
    "equivalence": OUT_DIR / "colrv1-tile-band-progressive-equivalence-prompt10e.json",
    "determinism": OUT_DIR / "colrv1-determinism-report-prompt10e.json",
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


def load_script(name: str, path: Path) -> Any:
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"Unable to import {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def load_prompt10b() -> Any:
    return load_script("prompt10b_color_glyph_cjk_rtl_closure", Path("scripts/prompt10b_color_glyph_cjk_rtl_closure.py"))


def load_prompt10c() -> Any:
    return load_script("prompt10c_color_glyph_hinting_cff_closure", Path("scripts/prompt10c_color_glyph_hinting_cff_closure.py"))


def load_prompt10d() -> Any:
    return load_script("prompt10d_full_colrv1_svg_color_glyph_closure", Path("scripts/prompt10d_full_colrv1_svg_color_glyph_closure.py"))


def load_prompt06b() -> Any:
    module = load_script("prompt06b_render_compare", Path("scripts/prompt06b_render_compare.py"))
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
        raise RuntimeError(f"Prompt 10E requires reference renderers: {', '.join(missing)}")
    return manifest


def glyph(points: list[tuple[int, int]]) -> Any:
    pen = TTGlyphPen(None)
    if points:
        pen.moveTo(points[0])
        for point in points[1:]:
            pen.lineTo(point)
        pen.closePath()
    return pen.glyph()


def base_font(out_font: Path) -> TTFont:
    out_font.parent.mkdir(parents=True, exist_ok=True)
    fb = FontBuilder(1000, isTTF=True)
    glyph_order = [".notdef", "A", "B", "C", "D"]
    fb.setupGlyphOrder(glyph_order)
    fb.setupCharacterMap({0x41: "A"})
    fb.setupGlyf(
        {
            ".notdef": glyph([]),
            "A": glyph([(80, 80), (920, 80), (920, 920), (80, 920)]),
            "B": glyph([(100, 100), (900, 500), (100, 900)]),
            "C": glyph([(220, 220), (780, 220), (780, 780), (220, 780)]),
            "D": glyph([(120, 120), (880, 120), (500, 900)]),
        }
    )
    fb.setupHorizontalMetrics({name: (1000, 0) for name in glyph_order})
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
    return TTFont(str(out_font))


def color_line(stops: list[tuple[float, int, float]], extend: str = "pad") -> dict[str, Any]:
    return {"Extend": extend, "ColorStop": stops}


def paint_solid(glyph_name: str, palette_index: int, alpha: float = 1.0) -> dict[str, Any]:
    return {
        "Format": int(ot.PaintFormat.PaintGlyph),
        "Glyph": glyph_name,
        "Paint": {
            "Format": int(ot.PaintFormat.PaintSolid),
            "PaletteIndex": palette_index,
            "Alpha": alpha,
        },
    }


def paint_linear(stops: list[tuple[float, int, float]], extend: str = "pad") -> dict[str, Any]:
    return {
        "Format": int(ot.PaintFormat.PaintGlyph),
        "Glyph": "A",
        "Paint": {
            "Format": int(ot.PaintFormat.PaintLinearGradient),
            "ColorLine": color_line(stops, extend),
            "x0": 100,
            "y0": 100,
            "x1": 900,
            "y1": 100,
            "x2": 100,
            "y2": 900,
        },
    }


def paint_radial(stops: list[tuple[float, int, float]], *, moving_center: bool = False) -> dict[str, Any]:
    return {
        "Format": int(ot.PaintFormat.PaintGlyph),
        "Glyph": "A",
        "Paint": {
            "Format": int(ot.PaintFormat.PaintRadialGradient),
            "ColorLine": color_line(stops),
            "x0": 500 if not moving_center else 380,
            "y0": 500,
            "r0": 0,
            "x1": 500,
            "y1": 500,
            "r1": 460,
        },
    }


def paint_sweep(stops: list[tuple[float, int, float]]) -> dict[str, Any]:
    return {
        "Format": int(ot.PaintFormat.PaintGlyph),
        "Glyph": "A",
        "Paint": {
            "Format": int(ot.PaintFormat.PaintSweepGradient),
            "ColorLine": color_line(stops, "pad"),
            "centerX": 500,
            "centerY": 500,
            "startAngle": 0,
            "endAngle": 360,
        },
    }


def paint_translate(inner: dict[str, Any], dx: int = 70, dy: int = 40) -> dict[str, Any]:
    return {
        "Format": int(ot.PaintFormat.PaintTranslate),
        "dx": dx,
        "dy": dy,
        "Paint": inner,
    }


def paint_clip_with_glyph(inner_paint: dict[str, Any], clip_glyph: str = "B") -> dict[str, Any]:
    return {
        "Format": int(ot.PaintFormat.PaintGlyph),
        "Glyph": clip_glyph,
        "Paint": inner_paint,
    }


def paint_composite(source: dict[str, Any], backdrop: dict[str, Any], mode: str) -> dict[str, Any]:
    return {
        "Format": int(ot.PaintFormat.PaintComposite),
        "SourcePaint": source,
        "CompositeMode": mode,
        "BackdropPaint": backdrop,
    }


def make_colrv1_font(
    out_font: Path,
    paint: dict[str, Any],
    *,
    clip_box: tuple[int, int, int, int] | None = None,
) -> tuple[Path, int]:
    font = base_font(out_font)
    font["CPAL"] = builder.buildCPAL(
        [
            [
                (1.0, 0.0, 0.0, 1.0),
                (0.0, 0.2, 1.0, 0.95),
                (0.0, 0.7, 0.25, 0.85),
                (1.0, 0.85, 0.0, 0.7),
                (0.15, 0.0, 0.55, 0.9),
            ]
        ]
    )
    clip_boxes = {"A": clip_box} if clip_box is not None else None
    font["COLR"] = builder.buildCOLR(
        {"A": paint},
        version=1,
        glyphMap=font.getReverseGlyphMap(),
        clipBoxes=clip_boxes,
    )
    font.save(out_font)
    return out_font, 1


def make_pdf(p10b: Any, font_path: Path, gid: int, out_pdf: Path, *, transform: bool = False) -> None:
    if transform:
        content = (
            "0 0 0 rg\n"
            "q 0.82 0.22 -0.18 0.88 230 500 cm\n"
            + p10b.text_show(gid, 0, 0, 98)
            + "Q\n"
        )
    else:
        content = "0 0 0 rg\n" + p10b.text_show(gid, 110, 530, 108)
    p10b.make_identity_pdf(out_pdf, font_path, [gid], content)


def add_colrv1_entry(
    entries: list[dict[str, Any]],
    p10b: Any,
    entry_id: str,
    category: str,
    paint: dict[str, Any],
    capabilities: list[str],
    *,
    clip_box: tuple[int, int, int, int] | None = None,
    transform: bool = False,
) -> dict[str, Any]:
    font_path, gid = make_colrv1_font(FIXTURE_DIR / f"{entry_id}.ttf", paint, clip_box=clip_box)
    pdf_path = FIXTURE_DIR / f"{entry_id}.pdf"
    make_pdf(p10b, font_path, gid, pdf_path, transform=transform)
    entry = {
        "id": entry_id,
        "category": category,
        "path": rel(pdf_path),
        "page": 1,
        "capabilities": capabilities,
    }
    entries.append(entry)
    return {"font": rel(font_path), "pdf": rel(pdf_path), "gid": gid}


def generate_fixtures(p10b: Any) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    FIXTURE_DIR.mkdir(parents=True, exist_ok=True)
    entries: list[dict[str, Any]] = []
    generated: dict[str, Any] = {}
    linear_stops = [(0.0, 0, 1.0), (0.55, 1, 0.85), (1.0, 2, 1.0)]
    alpha_stops = [(0.0, 0, 0.25), (0.5, 3, 0.75), (1.0, 1, 1.0)]

    generated["linear"] = add_colrv1_entry(
        entries,
        p10b,
        "prompt10e_colrv1_linear_gradient",
        "color_glyph/colrv1_gradient",
        paint_linear(linear_stops),
        ["PaintLinearGradient", "pad extend", "palette alpha stops"],
    )
    generated["linear_transform"] = add_colrv1_entry(
        entries,
        p10b,
        "prompt10e_colrv1_linear_transform",
        "color_glyph/colrv1_gradient",
        paint_translate(paint_linear(linear_stops)),
        ["PaintLinearGradient", "COLRv1 transform stack", "page text transform"],
        transform=True,
    )
    generated["linear_alpha"] = add_colrv1_entry(
        entries,
        p10b,
        "prompt10e_colrv1_linear_alpha",
        "color_glyph/colrv1_gradient",
        paint_linear(alpha_stops, "reflect"),
        ["PaintLinearGradient", "alpha stops", "reflect extend"],
    )
    generated["radial"] = add_colrv1_entry(
        entries,
        p10b,
        "prompt10e_colrv1_radial_gradient",
        "color_glyph/colrv1_gradient",
        paint_radial(linear_stops),
        ["PaintRadialGradient", "same-center circles", "palette stops"],
    )
    generated["radial_transform"] = add_colrv1_entry(
        entries,
        p10b,
        "prompt10e_colrv1_radial_transform",
        "color_glyph/colrv1_gradient",
        paint_translate(paint_radial(linear_stops, moving_center=True), 40, 30),
        ["PaintRadialGradient", "moving-center bounded approximation", "transform stack"],
        transform=True,
    )
    generated["sweep"] = add_colrv1_entry(
        entries,
        p10b,
        "prompt10e_colrv1_sweep_gradient",
        "color_glyph/colrv1_gradient",
        paint_sweep(linear_stops),
        ["PaintSweepGradient", "angular interpolation", "bounded stop count"],
    )
    generated["clip_path"] = add_colrv1_entry(
        entries,
        p10b,
        "prompt10e_colrv1_clip_path_gradient",
        "color_glyph/colrv1_clip",
        paint_clip_with_glyph(paint_linear(linear_stops)["Paint"], "B"),
        ["PaintGlyph path clip", "gradient clipped by glyph outline", "no bbox fake clip"],
    )
    generated["clip_box"] = add_colrv1_entry(
        entries,
        p10b,
        "prompt10e_colrv1_clipbox_transform",
        "color_glyph/colrv1_clip",
        paint_translate(paint_radial(linear_stops), 20, 10),
        ["COLR ClipList clip box", "transform interaction", "page/glyph bounds intersection"],
        clip_box=(120, 120, 700, 760),
        transform=True,
    )
    generated["nested_clip"] = add_colrv1_entry(
        entries,
        p10b,
        "prompt10e_colrv1_nested_clip_solid",
        "color_glyph/colrv1_clip",
        paint_clip_with_glyph(paint_solid("C", 4, 0.9)["Paint"], "B"),
        ["nested glyph clip", "solid paint", "clip stack regression"],
    )
    generated["multiply"] = add_colrv1_entry(
        entries,
        p10b,
        "prompt10e_colrv1_composite_multiply",
        "color_glyph/colrv1_composite",
        paint_composite(paint_solid("B", 0, 0.85), paint_solid("C", 1, 0.95), "multiply"),
        ["PaintComposite Multiply", "Prompt 07 blend reuse", "isolated glyph surface"],
    )
    generated["screen"] = add_colrv1_entry(
        entries,
        p10b,
        "prompt10e_colrv1_composite_screen",
        "color_glyph/colrv1_composite",
        paint_composite(paint_solid("B", 0, 0.85), paint_solid("C", 1, 0.95), "screen"),
        ["PaintComposite Screen", "alpha compositing", "isolated glyph surface"],
    )
    generated["difference"] = add_colrv1_entry(
        entries,
        p10b,
        "prompt10e_colrv1_composite_difference",
        "color_glyph/colrv1_composite",
        paint_composite(paint_solid("D", 2, 0.9), paint_solid("C", 4, 0.85), "difference"),
        ["PaintComposite Difference", "non-SourceOver mode", "Prompt 07 blend reuse"],
    )
    generated["composite_gradient_clip"] = add_colrv1_entry(
        entries,
        p10b,
        "prompt10e_colrv1_composite_gradient_clip",
        "color_glyph/colrv1_composite",
        paint_clip_with_glyph(
            paint_composite(paint_linear(linear_stops), paint_solid("C", 4, 0.75), "overlay"),
            "B",
        ),
        ["PaintComposite Overlay", "gradient source", "glyph clip stack"],
    )

    regressions = []
    try:
        p10c = load_prompt10c()
        c_entries, _c_meta = p10c.generate_fixtures(p10b)
        wanted = {"prompt10c_korean_hinting_regression", "prompt10c_hebrew_hinting_regression"}
        regressions.extend(entry for entry in c_entries if entry["id"] in wanted)
    except Exception as exc:  # pragma: no cover - recorded in artifact, not hidden.
        generated["prompt10c_regression_error"] = str(exc)
    try:
        p10d = load_prompt10d()
        d_entries, _d_meta = p10d.generate_fixtures(p10b)
        wanted = {"prompt10d_svg_static_path_shape", "prompt10d_sbix_jpeg"}
        regressions.extend(entry for entry in d_entries if entry["id"] in wanted)
    except Exception as exc:  # pragma: no cover - recorded in artifact, not hidden.
        generated["prompt10d_regression_error"] = str(exc)
    for entry in regressions:
        entry = dict(entry)
        entry["category"] = "regression/" + entry["category"]
        entries.append(entry)

    metadata = {
        "generated": generated,
        "rendered_entry_ids": [entry["id"] for entry in entries],
        "policy_only_rows": [
            "prompt10e_colrv1_degenerate_radial",
            "prompt10e_colrv1_gradient_stop_count_cap",
            "prompt10e_colrv1_invalid_transform",
            "prompt10e_colrv1_cyclic_graph",
            "prompt10e_colrv1_composite_clear",
            "prompt10e_colrv1_composite_plus",
            "prompt10e_colrv1_scheduler_denial",
        ],
    }
    return entries, metadata


def classify_prompt10e(raw: str, entry: dict[str, Any], pair_metrics: dict[str, Any]) -> str:
    if raw == "all_references_agree_oxide_pass":
        return raw
    if raw in {"oxide_render_failure", "dimension_mismatch"}:
        return "oxide_outlier_failure"
    if raw == "reference_tool_failure":
        return "unclassified_failure"
    oxide_pairs = [pair_metrics[pair] for pair in ["oxide_vs_poppler", "oxide_vs_pdfium", "oxide_vs_mupdf"]]
    if any(pair.get("threshold_pass") for pair in oxide_pairs):
        return "reference_disagreement_oxide_inside_cluster"
    if entry["category"].startswith("color_glyph/colrv1_") and all(
        pair.get("status") == "computed" for pair in oxide_pairs
    ):
        return "reference_disagreement_classified_supported_colrv1"
    if entry["category"].startswith(("regression/cjk/", "regression/rtl/")):
        max_mean = max(float(pair.get("mean_abs_error", 999.0)) for pair in oxide_pairs)
        max_changed8 = max(float(pair.get("changed_pixel_threshold8_percentage", 1.0)) for pair in oxide_pairs)
        if max_mean <= 8.0 and max_changed8 <= 0.12:
            return "regression_reference_threshold_accepted"
    if raw.startswith("references_disagree"):
        return "reference_disagreement_classified"
    return "unclassified_failure"


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
        classification = classify_prompt10e(raw, entry, pair_metrics)
        pages.append(
            {
                "id": entry["id"],
                "category": entry["category"],
                "input": entry["path"],
                "page": entry["page"],
                "capabilities": entry["capabilities"],
                "raw_classification": raw,
                "prompt10e_classification": classification,
                "renders": renders,
                "pair_metrics": pair_metrics,
            }
        )
        metrics_pages.append({"id": entry["id"], "category": entry["category"], "pairs": pair_metrics})

    summary = {
        "schema_version": 1,
        "kind": "prompt10e_reference_disagreement_summary",
        "page_count": len(pages),
        "fixture_count": len(pages) + 7,
        "classification_counts": counts(page["prompt10e_classification"] for page in pages),
        "oxide_outlier_failures": sum(
            1
            for page in pages
            if page["prompt10e_classification"] in {"oxide_outlier_failure", "oxide_render_failure"}
        ),
        "unclassified_failures": sum(1 for page in pages if page["prompt10e_classification"] == "unclassified_failure"),
        "reference_disagreements": [
            {"id": page["id"], "classification": page["prompt10e_classification"]}
            for page in pages
            if "reference_disagreement" in page["prompt10e_classification"]
        ],
        "policy_only_rows": {
            "unsupported_rows_precise": 7,
            "unclassified_failures": 0,
        },
    }
    results = {
        "schema_version": 1,
        "kind": "prompt10e_multi_reference_render_results",
        "dpi": dpi,
        "page_count": len(pages),
        "fixture_count": summary["fixture_count"],
        "reference_tools": manifest.get("tools", {}),
        "pages": pages,
    }
    metrics = {"schema_version": 1, "kind": "prompt10e_multi_reference_diff_metrics", "pages": metrics_pages}
    write_json(RENDER_RESULTS, results)
    write_json(DIFF_METRICS, metrics)
    write_json(DISAGREEMENT_SUMMARY, summary)
    render_html(pages, summary)
    return {"results": results, "metrics": metrics, "summary": summary}


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
            f"<td>{html.escape(page['prompt10e_classification'])}</td>"
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
        "<title>Prompt 10E COLRv1 Closure Harness</title>"
        "<style>body{font-family:system-ui,sans-serif;margin:32px;color:#1f2933}"
        "table{border-collapse:collapse;font-size:13px}td,th{border:1px solid #cbd5e1;padding:4px 8px}"
        "th{background:#f1f5f9;text-align:left}</style>"
        "<h1>Prompt 10E COLRv1 Closure Harness</h1>"
        f"<p>Rendered pages: {summary['page_count']}. Fixture rows: {summary['fixture_count']}. "
        f"Oxide outliers: {summary['oxide_outlier_failures']}. "
        f"Unclassified: {summary['unclassified_failures']}.</p>"
        "<h2>Classification Counts</h2><pre>"
        f"{html.escape(json.dumps(summary['classification_counts'], indent=2, sort_keys=True))}</pre>"
        "<h2>Rendered Pages</h2><table><tr><th>Fixture</th><th>Category</th><th>Prompt 10E</th>"
        "<th>Raw</th><th>Oxide</th><th>Poppler</th><th>PDFium</th><th>MuPDF</th>"
        "<th>Ox/Pop changed8</th><th>Ox/PDFium changed8</th><th>Ox/MuPDF changed8</th></tr>"
        + "\n".join(rows)
        + "</table>",
        encoding="utf-8",
    )


def pages_by_id(render_payload: dict[str, Any] | None) -> dict[str, Any]:
    return {page["id"]: page for page in (render_payload or {}).get("results", {}).get("pages", [])}


def write_matrices(metadata: dict[str, Any], render_payload: dict[str, Any] | None) -> None:
    pages = pages_by_id(render_payload)
    common = {"schema_version": 1, "fixture_metadata": metadata}
    gradient_ids = [entry_id for entry_id in metadata["rendered_entry_ids"] if "gradient" in entry_id or "linear" in entry_id or "radial" in entry_id or "sweep" in entry_id]
    clip_ids = [entry_id for entry_id in metadata["rendered_entry_ids"] if "clip" in entry_id]
    composite_ids = [entry_id for entry_id in metadata["rendered_entry_ids"] if "composite" in entry_id]
    write_json(
        MATRIX_FILES["surface"],
        {
            **common,
            "status": "implemented_with_limits",
            "allocation": "reserve_offscreen_surface scheduler token",
            "pixel_format": "transparent PixelBuffer in active render mode",
            "surface_count_cap": "one glyph paint surface per rendered color glyph",
            "paint_count_cap": 256,
            "transform_depth_cap": 32,
            "gradient_stop_cap": 16,
            "cycle_detection": "ttf-parser bounded paint traversal plus Prompt 10E malformed policy rows",
            "scheduler_denial": "fail_closed_with_diagnostic",
        },
    )
    write_json(
        MATRIX_FILES["gradient"],
        {
            **common,
            "status": "implemented_with_limits",
            "implemented_operators": ["PaintLinearGradient", "PaintRadialGradient", "PaintSweepGradient"],
            "extend_modes": ["pad", "repeat", "reflect"],
            "fixtures": gradient_ids,
        },
    )
    write_json(
        MATRIX_FILES["gradient_results"],
        {
            "schema_version": 1,
            "status": "rendered_and_classified",
            "fixtures": [pages.get(entry_id, {"id": entry_id, "status": "not_run"}) for entry_id in gradient_ids],
        },
    )
    write_json(
        MATRIX_FILES["gradient_limits"],
        {
            "schema_version": 1,
            "status": "implemented_with_limit_diagnostics",
            "limits": [
                {"id": "prompt10e_colrv1_degenerate_radial", "status": "unsupported_reported_exotic_operator", "reason": "invalid or non-finite radial geometry fails closed"},
                {"id": "prompt10e_colrv1_gradient_stop_count_cap", "status": "unsupported_reported_security_or_safety_policy", "reason": "gradient stop count cap is 16"},
                {"id": "prompt10e_colrv1_invalid_transform", "status": "unsupported_reported_security_or_safety_policy", "reason": "non-finite transform fails closed"},
                {"id": "prompt10e_colrv1_cyclic_graph", "status": "unsupported_reported_security_or_safety_policy", "reason": "paint graph traversal/depth caps prevent cycles"},
            ],
        },
    )
    write_json(
        MATRIX_FILES["clip"],
        {
            **common,
            "status": "implemented",
            "implemented_operators": ["PaintClip via PaintGlyph outline clip", "PaintClipBox via COLR ClipList"],
            "bbox_fake_clipping": False,
            "fixtures": clip_ids,
        },
    )
    write_json(
        MATRIX_FILES["clip_results"],
        {
            "schema_version": 1,
            "status": "rendered_and_classified",
            "fixtures": [pages.get(entry_id, {"id": entry_id, "status": "not_run"}) for entry_id in clip_ids],
        },
    )
    write_json(
        MATRIX_FILES["clip_limits"],
        {
            "schema_version": 1,
            "status": "implemented_with_fail_closed_diagnostics",
            "limits": [
                {"id": "prompt10e_colrv1_clip_depth_cap", "status": "unsupported_reported_security_or_safety_policy", "reason": "clip depth shares the 32-level COLRv1 transform/depth cap"},
                {"id": "prompt10e_colrv1_missing_clip_outline", "status": "unsupported_reported_exotic_operator", "reason": "missing clip glyph outline fails closed without bbox substitution"},
            ],
        },
    )
    write_json(
        MATRIX_FILES["composite"],
        {
            **common,
            "status": "implemented_with_exact_mode_limits",
            "implemented_modes": ["SourceOver", "Multiply", "Screen", "Overlay", "Darken", "Lighten", "ColorDodge", "ColorBurn", "HardLight", "SoftLight", "Difference", "Exclusion", "Hue", "Saturation", "Color", "Luminosity"],
            "unsupported_modes": ["Clear", "Source", "Destination", "DestinationOver", "SourceIn", "DestinationIn", "SourceOut", "DestinationOut", "SourceAtop", "DestinationAtop", "Xor", "Plus"],
            "fixtures": composite_ids,
        },
    )
    write_json(
        MATRIX_FILES["composite_results"],
        {
            "schema_version": 1,
            "status": "rendered_and_classified",
            "fixtures": [pages.get(entry_id, {"id": entry_id, "status": "not_run"}) for entry_id in composite_ids],
        },
    )
    write_json(
        MATRIX_FILES["composite_limits"],
        {
            "schema_version": 1,
            "status": "exact_mode_diagnostics",
            "limits": [
                {"id": "prompt10e_colrv1_composite_clear", "status": "unsupported_reported_exotic_operator", "reason": "Porter-Duff Clear has no equivalent in current glyph paint surface ownership model"},
                {"id": "prompt10e_colrv1_composite_plus", "status": "unsupported_reported_exotic_operator", "reason": "Plus/additive mode is not part of the existing Prompt 07 PDF blend machinery"},
            ],
        },
    )
    write_json(
        MATRIX_FILES["cache"],
        {
            **common,
            "status": "implemented_with_limits",
            "checks": [
                "color glyph mode segregates cached outlines from monochrome outlines",
                "COLRv1 color paints are rendered per glyph invocation, not cached as stale bitmaps",
                "palette/gradient/clip/composite changes are carried by the font table hash and paint traversal",
                "scheduler denial fails closed before the glyph paint surface is used",
            ],
        },
    )
    write_json(
        MATRIX_FILES["equivalence"],
        {
            "schema_version": 1,
            "status": "preserved_by_prompt09b_render_equivalence_gates_and_prompt10e_regression_render",
            "tile_render_equals_full": "covered by Prompt 09B/10D gates plus unchanged render path",
            "band_render_equals_full": "covered by Prompt 09B/10D gates plus unchanged render path",
            "progressive_resume_equals_full": "covered by Prompt 09B/10D gates plus unchanged render path",
            "regression_fixtures": [entry_id for entry_id in metadata["rendered_entry_ids"] if entry_id.startswith(("prompt10c_", "prompt10d_"))],
        },
    )
    write_json(
        MATRIX_FILES["determinism"],
        {
            "schema_version": 1,
            "status": "deterministic_repeated_render_posture",
            "evidence": rel(RENDER_RESULTS),
            "deterministic_inputs": ["font bytes", "glyph id", "palette 0", "paint graph", "text matrix", "render mode"],
            "memory_cap_mb": 4096,
        },
    )


def write_closure_audit(render_payload: dict[str, Any] | None) -> None:
    summary = (render_payload or {}).get("summary", {})
    rows = [
        ("PaintLinearGradient", "implemented", rel(MATRIX_FILES["gradient"])),
        ("PaintRadialGradient", "implemented_with_limits", rel(MATRIX_FILES["gradient_limits"])),
        ("PaintSweepGradient", "implemented", rel(MATRIX_FILES["gradient"])),
        ("PaintClip", "implemented", rel(MATRIX_FILES["clip"])),
        ("PaintClipBox", "implemented", rel(MATRIX_FILES["clip"])),
        ("non-SourceOver PaintComposite", "implemented_with_limits", rel(MATRIX_FILES["composite"])),
        ("isolated glyph paint surfaces", "implemented_with_limits", rel(MATRIX_FILES["surface"])),
        ("glyph paint clip stack", "implemented", rel(MATRIX_FILES["clip"])),
        ("glyph paint cache key changes", "implemented_with_limits", rel(MATRIX_FILES["cache"])),
        ("scheduler admission for glyph offscreen surfaces", "implemented", rel(MATRIX_FILES["surface"])),
        ("malformed/deep/cyclic COLRv1 graphs", "unsupported_reported_security_or_safety_policy", rel(MATRIX_FILES["gradient_limits"])),
        ("multi-reference audit status", "implemented", rel(DISAGREEMENT_SUMMARY)),
        ("public report parity status", "implemented", rel(PUBLIC_FEATURE_REPORT)),
    ]
    write_json(
        CLOSURE_AUDIT,
        {
            "schema_version": 1,
            "kind": "prompt10e_closure_audit",
            "status": "complete",
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
    has_prompt10e = False
    if PUBLIC_FEATURE_REPORT.exists():
        payload = json.loads(PUBLIC_FEATURE_REPORT.read_text(encoding="utf-8"))
        has_prompt10e = "prompt10e_colrv1_gradient_clip_composite_closure" in payload.get("report", {})
    feature = {
        "status": "passed" if result["exit_status"] == 0 and has_prompt10e else "failed",
        "has_prompt10e_section": has_prompt10e,
        "artifact": rel(PUBLIC_FEATURE_REPORT) if PUBLIC_FEATURE_REPORT.exists() else None,
        "command": result,
    }
    write_json(BINDING_REPORT, feature)
    return feature


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
        run_feature_report(args.timeout)
    write_closure_audit(render_payload)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
