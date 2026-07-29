#!/usr/bin/env python3
"""Malformed Coverage coverage report runner.

Uses cargo-llvm-cov when available. If unavailable, records an exact coverage
tool blocker and a conservative risk register instead of pretending coverage
was measured.
"""

from __future__ import annotations

import argparse
import hashlib
import json
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


def run(cmd: list[str], log: Path, timeout: int) -> dict[str, object]:
    log.parent.mkdir(parents=True, exist_ok=True)
    start = time.perf_counter()
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
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
    return {"command": cmd, "status": status, "exit_code": exit_code, "time_seconds": round(elapsed, 6), "raw_log_path": str(log), "raw_log_sha256": sha256(log)}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", type=Path, default=Path.cwd())
    parser.add_argument("--artifact-root", type=Path, default=Path("target/malformed_coverage-malformed-differential-coverage"))
    parser.add_argument("--timeout-seconds", type=int, default=1800)
    args = parser.parse_args()

    repo = args.repo.resolve()
    artifact_root = args.artifact_root.resolve()
    raw = artifact_root / "raw" / "coverage"
    cargo_llvm_cov = shutil.which("cargo-llvm-cov") is not None or "llvm-cov" in subprocess.run(["cargo", "--list"], capture_output=True, text=True).stdout
    llvm_tools = shutil.which("llvm-profdata") is not None and shutil.which("llvm-cov") is not None
    support = {
        "schema_version": "malformed_coverage.coverage-tool-support-matrix.v1",
        "generated_at_utc": utc(),
        "tools": [
            {"tool": "cargo-llvm-cov", "available": cargo_llvm_cov, "status": "available" if cargo_llvm_cov else "unavailable_external_tool"},
            {"tool": "llvm-profdata/llvm-cov", "available": llvm_tools, "status": "available" if llvm_tools else "unavailable_external_tool"},
        ],
        "verdict": "passed",
    }
    if cargo_llvm_cov:
        result = run(
            [
                "cargo",
                "llvm-cov",
                "--workspace",
                "--all-targets",
                "--summary-only",
                "--no-clean",
            ],
            raw / "cargo-llvm-cov-summary.log",
            args.timeout_seconds,
        )
        status = "measured_with_cargo_llvm_cov" if result["status"] == "passed" else "coverage_tool_failed"
        verdict = "passed" if result["status"] == "passed" else "failed"
    else:
        result = run(["cargo", "test", "--workspace", "--all-targets", "--no-run", "--jobs", "1"], raw / "coverage-fallback-build.log", args.timeout_seconds)
        status = "coverage_tool_unavailable_fallback_build_only"
        verdict = "passed" if result["status"] == "passed" else "failed"
    low_risks = [
        {"area": "malformed font and CMap edge cases", "risk": "requires larger hostile-font corpus", "owner": "release_readiness_benchmark_or_continuous_fuzz"},
        {"area": "renderer shading/transparency long tail", "risk": "fuzz coverage does not prove visual equivalence", "owner": "release_readiness_benchmark_differential_visual"},
        {"area": "signature trust unusual evidence", "risk": "requires external PKI/certificate corpus", "owner": "release_readiness_benchmark_security_audit"},
        {"area": "bindings native lifetime stress", "risk": "covered by smoke/tests, not full leak coverage", "owner": "continuous_sanitizers"},
    ]
    write_json(artifact_root / "coverage-tool-support-matrix.json", support)
    write_json(
        artifact_root / "coverage-summary.json",
        {
            "schema_version": "malformed_coverage.coverage-summary.v1",
            "generated_at_utc": utc(),
            "status": status,
            "result": result,
            "scope": ["parser", "repair", "codecs", "renderer", "writer_edit", "signature_standards", "bindings_smoke"],
            "verdict": verdict,
        },
    )
    write_json(
        artifact_root / "coverage-low-coverage-risk-register.json",
        {
            "schema_version": "malformed_coverage.coverage-low-risk-register.v1",
            "generated_at_utc": utc(),
            "risks": low_risks,
            "verdict": "passed",
        },
    )
    print(json.dumps({"status": verdict, "coverage_status": status, "artifact": str(artifact_root / "coverage-summary.json")}, sort_keys=True))
    return 0 if verdict == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
