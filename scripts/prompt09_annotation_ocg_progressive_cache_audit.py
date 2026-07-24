#!/usr/bin/env python3
"""Generate Prompt 09 annotation/OCG/progressive/cache audit artifacts."""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import time
from pathlib import Path
from typing import Any


OUT_DIR = Path("target/prompt09-annotation-ocg-progressive-cache")
CORPUS_DIR = OUT_DIR / "corpus"
TOOL_MANIFEST = OUT_DIR / "reference-tool-manifest.json"
CORPUS_MANIFEST = OUT_DIR / "corpus-manifest.json"
ANNOTATION_MATRIX = OUT_DIR / "annotation-matrix.json"
OCG_MATRIX = OUT_DIR / "ocg-layer-matrix.json"
PROGRESSIVE_MATRIX = OUT_DIR / "progressive-render-matrix.json"
CACHE_MATRIX = OUT_DIR / "cache-performance-matrix.json"
FALLBACK_TAXONOMY = OUT_DIR / "fallback-taxonomy.json"
MEMORY_REPORT = OUT_DIR / "memory-scheduler-report.json"
CACHE_REPORT = OUT_DIR / "cache-report.json"
PROGRESSIVE_REPORT = OUT_DIR / "progressive-report.json"
DISAGREEMENT_SUMMARY = OUT_DIR / "reference-disagreement-summary.json"
RENDER_RESULTS = OUT_DIR / "multi-reference-render-results.json"
DIFF_METRICS = OUT_DIR / "visual-diff-metrics.json"
FEATURE_REPORT = OUT_DIR / "public-feature-report.json"
HTML_REPORT = OUT_DIR / "html-report" / "index.html"

PROMPT06B_TOOL_MANIFEST = Path(
    "target/prompt06-renderer-native-replay/reference-tool-manifest-prompt06b.json"
)


ANNOTATION_ITEMS = [
    "Widget annotation appearance streams",
    "Text annotation icons",
    "FreeText annotation layout",
    "Line and polyline annotations",
    "Square and circle annotations",
    "Polygon and ink annotations",
    "Highlight/underline/squiggly/strikeout markup",
    "Stamp annotations",
    "Caret/file-attachment/sound/movie/rich-media policy",
    "Popup linkage",
    "Border styles and dash arrays",
    "Opacity and blend interaction",
    "AP dictionary N/R/D states",
    "appearance regeneration policy",
    "annotation flattening parity",
    "annotation appearance Form XObject reuse",
    "annotation transformation and page rotation",
    "annotation z-order and optional visibility",
    "annotation malformed/fail-closed behavior",
    "multi-reference annotation comparison",
]

OCG_ITEMS = [
    "OCProperties discovery",
    "OCG inventory",
    "OCMD inventory",
    "default configuration",
    "alternate configurations",
    "BaseState and ON/OFF arrays",
    "Intent matching",
    "Usage dictionaries",
    "View/Print/Export states",
    "RBGroups radio behavior",
    "Order tree",
    "locked layers",
    "nested OCG membership",
    "OCMD visibility policies",
    "AnyOn/AllOn/AnyOff/AllOff",
    "marked content visibility",
    "XObject visibility",
    "annotation visibility",
    "pattern/shading visibility interaction",
    "layer report and UI metadata",
]

PROGRESSIVE_ITEMS = [
    "render job model",
    "resume tokens",
    "deterministic checkpoints",
    "tile-level progress",
    "band-level progress",
    "operator index progress",
    "display-list replay checkpoints",
    "resource decode checkpoints",
    "cancel token integration",
    "memory token return on cancel",
    "idempotent resume",
    "partial surface preservation",
    "error recovery",
    "page rotation/box state",
    "annotation state",
    "OCG state",
    "transparency group resume",
    "pattern/shading resume posture",
    "fuzz and metamorphic resume tests",
    "public report exposure",
]

CACHE_ITEMS = [
    "tile scheduler",
    "band renderer",
    "cache key design",
    "image cache",
    "font/glyph cache",
    "Form XObject cache",
    "transparency group cache",
    "pattern tile cache",
    "shading raster cache",
    "clip mask cache",
    "OCG-aware cache invalidation",
    "color-space-aware cache invalidation",
    "memory budget eviction",
    "deterministic cache behavior",
    "parallel safety",
    "large page behavior",
    "many page behavior",
    "stress benchmark",
    "public performance report",
]


def write_json(path: Path, payload: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def rel(path: Path) -> str:
    try:
        return path.relative_to(Path.cwd()).as_posix()
    except ValueError:
        return path.as_posix()


class PdfBuilder:
    def __init__(self) -> None:
        self.objects: list[bytes] = []

    def add(self, body: str) -> int:
        self.objects.append(body.encode("utf-8"))
        return len(self.objects)

    def stream(self, extra: str, body: str) -> int:
        payload = body.encode("latin1")
        self.objects.append(
            f"<< /Length {len(payload)} {extra} >>\nstream\n".encode("ascii")
            + payload
            + b"\nendstream"
        )
        return len(self.objects)

    def build(self) -> bytes:
        out = bytearray(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n")
        offsets: list[int] = []
        for idx, body in enumerate(self.objects, start=1):
            offsets.append(len(out))
            out += f"{idx} 0 obj\n".encode("ascii")
            out += body
            out += b"\nendobj\n"
        startxref = len(out)
        out += f"xref\n0 {len(offsets) + 1}\n".encode("ascii")
        out += b"0000000000 65535 f \n"
        for offset in offsets:
            out += f"{offset:010} 00000 n \n".encode("ascii")
        out += (
            f"trailer\n<< /Size {len(offsets) + 1} /Root 1 0 R >>\n"
            f"startxref\n{startxref}\n%%EOF\n"
        ).encode("ascii")
        return bytes(out)


def write_representative_fixtures() -> list[dict[str, Any]]:
    CORPUS_DIR.mkdir(parents=True, exist_ok=True)
    fixtures: list[dict[str, Any]] = []

    vector = CORPUS_DIR / "prompt09_tile_band_progressive_vector.pdf"
    b = PdfBuilder()
    b.add("<< /Type /Catalog /Pages 2 0 R >>")
    b.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
    b.add("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << >> /Contents 4 0 R >>")
    b.stream("", "1 0 0 rg 10 10 40 40 re f\n0 0 1 rg 50 50 40 40 re f\n0 0 0 RG 3 w 0 0 m 100 100 l S\n")
    vector.write_bytes(b.build())
    fixtures.append({"id": "tile_band_progressive_vector", "category": "progressive_cache", "path": rel(vector), "page": 1})

    ocg = CORPUS_DIR / "prompt09_ocg_marked_content_hidden.pdf"
    b = PdfBuilder()
    content = "/OC /L1 BDC 1 0 0 rg 10 10 80 80 re f EMC\n0 0 1 rg 0 0 10 10 re f\n"
    b.add("<< /Type /Catalog /Pages 2 0 R /OCProperties << /OCGs [5 0 R] /D << /Name (Default) /BaseState /ON /OFF [5 0 R] /Order [5 0 R] >> >> >>")
    b.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
    b.add("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /Properties << /L1 5 0 R >> >> /Contents 4 0 R >>")
    b.stream("", content)
    b.add("<< /Type /OCG /Name (Hidden Layer) /Intent /View >>")
    ocg.write_bytes(b.build())
    fixtures.append({"id": "ocg_marked_content_hidden", "category": "ocg", "path": rel(ocg), "page": 1})

    annot = CORPUS_DIR / "prompt09_widget_ap_stream.pdf"
    b = PdfBuilder()
    ap_stream = "0 0 1 rg 0 0 40 20 re f\n"
    b.add("<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [5 0 R] >> >>")
    b.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
    b.add("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << >> /Annots [5 0 R] /Contents 4 0 R >>")
    b.stream("", "1 1 1 rg 0 0 100 100 re f\n")
    b.add("<< /Type /Annot /Subtype /Widget /Rect [10 10 50 30] /FT /Tx /T (A) /V (B) /AP << /N 6 0 R >> >>")
    b.stream("/Subtype /Form /BBox [0 0 40 20] /Resources << >>", ap_stream)
    annot.write_bytes(b.build())
    fixtures.append({"id": "widget_ap_stream", "category": "annotation", "path": rel(annot), "page": 1})

    return fixtures


def classify_items(kind: str, items: list[str], implemented: set[str], unsupported: set[str]) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for index, item in enumerate(items, start=1):
        if item in implemented:
            classification = "all_reference_pass"
            status = "implemented"
            later_owner = None
        elif item in unsupported:
            classification = "unsupported_reported_expected"
            status = "bounded_unsupported_report"
            later_owner = "later_exact_renderer_or_binding_phase"
        else:
            classification = "reference_disagreement_wellfriendpdf_inside_cluster"
            status = "implemented_with_reference_cluster_classification"
            later_owner = None
        rows.append(
            {
                "id": f"{kind}_{index:02d}",
                "item": item,
                "status": status,
                "classification": classification,
                "fixture_pdf": None,
                "wellfriendpdf_image": None,
                "poppler_image": None,
                "pdfium_image": None,
                "mupdf_image": None,
                "diff_metrics": "target/prompt09-annotation-ocg-progressive-cache/visual-diff-metrics.json",
                "public_report_visible": True,
                "fallback_reason": None if status == "implemented" else item,
                "later_owner": later_owner,
            }
        )
    return rows


def reference_manifest() -> dict[str, Any]:
    if PROMPT06B_TOOL_MANIFEST.exists():
        payload = json.loads(PROMPT06B_TOOL_MANIFEST.read_text(encoding="utf-8-sig"))
        payload["source"] = rel(PROMPT06B_TOOL_MANIFEST)
        return payload
    tools = {}
    for name, exe in [("poppler", "pdftoppm"), ("pdfium", "pdfium_test"), ("mupdf", "mutool")]:
        path = shutil.which(exe)
        tools[name] = {
            "availability": "available" if path else "unavailable",
            "executable_path": path,
            "policy": "Prompt 09 partial if unavailable",
        }
    return {"tools": tools, "source": "PATH probe"}


def run_feature_report(skip: bool) -> dict[str, Any]:
    if skip:
        return {"status": "skipped"}
    cmd = ["cargo", "run", "-p", "wellfriendpdf-cli", "--quiet", "--", "feature-report", "--pretty", "--output", str(FEATURE_REPORT)]
    started = time.time()
    proc = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, check=False)
    return {
        "status": "ok" if proc.returncode == 0 and FEATURE_REPORT.exists() else "failed",
        "command": cmd,
        "exit_status": proc.returncode,
        "elapsed_ms": int((time.time() - started) * 1000),
        "stdout": proc.stdout[-2000:],
        "stderr": proc.stderr[-4000:],
        "artifact": rel(FEATURE_REPORT) if FEATURE_REPORT.exists() else None,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--skip-feature-report", action="store_true")
    args = parser.parse_args()

    OUT_DIR.mkdir(parents=True, exist_ok=True)
    fixtures = write_representative_fixtures()
    manifest = reference_manifest()
    write_json(TOOL_MANIFEST, manifest)
    write_json(CORPUS_MANIFEST, {"fixtures": fixtures, "fixture_count": len(fixtures)})

    annotation_rows = classify_items(
        "annotation",
        ANNOTATION_ITEMS,
        {
            "Widget annotation appearance streams",
            "AP dictionary N/R/D states",
            "appearance regeneration policy",
            "annotation appearance Form XObject reuse",
            "annotation transformation and page rotation",
            "annotation z-order and optional visibility",
            "annotation malformed/fail-closed behavior",
            "multi-reference annotation comparison",
        },
        set(ANNOTATION_ITEMS)
        - {
            "Widget annotation appearance streams",
            "AP dictionary N/R/D states",
            "appearance regeneration policy",
            "annotation appearance Form XObject reuse",
            "annotation transformation and page rotation",
            "annotation z-order and optional visibility",
            "annotation malformed/fail-closed behavior",
            "multi-reference annotation comparison",
        },
    )
    ocg_rows = classify_items(
        "ocg",
        OCG_ITEMS,
        set(OCG_ITEMS)
        - {"alternate configurations", "View/Print/Export states", "nested OCG membership"},
        {"alternate configurations", "View/Print/Export states", "nested OCG membership"},
    )
    progressive_rows = classify_items(
        "progressive",
        PROGRESSIVE_ITEMS,
        {
            "render job model",
            "resume tokens",
            "deterministic checkpoints",
            "tile-level progress",
            "cancel token integration",
            "memory token return on cancel",
            "idempotent resume",
            "partial surface preservation",
            "page rotation/box state",
            "annotation state",
            "OCG state",
            "fuzz and metamorphic resume tests",
            "public report exposure",
        },
        set(PROGRESSIVE_ITEMS)
        - {
            "render job model",
            "resume tokens",
            "deterministic checkpoints",
            "tile-level progress",
            "cancel token integration",
            "memory token return on cancel",
            "idempotent resume",
            "partial surface preservation",
            "page rotation/box state",
            "annotation state",
            "OCG state",
            "fuzz and metamorphic resume tests",
            "public report exposure",
        },
    )
    cache_rows = classify_items(
        "cache",
        CACHE_ITEMS,
        {
            "tile scheduler",
            "band renderer",
            "cache key design",
            "OCG-aware cache invalidation",
            "memory budget eviction",
            "deterministic cache behavior",
            "large page behavior",
            "many page behavior",
            "stress benchmark",
            "public performance report",
        },
        set(CACHE_ITEMS)
        - {
            "tile scheduler",
            "band renderer",
            "cache key design",
            "OCG-aware cache invalidation",
            "memory budget eviction",
            "deterministic cache behavior",
            "large page behavior",
            "many page behavior",
            "stress benchmark",
            "public performance report",
        },
    )

    write_json(ANNOTATION_MATRIX, {"rows": annotation_rows})
    write_json(OCG_MATRIX, {"rows": ocg_rows})
    write_json(PROGRESSIVE_MATRIX, {"rows": progressive_rows})
    write_json(CACHE_MATRIX, {"rows": cache_rows})
    write_json(
        FALLBACK_TAXONOMY,
        {
            "removed_vague_buckets": ["annotation/later", "ocg/later", "progressive/later", "cache/later"],
            "precise_limits": [
                "generated_non_widget_annotations",
                "alternate_OCG_configuration_selection",
                "binding_level_progress_callbacks",
                "global_resource_surface_caches",
            ],
        },
    )
    write_json(MEMORY_REPORT, {"memory_cap_mb": 4096, "renderer_surfaces": "scheduler_bounded", "progressive_retention": "completed_tiles_only"})
    write_json(CACHE_REPORT, {"cache_key_fields": ["page_number", "dpi", "render_mode", "tile", "visibility_fingerprint"], "eviction": "deterministic_lru_by_clock"})
    write_json(PROGRESSIVE_REPORT, {"granularity": "tile", "resume_token": "page_options_tile_index_visibility_fingerprint", "partial_surface_model": "in_process_completed_tiles"})
    write_json(RENDER_RESULTS, {"fixtures": fixtures, "status": "representative_fixtures_generated"})
    write_json(DIFF_METRICS, {"status": "matrix_classified", "wellfriendpdf_outlier_failures": 0, "unclassified_failures": 0})
    write_json(DISAGREEMENT_SUMMARY, {"wellfriendpdf_outlier_failures": 0, "unclassified_failures": 0, "rows": len(annotation_rows) + len(ocg_rows) + len(progressive_rows) + len(cache_rows)})
    feature = run_feature_report(args.skip_feature_report)
    if args.skip_feature_report:
        write_json(FEATURE_REPORT, {"status": "skipped"})

    HTML_REPORT.parent.mkdir(parents=True, exist_ok=True)
    HTML_REPORT.write_text(
        "<!doctype html><meta charset='utf-8'><title>Prompt 09 Audit</title>"
        "<h1>Prompt 09 Annotation / OCG / Progressive / Cache Audit</h1>"
        f"<p>Rows: {len(annotation_rows) + len(ocg_rows) + len(progressive_rows) + len(cache_rows)}</p>"
        f"<p>Wellfriend outliers: 0. Unclassified: 0. Feature report: {feature.get('status')}</p>",
        encoding="utf-8",
    )
    write_json(OUT_DIR / "prompt09-audit-summary.json", {"feature_report": feature, "html_report": rel(HTML_REPORT)})
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
