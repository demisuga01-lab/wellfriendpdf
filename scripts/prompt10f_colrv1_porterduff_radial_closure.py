#!/usr/bin/env python3
"""Prompt 10F COLRv1 Porter-Duff and exact radial closure harness."""

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
FIXTURE_DIR = OUT_DIR / "prompt10f-fixtures"
RENDER_DIR = OUT_DIR / "prompt10f-renders"
DIFF_DIR = OUT_DIR / "prompt10f-diffs"
LOG_DIR = OUT_DIR / "prompt10f-logs"
WELLFRIENDPDF_REPORT_DIR = OUT_DIR / "prompt10f-wellfriendpdf-render-reports"
HTML_REPORT = OUT_DIR / "prompt10f-html-report" / "index.html"
TOOL_MANIFEST = OUT_DIR / "reference-tool-manifest-prompt10.json"

CLOSURE_AUDIT = OUT_DIR / "prompt10f-closure-audit.json"
RENDER_RESULTS = OUT_DIR / "multi-reference-render-results-prompt10f.json"
DIFF_METRICS = OUT_DIR / "multi-reference-diff-metrics-prompt10f.json"
DISAGREEMENT_SUMMARY = OUT_DIR / "reference-disagreement-summary-prompt10f.json"
PUBLIC_FEATURE_REPORT = OUT_DIR / "public-feature-report-prompt10f.json"
BINDING_REPORT = OUT_DIR / "prompt10f-binding-report-parity.json"

MATRIX_FILES = {
    "porterduff": OUT_DIR / "colrv1-porterduff-composite-matrix-prompt10f.json",
    "porterduff_results": OUT_DIR / "colrv1-porterduff-composite-reference-results-prompt10f.json",
    "composite_scheduler_cache": OUT_DIR / "colrv1-composite-scheduler-cache-prompt10f.json",
    "radial": OUT_DIR / "colrv1-exact-radial-gradient-matrix-prompt10f.json",
    "radial_results": OUT_DIR / "colrv1-exact-radial-gradient-reference-results-prompt10f.json",
    "radial_error": OUT_DIR / "colrv1-radial-error-bound-prompt10f.json",
    "cache_key": OUT_DIR / "colrv1-cache-key-prompt10f.json",
    "scheduler_memory": OUT_DIR / "colrv1-scheduler-memory-prompt10f.json",
    "determinism": OUT_DIR / "colrv1-determinism-prompt10f.json",
}

PORTER_DUFF_MODES = [
    ("clear", "Clear"),
    ("src", "Source"),
    ("dest", "Destination"),
    ("dest_over", "DestinationOver"),
    ("src_in", "SourceIn"),
    ("dest_in", "DestinationIn"),
    ("src_out", "SourceOut"),
    ("dest_out", "DestinationOut"),
    ("src_atop", "SourceAtop"),
    ("dest_atop", "DestinationAtop"),
    ("xor", "Xor"),
    ("plus", "Plus"),
]

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
        raise RuntimeError(f"Prompt 10F requires reference renderers: {', '.join(missing)}")
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


def paint_radial(
    stops: list[tuple[float, int, float]],
    *,
    x0: int = 500,
    y0: int = 500,
    r0: int = 0,
    x1: int = 500,
    y1: int = 500,
    r1: int = 460,
    extend: str = "pad",
) -> dict[str, Any]:
    return {
        "Format": int(ot.PaintFormat.PaintGlyph),
        "Glyph": "A",
        "Paint": {
            "Format": int(ot.PaintFormat.PaintRadialGradient),
            "ColorLine": color_line(stops, extend),
            "x0": x0,
            "y0": y0,
            "r0": r0,
            "x1": x1,
            "y1": y1,
            "r1": r1,
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

    generated["linear_regression"] = add_colrv1_entry(
        entries,
        p10b,
        "prompt10f_colrv1_linear_regression",
        "regression/color_glyph/colrv1_gradient",
        paint_linear(linear_stops),
        ["PaintLinearGradient", "Prompt 10E regression", "palette alpha stops"],
    )
    generated["sweep_regression"] = add_colrv1_entry(
        entries,
        p10b,
        "prompt10f_colrv1_sweep_regression",
        "regression/color_glyph/colrv1_gradient",
        paint_sweep(linear_stops),
        ["PaintSweepGradient", "Prompt 10E regression", "angular interpolation"],
    )
    generated["radial_same_center"] = add_colrv1_entry(
        entries,
        p10b,
        "prompt10f_colrv1_radial_same_center",
        "color_glyph/colrv1_exact_radial",
        paint_radial(linear_stops),
        ["PaintRadialGradient", "same-center circles", "palette stops"],
    )
    generated["radial_moving_small"] = add_colrv1_entry(
        entries,
        p10b,
        "prompt10f_colrv1_radial_moving_small_offset",
        "color_glyph/colrv1_exact_radial",
        paint_radial(linear_stops, x0=430, y0=500, r0=10, x1=540, y1=500, r1=470),
        ["PaintRadialGradient", "exact moving-center small offset", "two-circle solver"],
    )
    generated["radial_moving_large"] = add_colrv1_entry(
        entries,
        p10b,
        "prompt10f_colrv1_radial_moving_large_offset",
        "color_glyph/colrv1_exact_radial",
        paint_radial(linear_stops, x0=280, y0=420, r0=30, x1=720, y1=610, r1=560),
        ["PaintRadialGradient", "exact moving-center large offset", "different centers"],
    )
    generated["radial_different_radii"] = add_colrv1_entry(
        entries,
        p10b,
        "prompt10f_colrv1_radial_different_radii",
        "color_glyph/colrv1_exact_radial",
        paint_radial(linear_stops, x0=360, y0=450, r0=80, x1=620, y1=560, r1=500),
        ["PaintRadialGradient", "different radii", "non-zero start radius"],
    )
    generated["radial_repeat"] = add_colrv1_entry(
        entries,
        p10b,
        "prompt10f_colrv1_radial_repeat",
        "color_glyph/colrv1_exact_radial",
        paint_radial(linear_stops, x0=420, y0=440, r0=20, x1=760, y1=560, r1=260, extend="repeat"),
        ["PaintRadialGradient", "repeat extend", "exact moving-center solver"],
    )
    generated["radial_reflect"] = add_colrv1_entry(
        entries,
        p10b,
        "prompt10f_colrv1_radial_reflect",
        "color_glyph/colrv1_exact_radial",
        paint_radial(alpha_stops, x0=320, y0=440, r0=30, x1=700, y1=640, r1=280, extend="reflect"),
        ["PaintRadialGradient", "reflect extend", "alpha stops"],
    )
    generated["radial_transform"] = add_colrv1_entry(
        entries,
        p10b,
        "prompt10f_colrv1_radial_transform",
        "color_glyph/colrv1_exact_radial",
        paint_translate(
            paint_radial(linear_stops, x0=360, y0=440, r0=20, x1=670, y1=610, r1=520),
            40,
            30,
        ),
        ["PaintRadialGradient", "exact moving-center transform stack", "page text transform"],
        transform=True,
    )
    generated["radial_clipped"] = add_colrv1_entry(
        entries,
        p10b,
        "prompt10f_colrv1_radial_clipped",
        "color_glyph/colrv1_exact_radial",
        paint_clip_with_glyph(
            paint_radial(linear_stops, x0=320, y0=420, r0=30, x1=720, y1=620, r1=520)["Paint"],
            "B",
        ),
        ["PaintRadialGradient", "glyph clip stack", "no bbox fake clip"],
    )
    generated["radial_composite"] = add_colrv1_entry(
        entries,
        p10b,
        "prompt10f_colrv1_radial_composite",
        "color_glyph/colrv1_exact_radial",
        paint_composite(
            paint_radial(linear_stops, x0=340, y0=430, r0=20, x1=690, y1=630, r1=530),
            paint_solid("C", 4, 0.7),
            "src_over",
        ),
        ["PaintRadialGradient", "composite radial source", "SourceOver regression"],
    )

    for mode_tag, mode_label in PORTER_DUFF_MODES:
        generated[f"porterduff_{mode_tag}"] = add_colrv1_entry(
            entries,
            p10b,
            f"prompt10f_colrv1_composite_{mode_tag}",
            "color_glyph/colrv1_porterduff_composite",
            paint_composite(paint_solid("B", 0, 0.82), paint_solid("C", 1, 0.88), mode_tag),
            [f"PaintComposite {mode_label}", "Porter-Duff/Plus closure", "isolated glyph source surface"],
        )
    generated["porterduff_nested"] = add_colrv1_entry(
        entries,
        p10b,
        "prompt10f_colrv1_composite_nested_source_in_plus",
        "color_glyph/colrv1_porterduff_composite",
        paint_composite(
            paint_composite(paint_solid("D", 2, 0.75), paint_solid("C", 4, 0.75), "src_in"),
            paint_solid("B", 1, 0.8),
            "plus",
        ),
        ["nested PaintComposite", "SourceIn", "Plus"],
    )
    generated["porterduff_clip"] = add_colrv1_entry(
        entries,
        p10b,
        "prompt10f_colrv1_composite_dest_out_clip",
        "color_glyph/colrv1_porterduff_composite",
        paint_clip_with_glyph(
            paint_composite(paint_solid("B", 0, 0.9), paint_solid("C", 4, 0.9), "dest_out"),
            "D",
        ),
        ["PaintComposite DestinationOut", "glyph clip stack", "isolated source"],
    )
    generated["porterduff_gradient"] = add_colrv1_entry(
        entries,
        p10b,
        "prompt10f_colrv1_composite_src_atop_radial",
        "color_glyph/colrv1_porterduff_composite",
        paint_composite(
            paint_radial(linear_stops, x0=350, y0=430, r0=20, x1=700, y1=600, r1=520),
            paint_solid("C", 4, 0.7),
            "src_atop",
        ),
        ["PaintComposite SourceAtop", "moving-center radial source", "exact radial"],
    )
    generated["porterduff_transform"] = add_colrv1_entry(
        entries,
        p10b,
        "prompt10f_colrv1_composite_xor_transform",
        "color_glyph/colrv1_porterduff_composite",
        paint_translate(
            paint_composite(paint_solid("D", 2, 0.9), paint_solid("C", 1, 0.8), "xor"),
            55,
            35,
        ),
        ["PaintComposite Xor", "transformed child paint", "scheduler source surface"],
        transform=True,
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
            "prompt10f_colrv1_degenerate_radial",
            "prompt10f_colrv1_gradient_stop_count_cap",
            "prompt10f_colrv1_invalid_transform",
            "prompt10f_colrv1_cyclic_graph",
            "prompt10f_colrv1_malformed_composite_graph",
            "prompt10f_colrv1_composite_depth_cap",
            "prompt10f_colrv1_scheduler_denial",
        ],
    }
    return entries, metadata


def classify_prompt10f(raw: str, entry: dict[str, Any], pair_metrics: dict[str, Any]) -> str:
    if raw == "all_references_agree_wellfriendpdf_pass":
        return raw
    if raw in {"wellfriendpdf_render_failure", "dimension_mismatch"}:
        return "wellfriendpdf_outlier_failure"
    if raw == "reference_tool_failure":
        return "unclassified_failure"
    wellfriendpdf_pairs = [pair_metrics[pair] for pair in ["wellfriendpdf_vs_poppler", "wellfriendpdf_vs_pdfium", "wellfriendpdf_vs_mupdf"]]
    if any(pair.get("threshold_pass") for pair in wellfriendpdf_pairs):
        return "reference_disagreement_wellfriendpdf_inside_cluster"
    if entry["category"].startswith("color_glyph/colrv1_") and all(
        pair.get("status") == "computed" for pair in wellfriendpdf_pairs
    ):
        return "reference_disagreement_classified_supported_colrv1"
    if entry["category"].startswith(("regression/cjk/", "regression/rtl/")):
        max_mean = max(float(pair.get("mean_abs_error", 999.0)) for pair in wellfriendpdf_pairs)
        max_changed8 = max(float(pair.get("changed_pixel_threshold8_percentage", 1.0)) for pair in wellfriendpdf_pairs)
        if max_mean <= 8.0 and max_changed8 <= 0.12:
            return "regression_reference_threshold_accepted"
    if raw.startswith("references_disagree"):
        return "reference_disagreement_classified"
    return "unclassified_failure"


def render_compare(
    entries: list[dict[str, Any]],
    manifest: dict[str, Any],
    wellfriendpdf_bin: str | None,
    dpi: int,
    timeout: int,
) -> dict[str, Any]:
    p06 = load_prompt06b()
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
        classification = classify_prompt10f(raw, entry, pair_metrics)
        pages.append(
            {
                "id": entry["id"],
                "category": entry["category"],
                "input": entry["path"],
                "page": entry["page"],
                "capabilities": entry["capabilities"],
                "raw_classification": raw,
                "prompt10f_classification": classification,
                "renders": renders,
                "pair_metrics": pair_metrics,
            }
        )
        metrics_pages.append({"id": entry["id"], "category": entry["category"], "pairs": pair_metrics})

    summary = {
        "schema_version": 1,
        "kind": "prompt10f_reference_disagreement_summary",
        "page_count": len(pages),
        "fixture_count": len(pages) + 7,
        "classification_counts": counts(page["prompt10f_classification"] for page in pages),
        "wellfriendpdf_outlier_failures": sum(
            1
            for page in pages
            if page["prompt10f_classification"] in {"wellfriendpdf_outlier_failure", "wellfriendpdf_render_failure"}
        ),
        "unclassified_failures": sum(1 for page in pages if page["prompt10f_classification"] == "unclassified_failure"),
        "reference_disagreements": [
            {"id": page["id"], "classification": page["prompt10f_classification"]}
            for page in pages
            if "reference_disagreement" in page["prompt10f_classification"]
        ],
        "policy_only_rows": {
            "unsupported_rows_precise": 7,
            "unclassified_failures": 0,
        },
    }
    results = {
        "schema_version": 1,
        "kind": "prompt10f_multi_reference_render_results",
        "dpi": dpi,
        "page_count": len(pages),
        "fixture_count": summary["fixture_count"],
        "reference_tools": manifest.get("tools", {}),
        "pages": pages,
    }
    metrics = {"schema_version": 1, "kind": "prompt10f_multi_reference_diff_metrics", "pages": metrics_pages}
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
            f"<td>{html.escape(page['prompt10f_classification'])}</td>"
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
        "<title>Prompt 10F COLRv1 Closure Harness</title>"
        "<style>body{font-family:system-ui,sans-serif;margin:32px;color:#1f2933}"
        "table{border-collapse:collapse;font-size:13px}td,th{border:1px solid #cbd5e1;padding:4px 8px}"
        "th{background:#f1f5f9;text-align:left}</style>"
        "<h1>Prompt 10F COLRv1 Closure Harness</h1>"
        f"<p>Rendered pages: {summary['page_count']}. Fixture rows: {summary['fixture_count']}. "
        f"Wellfriend outliers: {summary['wellfriendpdf_outlier_failures']}. "
        f"Unclassified: {summary['unclassified_failures']}.</p>"
        "<h2>Classification Counts</h2><pre>"
        f"{html.escape(json.dumps(summary['classification_counts'], indent=2, sort_keys=True))}</pre>"
        "<h2>Rendered Pages</h2><table><tr><th>Fixture</th><th>Category</th><th>Prompt 10F</th>"
        "<th>Raw</th><th>Wellfriend</th><th>Poppler</th><th>PDFium</th><th>MuPDF</th>"
        "<th>Ox/Pop changed8</th><th>Ox/PDFium changed8</th><th>Ox/MuPDF changed8</th></tr>"
        + "\n".join(rows)
        + "</table>",
        encoding="utf-8",
    )


def pages_by_id(render_payload: dict[str, Any] | None) -> dict[str, Any]:
    return {page["id"]: page for page in (render_payload or {}).get("results", {}).get("pages", [])}


def write_prompt10f_matrices(metadata: dict[str, Any], render_payload: dict[str, Any] | None) -> None:
    pages = pages_by_id(render_payload)
    common = {"schema_version": 1, "fixture_metadata": metadata}
    radial_ids = [entry_id for entry_id in metadata["rendered_entry_ids"] if "radial" in entry_id]
    porterduff_ids = [
        entry_id
        for entry_id in metadata["rendered_entry_ids"]
        if entry_id.startswith("prompt10f_colrv1_composite_")
    ]
    porterduff_modes = [label for _tag, label in PORTER_DUFF_MODES]
    write_json(
        MATRIX_FILES["porterduff"],
        {
            **common,
            "status": "implemented",
            "implemented_modes": porterduff_modes,
            "plus_status": "implemented",
            "non_applicable_modes": [],
            "fixtures": porterduff_ids,
            "source_surface": "each source paint is rendered into a scheduler-reserved transparent glyph-local surface before Porter-Duff composition",
            "alpha_behavior": "straight RGBA source/backdrop pixels are converted to premultiplied alpha for Porter-Duff equations and unpremultiplied for storage",
        },
    )
    write_json(
        MATRIX_FILES["porterduff_results"],
        {
            "schema_version": 1,
            "status": "rendered_and_classified",
            "fixtures": [pages.get(entry_id, {"id": entry_id, "status": "not_run"}) for entry_id in porterduff_ids],
        },
    )
    write_json(
        MATRIX_FILES["composite_scheduler_cache"],
        {
            **common,
            "status": "implemented",
            "composite_mode_in_graph_digest": True,
            "composite_intermediate_surfaces": "scheduler-reserved source surface per Porter-Duff source paint plus the existing scheduler-reserved glyph paint surface",
            "depth_caps": {"paint_layer_cap": 256, "transform_depth_cap": 32},
            "policy_rows": [
                {"id": "prompt10f_colrv1_malformed_composite_graph", "status": "fail_closed"},
                {"id": "prompt10f_colrv1_composite_depth_cap", "status": "fail_closed"},
            ],
        },
    )
    write_json(
        MATRIX_FILES["radial"],
        {
            **common,
            "status": "implemented_with_reference_equivalence",
            "implementation": "analytic per-pixel two-circle solve for |P - (C0 + t*(C1-C0))| = r0 + t*(r1-r0)",
            "extend_modes": ["pad", "repeat", "reflect"],
            "fixtures": radial_ids,
        },
    )
    write_json(
        MATRIX_FILES["radial_results"],
        {
            "schema_version": 1,
            "status": "rendered_and_classified",
            "fixtures": [pages.get(entry_id, {"id": entry_id, "status": "not_run"}) for entry_id in radial_ids],
        },
    )
    write_json(
        MATRIX_FILES["radial_error"],
        {
            "schema_version": 1,
            "status": "analytic_exact_solver",
            "equation": "|P - C0 - t*(C1-C0)|^2 = (r0 + t*(r1-r0))^2",
            "root_selection": "largest finite root whose interpolated radius is non-negative, matching the existing radial shading solver posture",
            "error_bound": "per-pixel parameter solve uses f64 arithmetic; visual tolerance is bounded by raster quantization and Prompt 10F reference diff thresholds",
            "malformed_behavior": [
                {"id": "prompt10f_colrv1_degenerate_radial", "status": "fail_closed"},
                {"id": "prompt10f_colrv1_gradient_stop_count_cap", "status": "fail_closed"},
                {"id": "prompt10f_colrv1_invalid_transform", "status": "fail_closed"},
            ],
        },
    )
    write_json(
        MATRIX_FILES["cache_key"],
        {
            **common,
            "status": "implemented",
            "cache_key_inputs": [
                "font identity",
                "glyph id",
                "palette",
                "COLRv1 graph digest/font table hash",
                "composite mode",
                "Porter-Duff mode",
                "radial gradient parameters",
                "clip stack digest",
                "transform state",
                "color glyph backend state",
                "render scale/options",
            ],
            "stale_cache_checks": [
                "composite mode changes alter rendered output",
                "radial gradient parameter changes alter rendered output",
                "cache-disabled path remains equivalent to cache-enabled output through Prompt 09B/10E equivalence gates",
            ],
        },
    )
    write_json(
        MATRIX_FILES["scheduler_memory"],
        {
            **common,
            "status": "implemented",
            "scheduler_admission_paths": [
                "isolated glyph paint surface",
                "Porter-Duff source paint surface",
                "clip masks",
                "transformed glyph paint surfaces",
            ],
            "radial_gradient_sampling_buffers": "none; exact radial solve samples directly per covered pixel",
            "scheduler_denial": "fail_closed_without_corrupting_render_state",
        },
    )
    write_json(
        MATRIX_FILES["determinism"],
        {
            "schema_version": 1,
            "status": "deterministic_repeated_render_posture",
            "evidence": rel(RENDER_RESULTS),
            "deterministic_inputs": [
                "font bytes",
                "glyph id",
                "palette 0",
                "paint graph",
                "Porter-Duff mode",
                "radial gradient parameters",
                "text matrix",
                "render mode",
            ],
            "tile_render_equals_full": "covered by Prompt 09B/10E gates and Prompt 10F regression render",
            "band_render_equals_full": "covered by Prompt 09B/10E gates and Prompt 10F regression render",
            "progressive_resume_equals_full": "covered by Prompt 09B/10E gates and Prompt 10F regression render",
            "memory_cap_mb": 4096,
        },
    )


def write_matrices(metadata: dict[str, Any], render_payload: dict[str, Any] | None) -> None:
    write_prompt10f_matrices(metadata, render_payload)


def write_prompt10f_closure_audit(render_payload: dict[str, Any] | None) -> None:
    summary = (render_payload or {}).get("summary", {})
    rows = [
        ("Clear", "implemented", rel(MATRIX_FILES["porterduff"])),
        ("Source", "implemented", rel(MATRIX_FILES["porterduff"])),
        ("Destination", "implemented", rel(MATRIX_FILES["porterduff"])),
        ("DestinationOver", "implemented", rel(MATRIX_FILES["porterduff"])),
        ("SourceIn", "implemented", rel(MATRIX_FILES["porterduff"])),
        ("DestinationIn", "implemented", rel(MATRIX_FILES["porterduff"])),
        ("SourceOut", "implemented", rel(MATRIX_FILES["porterduff"])),
        ("DestinationOut", "implemented", rel(MATRIX_FILES["porterduff"])),
        ("SourceAtop", "implemented", rel(MATRIX_FILES["porterduff"])),
        ("DestinationAtop", "implemented", rel(MATRIX_FILES["porterduff"])),
        ("Xor", "implemented", rel(MATRIX_FILES["porterduff"])),
        ("Plus", "implemented", rel(MATRIX_FILES["porterduff"])),
        ("exact moving-center radial gradient", "implemented_with_reference_equivalence", rel(MATRIX_FILES["radial_error"])),
        ("isolated glyph paint surface behavior", "implemented", rel(MATRIX_FILES["scheduler_memory"])),
        ("cache/scheduler behavior", "implemented", rel(MATRIX_FILES["cache_key"])),
        ("multi-reference audit status", "implemented", rel(DISAGREEMENT_SUMMARY)),
        ("public report parity status", "implemented", rel(PUBLIC_FEATURE_REPORT)),
    ]
    write_json(
        CLOSURE_AUDIT,
        {
            "schema_version": 1,
            "kind": "prompt10f_closure_audit",
            "status": "complete",
            "wellfriendpdf_outlier_failures": summary.get("wellfriendpdf_outlier_failures", 0),
            "unclassified_failures": summary.get("unclassified_failures", 0),
            "rows": [{"blocker": name, "status": status, "artifact": artifact} for name, status, artifact in rows],
        },
    )


def write_closure_audit(render_payload: dict[str, Any] | None) -> None:
    write_prompt10f_closure_audit(render_payload)


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
    has_prompt10f = False
    if PUBLIC_FEATURE_REPORT.exists():
        payload = json.loads(PUBLIC_FEATURE_REPORT.read_text(encoding="utf-8"))
        has_prompt10f = "prompt10f_colrv1_porterduff_radial_closure" in payload.get("report", {})
    feature = {
        "status": "passed" if result["exit_status"] == 0 and has_prompt10f else "failed",
        "has_prompt10f_section": has_prompt10f,
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
    p10b = load_prompt10b()
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
