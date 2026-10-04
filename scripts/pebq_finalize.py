#!/usr/bin/env python3
"""Finalize a completed PEBQ run without rerunning the benchmark campaign."""

from __future__ import annotations

import argparse
import importlib.util
import json
from pathlib import Path
from typing import Any


RAW_FILES = (
    "parse-persistent.jsonl",
    "parse-fresh.jsonl",
    "render-persistent.jsonl",
    "render-retained-resources.jsonl",
    "render-fresh.jsonl",
    "quality-pages.jsonl",
)


def load_jsonl(path: Path) -> list[dict[str, Any]]:
    with path.open(encoding="utf-8") as stream:
        return [json.loads(line) for line in stream if line.strip()]


def load_benchmark_module(path: Path):
    spec = importlib.util.spec_from_file_location("pebq_benchmark", path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot import benchmark module from {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--results", type=Path, required=True)
    parser.add_argument("--benchmark-script", type=Path, required=True)
    parser.add_argument("--source-revision", required=True)
    parser.add_argument("--source", type=Path, action="append", default=[])
    parser.add_argument(
        "--adapter-dir",
        type=Path,
        help="refresh adapter-derived environment metadata before reporting",
    )
    parser.add_argument(
        "--benchmark-execution-sha256",
        help="hash of the benchmark script used for the timed campaign when reporting code changed later",
    )
    parser.add_argument("--seed", type=int, default=20260936)
    parser.add_argument(
        "--recompute-quality",
        action="store_true",
        help="recompute quality from retained rasters before finalizing",
    )
    parser.add_argument(
        "--corpus",
        type=Path,
        help="corpus root required with --recompute-quality",
    )
    args = parser.parse_args()

    benchmark = load_benchmark_module(args.benchmark_script)
    summary_path = args.results / "summary.json"
    summary = json.loads(summary_path.read_text(encoding="utf-8"))
    manifest = json.loads(
        (args.results / "corpus-manifest.json").read_text(encoding="utf-8")
    )
    summary.setdefault("configuration", {})["document_count"] = len(manifest)

    parse_persistent_rows = load_jsonl(args.results / "parse-persistent.jsonl")
    render_persistent_rows = load_jsonl(args.results / "render-persistent.jsonl")
    retained_resource_path = args.results / "render-retained-resources.jsonl"
    retained_resource_rows = (
        load_jsonl(retained_resource_path) if retained_resource_path.is_file() else []
    )
    persistent_repetitions = int(
        summary.get("configuration", {}).get("persistent_repetitions", 1)
    )
    parse_document_medians = {
        engine: benchmark.document_median_distribution(
            parse_persistent_rows, engine, "parse_ms", persistent_repetitions
        )
        for engine in benchmark.PARSER_ENGINES
    }
    render_document_medians = {
        engine: benchmark.document_median_distribution(
            render_persistent_rows, engine, "render_ms", persistent_repetitions
        )
        for engine in benchmark.RENDER_ENGINES
    }
    parse_qualification = benchmark.page_count_qualification(
        parse_persistent_rows, persistent_repetitions
    )
    wellfriend_document_medians = benchmark.median_by_key(
        (
            row
            for row in parse_persistent_rows
            if row.get("engine") == "wellfriendpdf"
        ),
        "parse_ms",
    )
    poppler_document_medians = benchmark.median_by_key(
        (
            row
            for row in parse_persistent_rows
            if row.get("engine") == "poppler"
        ),
        "parse_ms",
    )
    qualified_parse_paths = set(wellfriend_document_medians)
    qualified_parse_paths -= {
        str(row["relative_path"])
        for row in parse_qualification.get("disagreements", [])
    }
    qualified_parse_paths -= {
        str(row["relative_path"])
        for row in parse_qualification.get("repetition_failures", [])
    }
    summary["schema"] = "wellfriendpdf.pebq.v2"
    summary["parsing"].update(
        {
            "profile": "process-resident adapter; document reopened for every request; file read excluded",
            "qualification": parse_qualification,
            "persistent_parse_ms": parse_document_medians,
            "process_resident_document_cold_document_medians_ms": parse_document_medians,
            "process_resident_document_cold_observations_ms": {
                engine: benchmark.distribution(
                    row["parse_ms"]
                    for row in parse_persistent_rows
                    if row.get("engine") == engine and row.get("status") == "ok"
                )
                for engine in benchmark.PARSER_ENGINES
            },
            "slowest_document_medians_ms": {
                engine: benchmark.slowest_document_medians(
                    parse_persistent_rows,
                    engine,
                    "parse_ms",
                    expected_rows=persistent_repetitions,
                )
                for engine in benchmark.PARSER_ENGINES
            },
            "wellfriend_vs_poppler_persistent_paired_ratio": benchmark.bootstrap_ratio_ci(
                {
                    path: poppler_document_medians[path]
                    for path in qualified_parse_paths
                    if path in poppler_document_medians
                },
                {
                    path: wellfriend_document_medians[path]
                    for path in qualified_parse_paths
                    if path in wellfriend_document_medians
                },
                args.seed,
            ),
        }
    )
    summary["rendering"].update(
        {
            "profile": "process-resident adapter; document reopened for every request; final-raster reuse disabled by construction",
            "accepted": benchmark.successful_engine_documents(
                render_persistent_rows,
                benchmark.RENDER_ENGINES,
                persistent_repetitions,
                ("render_ms", "width", "height"),
            ),
            "persistent_render_ms": render_document_medians,
            "process_resident_document_cold_document_medians_ms": render_document_medians,
            "process_resident_document_cold_observations_ms": {
                engine: benchmark.distribution(
                    row["render_ms"]
                    for row in render_persistent_rows
                    if row.get("engine") == engine and row.get("status") == "ok"
                )
                for engine in benchmark.RENDER_ENGINES
            },
            "slowest_document_medians_ms": {
                engine: benchmark.slowest_document_medians(
                    render_persistent_rows,
                    engine,
                    "render_ms",
                    expected_rows=persistent_repetitions,
                )
                for engine in benchmark.RENDER_ENGINES
            },
            "raster_determinism": benchmark.raster_determinism(
                render_persistent_rows, persistent_repetitions
            ),
        }
    )
    if retained_resource_rows:
        summary["rendering"].update(
            {
                "retained_resource_render_document_medians_ms": {
                    engine: benchmark.distribution(
                        row["render_ms"]
                        for row in retained_resource_rows
                        if row.get("engine") == engine and row.get("status") == "ok"
                    )
                    for engine in benchmark.RENDER_ENGINES
                },
                "retained_resource_render_samples_ms": {
                    engine: benchmark.distribution(
                        sample
                        for row in retained_resource_rows
                        if row.get("engine") == engine and row.get("status") == "ok"
                        for sample in row.get("render_samples_ms", [])
                    )
                    for engine in benchmark.RENDER_ENGINES
                },
                "retained_resource_raster_determinism": benchmark.retained_sample_determinism(
                    retained_resource_rows
                ),
            }
        )

    if args.adapter_dir is not None:
        environment_path = args.results / "environment.json"
        environment = json.loads(environment_path.read_text(encoding="utf-8"))
        environment.setdefault("versions", {})["pdfium"] = benchmark.pdfium_version(
            args.adapter_dir / "pebq-pdfium"
        )
        environment_path.write_text(
            json.dumps(environment, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        summary["environment"] = environment

    if args.recompute_quality:
        if args.corpus is None:
            parser.error("--corpus is required with --recompute-quality")
        inputs = [args.corpus / row["relative_path"] for row in manifest]
        quality = benchmark.quality_metrics(
            args.results / "rasters", inputs, args.corpus
        )
        page_rows = quality.pop("pages", None)
        if page_rows is not None:
            with (args.results / "quality-pages.jsonl").open(
                "w", encoding="utf-8", newline="\n"
            ) as stream:
                for row in page_rows:
                    stream.write(json.dumps(row, sort_keys=True) + "\n")
        summary["quality"] = quality

    contact_sheets = sorted((args.results / "visual" / "contacts").glob("*.webp"))
    page_sheets = args.results / "visual" / "pages"
    if contact_sheets or page_sheets.is_dir():
        links = [
            {
                "label": path.stem.replace("contact-", "Pages ").replace("-", "–"),
                "path": path.relative_to(args.results).as_posix(),
            }
            for path in contact_sheets
        ]
        if page_sheets.is_dir():
            links.append(
                {
                    "label": f"All {len(manifest)} full comparison sheets",
                    "path": page_sheets.relative_to(args.results).as_posix() + "/",
                }
            )
        summary["quality"]["visual_evidence"] = {
            "description": (
                "Each comparison sheet contains unscaled renderer output and "
                "amplified absolute-difference maps against the leave-one-engine-out consensus."
            ),
            "links": links,
        }

    fresh_rows = load_jsonl(args.results / "parse-fresh.jsonl")
    wellfriend = benchmark.median_by_key(
        (row for row in fresh_rows if row.get("engine") == "wellfriendpdf"),
        "process_ms",
    )
    poppler = benchmark.median_by_key(
        (row for row in fresh_rows if row.get("engine") == "poppler"),
        "process_ms",
    )
    summary["parsing"]["wellfriend_vs_poppler_fresh_paired_ratio"] = (
        benchmark.bootstrap_ratio_ci(poppler, wellfriend, args.seed)
    )
    summary["source_revision"] = args.source_revision
    summary["raw_row_counts"] = {
        name: sum(1 for line in (args.results / name).open(encoding="utf-8") if line.strip())
        for name in RAW_FILES
        if (args.results / name).is_file()
    }
    summary["source_sha256"] = {
        path.name: benchmark.sha256_file(path)
        for path in [args.benchmark_script, *args.source]
    }
    if args.benchmark_execution_sha256:
        summary["source_sha256"]["pebq_benchmark_execution.py"] = (
            args.benchmark_execution_sha256
        )
        summary["source_sha256"]["pebq_benchmark_reporter.py"] = (
            summary["source_sha256"].pop(args.benchmark_script.name)
        )

    summary_path.write_text(
        json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    (args.results / "README.md").write_text(
        benchmark.build_report(summary), encoding="utf-8", newline="\n"
    )
    print(
        json.dumps(
            {
                "status": "finalized",
                "fresh_paired_ratio": summary["parsing"][
                    "wellfriend_vs_poppler_fresh_paired_ratio"
                ],
                "raw_row_counts": summary["raw_row_counts"],
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
