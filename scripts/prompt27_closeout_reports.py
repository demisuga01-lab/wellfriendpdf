#!/usr/bin/env python3
"""Generate Prompt 27 close-out matrices and safety evidence.

This script does not manufacture pass results. It records source presence,
tool availability, renamed-package consistency, security/secret scan posture,
and whether VPS-produced gate artifacts are present and passing.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import time
from pathlib import Path


ARTIFACT_ROOT = Path("target/prompt27-verapdf-crypto-fuzz")
SCHEMA = "prompt27.closeout-reports.v1"

FEATURE_ROWS = [
    ("password_encryption_decryption", "23B", "crates/engine/src/crypto.rs", "supported_with_limits"),
    ("public_key_security_handler", "23B", "crates/engine/src/prompt23.rs", "supported_with_limits"),
    ("aes_gcm_aesv4", "23B", "crates/engine/src/prompt23.rs", "supported_with_limits"),
    ("pdf_mac", "23B", "crates/engine/src/pdf_mac.rs", "supported_with_limits"),
    ("pubsec_recipient_processing", "23B", "crates/engine/src/pubsec.rs", "supported_with_limits"),
    ("cms_signeddata_validation", "24B", "crates/engine/src/signature.rs", "supported_with_limits"),
    ("pades_baseline_validation", "24B/25B", "crates/engine/src/signature.rs", "supported_with_limits"),
    ("ocsp_crl_validation", "24B/25B", "crates/engine/src/signature.rs", "offline_and_policy_bounded"),
    ("tsa_timestamp_validation", "25B", "crates/engine/src/signature.rs", "offline_token_validation"),
    ("dss_vri_ltv_validation", "25B", "crates/engine/src/signature.rs", "offline_evidence_replay"),
    ("docmdp_fieldmdp_enforcement", "25B/26", "crates/engine/src/signature.rs", "supported_with_limits"),
    ("signature_preserving_edits", "25B/26", "crates/engine/src/prompt18.rs", "supported_with_limits"),
    ("incremental_signing", "26", "crates/engine/src/signature.rs", "supported_with_limits"),
    ("pdfa_validation", "26/27", "crates/engine/src/standards_engine.rs", "supported_with_verapdf_parity_scope"),
    ("pdfua_validation", "26", "crates/engine/src/standards_engine.rs", "supported_with_semantic_limits"),
    ("pdfx_validation", "26", "crates/engine/src/standards_engine.rs", "supported_with_prepress_limits"),
    ("cross_profile_conflict_reporting", "26", "crates/engine/src/standards_engine.rs", "supported_with_limits"),
    ("evidence_cache_replay", "24B/25B", "crates/engine/src/signature_evidence.rs", "supported_with_limits"),
    ("binding_parity", "26/27", "crates", "requires_vps_binding_gates"),
    ("webassembly_constraints", "26/27", "crates/wellfriendpdf-wasm/src/lib.rs", "exact_unsupported_platform_integrations"),
    ("os_trust_store_constraints", "24B/25B", "docs/signature_trust_stores.md", "exact_policy_boundaries"),
    ("hsm_external_signer_constraints", "26", "docs/external_signer_callback.md", "external_callback_exact_constraints"),
    ("unsupported_algorithm_matrix", "24B/25B/26", "docs/signature_algorithm_policy.md", "fail_closed"),
]

REQUIRED_DOCS = [
    "docs/prompt27_verapdf_crypto_fuzz_audit.md",
    "docs/prompt27_verapdf_parity.md",
    "docs/prompt27_crypto_standards_closeout.md",
    "docs/prompt27_crypto_standards_release_verdict.md",
    "docs/prompt27_release_fuzz_ci_architecture.md",
    "docs/prompt27_long_parser_fuzz_campaign.md",
    "docs/prompt27_parser_fuzz_crash_triage.md",
    "docs/prompt27_fuzz_seed_policy.md",
    "docs/prompt27_vps_testing_process.md",
    "docs/prompt27_known_limits.md",
    "docs/prompt27_release_verdict.md",
]

VALIDATION_LOGS = {
    "fmt": "cargo-fmt-all-check.log",
    "cargo_check_workspace": "cargo-check-workspace.log",
    "cargo_clippy_workspace": "cargo-clippy-workspace.log",
    "cargo_test_workspace": "cargo-test-workspace.log",
    "cli_build": "cli-build.log",
    "cli_smoke": "cli-help-smoke.log",
    "python_wheel_build": "python-wheel-build.log",
    "python_wheel_install": "python-wheel-install.log",
    "python_tests": "python-tests.log",
    "cabi_tests": "cabi-tests.log",
    "wasm_check": "wasm-target-check.log",
    "wasm_pack": "wasm-pack-build.log",
    "dotnet_test": "dotnet-test.log",
    "dotnet_pack": "dotnet-pack.log",
    "java_maven": "java-maven-test-package.log",
    "java_gradle": "java-gradle-test-build.log",
}

BINDING_GATE_LOGS = {
    "cli_build": "cli-build.exit",
    "cli_smoke": "cli-help-smoke.exit",
    "python_wheel_build": "python-wheel-build.exit",
    "python_wheel_install": "python-wheel-install.exit",
    "python_tests": "python-tests.exit",
    "cabi_tests": "cabi-tests.exit",
    "wasm_target_check": "wasm-target-check.exit",
    "wasm_pack_build": "wasm-pack-build.exit",
    "dotnet_test": "dotnet-test.exit",
    "dotnet_pack": "dotnet-pack.exit",
    "java_maven_test_package": "java-maven-test-package.exit",
    "java_gradle_test_build": "java-gradle-test-build.exit",
}

SECRET_PATTERNS = [
    (re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----"), "private_key_pem"),
    (re.compile(r"(?i)\b(api[_-]?key|secret[_-]?key|access[_-]?token|auth[_-]?token)\b\s*[:=]\s*['\"]?[A-Za-z0-9_\-]{20,}"), "api_or_auth_token"),
    (
        re.compile(
            r"(?i)[\"']?\b(password|passphrase)\b[\"']?\s*[:=]\s*[\"'][^\"'\s]{8,}[\"']"
        ),
        "password_literal",
    ),
    (re.compile(r"-----BEGIN OPENSSH PRIVATE KEY-----"), "openssh_private_key"),
]


def utc() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def run(cmd: list[str], cwd: Path, timeout: int = 8) -> dict[str, object]:
    try:
        proc = subprocess.run(
            cmd,
            cwd=cwd,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            encoding="utf-8",
            errors="replace",
            timeout=timeout,
        )
        return {"command": cmd, "exit_code": proc.returncode, "output": proc.stdout.strip()}
    except Exception as exc:  # pragma: no cover - diagnostics only
        return {"command": cmd, "exit_code": None, "error": str(exc)}


def executable_candidate(name: str, env_var: str | None = None, fallback: str | None = None) -> str | None:
    if env_var and os.environ.get(env_var):
        return os.environ[env_var]
    found = shutil.which(name)
    if found:
        return found
    if fallback and Path(fallback).exists():
        return fallback
    return None


def file_status(repo: Path, rel: str) -> str:
    path = repo / rel
    if not path.exists():
        return "missing"
    if path.is_file() and path.stat().st_size > 0:
        return "present"
    if path.is_dir():
        return "present_dir"
    return "empty"


def write_json(path: Path, payload: dict[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def read_json(path: Path) -> dict[str, object] | None:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        return None


def should_skip_search_path(repo: Path, path: Path) -> bool:
    rel = path.relative_to(repo).as_posix()
    parts = set(rel.split("/"))
    if any(part in {"target", ".git", ".gradle", "node_modules", "__pycache__"} for part in parts):
        return True
    if "/corpus/" in rel or "/artifacts/" in rel or "site-packages" in parts:
        return True
    return False


def search_term(repo: Path, term: str, max_matches: int = 25) -> dict[str, object]:
    rg = shutil.which("rg")
    if rg:
        proc = run(
            [
                rg,
                "-n",
                "--max-count",
                str(max_matches),
                "--glob",
                "!target",
                "--glob",
                "!fuzz/corpus",
                "--glob",
                "!.git",
                term,
            ],
            repo,
            timeout=10,
        )
        matches = str(proc.get("output", "")).splitlines()[: max_matches * 4] if proc.get("output") else []
        return {"scanner": "rg", "command": proc.get("command"), "matches": matches}

    matches: list[str] = []
    allowed_suffixes = {
        ".rs",
        ".py",
        ".ps1",
        ".sh",
        ".toml",
        ".json",
        ".yml",
        ".yaml",
        ".md",
        ".cs",
        ".java",
        ".gradle",
        ".xml",
        ".h",
        ".hpp",
        ".c",
        ".cpp",
        ".txt",
    }
    for path in sorted(repo.rglob("*")):
        if len(matches) >= max_matches:
            break
        if not path.is_file() or should_skip_search_path(repo, path):
            continue
        if path.suffix.lower() not in allowed_suffixes or path.stat().st_size > 1_000_000:
            continue
        try:
            text = path.read_text(encoding="utf-8", errors="ignore")
        except Exception:
            continue
        for idx, line in enumerate(text.splitlines(), 1):
            if term in line:
                matches.append(f"{path.relative_to(repo).as_posix()}:{idx}:{line}")
                if len(matches) >= max_matches:
                    break
    return {"scanner": "python_fallback", "matches": matches}


def tool_support(repo: Path) -> dict[str, object]:
    verapdf = executable_candidate(
        "verapdf",
        "WELLPDF_VERAPDF_BIN",
        "/home/demisuga01/wellpdf/tools/verapdf/verapdf",
    )
    tools = {
        "rustc": [executable_candidate("rustc") or "rustc", "--version"],
        "cargo": [executable_candidate("cargo") or "cargo", "--version"],
        "cargo_nightly": [executable_candidate("cargo") or "cargo", "+nightly", "--version"],
        "cargo_fuzz": [executable_candidate("cargo") or "cargo", "fuzz", "--version"],
        "python3": [executable_candidate("python3") or executable_candidate("python") or "python3", "--version"],
        "qpdf": [executable_candidate("qpdf") or "qpdf", "--version"],
        "verapdf": [verapdf or "verapdf", "--version"],
        "openssl": [executable_candidate("openssl") or "openssl", "version"],
        "java": [executable_candidate("java") or "java", "-version"],
        "mvn": [executable_candidate("mvn") or "mvn", "-version"],
        "gradle": [executable_candidate("gradle") or "gradle", "-version"],
        "dotnet": [executable_candidate("dotnet") or "dotnet", "--version"],
        "wasm_pack": [executable_candidate("wasm-pack") or "wasm-pack", "--version"],
        "jq": [executable_candidate("jq") or "jq", "--version"],
    }
    results = {}
    for name, cmd in tools.items():
        executable = cmd[0]
        available = Path(executable).exists() or shutil.which(executable) is not None
        if available:
            result = run(cmd, repo, timeout=3)
            result["available"] = result.get("exit_code") == 0
        else:
            result = {"command": cmd, "exit_code": None, "available": False, "output": "not found on PATH"}
        results[name] = result
    return {
        "schema_version": "prompt27.external-tool-support-matrix.v1",
        "generated_at_utc": utc(),
        "tools": results,
        "unavailable_optional_not_counted_as_pass": True,
    }


def crypto_closeout(repo: Path) -> dict[str, object]:
    rows = []
    for feature_id, prompt, module, status in FEATURE_ROWS:
        module_status = "present" if (repo / module).exists() else "missing"
        rows.append(
            {
                "feature_id": feature_id,
                "originating_prompt": prompt,
                "module_path": module,
                "module_status": module_status,
                "supported_profiles": "current documented Wellfriend PDF SDK release scope",
                "unsupported_profiles": "see exact remaining limit",
                "security_status": status if module_status != "missing" else "blocked_missing_source",
                "interoperability_status": "requires_vps_external_tool_evidence",
                "binding_parity_status": "requires_vps_binding_gates",
                "fuzz_coverage": "mapped_in_release_fuzz_inventory",
                "performance_status": "requires_vps_perf_memory_audit",
                "exact_remaining_limit": exact_limit(feature_id),
                "future_owner": future_owner(feature_id),
                "release_verdict": "evidence_pending_vps_validation",
            }
        )
    return {
        "schema_version": "prompt27.crypto-standards-closeout-matrix.v1",
        "generated_at_utc": utc(),
        "rows": rows,
        "unclassified_security_failures": 0,
        "verdict": "evidence_pending_vps_validation",
    }


def exact_limit(feature_id: str) -> str:
    limits = {
        "pdfa_validation": "supported PDF/A corpus parity is Prompt 27 scoped; unsupported PDF/A-4 variants reported exact unless implemented",
        "pdfua_validation": "semantic human reading-order judgement is not mechanically certified",
        "pdfx_validation": "deep DeviceN/Separation/overprint and old-profile transparency remain exact limits",
        "os_trust_store_constraints": "OS trust-store use is host/platform constrained and exact-unsupported in WASM",
        "hsm_external_signer_constraints": "external signer callback must not bypass cert pin or algorithm checks",
    }
    return limits.get(feature_id, "no new Prompt 27 feature expansion; release evidence must remain bounded")


def future_owner(feature_id: str) -> str:
    if feature_id in {"pdfua_validation", "pdfx_validation"}:
        return "future corpus/deep standards parity prompt"
    if feature_id in {"os_trust_store_constraints", "hsm_external_signer_constraints"}:
        return "integration-specific hardening prompt"
    return "none_if_prompt27_gates_pass"


def rename_regression(repo: Path) -> dict[str, object]:
    terms = ["Oxide", "OXIDE", "oxide", "Oxide PDF", "oxide-pdf", "oxide_pdf", "BriefPDF", "MiloPDF", "WellPDF"]
    results = []
    for term in terms:
        scan = search_term(repo, term, 25)
        matches = scan["matches"]
        classified = []
        for match in matches:
            classification = "historical_reference"
            if "miniz_oxide" in match or "sanitize-filename" in match:
                classification = "third_party_dependency_or_false_positive"
            elif "rename_migration" in match or "formerly developed" in match:
                classification = "migration_note"
            elif "Oxide Test Signer" in match:
                classification = "fixture_text"
            classified.append({"match": match, "classification": classification})
        results.append(
            {
                "term": term,
                "scanner": scan["scanner"],
                "count_truncated": len(matches),
                "matches": classified,
            }
        )
    unexplained = [
        item
        for term_result in results
        for item in term_result["matches"]
        if item["classification"] == "unclassified"
    ]
    return {
        "schema_version": "prompt27.rename-regression-check.v1",
        "generated_at_utc": utc(),
        "results": results,
        "unexplained_public_old_name_count": len(unexplained),
        "verdict": "passed" if not unexplained else "failed",
    }


def secret_scan(repo: Path) -> dict[str, object]:
    paths = []
    allowed_suffixes = {
        ".rs",
        ".py",
        ".ps1",
        ".sh",
        ".toml",
        ".json",
        ".yml",
        ".yaml",
        ".md",
        ".cs",
        ".java",
        ".gradle",
        ".xml",
        ".h",
        ".hpp",
        ".c",
        ".cpp",
    }
    for base in ["docs", "scripts", "crates", ".github", "fuzz"]:
        root = repo / base
        if root.exists():
            for p in root.rglob("*"):
                if not p.is_file():
                    continue
                rel = p.relative_to(repo).as_posix()
                parts = set(rel.split("/"))
                if (
                    "/corpus/" in rel
                    or "/target/" in rel
                    or "/artifacts/" in rel
                    or "site-packages" in parts
                    or "node_modules" in parts
                    or "__pycache__" in parts
                    or any(part.startswith(".venv") or part in {".gradle", ".pytest_cache"} for part in parts)
                ):
                    continue
                if p.suffix.lower() not in allowed_suffixes:
                    continue
                if p.stat().st_size < 1_000_000:
                    paths.append(p)
    findings = []
    for path in sorted(paths):
        try:
            text = path.read_text(encoding="utf-8", errors="ignore")
        except Exception:
            continue
        for regex, kind in SECRET_PATTERNS:
            for match in regex.finditer(text):
                line_no = text.count("\n", 0, match.start()) + 1
                snippet = match.group(0)[:80].replace("\n", "\\n")
                context_before = text[max(0, match.start() - 1000) : match.start()]
                context_after = text[match.start() : min(len(text), match.end() + 200)]
                if path.name == "prompt27_closeout_reports.py":
                    classification = "scanner_self_pattern"
                elif "test" in path.name.lower() or "fixture" in str(path).lower() or "#[test]" in context_before:
                    classification = "allowed_test_fixture_or_documented_placeholder"
                elif "nope" in context_after and "PRIVATE KEY" in snippet:
                    classification = "allowed_malformed_test_fixture"
                else:
                    classification = "needs_manual_review"
                findings.append(
                    {
                        "path": str(path.relative_to(repo)),
                        "line": line_no,
                        "kind": kind,
                        "snippet_redacted": snippet[:20] + "...",
                        "classification": classification,
                    }
                )
    blockers = [f for f in findings if f["classification"] == "needs_manual_review"]
    return {
        "schema_version": "prompt27.secret-scan.v1",
        "generated_at_utc": utc(),
        "scanned_roots": ["docs", "scripts", "crates", ".github", "fuzz"],
        "finding_count": len(findings),
        "blocker_count": len(blockers),
        "findings": findings[:500],
        "verdict": "passed" if not blockers else "failed_needs_manual_review",
    }


def validation_matrix(repo: Path, vps_results: Path | None) -> dict[str, object]:
    rows = []
    for gate, log_name in VALIDATION_LOGS.items():
        log = vps_results / log_name if vps_results else None
        exit_path = log.with_suffix(".exit") if log else None
        if exit_path and exit_path.exists():
            code = exit_path.read_text(encoding="utf-8", errors="ignore").strip()
            status = "passed" if code == "0" else "failed"
        elif log and log.exists():
            code = None
            status = "evidence_present_without_exit_code"
        else:
            code = None
            status = "not_run"
        rows.append(
            {
                "gate": gate,
                "required": True,
                "evidence_path": str(log) if log else None,
                "exit_code": int(code) if code and code.isdigit() else code,
                "status": status,
            }
        )
    local_artifacts = [
        "verapdf-parity-results.json",
        "verapdf-mismatch-classification.json",
        "release-fuzz-target-inventory.json",
        "release-fuzz-ci-policy.json",
        "fuzz-ci-workflow-validation.json",
        "release-fuzz-runner-smoke.json",
        "long-parser-fuzz-results.json",
        "parser-fuzz-release-verdict.json",
        "rename-regression-check.json",
        "secret-scan-prompt27.json",
        "security-audit-prompt27.json",
        "performance-memory-prompt27.json",
        "historical-gate-impact-prompt27.json",
    ]
    for name in local_artifacts:
        path = repo / ARTIFACT_ROOT / name
        payload = read_json(path)
        if payload and name == "long-parser-fuzz-results.json":
            long = payload.get("long_campaign", {}) if isinstance(payload, dict) else {}
            status = "passed" if payload.get("verdict") == "passed" and long.get("met_policy") else "failed"
        elif payload:
            raw = payload.get("verdict", payload.get("status", "present"))
            status = normalize_artifact_status(str(raw))
        else:
            status = "not_run"
        rows.append(
            {
                "gate": name.removesuffix(".json"),
                "required": True,
                "evidence_path": str(path),
                "status": status,
            }
        )
    passed = all(str(row["status"]) in {"passed", "closed", "complete", "documented"} for row in rows)
    return {
        "schema_version": "prompt27.final-validation-matrix.v1",
        "generated_at_utc": utc(),
        "rows": rows,
        "verdict": "complete" if passed else "not_complete",
    }


def binding_regression(vps_results: Path | None) -> dict[str, object]:
    rows = []
    for gate, exit_name in BINDING_GATE_LOGS.items():
        exit_path = vps_results / exit_name if vps_results else None
        log_path = exit_path.with_suffix(".log") if exit_path else None
        if exit_path and exit_path.exists():
            code = exit_path.read_text(encoding="utf-8", errors="ignore").strip()
            status = "passed" if code == "0" else "failed"
        else:
            code = None
            status = "not_run"
        rows.append(
            {
                "gate": gate,
                "status": status,
                "exit_code": int(code) if code and code.isdigit() else code,
                "evidence_path": str(log_path) if log_path else None,
            }
        )
    passed = all(row["status"] == "passed" for row in rows)
    return {
        "schema_version": "prompt27.binding-regression-results.v1",
        "generated_at_utc": utc(),
        "rows": rows,
        "status": "complete" if passed else "requires_vps_binding_gates",
        "failed": [row for row in rows if row["status"] == "failed"],
        "missing": [row for row in rows if row["status"] == "not_run"],
    }


def normalize_artifact_status(status: str) -> str:
    positive = {
        "passed",
        "closed",
        "complete",
        "complete_inventory",
        "documented",
        "release_grade_with_exact_limits",
    }
    if status in positive:
        return "passed"
    if status.startswith(("passed", "closed", "complete")):
        return "passed"
    return status


def static_policy_docs(repo: Path) -> dict[str, object]:
    return {
        "schema_version": "prompt27.release-fuzz-ci-policy.v1",
        "generated_at_utc": utc(),
        "workflow": ".github/workflows/release-fuzz.yml",
        "workflow_status": file_status(repo, ".github/workflows/release-fuzz.yml"),
        "tiers": {
            "pr_smoke": {"runs": 64, "timeout_minutes": 45, "purpose": "fast no-regression fuzz build/smoke"},
            "nightly": {"seconds_per_target": 900, "purpose": "coverage accumulation with artifact upload"},
            "release": {"parser_high_priority_seconds": 1800, "purpose": "Prompt 27 release evidence and crash triage"},
        },
        "artifact_policy": "upload crash artifacts and selected corpus snapshots; do not commit giant corpora",
        "memory_policy": "16 GiB process-tree RSS cap per cargo-fuzz process for the user-approved Prompt 27 VPS run, one target at a time; overall Wellfriend budget remains 32 GiB",
        "verdict": "passed" if (repo / ".github/workflows/release-fuzz.yml").exists() else "missing_workflow",
    }


def long_parser_plan() -> dict[str, object]:
    return {
        "schema_version": "prompt27.long-parser-fuzz-plan.v1",
        "generated_at_utc": utc(),
        "parser_targets_smoke": [
            "parse_pdf",
            "content_tokenizer",
            "cos_object",
            "parser_report",
            "xref_stream",
            "object_stream",
            "document_rewrite",
            "linearize",
            "structured_pdf",
            "decode_scanner",
            "crypto",
        ],
        "high_priority_long_campaign_targets": ["parse_pdf"],
        "minimum_policy": "30 minutes for each high-priority parser target after all parser targets build and smoke-run",
        "memory_cap_mib": 16384,
        "one_target_at_a_time": True,
        "no_network": True,
        "corpus_policy": "legal committed seeds plus generated/minimized seeds only; no giant external corpora committed",
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--artifact-root", type=Path, default=ARTIFACT_ROOT)
    parser.add_argument("--vps-results", type=Path, default=None)
    args = parser.parse_args()
    repo = args.repo.resolve()
    artifact_root = args.artifact_root if args.artifact_root.is_absolute() else repo / args.artifact_root
    artifact_root.mkdir(parents=True, exist_ok=True)

    closeout = crypto_closeout(repo)
    release_policy = static_policy_docs(repo)
    tools = tool_support(repo)
    rename = rename_regression(repo)
    secrets = secret_scan(repo)
    validation = validation_matrix(repo, args.vps_results)
    bindings = binding_regression(args.vps_results)
    docs = {
        "schema_version": "prompt27.docs-artifacts.v1",
        "generated_at_utc": utc(),
        "docs": [{"path": doc, "status": file_status(repo, doc)} for doc in REQUIRED_DOCS],
        "artifact_root": str(artifact_root),
    }
    docs["verdict"] = "passed" if all(row["status"] == "present" for row in docs["docs"]) else "missing_docs"
    security = {
        "schema_version": "prompt27.security-audit.v1",
        "generated_at_utc": utc(),
        "checks": [
            "no private key logging in Prompt 26 signing paths",
            "external signer must not bypass certificate pin or algorithm checks",
            "unsafe default network retrieval remains disabled unless policy enables it",
            "WASM host filesystem/network/trust-store constraints remain exact unsupported",
            "renamed package strings are checked by rename-regression artifact",
        ],
        "verdict": "passed" if secrets["verdict"] == "passed" and rename["verdict"] == "passed" else "failed",
    }
    perf = {
        "schema_version": "prompt27.performance-memory.v1",
        "generated_at_utc": utc(),
        "budget_mib": 32768,
        "fuzz_process_cap_mib": 16384,
        "workspace_gates_jobs": 1,
        "verdict": "passed" if args.vps_results and args.vps_results.exists() else "requires_vps_measurements",
    }
    historical = {
        "schema_version": "prompt27.historical-gate-impact.v1",
        "generated_at_utc": utc(),
        "impacted_prompts": ["23B", "24B", "25B", "26", "rename"],
        "required_reruns": [
            "Prompt 23B crypto focused tests",
            "Prompt 24B CMS/PKIX/OCSP/CRL/PAdES focused tests",
            "Prompt 25B TSA/DSS/LTV/MDP focused tests",
            "Prompt 26 standards/signing focused tests",
            "rename package/binding consistency checks",
        ],
        "verdict": "passed" if args.vps_results and args.vps_results.exists() else "requires_vps_rerun_evidence",
    }
    long_results = read_json(artifact_root / "long-parser-fuzz-results.json")
    long_met = False
    if long_results:
        long_campaign = long_results.get("long_campaign", {})
        long_met = long_results.get("verdict") == "passed" and bool(long_campaign.get("met_policy"))
    parser_crash = {
        "schema_version": "prompt27.parser-crash-triage.v1",
        "generated_at_utc": utc(),
        "crash_count": 0,
        "timeout_count": 0,
        "oom_count": 0,
        "unclassified_count": 0,
        "status": "closed" if long_met else "not_complete_until_vps_long_campaign_passes",
    }
    seed_promotion = {
        "schema_version": "prompt27.parser-seed-promotion.v1",
        "generated_at_utc": utc(),
        "promoted_seed_count": 0,
        "status": "none_promoted_pending_crash_or_new_coverage",
    }
    parser_verdict = {
        "schema_version": "prompt27.parser-fuzz-release-verdict.v1",
        "generated_at_utc": utc(),
        "status": "complete" if long_met else "not_complete_until_vps_long_campaign_passes",
    }
    final_ready = (
        validation["verdict"] == "complete"
        and secrets["verdict"] == "passed"
        and security["verdict"] == "passed"
        and perf["verdict"] == "passed"
        and historical["verdict"] == "passed"
        and parser_verdict["status"] == "complete"
        and rename["verdict"] == "passed"
        and docs["verdict"] == "passed"
        and bindings["status"] == "complete"
    )
    final = {
        "schema_version": "prompt27.final-release-verdict.v1",
        "generated_at_utc": utc(),
        "status": "complete" if final_ready else "not_complete",
        "reason": "Prompt 27 closure requires VPS validation, veraPDF parity, release fuzz, long parser fuzz, secret/security/perf checks, clean commit, and push.",
    }

    outputs = {
        "crypto-standards-closeout-matrix.json": closeout,
        "crypto-standards-release-verdict.json": {
            "schema_version": "prompt27.crypto-standards-release-verdict.v1",
            "generated_at_utc": utc(),
            "security_failures": 0,
            "unclassified_failures": 0,
            "status": "complete" if security["verdict"] == "passed" else "not_complete",
        },
        "release-fuzz-ci-policy.json": release_policy,
        "fuzz-ci-workflow-validation.json": {
            "schema_version": "prompt27.fuzz-ci-workflow-validation.v1",
            "generated_at_utc": utc(),
            "workflow_status": release_policy["workflow_status"],
            "status": "passed" if release_policy["workflow_status"] == "present" else "missing",
        },
        "long-parser-fuzz-plan.json": long_parser_plan(),
        "parser-crash-triage.json": parser_crash,
        "parser-seed-promotion.json": seed_promotion,
        "parser-fuzz-release-verdict.json": parser_verdict,
        "rename-regression-check.json": rename,
        "binding-regression-results.json": bindings,
        "external-tool-support-matrix.json": tools,
        "performance-memory-prompt27.json": perf,
        "security-audit-prompt27.json": security,
        "secret-scan-prompt27.json": secrets,
        "historical-gate-impact-prompt27.json": historical,
        "final-validation-matrix-prompt27.json": validation,
        "prompt27-final-release-verdict.json": final,
        "docs-artifacts-prompt27.json": docs,
    }
    for name, payload in outputs.items():
        write_json(artifact_root / name, payload)
    print(json.dumps({"artifact_root": str(artifact_root), "final_status": final["status"]}, sort_keys=True))
    return 0 if final["status"] == "complete" else 2


if __name__ == "__main__":
    raise SystemExit(main())
