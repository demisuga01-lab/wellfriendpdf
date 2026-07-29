#!/usr/bin/env python3
"""Minimize a cargo-fuzz crash artifact and record Crypto Standards Fuzz evidence."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
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
    parser.add_argument("crash", type=Path)
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--output", type=Path, default=Path("target/crypto_standards_fuzz-verapdf-crypto-fuzz/parser-crash-triage.json"))
    parser.add_argument("--timeout", type=int, default=900)
    args = parser.parse_args()
    repo = args.repo.resolve()
    crash = args.crash.resolve()
    minimized = repo / "target" / "crypto_standards_fuzz-verapdf-crypto-fuzz" / "minimized" / args.target / crash.name
    minimized.parent.mkdir(parents=True, exist_ok=True)
    cmd = ["cargo", "+nightly", "fuzz", "tmin", args.target, str(crash), str(minimized), "-D"]
    started = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    proc = subprocess.run(
        cmd,
        cwd=repo / "fuzz",
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        timeout=args.timeout,
    )
    payload = {
        "schema_version": "crypto_standards_fuzz.parser-crash-triage.v1",
        "generated_at_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "started_at_utc": started,
        "target": args.target,
        "input_crash": str(crash),
        "input_sha256": sha256(crash),
        "minimized": str(minimized) if minimized.exists() else None,
        "minimized_sha256": sha256(minimized) if minimized.exists() else None,
        "command": cmd,
        "exit_code": proc.returncode,
        "status": "minimized" if proc.returncode == 0 and minimized.exists() else "failed",
        "tail": proc.stdout.splitlines()[-120:],
    }
    output = args.output if args.output.is_absolute() else repo / args.output
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"output": str(output), "status": payload["status"]}, sort_keys=True))
    return 0 if payload["status"] == "minimized" else 2


if __name__ == "__main__":
    raise SystemExit(main())
