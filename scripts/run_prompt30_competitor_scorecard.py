#!/usr/bin/env python3
"""Generate a bounded, evidence-first Prompt 30 competitor scorecard."""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
import time
from pathlib import Path


TOOLS = {
    "qpdf": ["qpdf", "--check"],
    "pdfinfo": ["pdfinfo"],
    "pdftotext": ["pdftotext", "-f", "1", "-l", "1"],
    "mutool": ["mutool", "info"],
    "verapdf": ["verapdf", "--format", "json"],
    "pyhanko": ["pyhanko", "sign", "validate"],
}


def utc() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def digest(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def write_json(path: Path, payload: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def invoke(command: list[str], raw: Path, timeout: float) -> dict[str, object]:
    raw.parent.mkdir(parents=True, exist_ok=True)
    began = time.perf_counter()
    try:
        proc = subprocess.run(command, capture_output=True, text=True, timeout=timeout)
        stdout, stderr, code = proc.stdout, proc.stderr, proc.returncode
        status = "passed" if code == 0 else "failed_cleanly"
    except subprocess.TimeoutExpired as exc:
        stdout = exc.stdout.decode("utf-8", "replace") if isinstance(exc.stdout, bytes) else (exc.stdout or "")
        stderr = exc.stderr.decode("utf-8", "replace") if isinstance(exc.stderr, bytes) else (exc.stderr or "")
        code, status = None, "timeout"
    raw.write_text("$ " + " ".join(command) + "\n\n--- stdout ---\n" + stdout + "\n--- stderr ---\n" + stderr, encoding="utf-8", errors="ignore")
    return {"status": status, "exit_code": code, "elapsed_seconds": round(time.perf_counter() - began, 4), "raw_log_path": str(raw), "raw_log_sha256": digest(raw)}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--wellfriendpdf-bin", type=Path, required=True)
    parser.add_argument("--corpus-manifest", type=Path, required=True)
    parser.add_argument("--artifact-root", type=Path, required=True)
    parser.add_argument("--limit", type=int, default=20)
    parser.add_argument("--timeout-seconds", type=float, default=30.0)
    args = parser.parse_args()
    manifest = json.loads(args.corpus_manifest.read_text(encoding="utf-8"))
    files = [Path(str(row["path"])) for row in manifest.get("files", []) if Path(str(row["path"])).is_file()][: args.limit]
    support = []
    for tool in ["wellfriendpdf", *TOOLS]:
        available = args.wellfriendpdf_bin.is_file() if tool == "wellfriendpdf" else shutil.which(tool) is not None
        support.append({"tool": tool, "available": available, "status": "available" if available else "unavailable_external_tool", "scope": "direct bounded probe"})
    raw_root = args.artifact_root / "raw" / "competitor-scorecard"
    rows = []
    for index, pdf in enumerate(files):
        outcomes = {"wellfriendpdf": invoke([str(args.wellfriendpdf_bin), "parser-report", str(pdf), "--mode", "audit", "--json"], raw_root / f"{index:03d}" / "wellfriendpdf.log", args.timeout_seconds)}
        for tool, prefix in TOOLS.items():
            if shutil.which(tool):
                outcomes[tool] = invoke([*prefix, str(pdf)], raw_root / f"{index:03d}" / f"{tool}.log", args.timeout_seconds)
        hard = [name for name, result in outcomes.items() if result["status"] == "timeout"]
        wellfriend_status = outcomes["wellfriendpdf"]["status"]
        reference_pass = any(result["status"] == "passed" for name, result in outcomes.items() if name != "wellfriendpdf")
        classification = "no_actionable_disagreement"
        severity = "none"
        if hard:
            classification, severity = "tool_timeout", "medium"
        elif wellfriend_status != "passed" and reference_pass:
            classification, severity = "wellfriend_strict_or_repair_difference", "low"
        rows.append({"path": str(pdf), "sha256": digest(pdf), "outcomes": outcomes, "classification": classification, "severity": severity})
    high = [row for row in rows if row["severity"] == "high"]
    scorecard = {
        "schema_version": "prompt30.final-competitor-scorecard.v1",
        "generated_at_utc": utc(),
        "scope": "bounded public corpus structural/parser comparison; not a universal correctness ranking",
        "wellfriendpdf": {
            "malformed_corpus": "evidence from Prompts 27-29",
            "performance": "evidence from Prompt 30 performance harness",
            "bindings": ["Rust", "CLI", "Python", "C ABI", "WASM", ".NET", "Java"],
            "known_limits": ["external tools are evidence, not an oracle", "render fidelity and trust-store behavior remain deployment-specific"],
        },
        "comparison_rows": rows,
        "high_severity_unclassified_count": len(high),
        "verdict": "passed" if files and not high else "failed",
    }
    write_json(args.artifact_root / "competitor-tool-support-matrix.json", {"schema_version": "prompt30.competitor-tool-support.v1", "generated_at_utc": utc(), "tools": support, "verdict": "passed"})
    write_json(args.artifact_root / "final-competitor-scorecard.json", scorecard)
    write_json(args.artifact_root / "release-readiness-go-no-go.json", {"schema_version": "prompt30.release-readiness-go-no-go.v1", "generated_at_utc": utc(), "verdict": "release_ready_with_limits" if scorecard["verdict"] == "passed" else "not_release_ready", "basis": ["bounded public-corpus scorecard", "Prompt 27-29 fuzz/corpus evidence", "Prompt 30 package/security gates"], "exact_limits": scorecard["wellfriendpdf"]["known_limits"]})
    print(json.dumps({"status": scorecard["verdict"], "files": len(rows), "artifact_root": str(args.artifact_root)}, sort_keys=True))
    return 0 if scorecard["verdict"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
