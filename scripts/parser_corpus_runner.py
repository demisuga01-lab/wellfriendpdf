#!/usr/bin/env python3
"""Run a bounded parser-report corpus pass and emit JSONL.

The runner is SafeDocs-compatible by design: point --input at a SafeDocs checkout
or any PDF directory. It does not vendor external corpora and defaults to a
deterministic <=200-file gate.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import time
from pathlib import Path


def iter_pdfs(
    root: Path,
    limit: int,
    max_bytes_per_file: int | None,
    max_total_bytes: int | None,
    resume_seen: set[str],
) -> list[Path]:
    candidates = [root] if root.is_file() else sorted(root.rglob("*.pdf"))
    selected: list[Path] = []
    total = 0
    for pdf in candidates:
        key = str(pdf)
        if key in resume_seen:
            continue
        size = pdf.stat().st_size
        if max_bytes_per_file is not None and size > max_bytes_per_file:
            continue
        if max_total_bytes is not None and total + size > max_total_bytes:
            break
        selected.append(pdf)
        total += size
        if len(selected) >= limit:
            break
    return selected


def infer_category(root: Path, pdf: Path) -> str:
    try:
        relative = pdf.relative_to(root if root.is_dir() else root.parent)
    except ValueError:
        return "uncategorized"
    if len(relative.parts) > 1:
        return relative.parts[0]
    stem = pdf.stem.lower()
    for keyword in [
        "compact",
        "dialect",
        "inline",
        "target",
        "unicode",
        "malformed",
        "truncated",
        "xref",
        "stream",
    ]:
        if keyword in stem:
            return keyword
    return "uncategorized"


def run_one(
    oxide_bin: Path, pdf: Path, mode: str, timeout: float, category: str
) -> dict[str, object]:
    start = time.perf_counter()
    try:
        proc = subprocess.run(
            [str(oxide_bin), "parser-report", str(pdf), "--mode", mode, "--json"],
            check=False,
            capture_output=True,
            text=True,
            timeout=timeout,
        )
        elapsed = time.perf_counter() - start
        report = json.loads(proc.stdout) if proc.stdout.strip().startswith("{") else {}
        diagnostics = report.get("diagnostics", [])
        counts: dict[str, int] = {}
        for diagnostic in diagnostics:
            severity = diagnostic.get("severity", "unknown")
            counts[severity] = counts.get(severity, 0) + 1
        return {
            "file": str(pdf),
            "size": pdf.stat().st_size,
            "category": category,
            "mode": mode,
            "open_status": bool(report.get("opened", proc.returncode == 0)),
            "exit_code": proc.returncode,
            "diagnostics_by_severity": counts,
            "recovered_object_count": sum(
                1 for diagnostic in diagnostics if diagnostic.get("category") == "repair"
            ),
            "unrecoverable_object_count": 0,
            "memory_peak_bytes": None,
            "time_seconds": round(elapsed, 6),
            "panic_crash_timeout_oom": None,
            "notes": proc.stderr.strip(),
        }
    except subprocess.TimeoutExpired:
        return {
            "file": str(pdf),
            "size": pdf.stat().st_size if pdf.exists() else None,
            "category": category,
            "mode": mode,
            "open_status": False,
            "exit_code": None,
            "diagnostics_by_severity": {},
            "recovered_object_count": 0,
            "unrecoverable_object_count": 0,
            "memory_peak_bytes": None,
            "time_seconds": timeout,
            "panic_crash_timeout_oom": "timeout",
            "notes": f"parser-report exceeded {timeout} seconds",
        }


def load_resume_seen(path: Path) -> set[str]:
    if not path.exists():
        return set()
    seen = set()
    with path.open("r", encoding="utf-8") as handle:
        for line in handle:
            try:
                record = json.loads(line)
            except json.JSONDecodeError:
                continue
            if "file" in record:
                seen.add(record["file"])
    return seen


def write_summary(path: Path, records: list[dict[str, object]]) -> None:
    by_category: dict[str, dict[str, int]] = {}
    for record in records:
        category = str(record.get("category", "uncategorized"))
        stats = by_category.setdefault(category, {"files": 0, "opened": 0, "timeouts": 0})
        stats["files"] += 1
        stats["opened"] += int(bool(record.get("open_status")))
        stats["timeouts"] += int(record.get("panic_crash_timeout_oom") == "timeout")
    lines = ["# Parser Corpus Results", "", f"Files in this run: {len(records)}", ""]
    lines.extend(["## By Category", ""])
    for category, stats in sorted(by_category.items()):
        lines.append(
            f"- {category}: {stats['opened']}/{stats['files']} opened, {stats['timeouts']} timeout(s)"
        )
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--oxide-bin", type=Path, default=Path("target/debug/oxide.exe"))
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--summary", type=Path)
    parser.add_argument("--mode", choices=["strict", "repair", "audit"], default="audit")
    parser.add_argument("--limit", type=int, default=200)
    parser.add_argument("--max-bytes-per-file", type=int)
    parser.add_argument("--max-total-bytes", type=int)
    parser.add_argument("--timeout", type=float, default=30.0)
    parser.add_argument("--workers", type=int, default=1, help="reserved; serial for deterministic logs")
    parser.add_argument("--resume", action="store_true")
    args = parser.parse_args()

    if not args.oxide_bin.exists():
        raise SystemExit(f"oxide binary not found: {args.oxide_bin}")
    resume_seen = load_resume_seen(args.output) if args.resume else set()
    pdfs = iter_pdfs(
        args.input,
        args.limit,
        args.max_bytes_per_file,
        args.max_total_bytes,
        resume_seen,
    )
    args.output.parent.mkdir(parents=True, exist_ok=True)
    records = []
    mode = "a" if args.resume else "w"
    with args.output.open(mode, encoding="utf-8") as out:
        for pdf in pdfs:
            record = run_one(
                args.oxide_bin,
                pdf,
                args.mode,
                args.timeout,
                infer_category(args.input, pdf),
            )
            records.append(record)
            out.write(json.dumps(record, sort_keys=True) + "\n")
    if args.summary:
        args.summary.parent.mkdir(parents=True, exist_ok=True)
        write_summary(args.summary, records)
    print(f"wrote {len(pdfs)} parser corpus record(s) -> {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
