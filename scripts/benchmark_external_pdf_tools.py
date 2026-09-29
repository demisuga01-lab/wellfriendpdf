#!/usr/bin/env python3
"""Benchmark independently installed PDF inspection tools on a fixed corpus.

The commands intentionally remain distinct and are named in every observation:
qpdf performs a structural check, MuPDF inventories document information, and
Poppler reads document metadata.  Timings are therefore comparable as fresh
process costs, not as claims that the tools perform equivalent semantic work.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


TOOLS: tuple[tuple[str, tuple[str, ...], frozenset[int]], ...] = (
    # qpdf documents exit 3 as success with warnings.
    ("qpdf", ("qpdf", "--check"), frozenset((0, 3))),
    ("mupdf", ("mutool", "info"), frozenset((0,))),
    ("poppler", ("pdfinfo",), frozenset((0,))),
)

VERSION_COMMANDS: dict[str, tuple[tuple[str, ...], ...]] = {
    "qpdf": (("qpdf", "--version"),),
    "mupdf": (("mutool", "-v"), ("mutool", "--version")),
    "poppler": (("pdfinfo", "-v"),),
}


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def pdfs(root: Path, limit: int) -> list[Path]:
    found = sorted(
        (path for path in root.rglob("*") if path.is_file() and path.suffix.lower() == ".pdf"),
        key=lambda path: path.relative_to(root).as_posix(),
    )
    return found[:limit] if limit > 0 else found


def version(candidates: tuple[tuple[str, ...], ...]) -> dict[str, Any]:
    for invocation in candidates:
        try:
            result = subprocess.run(invocation, capture_output=True, text=True, timeout=10, check=False)
        except (OSError, subprocess.TimeoutExpired) as error:
            last_error = str(error)
            continue
        text = (result.stdout + "\n" + result.stderr).strip()
        if text:
            return {"command": list(invocation), "exit": result.returncode, "text": text[:1000]}
        last_error = f"exit {result.returncode} with no version output"
    return {"error": last_error}


def run_tool(
    prefix: tuple[str, ...], accepted_codes: frozenset[int], path: Path, timeout_sec: int
) -> dict[str, Any]:
    command = [*prefix, str(path)]
    started = time.perf_counter()
    try:
        result = subprocess.run(
            command,
            capture_output=True,
            text=True,
            timeout=timeout_sec,
            check=False,
        )
    except subprocess.TimeoutExpired as error:
        return {
            "command": command,
            "status": "timeout",
            "duration_ms": round((time.perf_counter() - started) * 1000.0, 6),
            "timeout_sec": timeout_sec,
            "stdout_tail": (error.stdout or "")[-2000:] if isinstance(error.stdout, str) else "",
            "stderr_tail": (error.stderr or "")[-2000:] if isinstance(error.stderr, str) else "",
        }
    except OSError as error:
        return {
            "command": command,
            "status": "launch_error",
            "duration_ms": round((time.perf_counter() - started) * 1000.0, 6),
            "error": str(error),
        }
    return {
        "command": command,
        "status": (
            "accepted_with_warnings"
            if result.returncode == 3 and result.returncode in accepted_codes
            else "accepted"
            if result.returncode in accepted_codes
            else "rejected"
        ),
        "exit": result.returncode,
        "duration_ms": round((time.perf_counter() - started) * 1000.0, 6),
        "stdout_tail": result.stdout[-2000:],
        "stderr_tail": result.stderr[-2000:],
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--limit", type=int, default=0)
    parser.add_argument("--timeout-sec", type=int, default=180)
    args = parser.parse_args()

    inputs = pdfs(args.corpus, args.limit)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    metadata = {
        "schema_version": "wellfriend.external_pdf_tool_benchmark.v1",
        "started_at_utc": utc_now(),
        "corpus": str(args.corpus.resolve()),
        "files": len(inputs),
        "timeout_sec": args.timeout_sec,
        "protocol": "one fresh process per tool and PDF; deterministic order rotation",
        "workload_warning": "commands are intentionally different and are not semantic-equivalence claims",
        "versions": {name: version(VERSION_COMMANDS[name]) for name, _, _ in TOOLS},
    }
    with args.output.open("w", encoding="utf-8") as stream:
        stream.write(json.dumps({"metadata": metadata}, sort_keys=True) + "\n")
        for index, path in enumerate(inputs, start=1):
            rotated = TOOLS[(index - 1) % len(TOOLS) :] + TOOLS[: (index - 1) % len(TOOLS)]
            commands = {
                name: run_tool(prefix, accepted_codes, path, args.timeout_sec)
                for name, prefix, accepted_codes in rotated
            }
            row = {
                "index": index,
                "relative_path": path.relative_to(args.corpus).as_posix(),
                "input_bytes": path.stat().st_size,
                "input_sha256": sha256_file(path),
                "order": [name for name, _, _ in rotated],
                "commands": commands,
            }
            stream.write(json.dumps(row, sort_keys=True) + "\n")
            stream.flush()
            statuses = ", ".join(f"{name}={commands[name]['status']}" for name, _, _ in TOOLS)
            print(f"[{index}/{len(inputs)}] {path.name}: {statuses}", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
