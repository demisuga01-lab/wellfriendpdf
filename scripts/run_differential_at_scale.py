#!/usr/bin/env python3
"""Prompt 29 availability-aware differential runner.

The harness compares Wellfriend PDF SDK with independent command-line tools that
are actually available on the runner. Raw tool output is retained under the
artifact root; JSON artifacts contain normalized summaries and disagreement
classifications only.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import time
from collections import Counter
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


def load_manifest(path: Path) -> list[dict[str, object]]:
    data = json.loads(path.read_text(encoding="utf-8"))
    return list(data.get("files", []))


def command_available(name: str) -> bool:
    return shutil.which(name) is not None


def run_tool(cmd: list[str], timeout_seconds: float, raw_log: Path) -> dict[str, object]:
    start = time.perf_counter()
    raw_log.parent.mkdir(parents=True, exist_ok=True)
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout_seconds)
        status = "passed" if proc.returncode == 0 else "failed_cleanly"
        stdout = proc.stdout
        stderr = proc.stderr
        exit_code: int | None = proc.returncode
    except subprocess.TimeoutExpired as exc:
        status = "timeout"
        stdout = exc.stdout.decode("utf-8", "replace") if isinstance(exc.stdout, bytes) else (exc.stdout or "")
        stderr = exc.stderr.decode("utf-8", "replace") if isinstance(exc.stderr, bytes) else (exc.stderr or "")
        exit_code = None
    elapsed = time.perf_counter() - start
    raw_log.write_text("$ " + " ".join(cmd) + "\n\n--- stdout ---\n" + stdout + "\n--- stderr ---\n" + stderr, encoding="utf-8", errors="ignore")
    lowered = (stdout + stderr).lower()
    if "segmentation fault" in lowered or "panic" in lowered or "addresssanitizer" in lowered:
        status = "panic_crash"
    return {
        "status": status,
        "exit_code": exit_code,
        "time_seconds": round(elapsed, 6),
        "raw_log_path": str(raw_log),
        "raw_log_sha256": sha256(raw_log),
    }


def parse_pdfinfo_pages(log_path: str | None) -> int | None:
    if not log_path:
        return None
    try:
        text = Path(log_path).read_text(encoding="utf-8", errors="ignore")
    except OSError:
        return None
    for line in text.splitlines():
        if line.lower().startswith("pages:"):
            try:
                return int(line.split(":", 1)[1].strip())
            except ValueError:
                return None
    return None


def classify_disagreement(results: dict[str, dict[str, object]], page_counts: dict[str, int | None]) -> tuple[str, str]:
    hard = [tool for tool, result in results.items() if result["status"] in {"panic_crash", "timeout"}]
    if hard:
        return "high", "tool_crash_or_timeout"
    wellfriend = results.get("wellfriend_parser", {})
    references = [result for name, result in results.items() if name != "wellfriend_parser"]
    if wellfriend.get("status") == "failed_cleanly" and any(ref.get("status") == "passed" for ref in references):
        return "medium", "wellfriend_only_strict_failure_or_reference_repair"
    if wellfriend.get("status") == "passed" and references and all(ref.get("status") == "failed_cleanly" for ref in references):
        return "low", "wellfriend_only_repair_success"
    non_null_counts = {k: v for k, v in page_counts.items() if v is not None}
    if len(set(non_null_counts.values())) > 1:
        return "medium", "page_count_reference_disagreement"
    return "none", "no_actionable_disagreement"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", type=Path, default=Path.cwd())
    parser.add_argument("--artifact-root", type=Path, default=Path("target/prompt29-malformed-differential-coverage"))
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--wellfriendpdf-bin", type=Path, required=True)
    parser.add_argument("--limit", type=int, default=300)
    parser.add_argument("--timeout-seconds", type=float, default=20.0)
    args = parser.parse_args()

    artifact_root = args.artifact_root.resolve()
    raw_root = artifact_root / "raw" / "differential"
    manifest = load_manifest(args.manifest)[: args.limit]
    tool_support = {
        "schema_version": "prompt29.differential-tool-support.v1",
        "generated_at_utc": utc(),
        "tools": [
            {"tool": "qpdf", "available": command_available("qpdf"), "status": "available" if command_available("qpdf") else "unavailable_external_tool"},
            {"tool": "pdfinfo", "available": command_available("pdfinfo"), "status": "available" if command_available("pdfinfo") else "unavailable_external_tool"},
            {"tool": "pdftotext", "available": command_available("pdftotext"), "status": "available" if command_available("pdftotext") else "unavailable_external_tool"},
            {"tool": "mutool", "available": command_available("mutool"), "status": "available" if command_available("mutool") else "unavailable_external_tool"},
            {"tool": "verapdf", "available": command_available("verapdf"), "status": "available" if command_available("verapdf") else "unavailable_external_tool"},
            {"tool": "pyhanko", "available": command_available("pyhanko"), "status": "available" if command_available("pyhanko") else "unavailable_external_tool"},
        ],
    }
    rows = []
    disagreements = []
    for index, item in enumerate(manifest):
        path = Path(str(item["path"]))
        file_root = raw_root / f"{index:05d}-{path.stem[:48]}"
        results: dict[str, dict[str, object]] = {
            "wellfriend_parser": run_tool(
                [str(args.wellfriendpdf_bin), "parser-report", str(path), "--mode", "audit", "--json"],
                args.timeout_seconds,
                file_root / "wellfriend-parser.log",
            )
        }
        if command_available("qpdf"):
            results["qpdf_check"] = run_tool(["qpdf", "--check", str(path)], args.timeout_seconds, file_root / "qpdf-check.log")
        if command_available("pdfinfo"):
            results["pdfinfo"] = run_tool(["pdfinfo", str(path)], args.timeout_seconds, file_root / "pdfinfo.log")
        if command_available("pdftotext"):
            results["pdftotext"] = run_tool(["pdftotext", "-f", "1", "-l", "1", str(path), "-"], args.timeout_seconds, file_root / "pdftotext.log")
        if command_available("mutool"):
            results["mutool_info"] = run_tool(["mutool", "info", str(path)], args.timeout_seconds, file_root / "mutool-info.log")
        if command_available("verapdf"):
            results["verapdf"] = run_tool(["verapdf", "--format", "json", str(path)], args.timeout_seconds, file_root / "verapdf.log")
        if command_available("pyhanko"):
            results["pyhanko"] = run_tool(["pyhanko", "sign", "validate", str(path)], args.timeout_seconds, file_root / "pyhanko.log")
        page_counts = {"pdfinfo": parse_pdfinfo_pages(results.get("pdfinfo", {}).get("raw_log_path"))}
        severity, classification = classify_disagreement(results, page_counts)
        row = {
            "path": str(path),
            "sha256": item.get("sha256") or sha256(path),
            "bytes": item.get("bytes") or path.stat().st_size,
            "results": results,
            "page_counts": page_counts,
            "disagreement_severity": severity,
            "classification": classification,
            "needs_manual_review": severity == "high",
        }
        rows.append(row)
        if severity != "none":
            disagreements.append(
                {
                    "path": row["path"],
                    "sha256": row["sha256"],
                    "severity": severity,
                    "classification": classification,
                    "needs_manual_review": row["needs_manual_review"],
                }
            )
    buckets = Counter(row["classification"] for row in rows)
    manual = [row for row in disagreements if row["needs_manual_review"]]
    write_json(artifact_root / "differential-tool-support-matrix.json", {**tool_support, "verdict": "passed"})
    write_json(artifact_root / "differential-corpus-manifest.json", {"schema_version": "prompt29.differential-corpus-manifest.v1", "generated_at_utc": utc(), "files": manifest, "file_count": len(manifest), "verdict": "passed" if manifest else "failed"})
    write_json(artifact_root / "differential-run-results.json", {"schema_version": "prompt29.differential-run-results.v1", "generated_at_utc": utc(), "rows": rows, "verdict": "passed" if rows and not manual else "failed"})
    write_json(artifact_root / "differential-disagreement-buckets.json", {"schema_version": "prompt29.differential-disagreement-buckets.v1", "generated_at_utc": utc(), "buckets": dict(buckets), "unclassified_high_severity": manual, "verdict": "passed" if not manual else "failed"})
    write_json(artifact_root / "differential-scale-scorecard.json", {"schema_version": "prompt29.differential-scale-scorecard.v1", "generated_at_utc": utc(), "attempted_count": len(rows), "disagreement_count": len(disagreements), "high_severity_unclassified_count": len(manual), "verdict": "passed" if rows and not manual else "failed"})
    write_json(artifact_root / "differential-manual-review-queue.json", {"schema_version": "prompt29.differential-manual-review-queue.v1", "generated_at_utc": utc(), "queue": manual, "verdict": "passed" if not manual else "failed"})
    print(json.dumps({"status": "passed" if rows and not manual else "failed", "attempted": len(rows), "manual_review": len(manual), "artifact": str(artifact_root / "differential-scale-scorecard.json")}, sort_keys=True))
    return 0 if rows and not manual else 1


if __name__ == "__main__":
    raise SystemExit(main())
