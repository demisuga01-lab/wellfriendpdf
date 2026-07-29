#!/usr/bin/env python3
"""Malformed Coverage bounded sanitizer runner.

Runs supported sanitizer smoke gates and records exact unsupported posture for
sanitizers that are not practical on the current toolchain. Raw sanitizer logs
are retained under the artifact root and summarized in JSON.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import time
from pathlib import Path


def utc() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def write_json(path: Path, payload: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def run(cmd: list[str], log: Path, timeout: int, env: dict[str, str] | None = None) -> dict[str, object]:
    log.parent.mkdir(parents=True, exist_ok=True)
    merged_env = os.environ.copy()
    if env:
        merged_env.update(env)
    start = time.perf_counter()
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, env=merged_env)
        status = "passed" if proc.returncode == 0 else "failed"
        exit_code: int | None = proc.returncode
        stdout = proc.stdout
        stderr = proc.stderr
    except subprocess.TimeoutExpired as exc:
        status = "timeout"
        exit_code = None
        stdout = exc.stdout.decode("utf-8", "replace") if isinstance(exc.stdout, bytes) else (exc.stdout or "")
        stderr = exc.stderr.decode("utf-8", "replace") if isinstance(exc.stderr, bytes) else (exc.stderr or "")
    elapsed = time.perf_counter() - start
    log.write_text("$ " + " ".join(cmd) + "\n\n--- stdout ---\n" + stdout + "\n--- stderr ---\n" + stderr, encoding="utf-8", errors="ignore")
    text = (stdout + stderr).lower()
    sanitizer_failure = any(token in text for token in ["addresssanitizer", "undefinedbehaviorsanitizer", "threadsanitizer", "memorysanitizer"])
    if sanitizer_failure and status != "passed":
        status = "sanitizer_failure"
    return {"command": cmd, "status": status, "exit_code": exit_code, "time_seconds": round(elapsed, 6), "raw_log_path": str(log), "raw_log_sha256": sha256(log)}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--artifact-root", type=Path, default=Path("target/malformed_coverage-malformed-differential-coverage"))
    parser.add_argument("--timeout-seconds", type=int, default=1800)
    parser.add_argument("--memory-mb", type=int, default=4096)
    args = parser.parse_args()

    artifact_root = args.artifact_root.resolve()
    raw = artifact_root / "raw" / "sanitizers"
    nightly_available = subprocess.run(["cargo", "+nightly", "--version"], capture_output=True, text=True).returncode == 0
    cargo_fuzz = shutil.which("cargo-fuzz") is not None or "fuzz" in subprocess.run(["cargo", "--list"], capture_output=True, text=True).stdout
    support_rows = [
        {"sanitizer": "address", "available": nightly_available and cargo_fuzz, "status": "available" if nightly_available and cargo_fuzz else "unavailable_external_tool"},
        {"sanitizer": "undefined", "available": nightly_available, "status": "available_if_nightly_build_std_succeeds" if nightly_available else "unavailable_external_tool"},
        {"sanitizer": "memory", "available": False, "status": "unsupported_reported_exact", "reason": "Rust MSan requires fully instrumented dependencies and is not a standard release gate here"},
        {"sanitizer": "thread", "available": False, "status": "unsupported_reported_exact", "reason": "TSan is tracked in CI posture; not run for this bounded Malformed Coverage VPS gate"},
    ]
    runs = []
    if nightly_available and cargo_fuzz:
        runs.append(
            run(
                [
                    "cargo",
                    "+nightly",
                    "fuzz",
                    "run",
                    "--sanitizer",
                    "address",
                    "parse_pdf",
                    "--",
                    "-runs=64",
                    "-max_len=262144",
                    f"-rss_limit_mb={args.memory_mb}",
                    "-timeout=30",
                ],
                raw / "asan-parse-pdf-fuzz.log",
                args.timeout_seconds,
                {"ASAN_OPTIONS": "detect_leaks=1:abort_on_error=1"},
            )
        )
    else:
        runs.append({"sanitizer": "address", "status": "unavailable_external_tool", "reason": "nightly cargo or cargo-fuzz unavailable"})
    failures = [row for row in runs if row.get("status") not in {"passed", "unavailable_external_tool"}]
    write_json(
        artifact_root / "sanitizer-support-matrix.json",
        {"schema_version": "malformed_coverage.sanitizer-support-matrix.v1", "generated_at_utc": utc(), "rows": support_rows, "verdict": "passed"},
    )
    write_json(
        artifact_root / "sanitizer-run-results.json",
        {"schema_version": "malformed_coverage.sanitizer-run-results.v1", "generated_at_utc": utc(), "runs": runs, "failure_count": len(failures), "verdict": "passed" if not failures else "failed"},
    )
    write_json(
        artifact_root / "sanitizer-failure-triage.json",
        {"schema_version": "malformed_coverage.sanitizer-failure-triage.v1", "generated_at_utc": utc(), "findings": failures, "unclassified_count": len(failures), "verdict": "passed" if not failures else "failed"},
    )
    print(json.dumps({"status": "passed" if not failures else "failed", "failures": len(failures), "artifact": str(artifact_root / "sanitizer-run-results.json")}, sort_keys=True))
    return 0 if not failures else 1


if __name__ == "__main__":
    raise SystemExit(main())
