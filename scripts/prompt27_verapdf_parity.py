#!/usr/bin/env python3
"""Prompt 27 veraPDF corpus parity runner.

The runner compares veraPDF PDF/A decisions with the Wellfriend PDF SDK
clause-mapped PDF/A validator on the same corpus files. It does not treat
unavailable tools as passes and it does not log PDF bytes.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import time
import xml.etree.ElementTree as ET
from pathlib import Path


SCHEMA_VERSION = "prompt27.verapdf-parity.v1"
ARTIFACT_ROOT = Path("target/prompt27-verapdf-crypto-fuzz")
SUPPORTED_WELLFRIEND_PROFILES = {"1b", "2a", "2b", "3a", "3b"}
UNSUPPORTED_EXACT_PROFILES = {"1a", "2u", "3u", "4", "4e", "4f"}
PROFILE_RE = re.compile(r"pdf[_\-/ ]?a[_\-/ ]?(?P<part>[1234])(?P<level>[abuef])?", re.I)


def utc() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def run_command(cmd: list[str], *, cwd: Path, timeout: int) -> dict[str, object]:
    started = utc()
    start = time.monotonic()
    try:
        proc = subprocess.run(
            cmd,
            cwd=cwd,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            encoding="utf-8",
            errors="replace",
            timeout=timeout,
        )
        timed_out = False
    except subprocess.TimeoutExpired as exc:
        return {
            "command": cmd,
            "cwd": str(cwd),
            "started_at_utc": started,
            "elapsed_seconds": round(time.monotonic() - start, 3),
            "timeout_seconds": timeout,
            "timed_out": True,
            "exit_code": None,
            "stdout": exc.stdout.decode("utf-8", "replace") if isinstance(exc.stdout, bytes) else (exc.stdout or ""),
            "stderr": exc.stderr.decode("utf-8", "replace") if isinstance(exc.stderr, bytes) else (exc.stderr or ""),
            "status": "timeout",
        }
    return {
        "command": cmd,
        "cwd": str(cwd),
        "started_at_utc": started,
        "elapsed_seconds": round(time.monotonic() - start, 3),
        "timeout_seconds": timeout,
        "timed_out": timed_out,
        "exit_code": proc.returncode,
        "stdout": proc.stdout,
        "stderr": proc.stderr,
        "status": "passed" if proc.returncode == 0 else "failed",
    }


def run_command_light(cmd: list[str], cwd: Path) -> dict[str, object]:
    try:
        proc = subprocess.run(
            cmd,
            cwd=cwd,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            encoding="utf-8",
            errors="replace",
            timeout=30,
        )
        return {"command": cmd, "exit_code": proc.returncode, "output": proc.stdout.strip()}
    except Exception as exc:  # pragma: no cover - diagnostics only
        return {"command": cmd, "exit_code": None, "error": str(exc)}


def infer_profile(path: Path) -> str | None:
    haystack = " ".join(path.parts[-8:] + (path.name,))
    match = PROFILE_RE.search(haystack.replace("PDFA", "PDF/A"))
    if not match:
        return None
    part = match.group("part")
    level = (match.group("level") or ("b" if part != "4" else "")).lower()
    if part == "4" and level in {"a", "b", "u"}:
        level = ""
    return f"{part}{level}"


def profile_to_verapdf_flavour(profile: str) -> str:
    return profile.lower()


def profile_to_wellfriend_target(profile: str) -> str:
    if len(profile) == 1:
        return f"PDF/A-{profile}"
    return f"PDF/A-{profile[0]}{profile[1].upper()}"


def discover_pdf_files(corpus: Path, profiles: set[str] | None, limit: int | None) -> list[Path]:
    files: list[Path] = []
    for path in sorted(corpus.rglob("*.pdf")):
        profile = infer_profile(path)
        if profiles and profile not in profiles:
            continue
        files.append(path)
        if limit and len(files) >= limit:
            break
    return files


def parse_verapdf_xml(stdout: str) -> dict[str, object]:
    try:
        root = ET.fromstring(stdout)
    except ET.ParseError as exc:
        return {"parsed": False, "error": str(exc), "is_compliant": None, "failed_checks": []}
    report = None
    for elem in root.iter():
        if elem.tag.endswith("validationReport"):
            report = elem
            break
    if report is None:
        return {"parsed": False, "error": "validationReport element not found", "is_compliant": None, "failed_checks": []}
    is_compliant = str(report.attrib.get("isCompliant", "")).lower() == "true"
    failed = []
    for elem in root.iter():
        if elem.tag.endswith("rule") or elem.tag.endswith("test"):
            status = (elem.attrib.get("status") or elem.attrib.get("passed") or "").lower()
            if status in {"failed", "false"}:
                failed.append(
                    {
                        "id": elem.attrib.get("id") or elem.attrib.get("ruleId"),
                        "clause": elem.attrib.get("clause"),
                        "status": status,
                    }
                )
    return {"parsed": True, "is_compliant": is_compliant, "failed_checks": failed[:200]}


def parse_verapdf_batch_xml(stdout: str) -> dict[str, dict[str, object]]:
    try:
        root = ET.fromstring(stdout)
    except ET.ParseError:
        return {}
    results: dict[str, dict[str, object]] = {}
    for job in root.iter():
        if not job.tag.endswith("job"):
            continue
        name = None
        report = None
        for child in job.iter():
            if child.tag.endswith("name") and child.text:
                name = child.text.strip()
            if child.tag.endswith("validationReport"):
                report = child
        if not name or report is None:
            continue
        failed = []
        for elem in report.iter():
            if elem.tag.endswith("rule") or elem.tag.endswith("test"):
                status = (elem.attrib.get("status") or elem.attrib.get("passed") or "").lower()
                if status in {"failed", "false"}:
                    failed.append(
                        {
                            "id": elem.attrib.get("id") or elem.attrib.get("ruleId"),
                            "clause": elem.attrib.get("clause"),
                            "status": status,
                        }
                    )
        results[str(Path(name).resolve())] = {
            "parsed": True,
            "is_compliant": str(report.attrib.get("isCompliant", "")).lower() == "true",
            "failed_checks": failed[:200],
        }
    return results


def parse_wellfriend_json(stdout: str) -> dict[str, object]:
    try:
        parsed = json.loads(stdout)
    except json.JSONDecodeError as exc:
        return {"parsed": False, "error": str(exc), "conformant": None, "unsupported_exact": 0, "fail_count": None}
    report = parsed.get("report", parsed) if isinstance(parsed, dict) else {}
    conformance = str(report.get("conformance", "")).lower()
    counts = report.get("counts", {}) if isinstance(report, dict) else {}
    fail_count = counts.get("fail")
    unsupported = counts.get("unsupported_reported_exact", 0)
    if "nonconformant" in conformance or "non_conformant" in conformance:
        conformant = False
    elif "conformant" in conformance:
        conformant = True
    elif isinstance(fail_count, int):
        conformant = fail_count == 0
    else:
        conformant = None
    rules = report.get("rules", []) if isinstance(report, dict) else []
    unsupported_rule_ids = []
    if isinstance(rules, list):
        for rule in rules:
            if isinstance(rule, dict) and "unsupported" in str(rule.get("status", "")).lower():
                unsupported_rule_ids.append(rule.get("rule_id") or rule.get("id"))
    return {
        "parsed": True,
        "conformant": conformant,
        "unsupported_exact": unsupported,
        "fail_count": fail_count,
        "unsupported_rule_ids": unsupported_rule_ids,
        "schema_version": report.get("schema_version") if isinstance(report, dict) else None,
        "profile": report.get("target_profile") or report.get("profile") if isinstance(report, dict) else None,
    }


def wellfriend_command(args: argparse.Namespace, pdf: Path, profile: str) -> list[str]:
    cli_args = [
        "pdfa-validate",
        str(pdf),
        "--target",
        profile_to_wellfriend_target(profile),
        "--json",
        "--fail-on",
        "never",
    ]
    if args.wellfriendpdf_bin:
        return [str(args.wellfriendpdf_bin)] + cli_args
    return ["cargo", "run", "--quiet", "-p", "wellfriendpdf-cli", "--"] + cli_args


def profile_roots(corpus: Path, files: list[Path]) -> dict[tuple[str, Path], list[Path]]:
    groups: dict[tuple[str, Path], list[Path]] = {}
    for pdf in files:
        profile = infer_profile(pdf)
        if not profile:
            continue
        rel_parts = pdf.relative_to(corpus).parts
        profile_root = corpus / rel_parts[0] if rel_parts else pdf.parent
        groups.setdefault((profile, profile_root), []).append(pdf)
    return groups


def run_verapdf_batches(args: argparse.Namespace, repo: Path, corpus: Path, files: list[Path]) -> dict[str, dict[str, object]]:
    if args.per_file_verapdf or args.limit:
        return {}
    mapped: dict[str, dict[str, object]] = {}
    for (profile, root), group_files in sorted(profile_roots(corpus, files).items(), key=lambda item: (item[0][0], str(item[0][1]))):
        cmd = [str(args.verapdf_bin), "-f", profile_to_verapdf_flavour(profile), "--recurse", str(root)]
        run_result = run_command(cmd, cwd=repo, timeout=max(args.timeout, 300))
        batch = parse_verapdf_batch_xml(str(run_result.get("stdout", "")))
        for pdf in group_files:
            key = str(pdf.resolve())
            parsed = batch.get(key)
            if parsed is None:
                parsed = {
                    "parsed": False,
                    "is_compliant": None,
                    "failed_checks": [],
                    "error": "file missing from veraPDF batch output",
                }
            parsed = {
                **parsed,
                "command": cmd,
                "exit_code": run_result.get("exit_code"),
                "status": run_result.get("status"),
                "elapsed_seconds": run_result.get("elapsed_seconds"),
                "stderr_tail": str(run_result.get("stderr", "")).splitlines()[-40:],
            }
            mapped[key] = parsed
    return mapped


def classify_case(profile: str | None, verapdf: dict[str, object], wellfriend: dict[str, object]) -> str:
    if not profile:
        return "not_applicable_profile_unknown"
    if profile in UNSUPPORTED_EXACT_PROFILES:
        if wellfriend.get("unsupported_exact", 0) or wellfriend.get("unsupported_rule_ids"):
            return "wellfriend_exact_unsupported"
        if not wellfriend.get("parsed") and wellfriend.get("exit_code") not in (None, 0):
            return "wellfriend_safe_parse_rejection_for_unsupported_profile"
        return "unsupported_profile_not_reported_exact"
    if not verapdf.get("parsed"):
        return "verapdf_parse_or_execution_failure"
    if not wellfriend.get("parsed"):
        if verapdf.get("is_compliant") is False and wellfriend.get("status") == "timeout":
            return "wellfriend_timeout_on_noncompliant_file"
        if verapdf.get("is_compliant") is False and wellfriend.get("exit_code") not in (None, 0):
            return "wellfriend_safe_parse_rejection"
        return "wellfriend_parse_or_execution_failure"
    v = verapdf.get("is_compliant")
    w = wellfriend.get("conformant")
    if v is True and w is True:
        return "aligned_pass"
    if v is False and w is False:
        return "aligned_fail"
    if v is False and w is True:
        return "wellfriend_false_conformant"
    if v is True and w is False:
        return "wellfriend_stricter_or_missing_rule"
    return "indeterminate"


def corpus_manifest(corpus: Path, files: list[Path]) -> dict[str, object]:
    git_head = run_command_light(["git", "rev-parse", "HEAD"], corpus)
    git_remote = run_command_light(["git", "remote", "-v"], corpus)
    license_files = [p for p in corpus.iterdir() if p.name.lower().startswith("license")]
    return {
        "schema_version": "prompt27.verapdf-corpus-manifest.v1",
        "generated_at_utc": utc(),
        "corpus_path": str(corpus),
        "source": "veraPDF/veraPDF-corpus public GitHub repository when cloned from upstream",
        "license": "CC-BY-4.0 per upstream repository documentation; confirm local checkout license file before redistribution",
        "git_head": git_head,
        "git_remote": git_remote,
        "license_files": [
            {"path": str(path), "sha256": sha256(path), "size_bytes": path.stat().st_size}
            for path in license_files
            if path.is_file()
        ],
        "selected_file_count": len(files),
        "selected_files": [
            {
                "path": str(path),
                "relative_path": str(path.relative_to(corpus)),
                "profile": infer_profile(path),
                "sha256": sha256(path),
                "size_bytes": path.stat().st_size,
            }
            for path in files
        ],
    }


def tool_manifest(verapdf_bin: Path | str) -> dict[str, object]:
    resolved = shutil.which(str(verapdf_bin)) or str(verapdf_bin)
    result = run_command_light([str(verapdf_bin), "--version"], Path.cwd()) if resolved else {"exit_code": None}
    return {
        "schema_version": "prompt27.verapdf-tool-manifest.v1",
        "generated_at_utc": utc(),
        "tool": "veraPDF",
        "path": resolved,
        "version_check": result,
        "available": result.get("exit_code") == 0,
        "source": "https://github.com/veraPDF/veraPDF-library",
        "license_posture": "external test tool, not vendored into repository",
    }


def write_markdown(results: dict[str, object], path: Path) -> None:
    lines = [
        "# Prompt 27 veraPDF parity",
        "",
        f"Generated: `{results['generated_at_utc']}`",
        f"Verdict: `{results['verdict']}`",
        "",
        "| profile | file | classification | veraPDF | Wellfriend |",
        "| --- | --- | --- | --- | --- |",
    ]
    for case in results["cases"]:
        lines.append(
            "| {profile} | {file} | {classification} | {v} | {w} |".format(
                profile=case.get("profile"),
                file=case.get("relative_path"),
                classification=case.get("classification"),
                v=case["verapdf"].get("is_compliant"),
                w=case["wellfriend"].get("conformant"),
            )
        )
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--verapdf-bin", default="verapdf")
    parser.add_argument("--wellfriendpdf-bin", type=Path, default=None)
    parser.add_argument("--profiles", default=None, help="comma-separated veraPDF flavours such as 1b,2b,3a,4,4f")
    parser.add_argument("--limit", type=int, default=None)
    parser.add_argument("--timeout", type=int, default=60)
    parser.add_argument("--artifact-root", type=Path, default=ARTIFACT_ROOT)
    parser.add_argument("--markdown-output", type=Path, default=Path("docs/prompt27_verapdf_parity.md"))
    parser.add_argument("--per-file-verapdf", action="store_true", help="disable profile-directory veraPDF batching")
    args = parser.parse_args()

    repo = args.repo.resolve()
    corpus = args.corpus.resolve()
    artifact_root = args.artifact_root if args.artifact_root.is_absolute() else repo / args.artifact_root
    artifact_root.mkdir(parents=True, exist_ok=True)
    profiles = {p.strip().lower() for p in args.profiles.split(",") if p.strip()} if args.profiles else None
    files = discover_pdf_files(corpus, profiles, args.limit)

    tool = tool_manifest(args.verapdf_bin)
    verapdf_batch = run_verapdf_batches(args, repo, corpus, files)
    cases = []
    for pdf in files:
        profile = infer_profile(pdf)
        if not profile:
            continue
        verapdf_cmd = [str(args.verapdf_bin), "-f", profile_to_verapdf_flavour(profile), str(pdf)]
        if str(pdf.resolve()) in verapdf_batch:
            verapdf_parsed = verapdf_batch[str(pdf.resolve())]
        else:
            verapdf_run = run_command(verapdf_cmd, cwd=repo, timeout=args.timeout)
            verapdf_parsed = {
                "command": verapdf_cmd,
                "exit_code": verapdf_run.get("exit_code"),
                "status": verapdf_run.get("status"),
                "elapsed_seconds": verapdf_run.get("elapsed_seconds"),
                **parse_verapdf_xml(str(verapdf_run.get("stdout", ""))),
                "stderr_tail": str(verapdf_run.get("stderr", "")).splitlines()[-40:],
            }
        wf_cmd = wellfriend_command(args, pdf, profile)
        wf_run = run_command(wf_cmd, cwd=repo, timeout=args.timeout)
        wf_parsed = parse_wellfriend_json(str(wf_run.get("stdout", "")))
        wf_case = {
            "command": wf_cmd,
            "exit_code": wf_run.get("exit_code"),
            "status": wf_run.get("status"),
            "elapsed_seconds": wf_run.get("elapsed_seconds"),
            **wf_parsed,
            "stderr_tail": str(wf_run.get("stderr", "")).splitlines()[-40:],
        }
        classification = classify_case(profile, verapdf_parsed, wf_case)
        cases.append(
            {
                "path": str(pdf),
                "relative_path": str(pdf.relative_to(corpus)),
                "sha256": sha256(pdf),
                "profile": profile,
                "supported_by_wellfriend": profile in SUPPORTED_WELLFRIEND_PROFILES,
                "verapdf": verapdf_parsed,
                "wellfriend": wf_case,
                "classification": classification,
            }
        )

    mismatch_cases = [
        case
        for case in cases
        if case["classification"]
        not in {
            "aligned_pass",
            "aligned_fail",
            "wellfriend_exact_unsupported",
            "wellfriend_safe_parse_rejection_for_unsupported_profile",
            "wellfriend_safe_parse_rejection",
            "not_applicable_profile_unknown",
        }
    ]
    unclassified_supported = [
        case
        for case in mismatch_cases
        if case.get("supported_by_wellfriend")
        and case["classification"]
        not in {"wellfriend_stricter_or_missing_rule", "external_validator_disagreement"}
    ]
    results = {
        "schema_version": SCHEMA_VERSION,
        "generated_at_utc": utc(),
        "repo": str(repo),
        "corpus": str(corpus),
        "selected_profiles": sorted(profiles) if profiles else "all_discovered",
        "case_count": len(cases),
        "supported_case_count": sum(1 for case in cases if case["supported_by_wellfriend"]),
        "classification_counts": classification_counts(cases),
        "cases": cases,
        "verdict": "passed" if not unclassified_supported else "failed",
        "supported_scope_unclassified_mismatch_count": len(unclassified_supported),
    }
    mismatch = {
        "schema_version": "prompt27.verapdf-mismatch-classification.v1",
        "generated_at_utc": utc(),
        "mismatch_count": len(mismatch_cases),
        "supported_scope_unclassified_mismatch_count": len(unclassified_supported),
        "mismatches": mismatch_cases,
        "verdict": "closed" if not unclassified_supported else "open_unclassified_supported_mismatches",
    }
    pdfa4 = {
        "schema_version": "prompt27.pdfa4-parity.v1",
        "generated_at_utc": utc(),
        "pdfa4_case_count": sum(1 for case in cases if str(case["profile"]).startswith("4")),
        "status": "unsupported_reported_exact",
        "cases": [case for case in cases if str(case["profile"]).startswith("4")],
    }

    write_json(artifact_root / "verapdf-tool-manifest.json", tool)
    write_json(artifact_root / "verapdf-corpus-manifest.json", corpus_manifest(corpus, files))
    write_json(artifact_root / "verapdf-parity-results.json", results)
    write_json(artifact_root / "verapdf-mismatch-classification.json", mismatch)
    write_json(artifact_root / "pdfa4-parity-results.json", pdfa4)
    md_path = args.markdown_output if args.markdown_output.is_absolute() else repo / args.markdown_output
    write_markdown(results, md_path)
    print(json.dumps({"result": str(artifact_root / "verapdf-parity-results.json"), "verdict": results["verdict"]}, sort_keys=True))
    return 0 if results["verdict"] == "passed" else 2


def classification_counts(cases: list[dict[str, object]]) -> dict[str, int]:
    counts: dict[str, int] = {}
    for case in cases:
        classification = str(case.get("classification"))
        counts[classification] = counts.get(classification, 0) + 1
    return dict(sorted(counts.items()))


def write_json(path: Path, payload: dict[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


if __name__ == "__main__":
    raise SystemExit(main())
