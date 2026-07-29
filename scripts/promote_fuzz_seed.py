#!/usr/bin/env python3
"""Promote a minimized fuzz artifact into the legal seed corpus."""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import time
from pathlib import Path


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("target")
    parser.add_argument("seed", type=Path)
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--reason", default="crypto_standards_fuzz minimized regression seed")
    parser.add_argument("--output", type=Path, default=Path("target/crypto_standards_fuzz-verapdf-crypto-fuzz/parser-seed-promotion.json"))
    args = parser.parse_args()
    repo = args.repo.resolve()
    source = args.seed.resolve()
    if not source.is_file():
        raise SystemExit(f"seed file not found: {source}")
    digest = sha256(source)
    dest = repo / "fuzz" / "corpus" / args.target / digest[:16]
    dest.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, dest)
    payload = {
        "schema_version": "crypto_standards_fuzz.parser-seed-promotion.v1",
        "generated_at_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "target": args.target,
        "source": str(source),
        "destination": str(dest),
        "sha256": digest,
        "size_bytes": source.stat().st_size,
        "reason": args.reason,
        "license_posture": "promote only test-generated, minimized, legal seed bytes",
    }
    output = args.output if args.output.is_absolute() else repo / args.output
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"destination": str(dest), "output": str(output), "sha256": digest}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
