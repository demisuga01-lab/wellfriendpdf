#!/usr/bin/env python3
"""Download a small, public-domain Prompt 30 PDF benchmark corpus.

The downloader is deliberately allow-list based.  It never crawls the web, never
uses credentials, and keeps PDFs in a caller-provided temporary directory.  The
repository receives only the manifests that describe the run.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import time
import urllib.error
import urllib.request
from pathlib import Path


SOURCES = (
    {
        "id": "irs_form_w4",
        "url": "https://www.irs.gov/pub/irs-pdf/fw4.pdf",
        "source_type": "public_us_government",
        "provenance": "United States Internal Revenue Service public form",
        "category": "form",
    },
    {
        "id": "irs_form_1040",
        "url": "https://www.irs.gov/pub/irs-pdf/f1040.pdf",
        "source_type": "public_us_government",
        "provenance": "United States Internal Revenue Service public form",
        "category": "form",
    },
    {
        "id": "ssa_retirement_benefits",
        "url": "https://www.ssa.gov/pubs/EN-05-10035.pdf",
        "source_type": "public_us_government",
        "provenance": "United States Social Security Administration public publication",
        "category": "text_heavy_publication",
    },
    {
        "id": "national_archives_constitution",
        "url": "https://www.archives.gov/files/founding-docs/constitution_transcript.pdf",
        "source_type": "public_us_government",
        "provenance": "United States National Archives public historical transcript",
        "category": "historical_text",
    },
)


def utc() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def write_json(path: Path, payload: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def download(source: dict[str, str], out_dir: Path, timeout: float, max_bytes: int) -> dict[str, object]:
    request = urllib.request.Request(
        source["url"],
        headers={"User-Agent": "WellfriendPDFSDK-Prompt30/0.1 public-corpus-runner"},
    )
    started = time.perf_counter()
    target = out_dir / f"{source['id']}.pdf"
    partial = target.with_suffix(".pdf.part")
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            content_type = response.headers.get_content_type()
            chunks: list[bytes] = []
            total = 0
            while True:
                chunk = response.read(64 * 1024)
                if not chunk:
                    break
                total += len(chunk)
                if total > max_bytes:
                    raise ValueError(f"download exceeds per-file cap of {max_bytes} bytes")
                chunks.append(chunk)
        payload = b"".join(chunks)
        if not payload.lstrip()[:1024].startswith(b"%PDF-"):
            raise ValueError("download did not have a PDF header")
        partial.write_bytes(payload)
        os.replace(partial, target)
        return {
            **source,
            "status": "downloaded",
            "retrieved_at_utc": utc(),
            "elapsed_seconds": round(time.perf_counter() - started, 4),
            "path": str(target.resolve()),
            "sha256": sha256(payload),
            "bytes": len(payload),
            "content_type": content_type,
            "safe_for_public_logs": True,
            "result_only": True,
        }
    except (OSError, ValueError, urllib.error.URLError, urllib.error.HTTPError) as exc:
        partial.unlink(missing_ok=True)
        return {
            **source,
            "status": "download_failed_cleanly",
            "retrieved_at_utc": utc(),
            "elapsed_seconds": round(time.perf_counter() - started, 4),
            "error": str(exc)[:300],
            "safe_for_public_logs": True,
            "result_only": True,
        }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument("--artifact-root", type=Path, required=True)
    parser.add_argument("--max-files", type=int, default=len(SOURCES))
    parser.add_argument("--max-file-bytes", type=int, default=12 * 1024 * 1024)
    parser.add_argument("--total-byte-cap", type=int, default=32 * 1024 * 1024)
    parser.add_argument("--timeout-seconds", type=float, default=30.0)
    args = parser.parse_args()

    args.out_dir.mkdir(parents=True, exist_ok=True)
    rows: list[dict[str, object]] = []
    total = 0
    for source in SOURCES[: max(0, args.max_files)]:
        if total >= args.total_byte_cap:
            rows.append({**source, "status": "skipped_total_byte_cap", "result_only": True})
            continue
        row = download(source, args.out_dir, args.timeout_seconds, min(args.max_file_bytes, args.total_byte_cap - total))
        rows.append(row)
        if row.get("status") == "downloaded":
            total += int(row["bytes"])

    downloaded = [row for row in rows if row.get("status") == "downloaded"]
    manifest = {
        "schema_version": "prompt30.public-pdf-corpus-manifest.v1",
        "generated_at_utc": utc(),
        "corpus_root": str(args.out_dir.resolve()),
        "downloaded_file_count": len(downloaded),
        "total_bytes": total,
        "license_policy": "public-government documents only; payloads remain result-only",
        "files": downloaded,
        "verdict": "passed" if downloaded else "failed",
    }
    results = {
        "schema_version": "prompt30.public-pdf-download-results.v1",
        "generated_at_utc": utc(),
        "requested_count": min(len(SOURCES), max(0, args.max_files)),
        "downloaded_count": len(downloaded),
        "failed_count": len([row for row in rows if row.get("status") == "download_failed_cleanly"]),
        "rows": rows,
        "verdict": "passed" if downloaded else "failed",
    }
    write_json(args.artifact_root / "public-pdf-corpus-manifest.json", manifest)
    write_json(args.artifact_root / "public-pdf-download-results.json", results)
    print(json.dumps({"status": manifest["verdict"], "downloaded": len(downloaded), "artifact_root": str(args.artifact_root)}, sort_keys=True))
    return 0 if downloaded else 1


if __name__ == "__main__":
    raise SystemExit(main())
