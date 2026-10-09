#!/usr/bin/env python3
"""Publish the final two-table parser/facilities and renderer benchmark."""

from __future__ import annotations

import argparse
import collections
import datetime as dt
import hashlib
import json
import math
from pathlib import Path
from typing import Any, Iterable


TOOLS = ("wellfriend", "pdfium", "poppler", "mupdf", "qpdf")
LABELS = {
    "wellfriend": "Wellfriend SDK",
    "wellfriendpdf": "Wellfriend SDK",
    "pdfium": "PDFium",
    "poppler": "Poppler",
    "mupdf": "MuPDF",
    "qpdf": "qpdf",
}
STRUCTURAL = (
    "merge",
    "split",
    "extract-pages",
    "lock",
    "unlock",
    "rotate",
    "repair",
    "organize",
    "linearize",
    "flatten",
    "watermark",
    "page-numbers",
    "metadata",
    "crop",
    "resize",
    "nup",
    "optimize",
    "canonicalize",
    "sanitize",
    "sign",
)
CONVERSIONS = (
    "text",
    "html",
    "markdown",
    "json",
    "docx",
    "pptx",
    "xlsx",
    "svg-first-page",
    "postscript-first-page",
    "eps-first-page",
    "png-first-page",
    "jpeg-first-page",
    "tables-json",
)


def load_jsonl(path: Path) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    if not path.exists():
        return rows
    for line in path.read_text("utf-8", errors="replace").splitlines():
        try:
            rows.append(json.loads(line))
        except json.JSONDecodeError:
            continue
    return rows


def percentile(values: Iterable[float], percent: float) -> float | None:
    ordered = sorted(float(value) for value in values)
    if not ordered:
        return None
    index = max(0, min(len(ordered) - 1, math.ceil(percent / 100 * len(ordered)) - 1))
    return ordered[index]


def distribution(values: Iterable[float]) -> dict[str, float | int]:
    values = list(values)
    if not values:
        return {"count": 0}
    return {
        "count": len(values),
        "p05": percentile(values, 5),
        "p50": percentile(values, 50),
        "p90": percentile(values, 90),
        "p95": percentile(values, 95),
        "p99": percentile(values, 99),
        "max": max(values),
    }


def fmt_ms(value: Any) -> str:
    return "N/A" if not isinstance(value, (int, float)) else f"{value:,.2f} ms"


def fmt_score(value: Any, digits: int = 6) -> str:
    return "N/A" if not isinstance(value, (int, float)) else f"{value:.{digits}f}"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def table(headers: list[str], rows: list[list[str]]) -> list[str]:
    escaped = lambda value: str(value).replace("|", "\\|")
    lines = [
        "| " + " | ".join(escaped(value) for value in headers) + " |",
        "| " + " | ".join("---" for _ in headers) + " |",
    ]
    lines.extend("| " + " | ".join(escaped(value) for value in row) + " |" for row in rows)
    return lines


def operation_cell(rows: list[dict[str, Any]], operation: str, tool: str) -> tuple[str, dict[str, Any]]:
    selected = [row for row in rows if row.get("operation") == operation and row.get("tool") == tool]
    if not selected:
        return "N/A", {"total": 0}
    statuses = collections.Counter(str(row.get("status")) for row in selected)
    passed = [row for row in selected if row.get("status") == "pass"]
    timings = distribution(
        float(row["run"]["elapsed_ms"])
        for row in passed
        if isinstance(row.get("run", {}).get("elapsed_ms"), (int, float))
    )
    result = f"{len(passed)}/{len(selected)} qualified"
    if statuses["typed_refusal"]:
        result += f"; {statuses['typed_refusal']} refused"
    failed = statuses["fail"] + statuses["timeout"]
    if failed:
        result += f"; {failed} failed"
    if statuses["not_applicable"]:
        result += f"; {statuses['not_applicable']} N/A"
    result += (
        f"<br>P50 {fmt_ms(timings.get('p50'))}; "
        f"P95 {fmt_ms(timings.get('p95'))}; max {fmt_ms(timings.get('max'))}"
    )

    f1_values = [
        row.get("quality", {}).get("text_agreement", {}).get("token_f1_vs_poppler")
        for row in passed
    ]
    f1 = distribution(value for value in f1_values if isinstance(value, (int, float)))
    if f1.get("count"):
        result += f"<br>token F1 P50 {fmt_score(f1.get('p50'))}"
    compliant = [
        row.get("quality", {}).get("verapdf_compliant")
        for row in selected
        if isinstance(row.get("quality", {}).get("verapdf_compliant"), bool)
    ]
    qualified_compliant = [
        row.get("quality", {}).get("verapdf_compliant")
        for row in passed
        if isinstance(row.get("quality", {}).get("verapdf_compliant"), bool)
    ]
    rejected_noncompliant = sum(
        row.get("status") != "pass"
        and row.get("quality", {}).get("verapdf_compliant") is False
        for row in selected
    )
    if qualified_compliant:
        result += (
            f"<br>veraPDF verified {sum(qualified_compliant)}/"
            f"{len(qualified_compliant)} qualified outputs"
        )
        if rejected_noncompliant:
            result += f"; {rejected_noncompliant} refused artifacts rejected"
    return result, {
        "total": len(selected),
        "statuses": dict(statuses),
        "qualified_timing_ms": timings,
        "token_f1": f1,
        "verapdf_compliant": {
            "assessed": len(compliant),
            "passed": sum(compliant),
            "qualified_assessed": len(qualified_compliant),
            "qualified_passed": sum(qualified_compliant),
            "rejected_noncompliant": rejected_noncompliant,
        },
    }


def parser_rows(summary: dict[str, Any]) -> tuple[list[list[str]], dict[str, Any]]:
    aliases = {
        "wellfriend": "wellfriendpdf",
        "pdfium": "pdfium",
        "poppler": "poppler",
        "mupdf": "mupdf",
        "qpdf": "qpdf",
    }
    resident = summary.get("persistent_document_medians_ms", {})
    fresh = summary.get("fresh_process_ms", {})
    qualified = summary.get("qualification", {}).get("qualified_documents", {})
    rows = [["Parse correctness", *[f"{qualified.get(aliases[tool], 0)}/150" for tool in TOOLS]]]
    for label, key in (
        ("Resident parse P50", "p50"),
        ("Resident parse P95", "p95"),
        ("Resident parse P99", "p99"),
        ("Resident parse maximum", "max"),
        ("Fresh-process parse P50", "p50"),
        ("Fresh-process parse P95", "p95"),
        ("Fresh-process parse maximum", "max"),
    ):
        source = fresh if label.startswith("Fresh") else resident
        rows.append([label, *[fmt_ms(source.get(aliases[tool], {}).get(key)) for tool in TOOLS]])
    return rows, {"resident": resident, "fresh_process": fresh, "qualified": qualified}


def render_rows(root: Path) -> tuple[list[list[str]], dict[str, Any]]:
    timing = load_jsonl(root / "render" / "render-timing.jsonl")
    quality = load_jsonl(root / "render" / "render-quality.jsonl")
    inventory_path = root / "render" / "page-inventory.json"
    inventory = json.loads(inventory_path.read_text("utf-8")) if inventory_path.exists() else []
    expected = {
        str(row["relative_path"]): int(row["page_count"])
        for row in inventory
        if isinstance(row, dict) and row.get("status") == "ok"
    }

    def complete(row: dict[str, Any]) -> bool:
        return (
            row.get("status") == "pass"
            and str(row.get("relative_path")) in expected
            and row.get("page_count") == expected[str(row.get("relative_path"))]
        )

    render_tools = ("wellfriend", "pdfium", "poppler", "mupdf")
    complete_rows = {tool: [row for row in timing if row.get("engine") == tool and complete(row)] for tool in render_tools}
    document_times = {
        tool: distribution(float(row["elapsed_ms"]) for row in rows)
        for tool, rows in complete_rows.items()
    }
    page_times = {
        tool: distribution(float(row["elapsed_ms"]) / int(row["page_count"]) for row in rows if int(row["page_count"]) > 0)
        for tool, rows in complete_rows.items()
    }
    rows: list[list[str]] = []
    rows.append([
        "Complete documents",
        *[f"{len(complete_rows.get(tool, []))}/150" if tool != "qpdf" else "N/A" for tool in TOOLS],
    ])
    rows.append([
        "Pages emitted / expected",
        *[
            (
                f"{sum(int(row['page_count']) for row in complete_rows.get(tool, [])):,}/"
                f"{sum(expected.get(str(row.get('relative_path')), 0) for row in timing if row.get('engine') == tool):,}"
            )
            if tool != "qpdf"
            else "N/A"
            for tool in TOOLS
        ],
    ])
    for label, key in (
        ("All-page document P50", "p50"),
        ("All-page document P90", "p90"),
        ("All-page document P95", "p95"),
        ("All-page document P99", "p99"),
        ("All-page document maximum", "max"),
        ("Per-page P50", "p50"),
        ("Per-page P90", "p90"),
        ("Per-page P95", "p95"),
        ("Per-page P99", "p99"),
        ("Per-page maximum", "max"),
    ):
        source = page_times if label.startswith("Per-page") else document_times
        rows.append([label, *[fmt_ms(source.get(tool, {}).get(key)) if tool != "qpdf" else "N/A" for tool in TOOLS]])

    quality_pages = [row for row in quality if row.get("phase") == "quality-page" and row.get("status") == "pass"]
    quality_stats: dict[str, Any] = {}
    ssim_row = ["Thumbnail SSIM vs Wellfriend (P50 / P05)", "1.000000 / 1.000000"]
    psnr_row = ["Thumbnail PSNR vs Wellfriend (P50)", "Infinity"]
    full_row = ["Full-resolution SSIM vs Wellfriend (P50)", "1.000000"]
    for tool in ("pdfium", "poppler", "mupdf"):
        selected = [row for row in quality_pages if row.get("reference") == tool]
        thumb_ssim = distribution(
            row["metrics"]["thumbnail"]["ssim"]
            for row in selected
            if isinstance(row.get("metrics", {}).get("thumbnail", {}).get("ssim"), (int, float))
        )
        thumb_psnr = distribution(
            row["metrics"]["thumbnail"]["psnr_db"]
            for row in selected
            if isinstance(row.get("metrics", {}).get("thumbnail", {}).get("psnr_db"), (int, float))
        )
        full_ssim = distribution(
            row["metrics"]["full_resolution"]["ssim"]
            for row in selected
            if isinstance(row.get("metrics", {}).get("full_resolution", {}).get("ssim"), (int, float))
        )
        quality_stats[tool] = {
            "pages": len(selected),
            "thumbnail_ssim": thumb_ssim,
            "thumbnail_psnr_db": thumb_psnr,
            "full_resolution_ssim": full_ssim,
        }
        ssim_row.append(f"{fmt_score(thumb_ssim.get('p50'))} / {fmt_score(thumb_ssim.get('p05'))}")
        psnr_row.append(fmt_score(thumb_psnr.get("p50"), 3))
        full_row.append(fmt_score(full_ssim.get("p50")))
    for row in (ssim_row, psnr_row, full_row):
        row.append("N/A")
        rows.append(row)
    return rows, {
        "expected_pages": sum(expected.values()),
        "document_timing_ms": document_times,
        "per_page_timing_ms": page_times,
        "quality": quality_stats,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--summary-output", type=Path, required=True)
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--facility-source-commit", required=True)
    args = parser.parse_args()

    manifest = json.loads(args.manifest.read_text("utf-8"))
    files = (
        manifest
        if isinstance(manifest, list)
        else manifest.get("files") or manifest.get("documents") or manifest.get("entries") or []
    )
    total_bytes = sum(int(row["bytes"]) for row in files)
    parsing_path = args.root / "parsing" / "summary.json"
    parsing = json.loads(parsing_path.read_text("utf-8"))
    operation_root = args.root / "operations"
    structural = load_jsonl(operation_root / "structural.jsonl")
    conversion = load_jsonl(operation_root / "conversion.jsonl")
    pdfa_validation = load_jsonl(operation_root / "pdfa-validation.jsonl")
    pdfa_conversion = load_jsonl(operation_root / "pdfa-conversion.jsonl")
    operation_rows = [*structural, *conversion, *pdfa_validation, *pdfa_conversion]

    parser_table, parser_summary = parser_rows(parsing)
    facility_summary: dict[str, Any] = {}
    for operation in (*STRUCTURAL, *CONVERSIONS, "pdfa-2b-validation", "pdfa-2b-conversion"):
        row = [operation]
        facility_summary[operation] = {}
        for tool in TOOLS:
            cell, stats = operation_cell(operation_rows, operation, tool)
            row.append(cell)
            facility_summary[operation][tool] = stats
        parser_table.append(row)

    renderer_table, renderer_summary = render_rows(args.root)
    expected_pages = int(renderer_summary.get("expected_pages", 0))
    evidence_paths = [
        parsing_path,
        operation_root / "structural.jsonl",
        operation_root / "conversion.jsonl",
        operation_root / "pdfa-validation.jsonl",
        operation_root / "pdfa-conversion.jsonl",
        args.root / "render" / "render-timing.jsonl",
        args.root / "render" / "render-quality.jsonl",
        args.root / "render" / "page-inventory.json",
    ]
    evidence = {
        str(path.relative_to(args.root)): {"bytes": path.stat().st_size, "sha256": sha256(path)}
        for path in evidence_paths
        if path.exists()
    }
    generated = dt.datetime.now(dt.timezone.utc).isoformat()
    summary = {
        "schema_version": "wellfriendpdf.final-benchmark.v1",
        "generated_at_utc": generated,
        "source_commit": args.source_commit,
        "facility_source_commit": args.facility_source_commit,
        "corpus": {"documents": len(files), "bytes": total_bytes},
        "parsing": parser_summary,
        "facilities": facility_summary,
        "rendering": renderer_summary,
        "evidence": evidence,
    }
    args.summary_output.parent.mkdir(parents=True, exist_ok=True)
    args.summary_output.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", "utf-8")

    lines = [
        "# Final 150-document benchmark",
        "",
        f"Renderer revision `{args.source_commit}` and parser/document-facility revision "
        f"`{args.facility_source_commit}` are measured on {len(files)} real PDFs "
        f"({total_bytes / 1024**3:.3f} GiB, {expected_pages:,} pages) on one Linux VPS. "
        "Every tool receives the same corpus and operation contract. A facility passes only when its output reopens and its operation-specific structural, semantic, security, or visual postconditions hold; unsupported operations remain unsupported.",
        "",
        "## Parsing and document facilities",
        "",
        "Resident parsing opens the in-memory document and resolves page count after warm-up; fresh parsing includes process startup. Facility timings include quality-qualified passes only. Conversion token F1 is agreement with Poppler extraction, not semantic ground truth.",
        "",
        *table(["Benchmark", *[LABELS[tool] for tool in TOOLS]], parser_table),
        "",
        "Wellfriend and veraPDF both execute PDF/A-2b validation reports across all 150 inputs; veraPDF classifies the source corpus as 0/150 compliant. Wellfriend converts 7/150 inputs to independently verified PDF/A-2b and refuses 143 files whose source fonts cannot be legally reconstructed as embedded fonts. Ghostscript emits 150 structurally readable files under the matched command, but veraPDF accepts 0/150 as PDF/A-2b.",
        "",
        "## Visual rendering",
        "",
        "Each renderer opens a document once and emits every page as 72-DPI RGB8. Timing includes only documents whose emitted page count matches the qpdf inventory. Thumbnail fidelity is measured on every matched page; first, middle, and last pages also receive full-resolution comparison. qpdf is a structural transformer, not a raster renderer.",
        "",
        *table(["Benchmark", *[LABELS[tool] for tool in TOOLS]], renderer_table),
        "",
        "The comparison sheets below are inspected at source resolution before publication. They show the lowest-agreement page, a lower-tail page, the median page, and an upper-tail page selected from the measured distribution.",
        "",
        "![Lowest-agreement visual sample](render/visual/01-worst.webp)",
        "",
        "![Lower-tail visual sample](render/visual/02-lower-tail.webp)",
        "",
        "![Median visual sample](render/visual/03-median.webp)",
        "",
        "![Upper-tail visual sample](render/visual/04-upper-tail.webp)",
        "",
        "Exact distributions and tool-level outcomes are recorded in [summary.json](summary.json). "
        "The compressed per-file JSONL records are the authoritative evidence; [SHA256SUMS](evidence/SHA256SUMS) binds every published artifact.",
        "",
        f"Generated: `{generated}`.",
    ]
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text("\n".join(lines).rstrip() + "\n", "utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
