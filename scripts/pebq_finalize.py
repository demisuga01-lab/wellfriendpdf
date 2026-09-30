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
    parser.add_argument("--seed", type=int, default=20260936)
    args = parser.parse_args()

    benchmark = load_benchmark_module(args.benchmark_script)
    summary_path = args.results / "summary.json"
    summary = json.loads(summary_path.read_text(encoding="utf-8"))

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
    }
    summary["source_sha256"] = {
        path.name: benchmark.sha256_file(path)
        for path in [args.benchmark_script, *args.source]
    }

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
