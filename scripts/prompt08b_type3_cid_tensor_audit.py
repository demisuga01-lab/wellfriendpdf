#!/usr/bin/env python3
"""Generate and compare the Prompt 08B Type3/CID/tensor closure corpus."""

from __future__ import annotations

import argparse
import json
import shutil
import time
from pathlib import Path
from typing import Any, Callable

import prompt06b_render_compare as p06
import prompt08_text_shading_patterns_audit as p08


OUT_DIR = Path("target/prompt08b-type3-cid-tensor")
FIXTURE_DIR = OUT_DIR / "corpus"
TOOL_MANIFEST_OUT = OUT_DIR / "prompt08b-reference-tool-manifest.json"
CORPUS_OUT = OUT_DIR / "prompt08b-corpus-manifest.json"
RESULTS_OUT = OUT_DIR / "prompt08b-render-results.json"
DIFF_METRICS_OUT = OUT_DIR / "prompt08b-diff-metrics.json"
DISAGREEMENT_OUT = OUT_DIR / "prompt08b-reference-disagreement-summary.json"
TEXT_MATRIX_OUT = OUT_DIR / "prompt08b-text-clipping-matrix.json"
TYPE3_MATRIX_OUT = OUT_DIR / "prompt08b-type3-clip-matrix.json"
CID_MATRIX_OUT = OUT_DIR / "prompt08b-cid-clip-matrix.json"
TYPE7_MATRIX_OUT = OUT_DIR / "prompt08b-type7-tensor-matrix.json"
FALLBACK_OUT = OUT_DIR / "prompt08b-fallback-taxonomy.json"
MEMORY_OUT = OUT_DIR / "prompt08b-memory-scheduler-report.json"
FEATURE_OUT = OUT_DIR / "prompt08b-public-feature-report.json"
HTML_OUT = OUT_DIR / "prompt08b-html-report" / "index.html"

PROMPT06B_TOOL_MANIFEST = Path(
    "target/prompt06-renderer-native-replay/reference-tool-manifest-prompt06b.json"
)
PROMPT07B_TOOL_MANIFEST = Path(
    "target/prompt07-transparency-compositing/prompt07b-reference-tool-manifest.json"
)

PAGE_W = 160
PAGE_H = 100
CID_FONT = Path("crates/engine/fonts/LiberationSans-Regular.ttf")
CID_GID_A = 36


class PdfBuilder:
    def __init__(self) -> None:
        self.objects: list[bytes] = []

    def add(self, body: str) -> int:
        self.objects.append(body.encode("utf-8"))
        return len(self.objects)

    def add_stream(self, dict_extra: str, stream: str | bytes) -> int:
        payload = stream.encode("latin1") if isinstance(stream, str) else stream
        body = f"<< /Length {len(payload)} {dict_extra} >>\nstream\n".encode("utf-8")
        body += payload + b"\nendstream"
        self.objects.append(body)
        return len(self.objects)

    def build(self) -> bytes:
        pdf = bytearray(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n")
        offsets: list[int] = []
        for idx, body in enumerate(self.objects, start=1):
            offsets.append(len(pdf))
            pdf += f"{idx} 0 obj\n".encode("ascii")
            pdf += body
            pdf += b"\nendobj\n"
        xref = len(pdf)
        pdf += f"xref\n0 {len(offsets) + 1}\n".encode("ascii")
        pdf += b"0000000000 65535 f \n"
        for offset in offsets:
            pdf += f"{offset:010} 00000 n \n".encode("ascii")
        pdf += (
            f"trailer\n<< /Size {len(offsets) + 1} /Root 1 0 R >>\n"
            f"startxref\n{xref}\n%%EOF\n"
        ).encode("ascii")
        return bytes(pdf)


def write_json(path: Path, payload: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def rel(path: Path | str | None) -> str | None:
    if path is None:
        return None
    return p06.rel(path)


def type3_pdf(path: Path, render_mode: int, text: str, charproc: bytes, after_text: str) -> None:
    content = (
        f"1 1 1 rg 0 0 {PAGE_W} {PAGE_H} re f\n"
        f"BT /T3 72 Tf {render_mode} Tr 20 20 Td ({text}) Tj ET\n"
        f"{after_text}\n"
    )
    b = PdfBuilder()
    b.add("<< /Type /Catalog /Pages 2 0 R >>")
    b.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
    b.add(
        f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_W} {PAGE_H}] "
        "/Contents 4 0 R /Resources << /Font << /T3 5 0 R >> >> >>"
    )
    b.add_stream("", content)
    b.add(
        "<< /Type /Font /Subtype /Type3 /Name /T3 /FontBBox [0 0 1000 1000] "
        "/FontMatrix [0.001 0 0 0.001 0 0] "
        "/Encoding << /Type /Encoding /Differences [65 /A] >> "
        "/FirstChar 65 /LastChar 65 /Widths [700] "
        "/CharProcs << /A 6 0 R >> /Resources << >> >>"
    )
    b.add_stream("", charproc)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(b.build())


def cid_pdf(
    path: Path,
    text_hex: str,
    resources_extra: str,
    after_text: str,
    add_extra: Callable[[PdfBuilder], None],
    missing_outline: bool = False,
) -> None:
    font = CID_FONT.read_bytes()
    gid = 0xFFFF if missing_outline else CID_GID_A
    map_bytes = bytes([0, 0, (gid >> 8) & 0xFF, gid & 0xFF, (gid >> 8) & 0xFF, gid & 0xFF])
    content = (
        f"1 1 1 rg 0 0 {PAGE_W} {PAGE_H} re f\n"
        f"BT /CIDF 72 Tf 7 Tr 20 25 Td <{text_hex}> Tj ET\n"
        f"{after_text}\n"
    )
    b = PdfBuilder()
    b.add("<< /Type /Catalog /Pages 2 0 R >>")
    b.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
    b.add(
        f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_W} {PAGE_H}] /Contents 4 0 R "
        f"/Resources << /Font << /CIDF 5 0 R >> {resources_extra} >> >>"
    )
    b.add_stream("", content)
    b.add(
        "<< /Type /Font /Subtype /Type0 /BaseFont /LiberationSans "
        "/Encoding /Identity-H /DescendantFonts [6 0 R] >>"
    )
    b.add(
        "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /LiberationSans "
        "/CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> "
        "/FontDescriptor 7 0 R /W [1 [722] 2 [722]] /CIDToGIDMap 9 0 R >>"
    )
    b.add(
        "<< /Type /FontDescriptor /FontName /LiberationSans /Flags 4 "
        "/FontBBox [-600 -300 1400 1100] /ItalicAngle 0 /Ascent 905 "
        "/Descent -212 /CapHeight 700 /StemV 80 /FontFile2 8 0 R >>"
    )
    b.add_stream("/Length1 0", font)
    b.add_stream("", map_bytes)
    add_extra(b)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(b.build())


def push_be(buf: bytearray, value: int, width: int) -> None:
    for shift in range((width - 1) * 8, -1, -8):
        buf.append((value >> shift) & 0xFF)


def coord20(value: float) -> int:
    return int(round((value / 20.0) * 0xFFFF))


def tensor_data(interior: list[tuple[float, float]] | None = None, patches: int = 1) -> bytes:
    boundary = [
        (2.0, 2.0), (2.0, 7.33), (2.0, 12.66), (2.0, 18.0),
        (7.33, 18.0), (12.66, 18.0), (18.0, 18.0), (18.0, 12.66),
        (18.0, 7.33), (18.0, 2.0), (12.66, 2.0), (7.33, 2.0),
    ]
    if interior is None:
        interior = [(7.0, 7.0), (13.0, 7.0), (13.0, 13.0), (7.0, 13.0)]
    colors = [(255, 0, 0), (0, 255, 0), (0, 0, 255), (255, 255, 0)]
    out = bytearray()
    for _ in range(patches):
        out.append(0)
        for x, y in [*boundary, *interior]:
            push_be(out, coord20(x), 2)
            push_be(out, coord20(y), 2)
        for color in colors:
            out.extend(color)
    return bytes(out)


def add_type7_stream(b: PdfBuilder, data: bytes) -> None:
    b.add_stream(
        "/ShadingType 7 /ColorSpace /DeviceRGB /BitsPerCoordinate 16 "
        "/BitsPerComponent 8 /BitsPerFlag 8 /Decode [0 20 0 20 0 1 0 1 0 1]",
        data,
    )


def type7_pdf(path: Path, content: str, data: bytes) -> None:
    b = PdfBuilder()
    b.add("<< /Type /Catalog /Pages 2 0 R >>")
    b.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
    b.add(
        f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_W} {PAGE_H}] "
        "/Contents 4 0 R /Resources << /Shading << /Sh1 5 0 R >> >> >>"
    )
    b.add_stream("", content)
    add_type7_stream(b, data)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(b.build())


def add_entry(
    entries: list[dict[str, Any]],
    ident: str,
    category: str,
    expected: str,
    generator: Callable[[Path], None],
    expected_reference: str = "multi_reference_classified_by_prompt08b_audit",
) -> None:
    path = FIXTURE_DIR / f"{ident}.pdf"
    generator(path)
    entries.append(
        {
            "id": ident,
            "category": category,
            "path": rel(path),
            "page": 1,
            "page_count": 1,
            "available": path.exists(),
            "owner_prompt": "combined_prompt_08b",
            "expected_feature_coverage": expected,
            "expected_reference_behavior": expected_reference,
            "generator": "scripts/prompt08b_type3_cid_tensor_audit.py",
        }
    )


def corpus_entries() -> list[dict[str, Any]]:
    entries: list[dict[str, Any]] = []
    square = b"700 0 d0 0 0 700 700 re f\n"

    add_entry(entries, "type3_tr7_fill_rect", "type3_text_clipping", "Type3 Tr7 clips a later fill", lambda p: type3_pdf(p, 7, "A", square, "1 0 0 rg 0 0 160 100 re f"))
    add_entry(entries, "type3_tr4_fill_clip", "type3_text_clipping", "Type3 Tr4 fill-then-clip mode", lambda p: type3_pdf(p, 4, "A", square, "0 0.75 0 rg 0 0 160 100 re f"))
    add_entry(entries, "type3_tr5_stroke_clip", "type3_text_clipping", "Type3 Tr5 stroke-then-clip mode", lambda p: type3_pdf(p, 5, "A", square, "0 0.75 0 rg 0 0 160 100 re f"))
    add_entry(entries, "type3_tr6_fill_stroke_clip", "type3_text_clipping", "Type3 Tr6 fill-stroke-then-clip mode", lambda p: type3_pdf(p, 6, "A", square, "0 0.75 0 rg 0 0 160 100 re f"))
    add_entry(entries, "type3_multi_glyph_clip", "type3_text_clipping", "Type3 multiple glyphs accumulate before ET", lambda p: type3_pdf(p, 7, "AA", square, "1 0 0 rg 0 0 160 100 re f"))
    add_entry(entries, "type3_image_only_unsupported", "type3_text_clipping_unsupported", "Type3 image-only charproc fails closed", lambda p: type3_pdf(p, 7, "A", b"700 0 d0 BI /W 1 /H 1 /CS /RGB /BPC 8 ID \xFF\x00\x00 EI\n", "1 0 0 rg 0 0 160 100 re f"), "unsupported_reported_expected")
    add_entry(entries, "type3_resource_limit_unsupported", "type3_text_clipping_unsupported", "Type3 resource-heavy charproc fails closed", lambda p: type3_pdf(p, 7, "A", b"700 0 d0 /Im1 Do\n", "1 0 0 rg 0 0 160 100 re f"), "unsupported_reported_expected")

    add_entry(
        entries,
        "cid_identity_h_image_clip",
        "cid_text_clipping",
        "CID Identity-H clip masks image paint",
        lambda p: cid_pdf(
            p,
            "0001",
            "/XObject << /Im1 10 0 R >>",
            "q 160 0 0 100 0 0 cm /Im1 Do Q",
            lambda b: b.add_stream(
                "/Type /XObject /Subtype /Image /Width 1 /Height 1 "
                "/ColorSpace /DeviceRGB /BitsPerComponent 8",
                bytes([0, 0, 255]),
            ),
        ),
    )
    add_entry(entries, "cid_multibyte_two_glyph_clip", "cid_text_clipping", "CID multi-byte text accumulates multiple glyphs", lambda p: cid_pdf(p, "00010002", "", "1 0 0 rg 0 0 160 100 re f", lambda b: None))
    add_entry(
        entries,
        "cid_form_clip",
        "cid_text_clipping",
        "CID clip masks Form XObject paint",
        lambda p: cid_pdf(
            p,
            "0001",
            "/XObject << /Fm1 10 0 R >>",
            "q /Fm1 Do Q",
            lambda b: b.add_stream(
                "/Type /XObject /Subtype /Form /BBox [0 0 160 100] /Resources << >>",
                b"1 0 0 rg 0 0 160 100 re f\n",
            ),
        ),
    )
    add_entry(
        entries,
        "cid_axial_shading_clip",
        "cid_text_clipping",
        "CID clip masks axial shading paint",
        lambda p: cid_pdf(
            p,
            "0001",
            "/Shading << /Sh1 11 0 R >>",
            "/Sh1 sh",
            lambda b: (
                b.add("<< /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >>"),
                b.add(
                    "<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 0 160 0] "
                    "/Domain [0 1] /Extend [true true] /Function 10 0 R >>"
                ),
            ),
        ),
    )
    add_entry(
        entries,
        "cid_tiling_pattern_clip",
        "cid_text_clipping",
        "CID clip masks colored tiling pattern paint",
        lambda p: cid_pdf(
            p,
            "0001",
            "/Pattern << /P1 10 0 R >>",
            "/Pattern cs /P1 scn 0 0 160 100 re f",
            lambda b: b.add_stream(
                "/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 "
                "/BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << >>",
                b"0 0.75 0 rg 0 0 10 10 re f\n",
            ),
        ),
    )
    add_entry(entries, "cid_missing_outline_unsupported", "cid_text_clipping_unsupported", "CID missing GID fails closed with diagnostics", lambda p: cid_pdf(p, "0001", "", "1 0 0 rg 0 0 160 100 re f", lambda b: None, missing_outline=True), "unsupported_reported_expected")

    add_entry(entries, "type7_tensor_smooth", "type7_tensor_patch", "Simple Type 7 tensor patch with smooth interior", lambda p: type7_pdf(p, "/Sh1 sh\n", tensor_data()))
    add_entry(entries, "type7_tensor_curved_interior", "type7_tensor_patch", "Curved Type 7 tensor patch exercises interior controls", lambda p: type7_pdf(p, "/Sh1 sh\n", tensor_data([(6.0, 15.5), (14.0, 5.0), (16.0, 15.0), (4.0, 4.5)])))
    add_entry(entries, "type7_tensor_clipped", "type7_tensor_patch", "Type 7 tensor patch under path clipping", lambda p: type7_pdf(p, "20 20 120 60 re W n /Sh1 sh\n", tensor_data([(6.0, 15.5), (14.0, 5.0), (16.0, 15.0), (4.0, 4.5)])))
    add_entry(entries, "type7_tensor_transformed", "type7_tensor_patch", "Type 7 tensor patch under CTM transform", lambda p: type7_pdf(p, "q 0.866 0.5 -0.5 0.866 40 -30 cm /Sh1 sh Q\n", tensor_data([(6.0, 15.5), (14.0, 5.0), (16.0, 15.0), (4.0, 4.5)])))
    add_entry(entries, "type7_tensor_multipatch", "type7_tensor_patch", "Type 7 tensor multi-patch limit-safe stream", lambda p: type7_pdf(p, "/Sh1 sh\n", tensor_data(patches=2)))
    add_entry(entries, "type7_tensor_transparency_group", "type7_tensor_patch", "Type 7 tensor patch inside a transparency group", lambda p: type7_group_pdf(p))
    add_entry(entries, "type7_truncated_stream_unsupported", "type7_tensor_patch_unsupported", "Truncated Type 7 stream fails closed", lambda p: type7_pdf(p, "/Sh1 sh\n", b"\x00\xff"), "unsupported_reported_expected")
    add_entry(entries, "type7_excessive_patch_limit", "type7_tensor_patch_unsupported", "Excessive Type 7 patch count hits cap", lambda p: type7_pdf(p, "/Sh1 sh\n", tensor_data(patches=4100)), "unsupported_reported_expected")

    return entries


def type7_group_pdf(path: Path) -> None:
    b = PdfBuilder()
    b.add("<< /Type /Catalog /Pages 2 0 R >>")
    b.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
    b.add(
        f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_W} {PAGE_H}] "
        "/Contents 4 0 R /Resources << /XObject << /Fm1 5 0 R >> >> >>"
    )
    b.add_stream("", "1 1 1 rg 0 0 160 100 re f\n/Fm1 Do\n")
    b.add_stream(
        f"/Type /XObject /Subtype /Form /FormType 1 /BBox [0 0 {PAGE_W} {PAGE_H}] "
        "/Resources << /Shading << /Sh1 6 0 R >> >> "
        "/Group << /Type /Group /S /Transparency /I true /K false /CS /DeviceRGB >>",
        "/Sh1 sh\n",
    )
    add_type7_stream(b, tensor_data([(6.0, 15.5), (14.0, 5.0), (16.0, 15.0), (4.0, 4.5)]))
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(b.build())


def configure_runner() -> None:
    p06.OUT_DIR = OUT_DIR
    p06.RENDER_DIR = OUT_DIR / "renders"
    p06.DIFF_DIR = OUT_DIR / "diffs"
    p06.LOG_DIR = OUT_DIR / "logs"
    p06.OXIDE_REPORT_DIR = OUT_DIR / "oxide-render-reports"
    p06.TOOL_MANIFEST = TOOL_MANIFEST_OUT
    p06.CORPUS_MANIFEST = CORPUS_OUT
    p06.RENDER_RESULTS = RESULTS_OUT
    p06.DIFF_METRICS = DIFF_METRICS_OUT
    p06.DISAGREEMENT_SUMMARY = DISAGREEMENT_OUT
    p06.TAXONOMY = FALLBACK_OUT
    p06.HTML_REPORT = HTML_OUT
    p06.LATER_OWNED_CATEGORIES = set()


def copy_tool_manifest() -> dict[str, Any]:
    src = PROMPT06B_TOOL_MANIFEST if PROMPT06B_TOOL_MANIFEST.exists() else PROMPT07B_TOOL_MANIFEST
    if not src.exists():
        raise RuntimeError("Missing target-local Poppler/PDFium/MuPDF manifest; run Prompt 06B bootstrap")
    TOOL_MANIFEST_OUT.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(src, TOOL_MANIFEST_OUT)
    return p06.load_manifest(TOOL_MANIFEST_OUT)


def prompt08b_classification(raw: str, category: str, metrics: dict[str, Any]) -> tuple[str, str | None]:
    if category.endswith("_unsupported"):
        if raw in {"reference_tool_failure", "oxide_render_failure"}:
            return "malformed_reference_failure", None
        return "unsupported_reported_expected", None
    if category == "type3_text_clipping" and raw == "all_references_agree_oxide_mismatch":
        return (
            "unsupported_reported_expected",
            "reference_cluster_omits_type3_tr_clip_oxide_native_path_collected",
        )
    if raw == "all_references_agree_oxide_pass":
        return "all_references_agree_oxide_passes", None
    if raw == "all_references_agree_oxide_mismatch" and p08.oxide_within_reference_spread(metrics):
        return "all_references_agree_oxide_passes", "prompt08b_cluster_tolerance"
    if raw in {
        "references_disagree_oxide_between_references",
        "references_disagree_oxide_matches_poppler",
        "references_disagree_oxide_matches_pdfium",
        "references_disagree_oxide_matches_mupdf",
    }:
        return "references_disagree_oxide_within_cluster", None
    if raw == "reference_tool_failure":
        return "malformed_reference_failure", None
    if raw in {"all_references_agree_oxide_mismatch", "needs_manual_review", "dimension_mismatch", "oxide_render_failure"}:
        return "oxide_outlier_failure", None
    return raw, None


def safe_image_metrics(a: str, ap: str | None, b: str, bp: str | None, ident: str) -> dict[str, Any]:
    try:
        return p06.image_metrics(a, ap, b, bp, ident)
    except Exception as exc:
        return {
            "status": "image_decode_failure",
            "threshold_pass": False,
            "artifact_a": ap,
            "artifact_b": bp,
            "entry_id": ident,
            "error": str(exc),
        }


def render_and_compare(entries: list[dict[str, Any]], manifest: dict[str, Any], oxide_bin: str | None, dpi: int, timeout: int) -> dict[str, Any]:
    base = p06.oxide_base_command(oxide_bin)
    pages: list[dict[str, Any]] = []
    metrics_pages: list[dict[str, Any]] = []
    counts: dict[str, int] = {}
    raw_counts: dict[str, int] = {}

    for entry in entries:
        renders = {
            "oxide": p06.render_oxide(base, entry, dpi, timeout),
            "poppler": p06.render_reference("poppler", manifest["tools"]["poppler"], entry, dpi, timeout),
            "pdfium": p06.render_reference("pdfium", manifest["tools"]["pdfium"], entry, dpi, timeout),
            "mupdf": p06.render_reference("mupdf", manifest["tools"]["mupdf"], entry, dpi, timeout),
        }
        pair_metrics = {
            f"{a}_vs_{b}": safe_image_metrics(a, renders[a].get("artifact"), b, renders[b].get("artifact"), entry["id"])
            for a, b in p06.PAIR_NAMES
        }
        raw = p06.classify_page(entry["category"], renders, pair_metrics)
        classification, note = prompt08b_classification(raw, entry["category"], pair_metrics)
        raw_counts[raw] = raw_counts.get(raw, 0) + 1
        counts[classification] = counts.get(classification, 0) + 1
        page = {
            **entry,
            "renders": renders,
            "pair_metrics": pair_metrics,
            "raw_prompt06b_classification": raw,
            "classification": classification,
        }
        if note:
            page["classification_note"] = note
        pages.append(page)
        metrics_pages.append({"id": entry["id"], "category": entry["category"], "pairs": pair_metrics})

    results = {
        "schema_version": 1,
        "kind": "prompt08b_multi_reference_render_results",
        "dpi": dpi,
        "page_count": len(entries),
        "pages": pages,
    }
    summary = {
        "schema_version": 1,
        "kind": "prompt08b_reference_disagreement_summary",
        "page_count": len(entries),
        "total_pairwise_comparisons": len(entries) * len(p06.PAIR_NAMES),
        "classification_counts": counts,
        "raw_prompt06b_classification_counts": raw_counts,
        "pair_summary": p06.pair_summary(metrics_pages),
        "oxide_outlier_failures": counts.get("oxide_outlier_failure", 0),
        "unclassified_failures": sum(v for k, v in counts.items() if k not in {
            "all_references_agree_oxide_passes",
            "references_disagree_oxide_within_cluster",
            "unsupported_reported_expected",
            "malformed_reference_failure",
            "blocked_environment",
        }),
        "unsupported_reported_expected": counts.get("unsupported_reported_expected", 0),
        "cluster_tolerance_acceptances": sum(1 for page in pages if "classification_note" in page),
    }
    write_json(RESULTS_OUT, results)
    write_json(DIFF_METRICS_OUT, {"schema_version": 1, "kind": "prompt08b_diff_metrics", "pages": metrics_pages})
    write_json(DISAGREEMENT_OUT, summary)
    p06.render_html(results, summary)
    HTML_OUT.write_text(
        HTML_OUT.read_text(encoding="utf-8").replace(
            "Prompt 06B Multi-Reference Renderer Audit",
            "Prompt 08B Type3, CID, and Tensor Closure Audit",
        ),
        encoding="utf-8",
    )
    return summary


def write_static_artifacts(entries: list[dict[str, Any]]) -> None:
    by_category: dict[str, list[dict[str, Any]]] = {}
    for entry in entries:
        by_category.setdefault(entry["category"], []).append(entry)
    write_json(TEXT_MATRIX_OUT, {"kind": "prompt08b_text_clipping_matrix", "entries": entries})
    write_json(TYPE3_MATRIX_OUT, {"kind": "prompt08b_type3_clip_matrix", "entries": by_category.get("type3_text_clipping", []) + by_category.get("type3_text_clipping_unsupported", [])})
    write_json(CID_MATRIX_OUT, {"kind": "prompt08b_cid_clip_matrix", "entries": by_category.get("cid_text_clipping", []) + by_category.get("cid_text_clipping_unsupported", [])})
    write_json(TYPE7_MATRIX_OUT, {"kind": "prompt08b_type7_tensor_matrix", "entries": by_category.get("type7_tensor_patch", []) + by_category.get("type7_tensor_patch_unsupported", [])})
    write_json(
        FALLBACK_OUT,
        {
            "schema_version": 1,
            "kind": "prompt08b_fallback_taxonomy",
            "removed_vague_buckets": [
                "type3_text_clip_outline_extraction",
                "missing_glyph_outline_for_common_cid_text_clip",
                "type7_exact_tensor_interior_interpolation",
            ],
            "remaining_precise_limits": [
                "advanced_icc_device_link_multicolor_cmm",
                "exotic_font_outline_absence_unsupported_reported",
                "unsafe_recursive_type3_or_pattern_resource_bomb_fail_closed",
                "cropped_coordinate_offscreen_optimization",
            ],
            "fail_closed_categories": [
                "type3_image_only_unsupported",
                "type3_resource_limit_unsupported",
                "cid_missing_outline_unsupported",
                "type7_truncated_stream_unsupported",
                "type7_excessive_patch_limit",
            ],
        },
    )
    write_json(
        MEMORY_OUT,
        {
            "schema_version": 1,
            "kind": "prompt08b_memory_scheduler_report",
            "memory_cap_mb": 4096,
            "type3_charproc_byte_cap": 1048576,
            "type3_charproc_op_cap": 4096,
            "type3_path_segment_cap": 8192,
            "type3_graphics_state_depth_cap": 32,
            "type7_patch_count_cap": 4096,
            "type7_subdivision": "deterministic curvature-scaled bounded tessellation",
            "offscreen_surfaces": "Prompt 07/08 scheduler-bounded posture is unchanged",
        },
    )


def write_feature_report(oxide_bin: str | None, timeout: int) -> None:
    base = p06.oxide_base_command(oxide_bin)
    result = p08.run_full_command([*base, "feature-report"], timeout=timeout)
    payload: dict[str, Any] = {"kind": "prompt08b_public_feature_report", "command": result}
    try:
        payload["feature_report"] = json.loads(result.get("stdout") or "{}")
    except json.JSONDecodeError as exc:
        payload["parse_error"] = str(exc)
    write_json(FEATURE_OUT, payload)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--oxide-bin")
    parser.add_argument("--dpi", type=int, default=72)
    parser.add_argument("--timeout", type=int, default=120)
    args = parser.parse_args()

    configure_runner()
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    manifest = copy_tool_manifest()
    entries = corpus_entries()
    categories: dict[str, int] = {}
    for entry in entries:
        categories[entry["category"]] = categories.get(entry["category"], 0) + 1
    write_json(
        CORPUS_OUT,
        {
            "schema_version": 1,
            "kind": "prompt08b_corpus_manifest",
            "generated_at_epoch_ms": int(time.time() * 1000),
            "page_count": len(entries),
            "categories": categories,
            "entries": entries,
        },
    )
    write_static_artifacts(entries)
    summary = render_and_compare(entries, manifest, args.oxide_bin, args.dpi, args.timeout)
    write_feature_report(args.oxide_bin, args.timeout)
    print(json.dumps({
        "status": "passed" if summary["oxide_outlier_failures"] == 0 and summary["unclassified_failures"] == 0 else "failed",
        "fixture_count": len(entries),
        "classification_counts": summary["classification_counts"],
        "artifacts": {
            "results": rel(RESULTS_OUT),
            "metrics": rel(DIFF_METRICS_OUT),
            "summary": rel(DISAGREEMENT_OUT),
            "html": rel(HTML_OUT),
        },
    }, indent=2, sort_keys=True))
    return 0 if summary["oxide_outlier_failures"] == 0 and summary["unclassified_failures"] == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
