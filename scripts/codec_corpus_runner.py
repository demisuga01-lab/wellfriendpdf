#!/usr/bin/env python3
"""Run bounded decode/parser-report checks over hostile codec/PDF corpora."""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import subprocess
import time
from pathlib import Path


PDF_EXTS = {".pdf"}
RAW_CODEC_EXTS = {".jpg", ".jpeg", ".jpx", ".jp2", ".j2k", ".jbig2", ".jb2", ".ccitt", ".fax"}


def detect_kind(path: Path, prefix: bytes) -> str:
    suffix = path.suffix.lower()
    if suffix in PDF_EXTS or prefix.startswith(b"%PDF-"):
        return "pdf"
    if suffix in {".jpg", ".jpeg"} or prefix.startswith(b"\xff\xd8"):
        return "jpeg"
    if suffix in {".jp2", ".jpx", ".j2k"} or b"jP  " in prefix[:16]:
        return "jpx"
    if suffix in {".jbig2", ".jb2"} or prefix.startswith(b"\x97JB2"):
        return "jbig2"
    if suffix in {".ccitt", ".fax"}:
        return "ccitt"
    return "unknown"


def iter_files(root: Path, max_bytes: int):
    for path in sorted(root.rglob("*")):
        if not path.is_file():
            continue
        try:
            size = path.stat().st_size
        except OSError:
            continue
        if size > max_bytes:
            yield path, size, "skipped_too_large"
        else:
            yield path, size, "candidate"


def run_pdf(path: Path, wellfriendpdf: Path, timeout: int) -> dict:
    cmd = [
        str(wellfriendpdf),
        "parser-report",
        str(path),
        "--mode",
        "audit",
        "--json",
        "--include-decode",
    ]
    started = time.time()
    try:
        completed = subprocess.run(
            cmd,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=timeout,
        )
        elapsed = time.time() - started
        if completed.returncode == 0:
            try:
                report = json.loads(completed.stdout)
                decode = report.get("decode") or {}
                metrics = decode.get("metrics") or {}
                return {
                    "status": "ok",
                    "elapsed_sec": elapsed,
                    "streams_seen": metrics.get("streams_seen", 0),
                    "streams_failed": metrics.get("streams_failed", 0),
                    "unsupported_filters": metrics.get("unsupported_filters", 0),
                    "diagnostics": len(report.get("diagnostics", [])),
                }
            except json.JSONDecodeError:
                return {"status": "bad_json", "elapsed_sec": elapsed}
        return {
            "status": "failed",
            "elapsed_sec": elapsed,
            "returncode": completed.returncode,
            "stderr_tail": completed.stderr[-2000:],
        }
    except subprocess.TimeoutExpired:
        return {"status": "timeout", "elapsed_sec": timeout}


def process_file(item, wellfriendpdf: Path, timeout: int) -> dict:
    path, size, candidate_status = item
    record = {"path": str(path), "size": size}
    if candidate_status != "candidate":
        record["status"] = candidate_status
        return record
    prefix = path.read_bytes()[:64]
    kind = detect_kind(path, prefix)
    record["kind"] = kind
    if kind == "pdf":
        record.update(run_pdf(path, wellfriendpdf, timeout))
    elif kind in {"jpeg", "jpx", "jbig2", "ccitt"}:
        record.update(
            {
                "status": "raw_codec_metadata_only",
                "note": "raw codec samples are cataloged here; PDF stream decode uses parser-report --include-decode",
            }
        )
    else:
        record["status"] = "skipped_unknown_kind"
    return record


def write_markdown(path: Path, records: list[dict]) -> None:
    counts: dict[str, int] = {}
    for record in records:
        counts[record.get("status", "unknown")] = counts.get(record.get("status", "unknown"), 0) + 1
    lines = ["# Codec Corpus Runner", "", "## Summary", ""]
    for status, count in sorted(counts.items()):
        lines.append(f"- `{status}`: {count}")
    lines.extend(["", "## Files", ""])
    for record in records:
        lines.append(f"- `{record['status']}` `{record['path']}` kind={record.get('kind', 'n/a')} size={record['size']}")
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    parser.add_argument("--wellfriendpdf", type=Path, default=Path("target/debug/wellfriendpdf.exe"))
    parser.add_argument("--limit", type=int, default=50)
    parser.add_argument("--timeout", type=int, default=30)
    parser.add_argument("--jobs", type=int, default=1)
    parser.add_argument("--max-bytes", type=int, default=50 * 1024 * 1024)
    parser.add_argument("--jsonl", type=Path, default=None)
    parser.add_argument("--markdown", type=Path, default=None)
    args = parser.parse_args()

    files = list(iter_files(args.root, args.max_bytes))[: args.limit]
    records: list[dict] = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, args.jobs)) as pool:
        futures = [pool.submit(process_file, item, args.wellfriendpdf, args.timeout) for item in files]
        for future in concurrent.futures.as_completed(futures):
            records.append(future.result())
    records.sort(key=lambda item: item["path"])

    if args.jsonl:
        args.jsonl.parent.mkdir(parents=True, exist_ok=True)
        args.jsonl.write_text(
            "".join(json.dumps(record, sort_keys=True) + "\n" for record in records),
            encoding="utf-8",
        )
    if args.markdown:
        args.markdown.parent.mkdir(parents=True, exist_ok=True)
        write_markdown(args.markdown, records)
    print(json.dumps({"files": len(records), "records": records}, indent=2))
    return 0 if all(record.get("status") != "timeout" for record in records) else 1


if __name__ == "__main__":
    raise SystemExit(main())
