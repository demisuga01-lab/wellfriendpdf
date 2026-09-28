#!/usr/bin/env python3
"""Benchmark WellPDF parsing against qpdf and MuPDF on the same PDF set.

The harness records exit status, elapsed time, hashes, and bounded diagnostics.
It treats qpdf exit code 3 as a warning-bearing parse, while preserving the
exact exit code so input defects are never reported as clean validation.
"""

from __future__ import annotations

import argparse
from collections import Counter
from concurrent.futures import ThreadPoolExecutor, as_completed
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import time
from typing import Any


def digest_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def run(argv: list[str], timeout: float) -> dict[str, Any]:
    started = time.monotonic()
    try:
        process = subprocess.run(
            argv,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            errors="replace",
            timeout=timeout,
        )
        return {
            "exit": process.returncode,
            "timed_out": False,
            "duration_ms": round((time.monotonic() - started) * 1000, 3),
            "stdout_tail": process.stdout[-2000:],
            "stderr_tail": process.stderr[-2000:],
        }
    except subprocess.TimeoutExpired as error:
        return {
            "exit": None,
            "timed_out": True,
            "duration_ms": round((time.monotonic() - started) * 1000, 3),
            "stdout_tail": expired_text(error.stdout)[-2000:],
            "stderr_tail": expired_text(error.stderr)[-2000:],
        }


def expired_text(value: str | bytes | None) -> str:
    if value is None:
        return ""
    return value.decode(errors="replace") if isinstance(value, bytes) else value


def exercise(
    index: int,
    root: Path,
    pdf: Path,
    binary: Path,
    timeout: float,
) -> dict[str, Any]:
    with tempfile.TemporaryDirectory(prefix=f"wellpdf-parse-{index:03d}-") as directory:
        output = Path(directory) / "semantic.json"
        wellpdf = run(
            [
                str(binary),
                "parse",
                str(pdf),
                "--format",
                "json",
                "--profile",
                "layout-faithful",
                "--output",
                str(output),
            ],
            timeout,
        )
        valid_json = False
        output_bytes = output.stat().st_size if output.exists() else 0
        if output.exists():
            try:
                json.loads(output.read_text(encoding="utf-8", errors="strict"))
                valid_json = True
            except (OSError, UnicodeError, json.JSONDecodeError):
                pass

    qpdf = run(["qpdf", "--check", str(pdf)], min(timeout, 120.0))
    mupdf = run(["mutool", "info", str(pdf)], min(timeout, 120.0))
    return {
        "index": index,
        "relative_path": str(pdf.relative_to(root)),
        "sha256": digest_file(pdf),
        "bytes": pdf.stat().st_size,
        "wellpdf": {
            **wellpdf,
            "valid_json": valid_json,
            "output_bytes": output_bytes,
            "accepted": wellpdf["exit"] == 0 and valid_json,
        },
        "qpdf": {
            **qpdf,
            "accepted": qpdf["exit"] in {0, 3},
            "clean": qpdf["exit"] == 0,
        },
        "mupdf": {**mupdf, "accepted": mupdf["exit"] == 0},
    }


def percentile(values: list[float], fraction: float) -> float | None:
    if not values:
        return None
    ordered = sorted(values)
    position = min(len(ordered) - 1, max(0, round((len(ordered) - 1) * fraction)))
    return round(ordered[position], 3)


def tool_summary(rows: list[dict[str, Any]], name: str) -> dict[str, Any]:
    records = [row[name] for row in rows]
    durations = [float(record["duration_ms"]) for record in records]
    exits = Counter("timeout" if record["timed_out"] else str(record["exit"]) for record in records)
    return {
        "accepted": sum(bool(record["accepted"]) for record in records),
        "failed": sum(not bool(record["accepted"]) for record in records),
        "timeouts": sum(bool(record["timed_out"]) for record in records),
        "exit_counts": dict(sorted(exits.items())),
        "total_duration_ms": round(sum(durations), 3),
        "median_duration_ms": percentile(durations, 0.5),
        "p95_duration_ms": percentile(durations, 0.95),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--corpus", required=True, type=Path)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--pattern", default="*.pdf")
    parser.add_argument("--limit", type=int, default=100)
    parser.add_argument("--workers", type=int, default=4)
    parser.add_argument("--timeout-sec", type=float, default=300.0)
    args = parser.parse_args()

    pdfs = sorted(path for path in args.corpus.rglob(args.pattern) if path.is_file())[: args.limit]
    rows: list[dict[str, Any] | None] = [None] * len(pdfs)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    progress = args.output.with_suffix(args.output.suffix + ".progress.json")
    with ThreadPoolExecutor(max_workers=max(1, args.workers)) as executor:
        futures = {
            executor.submit(
                exercise,
                index,
                args.corpus,
                pdf,
                args.binary,
                args.timeout_sec,
            ): index
            for index, pdf in enumerate(pdfs, start=1)
        }
        completed = 0
        for future in as_completed(futures):
            index = futures[future]
            rows[index - 1] = future.result()
            completed += 1
            progress.write_text(
                json.dumps({"completed": completed, "total": len(pdfs)}, indent=2) + "\n",
                encoding="utf-8",
            )
            print(f"[{completed}/{len(pdfs)}] {rows[index - 1]['relative_path']}", flush=True)

    complete_rows = [row for row in rows if row is not None]
    summary = {
        "schema_version": "wellfriend.parse_reference_compare.v1",
        "corpus": str(args.corpus),
        "files_attempted": len(complete_rows),
        "binary": str(args.binary),
        "binary_sha256": digest_file(args.binary),
        "input_manifest_sha256": hashlib.sha256(
            "\n".join(f"{row['sha256']}  {row['relative_path']}" for row in complete_rows).encode()
        ).hexdigest(),
        "wellpdf": tool_summary(complete_rows, "wellpdf"),
        "qpdf": tool_summary(complete_rows, "qpdf"),
        "mupdf": tool_summary(complete_rows, "mupdf"),
        "rows": complete_rows,
    }
    args.output.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    all_accepted = all(
        row[tool]["accepted"] for row in complete_rows for tool in ("wellpdf", "qpdf", "mupdf")
    )
    return 0 if len(complete_rows) == len(pdfs) and all_accepted else 1


if __name__ == "__main__":
    raise SystemExit(main())
