#!/usr/bin/env python3
"""Collect compact all-feature corpus summaries into tracked evidence docs."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any


SUMMARY_FILES = {
    "info": "full-info-summary.json",
    "parser_report": "full-parser_report-summary.json",
    "security_report": "full-security_report-summary.json",
    "validate": "full-validate-2-summary.json",
    "fonts": "full-fonts-summary.json",
    "extract_text_structured": "full-extract_text_structured-summary.json",
    "parse_json": "full-parse_json-summary.json",
    "extract_tables": "full-extract_tables-summary.json",
    "forms_report": "full-forms_report-summary.json",
    "annotations_report": "full-annotations_report-2-summary.json",
    "document_subsystems_report": "full-document_subsystems_report-summary.json",
    "document_security_report": "full-document_security_report-summary.json",
    "layout_analyze_page1": "full-layout_analyze_page1-summary.json",
    "reading_order_report": "full-reading_order_report-summary.json",
    "flow_graph_report": "full-flow_graph_report-summary.json",
    "document_subsystems_analyze": "full-document_subsystems_analyze-2-summary.json",
    "document_security_analyze": "full-document_security_analyze-summary.json",
    "render_compare_page1": "full-render_compare_page1-summary.json",
    "editing_smoke": "full-editing-smoke-real5000-summary.json",
    "source_operator_apply": "full-source-operator-apply-summary.json",
}


def sha256_file(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load_stage(result_dir: Path, name: str, filename: str) -> dict[str, Any]:
    path = result_dir / filename
    if not path.exists():
        return {
            "stage": name,
            "available": False,
            "artifact": str(path),
            "sha256": None,
            "files": 0,
            "successes": 0,
            "failures": None,
        }
    data = json.loads(path.read_text())
    by_stage = data.get("by_stage", {})
    if name == "editing_smoke":
        files = data.get("pdf_files", 0)
        failures = sum(item.get("failures", 0) for item in by_stage.values())
        successes = files if failures == 0 else max(0, files - failures)
        return {
            "stage": name,
            "available": True,
            "artifact": str(path),
            "sha256": sha256_file(path),
            "files": files,
            "successes": successes,
            "failures": failures,
            "by_stage": by_stage,
        }
    metrics = by_stage.get(name) or next(iter(by_stage.values()), {})
    return {
        "stage": name,
        "available": True,
        "artifact": str(path),
        "sha256": sha256_file(path),
        "files": metrics.get("files", 0),
        "successes": metrics.get("successes", 0),
        "failures": metrics.get("failures", 0),
        "median_ms": metrics.get("median_ms"),
        "p95_ms": metrics.get("p95_ms"),
        "p99_ms": metrics.get("p99_ms"),
        "timeouts": metrics.get("timeouts"),
        "error_classes": metrics.get("error_classes", {}),
    }


def write_markdown(report: dict[str, Any], path: Path) -> None:
    lines = [
        "# Wellfriend 5,044-PDF all-feature corpus evidence",
        "",
        "This file is generated from compact VPS stage summaries. Raw PDFs and raw logs stay on the VPS.",
        "",
        f"- Corpus PDFs: {report['corpus_pdf_count']}",
        f"- Overall status: {report['status']}",
        f"- Result directory: `{report['result_dir']}`",
        f"- VPS raw-summary source: `{report['vps_result_dir']}`",
        "",
        "| Stage | Files | Successes | Failures | Median ms | P95 ms | Artifact SHA256 |",
        "|---|---:|---:|---:|---:|---:|---|",
    ]
    for stage in report["stages"]:
        lines.append(
            "| {stage} | {files} | {successes} | {failures} | {median} | {p95} | `{sha}` |".format(
                stage=stage["stage"],
                files=stage.get("files", 0),
                successes=stage.get("successes", 0),
                failures=stage.get("failures"),
                median=stage.get("median_ms", "see nested"),
                p95=stage.get("p95_ms", "see nested"),
                sha=stage.get("sha256") or "missing",
            )
        )
    lines.extend(
        [
            "",
            "## Editing smoke scope",
            "",
            "The editing smoke does not overwrite corpus PDFs. It extracts page-1 text, builds a scene report, checks operator-preserving edit eligibility, runs GeometricBlock reflow planning/report surfaces, and attempts temporary output-producing edit paths where source evidence permits.",
            "",
            "The `source_operator_apply` stage is a separate temporary-output corpus pass for operator-preserving text edits. Successful rows write edited PDFs in a temporary directory and record output/report sizes; unsupported source mappings remain typed refusals.",
            "",
            "## Scope notes",
            "",
            "- Visual rendering has a separate all-pages corpus campaign and a separate page-1 render-compare smoke in this file.",
            "- Public semantic/document-subsystem report commands use bounded document-report scope so real-corpus runs return typed evidence instead of hanging on very large documents.",
            "- Unsupported or unavailable edit targets are expected to return typed refusals; unclassified panics/timeouts/nonzero exits count as failures.",
        ]
    )
    path.write_text("\n".join(lines) + "\n")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("result_dir", type=Path)
    parser.add_argument("--json-out", type=Path, required=True)
    parser.add_argument("--md-out", type=Path, required=True)
    args = parser.parse_args()

    stages = [
        load_stage(args.result_dir, name, filename)
        for name, filename in SUMMARY_FILES.items()
    ]
    corpus_counts = {stage["files"] for stage in stages if stage.get("files")}
    corpus_pdf_count = max(corpus_counts) if corpus_counts else 0
    missing = [stage["stage"] for stage in stages if not stage["available"]]
    failures = sum(
        stage.get("failures", 0) or 0 for stage in stages if stage["available"]
    )
    report = {
        "schema_version": 1,
        "kind": "wellfriend_all_feature_corpus_evidence",
        "result_dir": str(args.result_dir),
        "vps_result_dir": "/mnt/wellpdf-block/results/all-feature-corpus-current",
        "corpus_pdf_count": corpus_pdf_count,
        "status": "pass" if not missing and failures == 0 else "incomplete_or_failed",
        "missing_stages": missing,
        "total_failures": failures,
        "stages": stages,
    }
    args.json_out.parent.mkdir(parents=True, exist_ok=True)
    args.md_out.parent.mkdir(parents=True, exist_ok=True)
    args.json_out.write_text(json.dumps(report, indent=2, sort_keys=True))
    write_markdown(report, args.md_out)
    return 0 if report["status"] == "pass" else 1


if __name__ == "__main__":
    raise SystemExit(main())
