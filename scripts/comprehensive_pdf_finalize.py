#!/usr/bin/env python3
"""Aggregate raw comprehensive PDF benchmark evidence into a dense report."""

from __future__ import annotations

import argparse
import collections
import json
import math
import statistics
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
    "verapdf": "veraPDF",
    "ghostscript": "Ghostscript",
}


def percentile(values: Iterable[float], percent: float) -> float | None:
    ordered = sorted(float(value) for value in values)
    if not ordered:
        return None
    index = max(0, min(len(ordered) - 1, math.ceil(percent / 100 * len(ordered)) - 1))
    return ordered[index]


def dist(values: Iterable[float]) -> dict[str, Any]:
    data = list(values)
    if not data:
        return {"count": 0}
    return {
        "count": len(data),
        "p05": percentile(data, 5),
        "p50": percentile(data, 50),
        "p90": percentile(data, 90),
        "p95": percentile(data, 95),
        "p99": percentile(data, 99),
        "min": min(data),
        "max": max(data),
        "mean": statistics.fmean(data),
    }


def fmt_ms(value: float | None) -> str:
    return "—" if value is None else f"{value:,.2f} ms"


def fmt_num(value: float | None, digits: int = 4) -> str:
    return "—" if value is None else f"{value:.{digits}f}"


def load_jsonl(path: Path) -> list[dict[str, Any]]:
    if not path.exists():
        return []
    rows = []
    for line in path.read_text("utf-8", errors="replace").splitlines():
        try:
            rows.append(json.loads(line))
        except json.JSONDecodeError:
            continue
    return rows


def markdown_table(headers: list[str], rows: list[list[str]]) -> list[str]:
    result = ["| " + " | ".join(headers) + " |", "| " + " | ".join("---" for _ in headers) + " |"]
    result += ["| " + " | ".join(str(cell).replace("|", "\\|") for cell in row) + " |" for row in rows]
    return result + [""]


def matrix_value(rows: list[dict[str, Any]], operation: str, tool: str) -> tuple[str, dict[str, Any]]:
    selected = [row for row in rows if row.get("operation") == operation and row.get("tool") == tool]
    if not selected:
        return "Unsupported / not applicable", {"count": 0}
    counts = collections.Counter(str(row.get("status")) for row in selected)
    timings = dist(
        float(row["run"]["elapsed_ms"])
        for row in selected
        if isinstance(row.get("run"), dict) and isinstance(row["run"].get("elapsed_ms"), (int, float))
    )
    cell = f"{counts.get('pass', 0)}/{len(selected)} pass"
    refused = counts.get("typed_refusal", 0)
    failed = counts.get("fail", 0) + counts.get("timeout", 0)
    if refused:
        cell += f"; {refused} refused"
    if failed:
        cell += f"; {failed} failed"
    not_applicable = counts.get("not_applicable", 0)
    if not_applicable:
        cell += f"; {not_applicable} not applicable"
    return cell, timings


def operation_section(title: str, rows: list[dict[str, Any]], operations: list[str]) -> list[str]:
    lines = [f"## {title}", ""]
    table_rows: list[list[str]] = []
    for operation in operations:
        values: dict[str, tuple[str, dict[str, Any]]] = {
            tool: matrix_value(rows, operation, tool) for tool in TOOLS
        }
        table_rows.append([f"{operation} — result", *[values[tool][0] for tool in TOOLS]])
        table_rows.append([f"{operation} — P50", *[fmt_ms(values[tool][1].get("p50")) for tool in TOOLS]])
        table_rows.append([f"{operation} — P95", *[fmt_ms(values[tool][1].get("p95")) for tool in TOOLS]])
        table_rows.append([f"{operation} — maximum", *[fmt_ms(values[tool][1].get("max")) for tool in TOOLS]])
    lines += markdown_table(["Benchmark", *[LABELS[tool] for tool in TOOLS]], table_rows)
    return lines


def conversion_quality_section(rows: list[dict[str, Any]]) -> list[str]:
    lines = ["### Conversion quality observations", ""]
    table_rows: list[list[str]] = []
    for operation in ("text", "html", "markdown", "json", "docx", "pptx", "xlsx"):
        for tool in TOOLS:
            selected = [
                row for row in rows
                if row.get("operation") == operation and row.get("tool") == tool
            ]
            if not selected:
                continue
            scores = [
                float(row["quality"]["text_agreement"]["token_f1_vs_poppler"])
                for row in selected
                if isinstance(row.get("quality", {}).get("text_agreement", {}).get("token_f1_vs_poppler"), (int, float))
            ]
            score_dist = dist(scores)
            table_rows.append(
                [
                    operation,
                    LABELS.get(tool, tool),
                    str(len(scores)),
                    fmt_num(score_dist.get("p50"), 6),
                    fmt_num(score_dist.get("p05"), 6),
                    fmt_num(score_dist.get("min"), 6) if "min" in score_dist else (fmt_num(min(scores), 6) if scores else "—"),
                ]
            )
    if table_rows:
        lines += markdown_table(
            ["Conversion", "Tool", "Scored files", "Token F1 P50", "Token F1 P05", "Token F1 minimum"],
            table_rows,
        )
    image_rows: list[list[str]] = []
    for operation in ("png-first-page", "jpeg-first-page"):
        for tool in TOOLS:
            selected = [row for row in rows if row.get("operation") == operation and row.get("tool") == tool]
            if not selected:
                continue
            decoded = sum(
                bool(row.get("quality", {}).get("valid") and row.get("quality", {}).get("width") and row.get("quality", {}).get("height"))
                for row in selected
            )
            image_rows.append([operation, LABELS.get(tool, tool), f"{decoded}/{len(selected)}"])
    if image_rows:
        lines += markdown_table(["Raster conversion", "Tool", "Decoded images"], image_rows)
    lines += [
        "Token scores measure agreement with Poppler text extraction, not semantic ground truth. "
        "Raster-conversion rows verify image decoding and dimensions; cross-renderer pixel fidelity is reported separately below.",
        "",
    ]
    return lines


def signing_interop_section(rows: list[dict[str, Any]]) -> list[str]:
    signed = [row for row in rows if row.get("operation") == "sign"]
    if not signed:
        return []
    table_rows: list[list[str]] = []
    for producer in ("wellfriend", "poppler"):
        selected = [row for row in signed if row.get("tool") == producer]
        if not selected:
            continue
        poppler_valid = sum(
            row.get("quality", {}).get("poppler_signature_verification", {}).get("exit_code") == 0
            for row in selected
        )
        wellfriend_valid = sum(
            row.get("quality", {}).get("signature_verification", {}).get("exit_code") == 0
            for row in selected
        )
        table_rows.append(
            [LABELS[producer], str(len(selected)), str(poppler_valid), str(wellfriend_valid)]
        )
    return ["### Signature interoperability", ""] + markdown_table(
        ["Signature producer", "Outputs", "Poppler verification pass", "Wellfriend verification pass"],
        table_rows,
    ) + [
        "Producer success requires a structurally valid, visually preserved PDF and successful Poppler verification. "
        "Wellfriend verification is also mandatory for Wellfriend-produced signatures and is recorded as an interoperability observation for Poppler-produced signatures. "
        "Poppler verifies all 150 Poppler-produced signatures, while Wellfriend rejects those same signatures because its verifier reports `CMS ContentInfo uses forbidden indefinite DER length`; this is an observed verifier-compatibility gap, not evidence that Poppler produced 150 invalid signatures.",
        "",
    ]


def parsing_section(root: Path) -> list[str]:
    path = root / "parsing" / "summary.json"
    lines = ["## Matched parsing: document open + resolved page count", ""]
    if not path.exists():
        return lines + ["Not completed.", ""]
    summary = json.loads(path.read_text("utf-8"))
    medians = summary.get("persistent_document_medians_ms", {})
    fresh = summary.get("fresh_process_ms", {})
    qualified = summary.get("qualification", {}).get("qualified_documents", {})
    tool_keys = ("wellfriendpdf", "pdfium", "poppler", "mupdf", "qpdf")
    rows = [
        ["Qualified page counts", *[f"{qualified.get(tool, 0)}/150" for tool in tool_keys]],
    ]
    for label, key in (("Resident P50", "p50"), ("Resident P90", "p90"), ("Resident P95", "p95"), ("Resident P99", "p99"), ("Resident maximum", "max")):
        rows.append([label, *[fmt_ms(medians.get(tool, {}).get(key)) for tool in tool_keys]])
    for label, key in (("Fresh-process P50", "p50"), ("Fresh-process P95", "p95"), ("Fresh-process maximum", "max")):
        rows.append([label, *[fmt_ms(fresh.get(tool, {}).get(key)) for tool in tool_keys]])
    lines += markdown_table(["Benchmark", *[LABELS[tool] for tool in tool_keys]], rows)
    disagreements = summary.get("qualification", {}).get("disagreements", [])
    lines += [f"Page-count disagreements: **{len(disagreements)}**.", ""]
    return lines


def environment_section(root: Path) -> list[str]:
    path = root / "parsing" / "environment.json"
    lines = ["## Reproducibility", ""]
    if not path.exists():
        return lines + ["Environment evidence is unavailable.", ""]
    environment = json.loads(path.read_text("utf-8"))
    lines += [
        f"- Host: `{environment.get('hostname', 'unknown')}`",
        f"- Platform: `{environment.get('platform', 'unknown')}`",
        f"- Start (UTC): `{environment.get('started_at_utc', 'unknown')}`",
        f"- Parser contract: `{environment.get('contract', 'unknown')}`",
        f"- Corpus-manifest SHA-256: `{environment.get('corpus_manifest_sha256', 'unknown')}`",
        "",
    ]
    hashes = environment.get("adapter_sha256", {})
    if isinstance(hashes, dict):
        rows = [[LABELS.get(str(tool), str(tool)), f"`{digest}`"] for tool, digest in sorted(hashes.items())]
        lines += markdown_table(["Adapter", "SHA-256"], rows)
    provenance_path = root / "provenance.json"
    if provenance_path.exists():
        provenance = json.loads(provenance_path.read_text("utf-8"))
        lines += [
            f"- Wellfriend source commit: `{provenance.get('wellfriend_source_commit', 'unknown')}`",
            f"- Logical CPUs visible: **{provenance.get('cpu_count', 'unknown')}**",
            f"- Scheduling: `{provenance.get('scheduling', 'unknown')}`",
            f"- Host isolation: **{provenance.get('host_isolation', 'unknown')}**",
            "",
        ]
        versions = provenance.get("versions", {})
        if isinstance(versions, dict):
            lines += markdown_table(
                ["Tool", "Version"],
                [[str(tool), str(version)] for tool, version in versions.items()],
            )
        binary_hashes = provenance.get("binary_sha256", {})
        if isinstance(binary_hashes, dict):
            lines += markdown_table(
                ["Benchmark binary", "SHA-256"],
                [[str(binary), f"`{digest}`"] for binary, digest in binary_hashes.items()],
            )
        lines += [
            "The VPS was shared with unrelated services and occasional build activity. Engines ran sequentially in a seeded randomized order, "
            "but these timings are not laboratory-isolated and should be reproduced on a dedicated host before making competitive speed claims.",
            "",
        ]
    return lines


def render_section(root: Path) -> list[str]:
    lines = ["## Visual rendering", ""]
    timing_rows = load_jsonl(root / "render" / "render-timing.jsonl")
    quality = load_jsonl(root / "render" / "render-quality.jsonl")
    quality_summaries = [row for row in quality if row.get("phase") == "quality"]
    actual_wellfriend_pages = {
        str(row.get("relative_path")): int(row.get("wellfriend_pages") or 0)
        for row in quality_summaries
    }
    inventory_path = root / "render" / "page-inventory.json"
    expected_pages: dict[str, int] = {}
    if inventory_path.exists():
        inventory = json.loads(inventory_path.read_text("utf-8"))
        if isinstance(inventory, list):
            expected_pages = {
                str(row["relative_path"]): int(row["page_count"])
                for row in inventory
                if isinstance(row, dict)
                and row.get("status") == "ok"
                and row.get("relative_path")
                and isinstance(row.get("page_count"), int)
            }

    def complete_render(row: dict[str, Any]) -> bool:
        expected = expected_pages.get(str(row.get("relative_path")))
        actual = row.get("page_count")
        return (
            row.get("status") == "pass"
            and isinstance(actual, int)
            and expected is not None
            and actual == expected
        )

    if timing_rows:
        rows = []
        tools = ("wellfriend", "pdfium", "poppler", "mupdf")
        process_values = []
        complete_values = []
        page_values = []
        for tool in tools:
            selected = [row for row in timing_rows if row.get("engine") == tool]
            process_values.append(f"{sum(row.get('status') == 'pass' for row in selected)}/{len(selected)}")
            complete_values.append(f"{sum(complete_render(row) for row in selected)}/{len(selected)}")
            if tool == "wellfriend" and actual_wellfriend_pages:
                rendered = sum(
                    actual_wellfriend_pages.get(str(row.get("relative_path")), 0)
                    for row in selected
                )
            else:
                rendered = sum(
                    int(row.get("page_count") or 0)
                    for row in selected
                    if row.get("status") == "pass" and isinstance(row.get("page_count"), int)
                )
            expected = sum(
                expected_pages.get(str(row.get("relative_path")), 0)
                for row in selected
            )
            page_values.append(f"{rendered:,}/{expected:,}")
        rows.append(["Process exits passed", *process_values])
        rows.append(["Complete documents", *complete_values])
        rows.append(["Pages emitted / expected", *page_values])
        distributions = {
            tool: dist(
                float(row["elapsed_ms"])
                for row in timing_rows
                if row.get("engine") == tool and complete_render(row)
            )
            for tool in tools
        }
        per_page = {
            tool: dist(
                float(row["elapsed_ms"]) / int(row["page_count"])
                for row in timing_rows
                if row.get("engine") == tool
                and complete_render(row)
                and int(row.get("page_count") or 0) > 0
            )
            for tool in tools
        }
        for label, key in (("Document P50", "p50"), ("Document P95", "p95"), ("Document maximum", "max")):
            rows.append([label, *[fmt_ms(distributions[tool].get(key)) for tool in tools]])
        for label, key in (("Per-page P50", "p50"), ("Per-page P95", "p95"), ("Per-page maximum", "max")):
            rows.append([label, *[fmt_ms(per_page[tool].get(key)) for tool in tools]])
        lines += markdown_table(["Benchmark", "Wellfriend SDK", "PDFium", "Poppler", "MuPDF"], rows)
        lines += [
            "Timing distributions include only complete documents whose emitted page count exactly matches the qpdf page inventory. "
            "Process-exit success is shown separately because a zero exit code can still accompany a truncated page stream.",
            "",
        ]
    else:
        lines += ["Matched stream timing is not completed.", ""]

    pages = [row for row in quality if row.get("phase") == "quality-page" and row.get("status") == "pass"]
    summaries = quality_summaries
    if pages:
        rows = []
        for reference in ("pdfium", "poppler", "mupdf"):
            selected = [row for row in pages if row.get("reference") == reference]
            thumb_ssim = dist(
                row["metrics"]["thumbnail"]["ssim"]
                for row in selected
                if isinstance(row.get("metrics", {}).get("thumbnail", {}).get("ssim"), (int, float))
            )
            thumb_psnr = dist(
                row["metrics"]["thumbnail"]["psnr_db"]
                for row in selected
                if isinstance(row.get("metrics", {}).get("thumbnail", {}).get("psnr_db"), (int, float))
            )
            full = [
                row["metrics"]["full_resolution"]
                for row in selected
                if isinstance(row.get("metrics", {}).get("full_resolution"), dict)
            ]
            full_ssim = dist(item["ssim"] for item in full if isinstance(item.get("ssim"), (int, float)))
            rows.append(
                [
                    f"Wellfriend vs {LABELS[reference]}",
                    str(len(selected)),
                    fmt_num(thumb_ssim.get("p50"), 6),
                    fmt_num(thumb_ssim.get("p05"), 6),
                    fmt_num(thumb_psnr.get("p50"), 3),
                    str(len(full)),
                    fmt_num(full_ssim.get("p50"), 6),
                ]
            )
        lines += markdown_table(
            ["Comparison", "Pages", "Thumbnail SSIM P50", "Thumbnail SSIM P05", "Thumbnail PSNR P50", "Full-res samples", "Full-res SSIM P50"],
            rows,
        )
        lines += [f"Documents with complete three-reference quality passes: **{sum(row.get('status') == 'pass' for row in summaries)}/{len(summaries)}**.", ""]
    else:
        lines += ["Streaming quality comparison is not completed.", ""]

    default_summary = root / "render" / "wellfriend-all-pages-summary.json"
    immediate_summary = root / "render" / "wellfriend-failed-immediate-summary.json"
    if default_summary.exists():
        default = json.loads(default_summary.read_text("utf-8"))
        lines += [
            "### Wellfriend public CLI coverage check",
            "",
            f"Default display-list pipeline: **{default.get('successful_files', 0)}/{default.get('files', 0)} files**, "
            f"**{default.get('pages_rendered', 0)}/{default.get('pages_attempted', 0)} attempted pages**, "
            f"with **{default.get('failed_files', 0)} failed files**.",
            "",
        ]
    if immediate_summary.exists():
        immediate = json.loads(immediate_summary.read_text("utf-8"))
        lines += [
            f"Immediate-pipeline retry of the failed set: **{immediate.get('successful_files', 0)}/{immediate.get('files', 0)} files**, "
            f"**{immediate.get('pages_rendered', 0)}/{immediate.get('pages_attempted', 0)} attempted pages**.",
            "",
        ]
    visual_manifest = root / "render" / "visual" / "visual-samples.json"
    if visual_manifest.exists():
        visual = json.loads(visual_manifest.read_text("utf-8"))
        samples = visual.get("samples", []) if isinstance(visual, dict) else []
        lines += [
            "### Distribution-selected visual evidence",
            "",
            "These sheets are selected reproducibly from successful three-reference page pairs: worst, lower-tail, median, and upper-tail by mean thumbnail SSIM. "
            "They supplement rather than replace the all-page metrics and do not include the 15 documents with terminal Wellfriend render failures.",
            "",
        ]
        for sample in samples:
            selection = str(sample.get("selection", "sample"))
            image_path = str(sample.get("image", ""))
            lines += [
                f"**{selection}** — `{sample.get('relative_path', 'unknown')}`, page {sample.get('page_number', '?')}/{sample.get('page_count', '?')}, "
                f"mean thumbnail SSIM **{float(sample.get('mean_thumbnail_ssim', 0.0)):.6f}**.",
                "",
                f"![{selection} four-renderer comparison](render/visual/{image_path})",
                "",
            ]
    lines += ["qpdf is shown as unsupported for visual rendering because it is a structural PDF transformer, not a raster renderer.", ""]
    return lines


def editing_section(rows: list[dict[str, Any]]) -> list[str]:
    lines = ["## Wellfriend source-editing qualification", ""]
    if not rows:
        return lines + ["Not completed.", ""]
    table = []
    for operation in ("operator-preserving", "scene-source-edit", "paragraph-reflow", "paragraph-reflow-sdk", "geometric-reflow", "semantic-reflow", "vector-duplicate"):
        selected = [row for row in rows if row.get("operation") == operation]
        counts = collections.Counter(str(row.get("status")) for row in selected)
        timings = dist(float(row["run"]["elapsed_ms"]) for row in selected if isinstance(row.get("run"), dict) and isinstance(row["run"].get("elapsed_ms"), (int, float)))
        replacement = sum(
            bool(
                row.get("quality", {}).get("replacement_present_after_reopen")
                or row.get("quality", {}).get("object_count_increased")
            )
            for row in selected
        )
        visual = sum((row.get("quality", {}).get("visual_change", {}).get("changed_pixels_threshold8") or 0) > 0 for row in selected)
        table.append([
            operation,
            str(len(selected)),
            str(counts.get("pass", 0)),
            str(counts.get("typed_refusal", 0)),
            str(counts.get("fail", 0) + counts.get("timeout", 0)),
            str(counts.get("not_applicable", 0)),
            str(replacement),
            str(visual),
            fmt_ms(timings.get("p50")),
            fmt_ms(timings.get("p95")),
        ])
    lines += markdown_table(
        ["Edit route", "Files", "Pass", "Typed refusal", "Fail", "No source", "Edit postcondition verified", "Visual change", "P50", "P95"],
        table,
    )
    return lines


def pdfa_section(validation: list[dict[str, Any]], conversion: list[dict[str, Any]]) -> list[str]:
    lines = ["## PDF/A validation and conversion", ""]
    rows: list[list[str]] = []
    for operation, source, tools in (
        ("pdfa-2b-validation", validation, ("wellfriend", "verapdf")),
        ("pdfa-2b-conversion", conversion, ("wellfriend", "ghostscript")),
    ):
        values = {tool: matrix_value(source, operation, tool) for tool in tools}
        result_label = "execution result" if operation.endswith("validation") else "result"
        rows.append([f"{operation} — {result_label}", *[values[tool][0] for tool in tools]])
        rows.append([f"{operation} — P50", *[fmt_ms(values[tool][1].get("p50")) for tool in tools]])
        rows.append([f"{operation} — P95", *[fmt_ms(values[tool][1].get("p95")) for tool in tools]])
        if operation.endswith("validation"):
            rows[-3].append("—")
            rows[-2].append("—")
            rows[-1].append("—")
        else:
            for row in rows[-3:]:
                row.insert(2, "—")
    lines += markdown_table(["Benchmark", "Wellfriend SDK", "veraPDF", "Ghostscript"], rows)
    validation_outcomes: list[list[str]] = []
    for tool in ("wellfriend", "verapdf"):
        selected = [row for row in validation if row.get("tool") == tool]
        reported = [
            row.get("quality", {}).get("reported_compliant")
            for row in selected
            if isinstance(row.get("quality", {}).get("reported_compliant"), bool)
        ]
        validation_outcomes.append(
            [LABELS[tool], str(len(reported)), str(sum(value is True for value in reported)), str(sum(value is False for value in reported))]
        )
    conversion_outcomes: list[list[str]] = []
    for tool in ("wellfriend", "ghostscript"):
        selected = [row for row in conversion if row.get("tool") == tool]
        assessed = [
            row.get("quality", {}).get("verapdf_compliant")
            for row in selected
            if isinstance(row.get("quality", {}).get("verapdf_compliant"), bool)
        ]
        conversion_outcomes.append(
            [LABELS[tool], str(len(assessed)), str(sum(value is True for value in assessed)), str(sum(value is False for value in assessed))]
        )
    if any(row[1] != "0" for row in validation_outcomes):
        lines += markdown_table(["Validator", "Files with outcome", "Reported compliant", "Reported non-compliant"], validation_outcomes)
        lines += [
            "A validation execution pass means the command completed and its report artifact passed the harness checks; it does not mean the input is PDF/A-compliant. "
            "The Wellfriend rows did not expose a machine-parsed compliance boolean, so no Wellfriend compliance claim is made. veraPDF classified all 150 source PDFs as non-compliant.",
            "",
        ]
    if any(row[1] != "0" for row in conversion_outcomes):
        lines += markdown_table(["Converter", "Outputs assessed", "veraPDF compliant", "veraPDF non-compliant"], conversion_outcomes)
    lines += ["veraPDF is the independent PDF/A validator; Ghostscript is included only as a conversion baseline, not as one of the five requested PDF engines.", ""]
    return lines


def failure_section(operation_rows: list[dict[str, Any]], editing_rows: list[dict[str, Any]]) -> list[str]:
    lines = ["## Failure ledger", ""]
    counter: collections.Counter[tuple[str, str, str]] = collections.Counter()
    for row in [*operation_rows, *editing_rows]:
        if row.get("status") in {"pass", "not_applicable"}:
            continue
        stderr = str(row.get("run", {}).get("stderr_tail", "")).strip().splitlines()
        reason = stderr[-1] if stderr else str(row.get("reason", row.get("status")))
        counter[(str(row.get("phase")), str(row.get("operation")), reason)] += 1
    if not counter:
        return lines + ["No failures recorded.", ""]
    rows = [[phase, operation, str(count), reason[:240]] for (phase, operation, reason), count in counter.most_common(30)]
    return lines + markdown_table(["Phase", "Operation", "Count", "Observed reason"], rows)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    manifest = json.loads(args.manifest.read_text("utf-8"))
    files = manifest["files"]
    total_bytes = sum(int(row["bytes"]) for row in files)

    operations_root = args.root / "operations-final"
    if not operations_root.exists():
        operations_root = args.root / "operations"
    structural = load_jsonl(operations_root / "structural.jsonl")
    conversions = load_jsonl(operations_root / "conversion.jsonl")
    editing = load_jsonl(operations_root / "editing.jsonl")
    pdfa_validation = load_jsonl(operations_root / "pdfa-validation.jsonl")
    pdfa_conversion = load_jsonl(operations_root / "pdfa-conversion.jsonl")
    all_operation_rows = [*structural, *conversions, *pdfa_validation, *pdfa_conversion]

    lines = [
        "# 150-document comprehensive PDF benchmark",
        "",
        "This report contains observed VPS results for the exact corpus and binaries named in the raw evidence. "
        "Unsupported cells are not timed as no-op successes. A successful command counts only after its artifact-specific postconditions pass.",
        "",
        "## Corpus and contracts",
        "",
        f"- PDFs: **{len(files)}** unique real-world files.",
        f"- Corpus bytes: **{total_bytes / (1024 ** 3):.3f} GiB**.",
        f"- Large-file partition: **{sum(row.get('corpus_partition') != 'original-100' for row in files)} files**.",
        "- Structural outputs: qpdf reopen/check, resolved page count, encryption/linearization state, and low-resolution visual postconditions.",
        "- Conversion quality: container/schema validity and token agreement with Poppler extraction. Agreement is not ground-truth semantic accuracy.",
        "- Rendering timing: open once, render every page to RGB8 PPM frames, drain without retaining rasters.",
        "- Rendering fidelity: every rendered page receives thumbnail metrics; first/middle/last pages receive full-resolution metrics.",
        "- Editing: save, qpdf reopen, Wellfriend re-extraction, replacement postcondition, and before/after rendering.",
        "",
    ]
    lines += environment_section(args.root)
    lines += parsing_section(args.root)
    lines += operation_section(
        "Structural and security operations",
        structural,
        ["merge", "split", "extract-pages", "lock", "unlock", "rotate", "repair", "organize", "linearize", "flatten", "watermark", "page-numbers", "metadata", "crop", "resize", "nup", "optimize", "canonicalize", "sanitize", "sign"],
    )
    lines += signing_interop_section(structural)
    lines += operation_section(
        "PDF conversion and extraction",
        conversions,
        ["text", "html", "markdown", "json", "docx", "pptx", "xlsx", "svg-first-page", "postscript-first-page", "eps-first-page", "png-first-page", "jpeg-first-page", "tables-json"],
    )
    lines += conversion_quality_section(conversions)
    lines += pdfa_section(pdfa_validation, pdfa_conversion)
    lines += render_section(args.root)
    lines += editing_section(editing)
    lines += failure_section(all_operation_rows, editing)
    lines += [
        "## Evidence files",
        "",
        "Raw JSON/JSONL files beside this report are authoritative. This Markdown is a deterministic aggregation and does not replace the per-file records.",
        "",
    ]
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text("\n".join(lines).rstrip() + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
