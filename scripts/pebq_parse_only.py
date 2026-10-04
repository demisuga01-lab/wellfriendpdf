#!/usr/bin/env python3
"""Run only the matched document-open + resolved-page-count PEBQ contract."""

from __future__ import annotations

import argparse
import hashlib
import json
import platform
from pathlib import Path

import pebq_benchmark as pebq


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--adapter-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--limit", type=int, default=150)
    parser.add_argument("--persistent-repetitions", type=int, default=3)
    parser.add_argument("--warmup-passes", type=int, default=1)
    parser.add_argument("--fresh-repetitions", type=int, default=1)
    parser.add_argument("--timeout", type=int, default=7200)
    parser.add_argument("--cpu", type=int, default=2)
    parser.add_argument("--seed", type=int, default=20261003)
    args = parser.parse_args()

    args.output.mkdir(parents=True, exist_ok=True)
    inputs = sorted(path for path in args.corpus.rglob("*.pdf") if path.is_file())[: args.limit]
    if len(inputs) != args.limit:
        raise SystemExit(f"expected {args.limit} PDFs, found {len(inputs)}")
    adapters = {engine: args.adapter_dir / f"pebq-{engine}" for engine in pebq.PARSER_ENGINES}
    missing = [str(path) for path in adapters.values() if not path.is_file()]
    if missing:
        raise SystemExit(f"missing adapters: {missing}")

    manifest = [
        {
            "relative_path": path.relative_to(args.corpus).as_posix(),
            "bytes": path.stat().st_size,
            "sha256": pebq.sha256_file(path),
        }
        for path in inputs
    ]
    manifest_bytes = (json.dumps(manifest, sort_keys=True, separators=(",", ":")) + "\n").encode()
    (args.output / "corpus-manifest.json").write_bytes(manifest_bytes)
    environment = {
        "schema": "wellfriendpdf.pebq.parse-only.v1",
        "started_at_utc": pebq.utc_now(),
        "hostname": platform.node(),
        "platform": platform.platform(),
        "corpus_manifest_sha256": hashlib.sha256(manifest_bytes).hexdigest(),
        "adapter_sha256": {name: pebq.sha256_file(path) for name, path in adapters.items()},
        "contract": "document open plus resolved page count",
        "persistent_repetitions": args.persistent_repetitions,
        "warmup_passes": args.warmup_passes,
        "fresh_repetitions": args.fresh_repetitions,
        "cpu": args.cpu,
    }
    (args.output / "environment.json").write_text(json.dumps(environment, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    persistent = pebq.run_persistent(
        "page-count",
        inputs,
        args.corpus,
        adapters,
        pebq.PARSER_ENGINES,
        args.persistent_repetitions,
        72,
        None,
        args.seed,
        args.cpu,
        args.warmup_passes,
        args.output / "parse-persistent.jsonl",
    )
    fresh = pebq.run_fresh(
        "page-count",
        inputs,
        args.corpus,
        adapters,
        pebq.PARSER_ENGINES,
        args.fresh_repetitions,
        72,
        args.seed + 1,
        args.cpu,
        args.timeout,
        args.output / "parse-fresh.jsonl",
    )
    qualification = pebq.page_count_qualification(persistent, args.persistent_repetitions)
    summary = {
        **environment,
        "finished_at_utc": pebq.utc_now(),
        "document_count": len(inputs),
        "qualification": qualification,
        "persistent_document_medians_ms": {
            engine: pebq.document_median_distribution(
                persistent, engine, "parse_ms", args.persistent_repetitions
            )
            for engine in pebq.PARSER_ENGINES
        },
        "persistent_raw_observations_ms": {
            engine: pebq.distribution(
                row["parse_ms"]
                for row in persistent
                if row.get("engine") == engine and row.get("status") == "ok"
            )
            for engine in pebq.PARSER_ENGINES
        },
        "fresh_process_ms": {
            engine: pebq.distribution(
                row["process_ms"]
                for row in fresh
                if row.get("engine") == engine and row.get("status") == "ok"
            )
            for engine in pebq.PARSER_ENGINES
        },
    }
    (args.output / "summary.json").write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
