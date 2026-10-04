#!/usr/bin/env python3
"""Run only the PEBQ retained-resource, fresh-raster renderer profile."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from pebq_benchmark import (
    RENDER_ENGINES,
    document_median_distribution,
    retained_sample_determinism,
    run_retained_resource_render,
)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--adapter-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--limit", type=int, default=100)
    parser.add_argument("--dpi", type=int, default=144)
    parser.add_argument("--iterations", type=int, default=5)
    parser.add_argument("--cpu", type=int, default=2)
    parser.add_argument("--seed", type=int, default=20261003)
    args = parser.parse_args()

    inputs = sorted(
        path
        for path in args.corpus.rglob("*")
        if path.is_file() and path.suffix.lower() == ".pdf"
    )
    if args.limit > 0:
        inputs = inputs[: args.limit]
    if len(inputs) != args.limit:
        raise SystemExit(f"expected {args.limit} PDFs, found {len(inputs)}")

    adapters = {
        engine: args.adapter_dir / f"pebq-{engine}" for engine in RENDER_ENGINES
    }
    missing = [str(path) for path in adapters.values() if not path.is_file()]
    if missing:
        raise SystemExit(f"missing adapters: {missing}")

    args.output.mkdir(parents=True, exist_ok=True)
    rows = run_retained_resource_render(
        inputs,
        args.corpus,
        adapters,
        args.iterations,
        args.dpi,
        args.seed,
        args.cpu,
        args.output / "render-retained-resources.jsonl",
    )
    summary = {
        "schema": "wellfriendpdf.pebq.retained-resources.v1",
        "configuration": {
            "document_count": len(inputs),
            "dpi": args.dpi,
            "iterations": args.iterations,
            "cpu": args.cpu,
            "seed": args.seed,
            "final_raster_cache": "disabled",
        },
        "document_medians_ms": {
            engine: document_median_distribution(rows, engine, "render_ms", 1)
            for engine in RENDER_ENGINES
        },
        "determinism": retained_sample_determinism(rows),
    }
    (args.output / "retained-summary.json").write_text(
        json.dumps(summary, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps(summary, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
