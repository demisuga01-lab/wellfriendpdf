#!/usr/bin/env python3
"""Small delimiter-scanner benchmark harness for Prompt 02B follow-up runs."""

from __future__ import annotations

import argparse
import json
import time
from pathlib import Path


MARKERS = [b"obj", b"endobj", b"stream", b"endstream", b"xref", b"trailer", b"startxref"]


def scan(data: bytes) -> int:
    count = 0
    for marker in MARKERS:
        start = 0
        while True:
            found = data.find(marker, start)
            if found < 0:
                break
            count += 1
            start = found + 1
    return count


def synthetic(size_mb: int, false_markers: bool) -> bytes:
    chunk = b"0123456789abcdef" * 4096
    data = bytearray()
    target = size_mb * 1024 * 1024
    while len(data) < target:
        data.extend(chunk)
        if false_markers:
            data.extend(b" binary endstream false obj trailer ")
        else:
            data.extend(b"\n1 0 obj\n<<>>\nendobj\n")
    return bytes(data[:target])


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--file", type=Path)
    parser.add_argument("--size-mb", type=int, default=16)
    parser.add_argument("--false-markers", action="store_true")
    parser.add_argument("--json", type=Path)
    args = parser.parse_args()

    data = args.file.read_bytes() if args.file else synthetic(args.size_mb, args.false_markers)
    started = time.perf_counter()
    candidates = scan(data)
    elapsed = time.perf_counter() - started
    mb = len(data) / (1024 * 1024)
    result = {
        "bytes": len(data),
        "candidates": candidates,
        "elapsed_sec": elapsed,
        "throughput_mb_s": mb / elapsed if elapsed else None,
        "implementation": "python_scalar_harness",
    }
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(result, indent=2), encoding="utf-8")
    print(json.dumps(result, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
