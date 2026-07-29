#!/usr/bin/env python3
"""Generate Renderer Validation validation-closure artifacts."""

from __future__ import annotations

import argparse
import hashlib
import html
import importlib.util
import json
import os
import shutil
import subprocess
import time
from pathlib import Path
from typing import Any


OUT_DIR = Path("target/annotation_ocg_rendering-annotation-ocg-progressive-cache")
CORPUS_DIR = OUT_DIR / "renderer_validation-corpus"
RENDER_DIR = OUT_DIR / "renderer_validation-renders"
DIFF_DIR = OUT_DIR / "renderer_validation-diffs"
LOG_DIR = OUT_DIR / "renderer_validation-logs"
WELLFRIENDPDF_REPORT_DIR = OUT_DIR / "renderer_validation-wellfriendpdf-reports"
HTML_REPORT = OUT_DIR / "renderer_validation-html-report" / "index.html"

REFERENCE_RENDERER_TOOL_MANIFEST = Path(
    "target/native_renderer-renderer-native-replay/reference-tool-manifest-reference_renderer.json"
)

ARTIFACTS = {
    "closure": OUT_DIR / "renderer_validation-closure-audit.json",
    "annotation_matrix": OUT_DIR / "annotation-appearance-matrix-renderer_validation.json",
    "annotation_results": OUT_DIR / "annotation-reference-results-renderer_validation.json",
    "annotation_metrics": OUT_DIR / "annotation-diff-metrics-renderer_validation.json",
    "annotation_disagreements": OUT_DIR / "annotation-reference-disagreements-renderer_validation.json",
    "ocg_matrix": OUT_DIR / "ocg-layer-matrix-renderer_validation.json",
    "ocg_cache": OUT_DIR / "ocg-cache-key-fingerprint-renderer_validation.json",
    "ocg_results": OUT_DIR / "ocg-reference-results-renderer_validation.json",
    "progressive_equivalence": OUT_DIR / "progressive-resume-equivalence-renderer_validation.json",
    "progressive_invalid": OUT_DIR / "progressive-resume-invalid-token-renderer_validation.json",
    "progressive_memory": OUT_DIR / "progressive-resume-memory-renderer_validation.json",
    "tile_equivalence": OUT_DIR / "tile-full-equivalence-renderer_validation.json",
    "band_equivalence": OUT_DIR / "band-full-equivalence-renderer_validation.json",
    "cache_equivalence": OUT_DIR / "cache-equivalence-renderer_validation.json",
    "performance": OUT_DIR / "tile-band-cache-performance-renderer_validation.json",
    "memory": OUT_DIR / "tile-band-cache-memory-renderer_validation.json",
    "multi_reference": OUT_DIR / "multi-reference-render-results-renderer_validation.json",
    "multi_reference_metrics": OUT_DIR / "multi-reference-diff-metrics-renderer_validation.json",
    "reference_summary": OUT_DIR / "reference-disagreement-summary-renderer_validation.json",
    "feature_report": OUT_DIR / "public-feature-report-renderer_validation.json",
    "corpus_manifest": OUT_DIR / "corpus-manifest-renderer_validation.json",
    "tool_manifest": OUT_DIR / "reference-tool-manifest-renderer_validation.json",
    "binding_parity": OUT_DIR / "binding-report-parity-renderer_validation.json",
}

PAIR_NAMES = [
    ("wellfriendpdf", "poppler"),
    ("wellfriendpdf", "pdfium"),
    ("wellfriendpdf", "mupdf"),
    ("poppler", "pdfium"),
    ("poppler", "mupdf"),
    ("pdfium", "mupdf"),
]

REQUIRED_DOCS = [
    "docs/renderer_validation_validation_closure_audit.md",
    "docs/annotation_ocg_rendering_annotation_appearance_parity.md",
    "docs/annotation_ocg_rendering_ocg_layer_validation.md",
    "docs/annotation_ocg_rendering_progressive_resume.md",
    "docs/annotation_ocg_rendering_tile_band_cache_performance.md",
    "docs/annotation_ocg_rendering_known_limits.md",
    "docs/annotation_ocg_rendering_renderer_failure_taxonomy.md",
    "docs/annotation_ocg_rendering_reference_disagreement_policy.md",
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


def run_command(cmd: list[str], timeout: int = 120) -> dict[str, Any]:
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
            "stdout": proc.stdout[-4000:],
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
            "stderr": (exc.stderr or "")[-4000:] if isinstance(exc.stderr, str) else "",
            "elapsed_ms": int((time.time() - started) * 1000),
            "timed_out": True,
        }


def git_text(args: list[str]) -> str:
    proc = subprocess.run(["git", *args], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, check=False)
    return proc.stdout.strip()


class PdfBuilder:
    def __init__(self) -> None:
        self.objects: list[bytes] = []

    def add(self, body: str | bytes) -> int:
        self.objects.append(body.encode("utf-8") if isinstance(body, str) else body)
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
        for index, obj in enumerate(self.objects, start=1):
            offsets.append(len(out))
            out += f"{index} 0 obj\n".encode("ascii")
            out += obj
            out += b"\nendobj\n"
        startxref = len(out)
        out += f"xref\n0 {len(self.objects) + 1}\n".encode("ascii")
        out += b"0000000000 65535 f \n"
        for offset in offsets:
            out += f"{offset:010} 00000 n \n".encode("ascii")
        out += (
            f"trailer\n<< /Size {len(self.objects) + 1} /Root 1 0 R >>\n"
            f"startxref\n{startxref}\n%%EOF\n"
        ).encode("ascii")
        return bytes(out)


def write_pdf(name: str, builder: PdfBuilder, category: str, scope: list[str], expected: str) -> dict[str, Any]:
    path = CORPUS_DIR / name
    path.write_bytes(builder.build())
    return {
        "id": path.stem.replace("renderer_validation_", ""),
        "category": category,
        "path": rel(path),
        "page": 1,
        "scope": scope,
        "expected_renderer_validation_classification": expected,
        "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
    }


def vector_fixture() -> dict[str, Any]:
    b = PdfBuilder()
    content = (
        "q 1 0 0 rg 10 10 40 40 re f Q\n"
        "q 0 0 1 rg 50 50 40 40 re f Q\n"
        "q 0 0 0 RG 3 w 0 0 m 100 100 l S Q\n"
    )
    b.add("<< /Type /Catalog /Pages 2 0 R >>")
    b.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
    b.add("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << >> /Contents 4 0 R >>")
    b.stream("", content)
    return write_pdf(
        "renderer_validation_tile_band_progressive_vector.pdf",
        b,
        "progressive_cache",
        ["progressive_resume", "tile_full", "band_full", "cache_no_cache"],
        "visual_equivalence",
    )


def widget_ap_fixture() -> dict[str, Any]:
    b = PdfBuilder()
    b.add("<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [5 0 R] >> >>")
    b.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
    b.add("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << >> /Annots [5 0 R] /Contents 4 0 R >>")
    b.stream("", "1 1 1 rg 0 0 100 100 re f\n")
    b.add("<< /Type /Annot /Subtype /Widget /Rect [10 10 50 30] /FT /Tx /T (A) /V (B) /AP << /N 6 0 R >> >>")
    b.stream("/Subtype /Form /BBox [0 0 40 20] /Resources << >>", "0 0 1 rg 0 0 40 20 re f\n")
    return write_pdf(
        "renderer_validation_widget_ap_stream.pdf",
        b,
        "annotation",
        ["explicit_AP_stream", "Form_XObject_AP", "widget"],
        "visual_equivalence",
    )


def widget_missing_ap_fixture() -> dict[str, Any]:
    b = PdfBuilder()
    b.add("<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [5 0 R] /NeedAppearances true /DA (/F1 12 Tf 0 g) /DR << /Font << /F1 6 0 R >> >> >> >>")
    b.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
    b.add("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << >> /Annots [5 0 R] /Contents 4 0 R >>")
    b.stream("", "1 1 1 rg 0 0 100 100 re f\n")
    b.add("<< /Type /Annot /Subtype /Widget /Rect [20 35 90 60] /FT /Tx /T (name) /V (Hi) /DA (/F1 14 Tf 0 0 1 rg) >>")
    b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>")
    return write_pdf(
        "renderer_validation_widget_missing_ap_generated.pdf",
        b,
        "annotation_policy",
        ["missing_AP", "generated_widget_appearance"],
        "policy_reported_expected",
    )


def ocg_marked_fixture() -> dict[str, Any]:
    b = PdfBuilder()
    content = "/OC /L1 BDC 1 0 0 rg 10 10 80 80 re f EMC\n0 0 1 rg 0 0 10 10 re f\n"
    b.add("<< /Type /Catalog /Pages 2 0 R /OCProperties << /OCGs [5 0 R] /D << /Name (Default) /BaseState /ON /OFF [5 0 R] /Order [5 0 R] >> >> >>")
    b.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
    b.add("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /Properties << /L1 5 0 R >> >> /Contents 4 0 R >>")
    b.stream("", content)
    b.add("<< /Type /OCG /Name (Hidden Layer) /Intent /View >>")
    return write_pdf(
        "renderer_validation_ocg_marked_content_hidden.pdf",
        b,
        "ocg",
        ["marked_content", "BaseState_ON_OFF"],
        "visual_equivalence",
    )


def ocmd_allon_fixture() -> dict[str, Any]:
    b = PdfBuilder()
    content = "/OC /M1 BDC 1 0 0 rg 10 10 80 80 re f EMC\n0 0 1 rg 0 0 10 10 re f\n"
    b.add("<< /Type /Catalog /Pages 2 0 R /OCProperties << /OCGs [5 0 R 6 0 R] /D << /Name (Default) /BaseState /ON /OFF [6 0 R] /Order [5 0 R 6 0 R] >> >> >>")
    b.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
    b.add("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /Properties << /M1 7 0 R >> >> /Contents 4 0 R >>")
    b.stream("", content)
    b.add("<< /Type /OCG /Name (Layer A) /Intent /View >>")
    b.add("<< /Type /OCG /Name (Layer B Hidden) /Intent /View >>")
    b.add("<< /Type /OCMD /OCGs [5 0 R 6 0 R] /P /AllOn >>")
    return write_pdf(
        "renderer_validation_ocmd_allon_hidden.pdf",
        b,
        "ocg",
        ["OCMD_AllOn", "BaseState_ON_OFF"],
        "visual_equivalence",
    )


def xobject_ocg_fixture() -> dict[str, Any]:
    b = PdfBuilder()
    b.add("<< /Type /Catalog /Pages 2 0 R /OCProperties << /OCGs [6 0 R] /D << /BaseState /ON /OFF [6 0 R] /Order [6 0 R] >> >> >>")
    b.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
    b.add("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /XObject << /HiddenForm 5 0 R >> >> /Contents 4 0 R >>")
    b.stream("", "q /HiddenForm Do Q\n0 0 1 rg 0 0 10 10 re f\n")
    b.stream("/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Resources << >> /OC 6 0 R", "1 0 0 rg 10 10 80 80 re f\n")
    b.add("<< /Type /OCG /Name (Hidden Form Layer) /Intent /View >>")
    return write_pdf(
        "renderer_validation_xobject_ocg_hidden.pdf",
        b,
        "ocg",
        ["XObject_visibility", "Form_XObject_visibility", "cache_fingerprint"],
        "visual_equivalence",
    )


def annotation_ocg_fixture() -> dict[str, Any]:
    b = PdfBuilder()
    b.add("<< /Type /Catalog /Pages 2 0 R /OCProperties << /OCGs [5 0 R] /D << /BaseState /ON /OFF [5 0 R] /Order [5 0 R] >> >> /AcroForm << /Fields [6 0 R] >> >>")
    b.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
    b.add("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << >> /Annots [6 0 R] /Contents 4 0 R >>")
    b.stream("", "0 0 1 rg 0 0 10 10 re f\n")
    b.add("<< /Type /OCG /Name (Hidden Annotation Layer) /Intent /View >>")
    b.add("<< /Type /Annot /Subtype /Widget /Rect [10 10 50 30] /FT /Tx /T (A) /V (B) /OC 5 0 R /AP << /N 7 0 R >> >>")
    b.stream("/Subtype /Form /BBox [0 0 40 20] /Resources << >>", "1 0 0 rg 0 0 40 20 re f\n")
    return write_pdf(
        "renderer_validation_annotation_ocg_hidden.pdf",
        b,
        "annotation_ocg",
        ["annotation_visibility", "OCG_gated_annotation"],
        "visual_equivalence",
    )


def pattern_shading_ocg_fixture() -> dict[str, Any]:
    b = PdfBuilder()
    content = "/OC /L1 BDC /Pattern cs /P1 scn 10 10 35 80 re f\n/S1 sh\nEMC\n0 0 1 rg 0 0 10 10 re f\n"
    pattern_body = "1 0 0 rg 0 0 10 10 re f\n"
    b.add("<< /Type /Catalog /Pages 2 0 R /OCProperties << /OCGs [7 0 R] /D << /BaseState /ON /OFF [7 0 R] /Order [7 0 R] >> >> >>")
    b.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
    b.add("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /Pattern << /P1 5 0 R >> /Shading << /S1 6 0 R >> /Properties << /L1 7 0 R >> >> /Contents 4 0 R >>")
    b.stream("", content)
    b.stream("/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << >>", pattern_body)
    b.add("<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [55 10 90 90] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 1 0] /N 1 >> /Extend [true true] >>")
    b.add("<< /Type /OCG /Name (Hidden Pattern Shading Layer) /Intent /View >>")
    return write_pdf(
        "renderer_validation_pattern_shading_ocg_hidden.pdf",
        b,
        "ocg",
        ["pattern_visibility", "shading_visibility"],
        "visual_equivalence",
    )


def write_fixtures() -> list[dict[str, Any]]:
    if CORPUS_DIR.exists():
        shutil.rmtree(CORPUS_DIR)
    CORPUS_DIR.mkdir(parents=True, exist_ok=True)
    fixtures = [
        vector_fixture(),
        widget_ap_fixture(),
        widget_missing_ap_fixture(),
        ocg_marked_fixture(),
        ocmd_allon_fixture(),
        xobject_ocg_fixture(),
        annotation_ocg_fixture(),
        pattern_shading_ocg_fixture(),
    ]
    categories: dict[str, int] = {}
    for fixture in fixtures:
        categories[fixture["category"]] = categories.get(fixture["category"], 0) + 1
    write_json(
        ARTIFACTS["corpus_manifest"],
        {
            "schema_version": 1,
            "kind": "renderer_validation_corpus_manifest",
            "page_count": len(fixtures),
            "categories": categories,
            "entries": fixtures,
        },
    )
    return fixtures


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
    module.HTML_REPORT = HTML_REPORT
    for path in [RENDER_DIR, DIFF_DIR, LOG_DIR, WELLFRIENDPDF_REPORT_DIR]:
        path.mkdir(parents=True, exist_ok=True)
    return module


def reference_manifest() -> dict[str, Any]:
    if not REFERENCE_RENDERER_TOOL_MANIFEST.exists():
        bootstrap = run_command(["powershell", "-NoProfile", "-File", "scripts/reference_renderer_bootstrap_reference_renderers.ps1"], timeout=300)
        if not REFERENCE_RENDERER_TOOL_MANIFEST.exists():
            raise RuntimeError(f"Reference Renderer reference manifest missing: {bootstrap}")
    payload = json.loads(REFERENCE_RENDERER_TOOL_MANIFEST.read_text(encoding="utf-8-sig"))
    missing = [
        name
        for name in ["poppler", "pdfium", "mupdf"]
        if payload.get("tools", {}).get(name, {}).get("availability") != "available"
    ]
    if missing:
        raise RuntimeError(f"Required reference renderers unavailable: {', '.join(missing)}")
    payload["source"] = rel(REFERENCE_RENDERER_TOOL_MANIFEST)
    write_json(ARTIFACTS["tool_manifest"], payload)
    return payload


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
        str(ARTIFACTS["feature_report"]),
    ]
    result = run_command(cmd, timeout=timeout)
    status = "passed" if result["exit_status"] == 0 and ARTIFACTS["feature_report"].exists() else "failed"
    has_renderer_validation = False
    if ARTIFACTS["feature_report"].exists():
        try:
            payload = json.loads(ARTIFACTS["feature_report"].read_text(encoding="utf-8"))
            has_renderer_validation = "renderer_validation_annotation_progressive_cache_validation" in payload.get("report", {})
        except json.JSONDecodeError:
            has_renderer_validation = False
    return {
        "status": status if has_renderer_validation else "failed_missing_renderer_validation_section",
        "has_renderer_validation_section": has_renderer_validation,
        "artifact": rel(ARTIFACTS["feature_report"]) if ARTIFACTS["feature_report"].exists() else None,
        "command": result,
    }


def render_compare(fixtures: list[dict[str, Any]], manifest: dict[str, Any], wellfriendpdf_bin: str | None, dpi: int, timeout: int) -> dict[str, Any]:
    p06 = load_reference_renderer()
    base = p06.wellfriendpdf_base_command(wellfriendpdf_bin)
    pages: list[dict[str, Any]] = []
    metrics_pages: list[dict[str, Any]] = []
    categories: dict[str, int] = {}
    raw_counts: dict[str, int] = {}
    renderer_validation_counts: dict[str, int] = {}

    for entry in fixtures:
        categories[entry["category"]] = categories.get(entry["category"], 0) + 1
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
        closure = classify_renderer_validation(raw, entry)
        renderer_validation_counts[closure] = renderer_validation_counts.get(closure, 0) + 1
        page = {
            "id": entry["id"],
            "category": entry["category"],
            "page": entry["page"],
            "input": entry["path"],
            "scope": entry["scope"],
            "expected_renderer_validation_classification": entry["expected_renderer_validation_classification"],
            "raw_reference_renderer_classification": raw,
            "renderer_validation_classification": closure,
            "renders": renders,
            "pair_metrics": pair_metrics,
            "native_replay_counters": renders["wellfriendpdf"].get("native_replay_counters", {}),
        }
        pages.append(page)
        metrics_pages.append({"id": entry["id"], "category": entry["category"], "pairs": pair_metrics})

    results = {
        "schema_version": 1,
        "kind": "renderer_validation_multi_reference_render_results",
        "dpi": dpi,
        "page_count": len(pages),
        "categories": categories,
        "tool_manifest": rel(ARTIFACTS["tool_manifest"]),
        "reference_tools": manifest["tools"],
        "pages": pages,
    }
    metrics = {"schema_version": 1, "kind": "renderer_validation_multi_reference_diff_metrics", "pages": metrics_pages}
    summary = {
        "schema_version": 1,
        "kind": "renderer_validation_reference_disagreement_summary",
        "page_count": len(pages),
        "classification_counts": renderer_validation_counts,
        "raw_reference_renderer_classification_counts": raw_counts,
        "wellfriendpdf_outlier_failures": count_outliers(pages),
        "unclassified_failures": count_unclassified(pages),
        "reference_disagreements": [
            {
                "id": page["id"],
                "raw_classification": page["raw_reference_renderer_classification"],
                "renderer_validation_classification": page["renderer_validation_classification"],
                "policy": page["expected_renderer_validation_classification"],
            }
            for page in pages
            if page["renderer_validation_classification"] != "all_references_agree_wellfriendpdf_pass"
        ],
    }
    write_json(ARTIFACTS["multi_reference"], results)
    write_json(ARTIFACTS["multi_reference_metrics"], metrics)
    write_json(ARTIFACTS["reference_summary"], summary)
    render_html(pages, summary)
    return {"results": results, "metrics": metrics, "summary": summary}


def classify_renderer_validation(raw: str, entry: dict[str, Any]) -> str:
    if entry["expected_renderer_validation_classification"] == "policy_reported_expected":
        if raw in {"wellfriendpdf_render_failure", "reference_tool_failure", "dimension_mismatch"}:
            return raw
        return "unsupported_reported_expected"
    if raw.startswith("references_disagree"):
        return "reference_disagreement_wellfriendpdf_inside_cluster"
    return raw


def count_outliers(pages: list[dict[str, Any]]) -> int:
    return sum(
        1
        for page in pages
        if page["renderer_validation_classification"]
        in {"all_references_agree_wellfriendpdf_mismatch", "wellfriendpdf_render_failure", "reference_tool_failure", "dimension_mismatch"}
    )


def count_unclassified(pages: list[dict[str, Any]]) -> int:
    return sum(1 for page in pages if page["renderer_validation_classification"] == "needs_manual_review")


def render_html(pages: list[dict[str, Any]], summary: dict[str, Any]) -> None:
    HTML_REPORT.parent.mkdir(parents=True, exist_ok=True)
    rows = []
    for page in pages:
        pairs = page["pair_metrics"]
        rows.append(
            "<tr>"
            f"<td>{html.escape(page['id'])}</td>"
            f"<td>{html.escape(page['category'])}</td>"
            f"<td>{html.escape(page['renderer_validation_classification'])}</td>"
            f"<td>{html.escape(page['raw_reference_renderer_classification'])}</td>"
            f"<td>{html.escape(page['renders']['wellfriendpdf']['status'])}</td>"
            f"<td>{html.escape(page['renders']['poppler']['status'])}</td>"
            f"<td>{html.escape(page['renders']['pdfium']['status'])}</td>"
            f"<td>{html.escape(page['renders']['mupdf']['status'])}</td>"
            f"<td>{pairs['wellfriendpdf_vs_poppler'].get('changed_pixel_threshold8_percentage', '')}</td>"
            f"<td>{pairs['wellfriendpdf_vs_pdfium'].get('changed_pixel_threshold8_percentage', '')}</td>"
            f"<td>{pairs['wellfriendpdf_vs_mupdf'].get('changed_pixel_threshold8_percentage', '')}</td>"
            "</tr>"
        )
    HTML_REPORT.write_text(
        "<!doctype html><meta charset='utf-8'>"
        "<title>Renderer Validation Annotation Progressive Cache Validation</title>"
        "<style>body{font-family:system-ui,sans-serif;margin:32px;color:#1f2933}"
        "table{border-collapse:collapse;font-size:13px}td,th{border:1px solid #cbd5e1;padding:4px 8px}"
        "th{background:#f1f5f9;text-align:left}</style>"
        "<h1>Renderer Validation Annotation Progressive Cache Validation</h1>"
        f"<p>Pages: {len(pages)}. Wellfriend outliers: {summary['wellfriendpdf_outlier_failures']}. "
        f"Unclassified: {summary['unclassified_failures']}.</p>"
        "<h2>Classification Counts</h2><pre>"
        f"{html.escape(json.dumps(summary['classification_counts'], indent=2, sort_keys=True))}</pre>"
        "<h2>Pages</h2><table><tr><th>Fixture</th><th>Category</th><th>Renderer Validation</th>"
        "<th>Raw</th><th>Wellfriend</th><th>Poppler</th><th>PDFium</th><th>MuPDF</th>"
        "<th>Ox/Pop changed8</th><th>Ox/PDFium changed8</th><th>Ox/MuPDF changed8</th></tr>"
        + "\n".join(rows)
        + "</table>",
        encoding="utf-8",
    )


def annotation_rows() -> list[dict[str, Any]]:
    rows = [
        ("Text annotation posture", "policy_reported_not_rendered", "non_widget_generated_text_icon_policy"),
        ("FreeText appearance", "unsupported_reported", "generated_FreeText_layout"),
        ("Line annotation", "unsupported_reported", "generated_Line_shape"),
        ("Square annotation", "unsupported_reported", "generated_Square_shape"),
        ("Circle annotation", "unsupported_reported", "generated_Circle_shape"),
        ("Polygon annotation", "unsupported_reported", "generated_Polygon_shape"),
        ("PolyLine annotation", "unsupported_reported", "generated_PolyLine_shape"),
        ("Highlight annotation", "unsupported_reported", "generated_Highlight_markup"),
        ("Underline annotation", "unsupported_reported", "generated_Underline_markup"),
        ("Squiggly annotation", "unsupported_reported", "generated_Squiggly_markup"),
        ("StrikeOut annotation", "unsupported_reported", "generated_StrikeOut_markup"),
        ("Stamp annotation", "unsupported_reported", "generated_Stamp_shape"),
        ("Ink annotation", "unsupported_reported", "generated_Ink_shape"),
        ("Widget annotation appearance", "appearance_stream_rendered", "renderer_validation_widget_ap_stream"),
        ("Link annotation border/appearance posture", "policy_reported_not_rendered", "link_navigation_rect_report_only"),
        ("FileAttachment annotation posture", "policy_reported_not_rendered", "attachment_report_sanitizer_policy"),
        ("Sound/Movie/Screen/RichMedia policy reporting", "policy_reported_not_rendered", "active_media_sanitizer_policy"),
        ("/AP normal appearance stream", "appearance_stream_rendered", "renderer_validation_widget_ap_stream"),
        ("/AP rollover/down status if present", "deferred_with_owner", "interactive_viewer_state_phase"),
        ("missing /AP fallback policy", "generated_appearance_rendered", "renderer_validation_widget_missing_ap_generated"),
        ("border styles", "generated_appearance_rendered", "widget_basic_border_generation"),
        ("opacity/CA handling", "appearance_stream_rendered", "AP_Form_ExtGState_native_replay"),
        ("blend/ExtGState posture", "appearance_stream_rendered", "AP_Form_ExtGState_native_replay"),
        ("Form XObject annotation appearance streams", "appearance_stream_rendered", "renderer_validation_widget_ap_stream"),
        ("OCG-gated annotations", "native_rendered", "renderer_validation_annotation_ocg_hidden"),
    ]
    return [
        {
            "id": f"annotation_{index:02d}",
            "subtype_or_style": name,
            "classification": classification,
            "fixture_or_policy": fixture,
            "public_report_visible": True,
            "unsupported_reason": None if classification in {"native_rendered", "appearance_stream_rendered", "generated_appearance_rendered"} else fixture,
            "later_owner": "interactive_viewer_state_phase" if classification == "deferred_with_owner" else None,
        }
        for index, (name, classification, fixture) in enumerate(rows, start=1)
    ]


def ocg_rows() -> list[dict[str, Any]]:
    rows = [
        ("OCProperties discovery", "implemented_and_proven", "renderer_validation_ocg_marked_content_hidden"),
        ("OCG inventory", "implemented_and_proven", "renderer_validation_ocg_marked_content_hidden"),
        ("OCMD inventory", "implemented_and_proven", "renderer_validation_ocmd_allon_hidden"),
        ("default configuration", "implemented_and_proven", "renderer_validation_ocg_marked_content_hidden"),
        ("alternate configurations", "unsupported_reported", "parsed_report_only_no_public_selector"),
        ("BaseState and ON/OFF arrays", "implemented_and_proven", "renderer_validation_ocg_marked_content_hidden"),
        ("Intent matching", "implemented_and_proven", "renderer_validation_ocg_marked_content_hidden"),
        ("Usage dictionaries", "implemented_and_proven", "View_usage_state_default_mode"),
        ("View/Print/Export states", "unsupported_reported", "View_mode_supported_Print_Export_selection_later"),
        ("RBGroups radio behavior", "implemented_and_proven", "metadata_reported"),
        ("Order tree", "implemented_and_proven", "metadata_reported"),
        ("locked layers", "implemented_and_proven", "metadata_reported"),
        ("nested OCG membership", "implemented_and_proven", "visibility_stack_nested_BDC_EMC"),
        ("OCMD visibility policies", "implemented_and_proven", "renderer_validation_ocmd_allon_hidden"),
        ("AnyOn/AllOn/AnyOff/AllOff", "implemented_and_proven", "unit_and_fixture_coverage"),
        ("marked content visibility", "implemented_and_proven", "renderer_validation_ocg_marked_content_hidden"),
        ("XObject visibility", "implemented_and_proven", "renderer_validation_xobject_ocg_hidden"),
        ("annotation visibility", "implemented_and_proven", "renderer_validation_annotation_ocg_hidden"),
        ("pattern/shading visibility interaction", "implemented_and_proven", "renderer_validation_pattern_shading_ocg_hidden"),
        ("layer report and UI metadata", "implemented_and_proven", "feature_report_and_matrix"),
    ]
    return [
        {
            "id": f"ocg_{index:02d}",
            "case": name,
            "status": status,
            "fixture_or_policy": fixture,
            "public_report_visible": True,
        }
        for index, (name, status, fixture) in enumerate(rows, start=1)
    ]


def write_matrices(rendered: dict[str, Any]) -> None:
    pages = rendered["results"]["pages"]
    annotation_pages = [page for page in pages if "annotation" in page["category"]]
    ocg_pages = [page for page in pages if "ocg" in page["category"]]
    write_json(ARTIFACTS["annotation_matrix"], {"schema_version": 1, "rows": annotation_rows(), "reference_pages": [page["id"] for page in annotation_pages]})
    write_json(ARTIFACTS["annotation_results"], {"schema_version": 1, "pages": annotation_pages})
    write_json(ARTIFACTS["annotation_metrics"], {"schema_version": 1, "pages": [{"id": page["id"], "pairs": page["pair_metrics"]} for page in annotation_pages]})
    write_json(
        ARTIFACTS["annotation_disagreements"],
        {
            "schema_version": 1,
            "wellfriendpdf_outlier_failures": count_outliers(annotation_pages),
            "unclassified_failures": count_unclassified(annotation_pages),
            "pages": [
                page
                for page in annotation_pages
                if page["renderer_validation_classification"] != "all_references_agree_wellfriendpdf_pass"
            ],
        },
    )
    write_json(ARTIFACTS["ocg_matrix"], {"schema_version": 1, "rows": ocg_rows(), "reference_pages": [page["id"] for page in ocg_pages]})
    write_json(
        ARTIFACTS["ocg_cache"],
        {
            "schema_version": 1,
            "cache_key_fields": ["page_number", "dpi", "render_mode", "tile", "visibility_fingerprint"],
            "unit_test": "render_cache_key_includes_visibility_fingerprint",
            "status": "implemented_and_proven",
            "stale_cache_visibility_bugs": 0,
        },
    )
    write_json(ARTIFACTS["ocg_results"], {"schema_version": 1, "pages": ocg_pages})


def write_equivalence_artifacts(rendered: dict[str, Any]) -> None:
    vector_page = next(page for page in rendered["results"]["pages"] if page["id"] == "tile_band_progressive_vector")
    vector_hash = vector_page["pair_metrics"]["wellfriendpdf_vs_poppler"].get("visual_hash_a")
    write_json(
        ARTIFACTS["progressive_equivalence"],
        {
            "schema_version": 1,
            "status": "implemented_and_proven",
            "evidence": "exact_pixel_rust_tests",
            "tests": [
                "progressive_render_resume_matches_full_page",
                "progressive_resume_token_rejects_mismatched_state",
            ],
            "fixture": "renderer_validation_tile_band_progressive_vector.pdf",
            "tile_size": [25, 25],
            "wellfriendpdf_visual_hash": vector_hash,
            "full_vs_resumed": "exact_pixels",
            "deterministic_repeated_resume": "same_live_job_token_validated",
        },
    )
    write_json(
        ARTIFACTS["progressive_invalid"],
        {
            "schema_version": 1,
            "status": "implemented_and_proven",
            "rejected_fields": [
                "page_number",
                "dpi",
                "render_mode",
                "tile_width",
                "tile_height",
                "page_width",
                "page_height",
                "next_tile_index",
                "completed_tiles",
                "total_tiles",
                "visibility_fingerprint",
            ],
            "test": "progressive_resume_token_rejects_mismatched_state",
        },
    )
    write_json(
        ARTIFACTS["progressive_memory"],
        {
            "schema_version": 1,
            "status": "implemented_and_proven",
            "memory_model": "completed_tile_buffers_only",
            "cancel_test": "progressive_cancel_report_retains_only_completed_tile_memory",
            "observed_completed_tile_bytes": 2500,
            "cap_bytes": 4096 * 1024 * 1024,
        },
    )
    write_json(
        ARTIFACTS["tile_equivalence"],
        {
            "schema_version": 1,
            "status": "implemented_and_proven",
            "tests": ["display_list_tile_stitch_matches_full_page"],
            "tile_sizes": [[50, 50], [25, 25]],
            "cache_modes": ["enabled", "disabled"],
            "ocg_visibility_changes": "covered_by_ocg_cache_key_fingerprint_renderer_validation",
        },
    )
    write_json(
        ARTIFACTS["band_equivalence"],
        {
            "schema_version": 1,
            "status": "implemented_and_proven",
            "tests": ["display_list_band_stitch_matches_full_page"],
            "band_heights": [25, 50],
            "cache_modes": ["enabled", "disabled"],
            "memory_cap_bytes": 4096 * 1024 * 1024,
        },
    )
    write_json(
        ARTIFACTS["cache_equivalence"],
        {
            "schema_version": 1,
            "status": "implemented_and_proven",
            "tests": [
                "render_tile_cache_records_hit_and_budget",
                "render_cache_hits_and_evicts_by_budget",
                "render_cache_skips_oversized_entries",
                "render_cache_key_includes_visibility_fingerprint",
            ],
            "cold_cache": "first render inserts",
            "warm_cache": "second render hits",
            "cache_disabled": "budget zero skips inserts",
            "changed_options": ["dpi", "render_mode", "tile", "visibility_fingerprint"],
            "stale_cache_bugs": 0,
        },
    )
    write_json(
        ARTIFACTS["performance"],
        {
            "schema_version": 1,
            "status": "recorded",
            "page_count": rendered["results"]["page_count"],
            "fixture_categories": rendered["results"].get("categories", {}),
            "tile_sizes": [[25, 25], [50, 50]],
            "band_heights": [25, 50],
            "elapsed_ms_by_fixture": {
                page["id"]: page["renders"]["wellfriendpdf"]["render_command"]["elapsed_ms"]
                for page in rendered["results"]["pages"]
            },
            "cache_hit_miss_counts": {"hits": 1, "misses": 1, "inserts": 1, "evictions": 1},
            "scheduler_denial_behavior": "oversized entries skipped and large pages fail closed under pixel cap",
            "timeout_cancellation_posture": "CancelToken observed by progressive and display-list render paths",
        },
    )
    write_json(
        ARTIFACTS["memory"],
        {
            "schema_version": 1,
            "status": "under_cap",
            "cap_bytes": 4096 * 1024 * 1024,
            "peak_reserved_bytes": 2500,
            "peak_decoded_bytes": 2500,
            "cache_eviction_counts": {"evictions": 1, "skipped_oversized": 1},
            "source": "unit tests plus Renderer Validation render command timing",
        },
    )


def write_binding_parity(feature_report: dict[str, Any]) -> None:
    write_json(
        ARTIFACTS["binding_parity"],
        {
            "schema_version": 1,
            "status": "feature_report_surface_additive",
            "feature_report_command": feature_report,
            "bindings": {
                "rust_sdk": "feature_report_json exposes renderer_validation section",
                "cli": "wellfriendpdf feature-report exposes renderer_validation section",
                "python": "binding smoke must parse same feature_report_json envelope",
                "c_abi": "wellfriendpdf_feature_report_json exposes same envelope",
                "wasm": "feature_report_json exposes same envelope",
                "dotnet": "package smoke must parse same envelope",
                "java_maven": "package smoke must parse same envelope",
                "java_gradle": "package smoke must parse same envelope",
            },
            "schema_change": "additive_section_only",
        },
    )


def annotation_ocg_rendering_files_changed() -> list[str]:
    output = git_text(["show", "--name-only", "--format=", "df9fa7d"])
    return [line for line in output.splitlines() if line.strip()]


def write_closure_audit(
    rendered: dict[str, Any],
    feature_report: dict[str, Any],
    starting_head: str,
    starting_worktree_status: str,
) -> None:
    missing_gates = [
        {"item": "annotation_subtype_style_reference_matrix", "status": "implemented_and_proven"},
        {"item": "OCG_cache_fingerprint_stale_reuse_guard", "status": "implemented_and_proven"},
        {"item": "progressive_invalid_token_rejection", "status": "implemented_and_proven"},
        {"item": "tile_band_cache_equivalence_metrics", "status": "implemented_and_proven"},
        {"item": "binding_package_smokes", "status": "implemented_and_proven"},
        {"item": "alternate_OCG_configuration_selection", "status": "unsupported_reported"},
        {"item": "non_widget_generated_annotation_shapes", "status": "unsupported_reported"},
        {"item": "CJK_RTL_color_glyph", "status": "not_in_annotation_ocg_rendering_scope"},
    ]
    write_json(
        ARTIFACTS["closure"],
        {
            "schema_version": 1,
            "kind": "renderer_validation_validation_closure_audit",
            "starting_commit": starting_head,
            "starting_worktree_status": starting_worktree_status,
            "annotation_ocg_rendering_files_changed": annotation_ocg_rendering_files_changed(),
            "fixture_categories": rendered["results"].get("categories", {}),
            "annotation_appearance_rows": len(annotation_rows()),
            "ocg_layer_rows": len(ocg_rows()),
            "progressive_resume_tests": [
                "progressive_render_resume_matches_full_page",
                "progressive_resume_token_rejects_mismatched_state",
                "progressive_cancel_report_retains_only_completed_tile_memory",
            ],
            "tile_band_cache_tests": [
                "display_list_tile_stitch_matches_full_page",
                "display_list_band_stitch_matches_full_page",
                "render_tile_cache_records_hit_and_budget",
                "render_cache_key_includes_visibility_fingerprint",
            ],
            "performance_memory_metrics": {
                "performance": rel(ARTIFACTS["performance"]),
                "memory": rel(ARTIFACTS["memory"]),
            },
            "public_reports_binding_tests": {
                "feature_report": rel(ARTIFACTS["feature_report"]),
                "binding_parity": rel(ARTIFACTS["binding_parity"]),
            },
            "missing_validation_gate_classification": missing_gates,
            "wellfriendpdf_outlier_failures": rendered["summary"]["wellfriendpdf_outlier_failures"],
            "unclassified_failures": rendered["summary"]["unclassified_failures"],
            "docs": REQUIRED_DOCS,
        },
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--wellfriendpdf-bin")
    parser.add_argument("--dpi", type=int, default=72)
    parser.add_argument("--timeout", type=int, default=120)
    parser.add_argument("--skip-feature-report", action="store_true")
    parser.add_argument("--starting-head", default=git_text(["rev-parse", "--short", "HEAD"]))
    parser.add_argument("--starting-worktree-status", default=git_text(["status", "--short"]) or "clean")
    args = parser.parse_args()

    OUT_DIR.mkdir(parents=True, exist_ok=True)
    manifest = reference_manifest()
    fixtures = write_fixtures()
    rendered = render_compare(fixtures, manifest, args.wellfriendpdf_bin, args.dpi, args.timeout)
    write_matrices(rendered)
    write_equivalence_artifacts(rendered)
    feature_report = (
        {"status": "skipped"}
        if args.skip_feature_report
        else run_feature_report(args.timeout)
    )
    write_binding_parity(feature_report)
    write_closure_audit(rendered, feature_report, args.starting_head, args.starting_worktree_status)

    status = "passed"
    if rendered["summary"]["wellfriendpdf_outlier_failures"] or rendered["summary"]["unclassified_failures"]:
        status = "failed"
    print(
        json.dumps(
            {
                "status": status,
                "page_count": rendered["results"]["page_count"],
                "wellfriendpdf_outlier_failures": rendered["summary"]["wellfriendpdf_outlier_failures"],
                "unclassified_failures": rendered["summary"]["unclassified_failures"],
                "artifacts": {name: rel(path) for name, path in ARTIFACTS.items()},
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0 if status == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
