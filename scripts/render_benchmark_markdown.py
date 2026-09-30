#!/usr/bin/env python3
"""Render a dense, tool-column benchmark report from canonical summaries."""

from __future__ import annotations

import argparse
import json
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


TOOLS = ("Wellfriend PDF", "qpdf", "MuPDF", "PDFium", "Poppler")


def value(stats: dict[str, Any] | None, field: str) -> str:
    if not stats or field not in stats:
        return "—"
    return f"{float(stats[field]):,.3f} ms"


def percent_value(stats: dict[str, Any] | None, field: str) -> str:
    if not stats or field not in stats:
        return "—"
    return f"{float(stats[field]):,.6f}%"


def table(headers: tuple[str, ...], rows: list[tuple[str, ...]]) -> str:
    output = ["| " + " | ".join(headers) + " |"]
    output.append(
        "|" + "|".join("---" if index == 0 else "---:" for index in range(len(headers))) + "|"
    )
    output.extend("| " + " | ".join(row) + " |" for row in rows)
    return "\n".join(output)


def external(summary: dict[str, Any], tool: str) -> dict[str, Any] | None:
    return (
        summary.get("external_parser_tools", {})
        .get("tools", {})
        .get(tool, {})
        .get("fresh_process_timing")
    )


def external_tool(summary: dict[str, Any], tool: str) -> dict[str, Any]:
    return (
        summary.get("external_parser_tools", {})
        .get("tools", {})
        .get(tool, {})
    )


def render_rows(visual: dict[str, Any]) -> list[tuple[str, ...]]:
    timings = visual.get("process_render_timings", {})
    attempted = int(visual.get("observations", 0)) + int(
        visual.get("failures", {}).get("count", 0)
    )
    completed = int(visual.get("observations", 0))
    rows = [
        (
            "Rendered outputs",
            f"{completed}/{attempted}",
            "—",
            f"{int(timings.get('mupdf', {}).get('count', 0))}/{attempted}",
            f"{int(timings.get('pdfium', {}).get('count', 0))}/{attempted}",
            f"{int(timings.get('poppler', {}).get('count', 0))}/{attempted}",
        )
    ]
    for label, field in (
        ("P50", "p50_ms"),
        ("P90", "p90_ms"),
        ("P95", "p95_ms"),
        ("P99", "p99_ms"),
        ("Maximum", "max_ms"),
    ):
        rows.append(
            (
                label,
                value(timings.get("wellfriendpdf"), field),
                "—",
                value(timings.get("mupdf"), field),
                value(timings.get("pdfium"), field),
                value(timings.get("poppler"), field),
            )
        )
    return rows


def quality_rows(visual: dict[str, Any]) -> list[tuple[str, ...]]:
    quality = visual.get("quality", {})
    pairs = (
        ("wellfriendpdf_vs_mupdf", "Wellfriend vs MuPDF"),
        ("wellfriendpdf_vs_pdfium", "Wellfriend vs PDFium"),
        ("wellfriendpdf_vs_poppler", "Wellfriend vs Poppler"),
        ("pdfium_vs_mupdf", "PDFium vs MuPDF"),
        ("pdfium_vs_poppler", "PDFium vs Poppler"),
        ("mupdf_vs_poppler", "MuPDF vs Poppler"),
    )
    rows = []
    for key, label in pairs:
        stats = quality.get(f"{key}_changed_gt_8_percent")
        rows.append(
            (
                label,
                percent_value(stats, "p50_percent"),
                percent_value(stats, "p95_percent"),
                percent_value(stats, "p99_percent"),
                percent_value(stats, "max_percent"),
            )
        )
    return rows


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--summary", required=True, type=Path)
    parser.add_argument("--edited-summary", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--corpus-manifest-sha", required=True)
    parser.add_argument("--campaign-label", required=True)
    args = parser.parse_args()

    summary = json.loads(args.summary.read_text(encoding="utf-8"))
    edited_summary = (
        json.loads(args.edited_summary.read_text(encoding="utf-8"))
        if args.edited_summary
        else {}
    )
    core = summary["core"]
    editing = summary.get("editing") or {}
    visual = summary.get("visual_rendering") or {}
    edited_visual = edited_summary.get("visual_rendering") or {}

    parsed = core["timings"]["metadata_page_count_e2e"]
    core_documents = int(core.get("exactness", {}).get("documents", 0))
    core_failures = int(core.get("failures", {}).get("count", 0))
    core_attempted = core_documents + core_failures
    qpdf = external_tool(summary, "qpdf")
    mupdf = external_tool(summary, "mupdf")
    poppler = external_tool(summary, "poppler")
    parsing_rows = [
        (
            "Accepted",
            f"{core_documents}/{core_attempted}",
            f"{int(qpdf.get('accepted', 0))}/{int(qpdf.get('observations', 0))}",
            f"{int(mupdf.get('accepted', 0))}/{int(mupdf.get('observations', 0))}",
            "not measured",
            f"{int(poppler.get('accepted', 0))}/{int(poppler.get('observations', 0))}",
        ),
        (
            "Workload",
            "xref open + indexed page count",
            "structural check",
            "document inventory",
            "not measured",
            "metadata + page count",
        )
    ]
    for label, field in (
        ("P50", "p50_ms"),
        ("P90", "p90_ms"),
        ("P95", "p95_ms"),
        ("P99", "p99_ms"),
        ("Maximum", "max_ms"),
    ):
        parsing_rows.append(
            (
                label,
                value(parsed, field),
                value(external(summary, "qpdf"), field),
                value(external(summary, "mupdf"), field),
                "—",
                value(external(summary, "poppler"), field),
            )
        )

    edit_times = editing.get("timings", {})
    editing_observations = int(editing.get("observations", 0))
    editing_failures = int(editing.get("failures", {}).get("count", 0))
    editing_rows = [
        (
            "Applicable and verified after reopen",
            f"{editing_observations}/{editing_observations}",
            "—",
            "—",
            "—",
            "—",
        ),
        (
            "Typed non-applicable",
            f"{editing_failures}/{editing_observations + editing_failures}",
            "—",
            "—",
            "—",
            "—",
        ),
    ]
    for label, key in (
        ("Plan", "plan"),
        ("Apply", "apply"),
        ("Independent reopen verification", "reopen_verify"),
        ("Verified end to end", "verified_end_to_end"),
    ):
        stats = edit_times.get(key)
        editing_rows.append(
            (
                f"{label} P50 / P95 / max",
                " / ".join(value(stats, field) for field in ("p50_ms", "p95_ms", "max_ms")),
                "—",
                "—",
                "—",
                "—",
            )
        )

    generated = datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")
    edited_render_section = ""
    if edited_visual:
        edited_render_section = f"""
### Verified edited outputs

{table(("Benchmark", *TOOLS), render_rows(edited_visual))}
"""
    quality_section = ""
    if visual:
        quality_section = f"""
## Visual divergence diagnostics

### Original inputs

{table(("Pair", "P50 changed > 8", "P95", "P99", "Maximum"), quality_rows(visual))}
"""
        if edited_visual:
            quality_section += f"""
### Verified edited outputs

{table(("Pair", "P50 changed > 8", "P95", "P99", "Maximum"), quality_rows(edited_visual))}
"""

    text = f"""# {args.campaign_label}

Generated `{generated}` from source `{args.source_sha}` and corpus manifest
`{args.corpus_manifest_sha}`. Percentiles use nearest rank. Repeated process
measurements are collapsed to a per-document median before corpus percentiles.
Failures and timeouts remain in the reported denominator.

## Parsing

{table(("Benchmark", *TOOLS), parsing_rows)}

The workload row is part of the result: unlike operations are not equivalent
speed claims. Requested-page materialization, full page-tree traversal, page
program parsing, and semantic extraction are reported separately in `summary.json`.

## Editing

{table(("Benchmark", *TOOLS), editing_rows)}

Apply includes authenticated transaction reuse, native mutation, serialization,
internal reopen, and postconditions. The final row also includes the harness's
independent reopen and extraction check.

## Visual rendering time

### Original inputs

{table(("Benchmark", *TOOLS), render_rows(visual))}
{edited_render_section}

These are fresh-process page-one measurements at the campaign DPI. Internal
cold raster, retained raster, and image encoding remain separate in
`summary.json`. Quality is not inferred from speed: pairwise raster metrics,
reference disagreement, hashes, full-page panels, and amplified difference
panels are separate evidence.
{quality_section}

`Changed > 8` is the fraction of pixels where at least one RGB channel differs
by more than eight levels. It exposes disagreement but is not an objective error
rate: antialiasing, hinting, and colour policy also contribute. Reference-to-reference
rows make that baseline disagreement visible.

## Visual evidence

The contact sheet keeps the corpus overview compact; each `pages/` directory
contains the corresponding full four-renderer sheet and amplified differences.

![Original corpus contact sheet](visual-originals/contacts/contact-076-100.webp)

![Edited i1040gi four-renderer comparison](visual-edited/pages/094-i1040gi-pdf.webp)

## Status

- Core documents: {core.get('observations', 0)}; raw repetitions: {core.get('exactness', {}).get('raw_observations', core.get('observations', 0))}; failures: {core.get('failures', {}).get('count', 0)}.
- Applicable verified edits: {editing.get('verification', {}).get('replacement_observed_after_reopen', 0)}; edit failures/refusals: {editing.get('failures', {}).get('count', 0)}.
- Original visual passes: {visual.get('observations', 0)}; failures: {visual.get('failures', {}).get('count', 0)}.
- Edited visual passes: {edited_visual.get('observations', 0)}; failures: {edited_visual.get('failures', {}).get('count', 0)}.
- Requested edit-apply latency gate: {editing.get('requested_apply_slo', {}).get('status', 'not_measured')}.
- Requested verified-edit latency gate: {editing.get('requested_verified_e2e_slo', {}).get('status', 'not_measured')}.
- Ten-percent original-render lead gate: {visual.get('ten_percent_faster_than_fastest_competitor', {}).get('status', 'not_measured')}.
- Ten-percent edited-render lead gate: {edited_visual.get('ten_percent_faster_than_fastest_competitor', {}).get('status', 'not_measured')}.
"""
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(text, encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
