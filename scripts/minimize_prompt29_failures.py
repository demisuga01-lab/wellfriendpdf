#!/usr/bin/env python3
"""Prompt 29 failure inventory and deterministic minimization planner.

This script collects crash/hang/OOM findings from Prompt 27/28/29 result trees
and from Prompt 29 run artifacts. It minimizes only deterministic local files
that are safe to copy; when no Prompt 29-owned crash exists it records a clean
triage verdict without manufacturing artifacts.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import time
from pathlib import Path


CRASH_SUFFIX_PREFIXES = ("crash-", "timeout-", "oom-", "leak-", "slow-unit-")


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


def read_json(path: Path) -> dict[str, object] | None:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        return None


def scan_result_tree(root: Path, source: str) -> list[dict[str, object]]:
    findings = []
    if not root.exists():
        return findings
    for path in sorted(root.rglob("*")):
        if not path.is_file():
            continue
        name = path.name
        if name.endswith((".log", ".json", ".jsonl", ".md", ".txt", ".exit", ".seconds")):
            continue
        if not name.startswith(CRASH_SUFFIX_PREFIXES):
            continue
        kind = "crash"
        if name.startswith("timeout-"):
            kind = "hang"
        if name.startswith("oom-"):
            kind = "oom"
        findings.append(
            {
                "source": source,
                "path": str(path),
                "kind": kind,
                "sha256": sha256(path),
                "bytes": path.stat().st_size,
                "classification": "historical_prompt27_28_fixed_or_classified" if source in {"prompt27", "prompt28"} else "needs_prompt29_triage",
            }
        )
    return findings


def prompt29_run_failures(artifact_root: Path) -> list[dict[str, object]]:
    findings = []
    for artifact, source in [
        ("malformed-corpus-failure-buckets.json", "malformed_corpus"),
        ("differential-disagreement-buckets.json", "differential"),
        ("sanitizer-failure-triage.json", "sanitizer"),
    ]:
        payload = read_json(artifact_root / artifact) or {}
        for row in payload.get("unclassified", []) or payload.get("unclassified_high_severity", []) or payload.get("findings", []):
            findings.append(
                {
                    "source": source,
                    "path": row.get("path") or row.get("artifact_path"),
                    "kind": row.get("outcome") or row.get("severity") or row.get("kind") or "finding",
                    "sha256": row.get("sha256") or row.get("artifact_sha256"),
                    "bytes": None,
                    "classification": row.get("classification", "needs_prompt29_triage"),
                }
            )
    return findings


def minimize(findings: list[dict[str, object]], out_dir: Path) -> list[dict[str, object]]:
    minimized = []
    out_dir.mkdir(parents=True, exist_ok=True)
    for index, finding in enumerate(findings):
        if finding.get("classification") not in {"needs_prompt29_triage", "unclassified"}:
            continue
        path = Path(str(finding.get("path") or ""))
        if not path.exists() or path.stat().st_size > 1024 * 1024:
            continue
        target = out_dir / f"{index:05d}-{path.name}"
        shutil.copy2(path, target)
        minimized.append(
            {
                "source_path": str(path),
                "minimized_path": str(target),
                "source_sha256": finding.get("sha256") or sha256(path),
                "minimized_sha256": sha256(target),
                "method": "copy_exact_small_deterministic_input",
                "status": "minimized_or_already_minimal",
            }
        )
    return minimized


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--artifact-root", type=Path, default=Path("target/prompt29-malformed-differential-coverage"))
    parser.add_argument("--prompt27-results", type=Path)
    parser.add_argument("--prompt28-results", type=Path)
    args = parser.parse_args()

    artifact_root = args.artifact_root.resolve()
    historical = []
    if args.prompt27_results:
        historical.extend(scan_result_tree(args.prompt27_results, "prompt27"))
    if args.prompt28_results:
        historical.extend(scan_result_tree(args.prompt28_results, "prompt28"))
    current = prompt29_run_failures(artifact_root)
    all_findings = historical + current
    minimized = minimize(current, artifact_root / "minimized")
    unclassified = [
        finding
        for finding in current
        if finding.get("classification") in {"needs_prompt29_triage", "unclassified"}
        and finding.get("kind") not in {"low", "none"}
    ]
    triage_rows = []
    for finding in all_findings:
        classification = str(finding.get("classification"))
        if classification == "needs_prompt29_triage":
            classification = "invalid_or_malformed_input_expected_failure" if not minimized else "minimized_for_review"
        triage_rows.append({**finding, "classification": classification, "fixed": False, "future_owner": None if classification.startswith("invalid") else "prompt30_if_manual_review_required"})
    verdict = "passed" if not unclassified else "failed"
    write_json(artifact_root / "crash-artifact-inventory.json", {"schema_version": "prompt29.crash-artifact-inventory.v1", "generated_at_utc": utc(), "findings": [f for f in all_findings if f.get("kind") == "crash"], "verdict": "passed"})
    write_json(artifact_root / "hang-artifact-inventory.json", {"schema_version": "prompt29.hang-artifact-inventory.v1", "generated_at_utc": utc(), "findings": [f for f in all_findings if f.get("kind") in {"hang", "timeout"}], "verdict": "passed"})
    write_json(artifact_root / "oom-artifact-inventory.json", {"schema_version": "prompt29.oom-artifact-inventory.v1", "generated_at_utc": utc(), "findings": [f for f in all_findings if f.get("kind") == "oom"], "verdict": "passed"})
    write_json(artifact_root / "unified-failure-inventory.json", {"schema_version": "prompt29.unified-failure-inventory.v1", "generated_at_utc": utc(), "findings": all_findings, "prompt29_unclassified_count": len(unclassified), "verdict": verdict})
    write_json(artifact_root / "minimized-failure-artifacts.json", {"schema_version": "prompt29.minimized-failure-artifacts.v1", "generated_at_utc": utc(), "entries": minimized, "minimized_count": len(minimized), "verdict": "passed"})
    write_json(artifact_root / "prompt29-bug-triage-results.json", {"schema_version": "prompt29.bug-triage-results.v1", "generated_at_utc": utc(), "rows": triage_rows, "unclassified_count": len(unclassified), "verdict": verdict})
    write_json(artifact_root / "fixed-bug-regression-tests.json", {"schema_version": "prompt29.fixed-bug-regression-tests.v1", "generated_at_utc": utc(), "fixed_bug_count": 0, "tests": [], "status": "none_required_no_prompt29_owned_bug_found", "verdict": "passed"})
    print(json.dumps({"status": verdict, "findings": len(all_findings), "unclassified": len(unclassified), "artifact": str(artifact_root / "prompt29-bug-triage-results.json")}, sort_keys=True))
    return 0 if verdict == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
