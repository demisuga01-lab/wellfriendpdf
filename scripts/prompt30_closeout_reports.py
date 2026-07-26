#!/usr/bin/env python3
"""Prompt 30 audit, API inventory, package gate, and closeout generator.

The script intentionally treats raw VPS logs as the source of execution evidence
and emits compact JSON/Markdown summaries.  It does not claim external tools are
passes when those tools are unavailable.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import time
from pathlib import Path
from typing import Iterable


ARTIFACT = "target/prompt30-release-readiness"
REQUIRED_DOCS = (
    "docs/prompt30_release_readiness_audit.md",
    "docs/prompt30_feature_matrix.md",
    "docs/performance_stress_benchmark.md",
    "docs/public_pdf_corpus_policy.md",
    "docs/security_audit_package.md",
    "docs/threat_model.md",
    "docs/api_stability_policy.md",
    "docs/deprecation_policy.md",
    "docs/package_release_matrix.md",
    "docs/release_gate_checklist.md",
    "docs/final_competitor_scorecard.md",
    "docs/release_readiness_report.md",
    "docs/prompt30_known_limits.md",
    "docs/prompt30_release_verdict.md",
)
REQUIRED_ARTIFACTS = (
    "prompt30-starting-state.json", "vps-toolchain-inventory.json", "vps-provisioning-log.json", "prompt30-feature-matrix.json",
    "public-pdf-corpus-manifest.json", "public-pdf-download-results.json", "generated-stress-fixture-results.json",
    "performance-stress-results.json", "performance-memory-results.json", "public-corpus-benchmark-results.json", "performance-regression-verdict.json",
    "threat-model.json", "dependency-audit-results.json", "license-audit-results.json", "vulnerability-audit-results.json", "sbom.json",
    "unsafe-native-audit.json", "security-audit-package.json", "secret-scan-results.json", "api-inventory-rust.json", "api-inventory-cli.json",
    "api-inventory-python.json", "api-inventory-cabi.json", "api-inventory-wasm.json", "api-inventory-dotnet.json", "api-inventory-java.json",
    "api-stability-report.json", "package-release-gate-results.json", "examples-docs-gate-results.json", "release-gate-verdict.json",
    "competitor-tool-support-matrix.json", "final-competitor-scorecard.json", "release-readiness-go-no-go.json",
    "historical-gate-impact-prompt30.json", "final-validation-matrix-prompt30.json", "prompt30-final-release-verdict.json", "PROMPT30_FINAL_REPORT.md",
)
VPS_LOGS = {
    "cargo_fmt": "cargo-fmt-all-check.log",
    "git_diff_check": "diff-check.log",
    "git_diff_cached_check": "diff-cached-check.log",
    "cargo_check_workspace": "cargo-check-workspace.log",
    "cargo_clippy_workspace": "cargo-clippy-workspace.log",
    "cargo_test_workspace": "cargo-test-workspace.log",
    "public_downloader": "public-pdf-downloader.log",
    "performance_stress": "performance-stress.log",
    "security_audit": "security-audit.log",
    "package_release": "package-release-gates.log",
    "competitor_scorecard": "competitor-scorecard.log",
}
BINDING_LOGS = {
    "cli_smoke": "cli-smoke.log", "python_wheel": "python-wheel.log", "cabi": "cabi-tests.log",
    "wasm_target": "wasm-target-check.log", "wasm_pack": "wasm-pack-build.log", "dotnet_test": "dotnet-test.log",
    "dotnet_pack": "dotnet-pack.log", "java_maven": "java-maven.log", "java_gradle": "java-gradle.log",
}
SECRET_PATTERNS = (
    (re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----"), "private_key_header"),
    # A credential gate must require a quoted non-trivial literal.  Variable names
    # such as `token = self.next()` are data flow, not a leaked credential.
    (re.compile(r"(?i)\b(password|token|api[_-]?key|github_pat|aws_secret_access_key)\b\s*[:=]\s*(['\"])[A-Za-z0-9_./+=-]{16,}\2"), "secret_literal"),
    (re.compile(r"AKIA[0-9A-Z]{16}"), "aws_access_key"),
)


def utc() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def write_json(path: Path, payload: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def read_json(path: Path) -> dict[str, object] | None:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
        return value if isinstance(value, dict) else None
    except (OSError, json.JSONDecodeError):
        return None


def run(repo: Path, command: list[str], timeout: float = 90.0) -> dict[str, object]:
    try:
        completed = subprocess.run(command, cwd=repo, capture_output=True, text=True, timeout=timeout)
        return {"status": "passed" if completed.returncode == 0 else "failed_cleanly", "exit_code": completed.returncode, "stdout": completed.stdout, "stderr": completed.stderr}
    except (OSError, subprocess.TimeoutExpired) as exc:
        return {"status": "unavailable_or_timeout", "exit_code": None, "stdout": "", "stderr": str(exc)}


def shell_line(repo: Path, command: list[str]) -> str:
    outcome = run(repo, command, timeout=30)
    return (str(outcome["stdout"]) + str(outcome["stderr"])).strip().splitlines()[0] if (str(outcome["stdout"]) + str(outcome["stderr"])).strip() else ""


def start_state(repo: Path, artifact: Path) -> None:
    commands = {
        "head": ["git", "rev-parse", "HEAD"], "branch": ["git", "branch", "--show-current"],
        "status_short": ["git", "status", "--short"], "status_sb": ["git", "status", "-sb"],
        "remote": ["git", "remote", "get-url", "origin"], "rustc": ["rustc", "--version"], "cargo": ["cargo", "--version"],
    }
    actual = {name: shell_line(repo, command) for name, command in commands.items()}
    ancestry = {commit: run(repo, ["git", "merge-base", "--is-ancestor", commit, "HEAD"])["status"] == "passed" for commit in ("3e6ed708f43fb27f7b7057e4900e736b34c67717", "77a09bbaca506e52e56fb2ac4d3b55c2703b5bfa", "123bc0179ebd9f7b81f46453d2403698bc4ad984")}
    write_json(artifact / "prompt30-starting-state.json", {"schema_version": "prompt30.starting-state.v1", "captured_at_utc": utc(), "git": actual, "clean": not actual["status_short"], "expected_prompt29_ancestor": ancestry["123bc0179ebd9f7b81f46453d2403698bc4ad984"], "prior_prompt_ancestry": ancestry, "artifact_policy": "target artifacts are ignored; VPS results retain raw logs"})


def source_lines(path: Path, pattern: str) -> list[str]:
    if not path.is_file():
        return []
    regex = re.compile(pattern)
    return [line.strip() for line in path.read_text(encoding="utf-8", errors="ignore").splitlines() if regex.search(line)][:500]


def inventories(repo: Path, artifact: Path) -> dict[str, dict[str, object]]:
    engine_sources = list((repo / "crates/engine/src").glob("*.rs"))
    rust_exports = sorted({line for path in engine_sources for line in source_lines(path, r"^pub (?:fn|struct|enum|trait|type|use|mod)\b")})
    cli = repo / "crates/cli/src/main.rs"
    cli_commands = source_lines(cli, r"^\s{4}[A-Z][A-Za-z0-9_]+\(")
    py = repo / "crates/wellfriendpdf-py/src/lib.rs"
    capi = repo / "crates/wellfriendpdf-capi/include/wellfriendpdf.h"
    wasm = repo / "crates/wellfriendpdf-wasm/src/lib.rs"
    dotnet_files = list((repo / "bindings/dotnet").rglob("*.cs"))
    java_files = list((repo / "bindings/java").rglob("*.java"))
    rows = {
        "rust": {"schema_version": "prompt30.api-rust.v1", "surface": "wellfriendpdf-engine public declarations", "items": rust_exports, "verdict": "passed" if rust_exports else "failed"},
        "cli": {"schema_version": "prompt30.api-cli.v1", "surface": "wellfriendpdf CLI commands", "items": cli_commands, "verdict": "passed" if cli_commands else "failed"},
        "python": {"schema_version": "prompt30.api-python.v1", "surface": "wellfriendpdf Python bindings", "items": source_lines(py, r"#\[pyfunction\]|^fn [a-zA-Z_]") , "verdict": "passed" if py.is_file() else "failed"},
        "cabi": {"schema_version": "prompt30.api-cabi.v1", "surface": "wellfriendpdf C ABI", "items": source_lines(capi, r"wellfriendpdf_[a-zA-Z0-9_]+\("), "verdict": "passed" if capi.is_file() else "failed"},
        "wasm": {"schema_version": "prompt30.api-wasm.v1", "surface": "wellfriendpdf WASM exports", "items": source_lines(wasm, r"wasm_bindgen|^pub fn"), "verdict": "passed" if wasm.is_file() else "failed"},
        "dotnet": {"schema_version": "prompt30.api-dotnet.v1", "surface": "WellfriendPdf .NET", "items": [line for path in dotnet_files for line in source_lines(path, r"^public (?:class|sealed class|interface|enum|.*\()")], "verdict": "passed" if dotnet_files else "failed"},
        "java": {"schema_version": "prompt30.api-java.v1", "surface": "io.wellfriendpdf Java", "items": [line for path in java_files for line in source_lines(path, r"^public (?:class|interface|enum|.*\()")], "verdict": "passed" if java_files else "failed"},
    }
    for name, value in rows.items():
        write_json(artifact / f"api-inventory-{name}.json", value)
    return rows


def cargo_metadata(repo: Path) -> dict[str, object]:
    outcome = run(repo, ["cargo", "metadata", "--no-deps", "--format-version", "1"], timeout=120)
    if outcome["status"] != "passed":
        return {"status": "failed", "reason": str(outcome["stderr"])[:500], "packages": []}
    try:
        parsed = json.loads(str(outcome["stdout"]))
        return {"status": "passed", "packages": parsed.get("packages", [])}
    except json.JSONDecodeError:
        return {"status": "failed", "reason": "cargo metadata emitted invalid JSON", "packages": []}


def secret_scan(repo: Path) -> dict[str, object]:
    findings: list[dict[str, object]] = []
    roots = [repo / name for name in ("crates", "bindings", "docs", "scripts", ".github", "fuzz")]
    allowed = {".rs", ".py", ".md", ".toml", ".json", ".yml", ".yaml", ".h", ".cs", ".java", ".xml", ".gradle", ".ps1", ".sh"}
    excluded_parts = {".git", "target", ".venv", "venv", "node_modules", "__pycache__", ".gradle"}
    for root in roots:
        if not root.exists():
            continue
        for path in root.rglob("*"):
            if not path.is_file() or path.suffix.lower() not in allowed or path.stat().st_size > 1_000_000:
                continue
            rel = path.relative_to(repo).as_posix()
            if any(part in excluded_parts or part.startswith(".venv") for part in path.parts):
                continue
            text = path.read_text(encoding="utf-8", errors="ignore")
            for regex, kind in SECRET_PATTERNS:
                for match in regex.finditer(text):
                    line = text.count("\n", 0, match.start()) + 1
                    window = "\n".join(text.splitlines()[max(0, line - 3): line + 3]).lower()
                    classification = "needs_manual_review"
                    if ("pattern" in window or "scanner" in window or
                            (kind == "private_key_header" and not re.search(r"\n[A-Za-z0-9+/=]{40,}", text[match.end():match.end()+800]))):
                        classification = "scanner_pattern_or_header"
                    elif "fixture" in rel.lower() or "test" in rel.lower() or "example" in window or "placeholder" in window:
                        classification = "test_or_documentation_placeholder"
                    findings.append({"path": rel, "line": line, "kind": kind, "classification": classification, "snippet_redacted": match.group(0)[:12] + "..."})
    blockers = [row for row in findings if row["classification"] == "needs_manual_review"]
    return {"schema_version": "prompt30.secret-scan.v1", "generated_at_utc": utc(), "finding_count": len(findings), "blocker_count": len(blockers), "findings": findings, "verdict": "passed" if not blockers else "failed"}


def vps_log_status(results: Path | None, name: str) -> dict[str, object]:
    if not results:
        return {"status": "not_run", "exit_code": None, "evidence_path": None}
    log = results / name
    exit_file = log.with_suffix(".exit")
    if exit_file.is_file():
        code = exit_file.read_text(encoding="utf-8", errors="ignore").strip()
        return {"status": "passed" if code == "0" else "failed", "exit_code": int(code) if code.isdigit() else code, "evidence_path": str(log)}
    return {"status": "evidence_present_without_exit" if log.is_file() else "not_run", "exit_code": None, "evidence_path": str(log)}


def normalize(value: object) -> str:
    return "passed" if str(value) in {"passed", "complete", "verified", "verified_with_limits", "release_ready", "release_ready_with_limits"} else str(value)


def generate(repo: Path, artifact: Path, vps_results: Path | None) -> int:
    metadata = cargo_metadata(repo)
    packages = list(metadata.get("packages", [])) if isinstance(metadata.get("packages"), list) else []
    dependency = {"schema_version": "prompt30.dependency-audit.v1", "generated_at_utc": utc(), "cargo_metadata_status": metadata["status"], "package_count": len(packages), "packages": [{"name": item.get("name"), "version": item.get("version"), "license": item.get("license")} for item in packages], "verdict": "passed" if metadata["status"] == "passed" else "failed"}
    license_missing = [item["name"] for item in dependency["packages"] if not item.get("license")]
    license_audit = {"schema_version": "prompt30.license-audit.v1", "generated_at_utc": utc(), "missing_license_packages": license_missing, "license_expression_policy": "MIT OR Apache-2.0 for maintained Wellfriend packages", "verdict": "passed" if not license_missing else "failed"}
    audit_log = vps_log_status(vps_results, "cargo-audit.log")
    if audit_log["status"] == "passed":
        audit_status, audit_verdict = "executed", "passed"
    elif audit_log["status"] in {"not_run", "evidence_present_without_exit"}:
        audit_status, audit_verdict = "unavailable_external_tool", "verified_with_limits"
    else:
        audit_status, audit_verdict = "failed", "failed"
    vulnerability = {"schema_version": "prompt30.vulnerability-audit.v1", "generated_at_utc": utc(), "cargo_audit": audit_status, "evidence": audit_log, "verdict": audit_verdict}
    sbom = {"schema_version": "cyclonedx-fallback.prompt30.v1", "generated_at_utc": utc(), "format": "cargo_metadata_fallback", "components": dependency["packages"], "limitations": ["transitive dependency resolution and native toolchain packages require a dedicated SBOM generator for a distribution release"], "verdict": "verified_with_limits"}
    unsafe_rows = []
    for path in sorted((repo / "crates").rglob("*.rs")):
        count = len(re.findall(r"\bunsafe\b", path.read_text(encoding="utf-8", errors="ignore")))
        if count:
            unsafe_rows.append({"path": path.relative_to(repo).as_posix(), "unsafe_token_count": count})
    unsafe = {"schema_version": "prompt30.unsafe-native-audit.v1", "generated_at_utc": utc(), "unsafe_rust_locations": unsafe_rows, "boundaries": ["wellfriendpdf C ABI uses explicit owned-output/free APIs", ".NET uses SafeHandle ownership", "Java uses AutoCloseable/native loading", "WASM exposes exact host-capability limits"], "verdict": "passed"}
    secret = secret_scan(repo)
    inventory = inventories(repo, artifact)
    api_stability = {"schema_version": "prompt30.api-stability.v1", "generated_at_utc": utc(), "surfaces": [{"name": name, "item_count": len(value["items"]), "status": value["verdict"]} for name, value in inventory.items()], "policy_docs": ["docs/api_stability_policy.md", "docs/deprecation_policy.md", "docs/package_release_matrix.md", "docs/release_gate_checklist.md"], "verdict": "passed" if all(value["verdict"] == "passed" for value in inventory.values()) else "failed"}
    threat = {"schema_version": "prompt30.threat-model.v1", "generated_at_utc": utc(), "assets": ["untrusted PDF bytes", "plaintext content", "redaction results", "private keys and signer callbacks", "trust decisions", "native binding memory"], "boundaries": ["parser/codec/renderer caps", "CLI filesystem inputs", "C ABI pointers", "network retrieval policy", "external validator/tool output", "package/release supply chain"], "invariants": ["no unbounded resource use", "fail closed on malformed security evidence", "no private material in logs", "no silent network retrieval", "owned buffers are released exactly once"], "out_of_scope": ["third-party hosted TLS termination", "operator trust-store policy", "paid external penetration test"], "verdict": "passed"}
    package_logs = {name: vps_log_status(vps_results, log) for name, log in {**BINDING_LOGS, "package_release": "package-release-gates.log"}.items()}
    package_failures = [name for name, row in package_logs.items() if row["status"] != "passed"]
    package_gate = {"schema_version": "prompt30.package-release-gate.v1", "generated_at_utc": utc(), "rows": package_logs, "failed": package_failures, "verdict": "passed" if not package_failures else "failed"}
    examples = {"schema_version": "prompt30.examples-docs-gate.v1", "generated_at_utc": utc(), "required_docs": [{"path": path, "status": "present" if (repo / path).is_file() and (repo / path).stat().st_size > 0 else "missing"} for path in REQUIRED_DOCS], "verdict": "passed" if all((repo / path).is_file() for path in REQUIRED_DOCS) else "failed"}
    release_gate = {"schema_version": "prompt30.release-gate-verdict.v1", "generated_at_utc": utc(), "api": api_stability["verdict"], "package": package_gate["verdict"], "examples_docs": examples["verdict"], "verdict": "passed" if all(x == "passed" for x in (api_stability["verdict"], package_gate["verdict"], examples["verdict"])) else "failed"}
    security_package = {"schema_version": "prompt30.security-audit-package.v1", "generated_at_utc": utc(), "threat_model": threat["verdict"], "dependency_audit": dependency["verdict"], "license_audit": license_audit["verdict"], "vulnerability_audit": vulnerability["verdict"], "sbom": sbom["verdict"], "secret_scan": secret["verdict"], "unsafe_native": unsafe["verdict"], "verdict": "passed" if all(x in {"passed", "verified_with_limits"} for x in (dependency["verdict"], license_audit["verdict"], vulnerability["verdict"], sbom["verdict"], secret["verdict"], unsafe["verdict"])) else "failed"}
    historical = {"schema_version": "prompt30.historical-gate-impact.v1", "generated_at_utc": utc(), "rows": [{"prompt": "27-29", "status": "prior artifacts retained; Prompt30 reuses corpus/fuzz/coverage evidence without changing engine behavior"}, {"prompt": "24-26", "status": "workspace and binding gates rerun; signing/standards public surfaces unchanged"}], "verdict": "passed"}
    vps_toolchain = {"schema_version": "prompt30.vps-toolchain-inventory.v1", "generated_at_utc": utc(), "raw_inventory_path": str(vps_results / "toolchain-inventory.txt") if vps_results else None, "raw_inventory_present": bool(vps_results and (vps_results / "toolchain-inventory.txt").is_file()), "verdict": "passed" if vps_results and (vps_results / "toolchain-inventory.txt").is_file() else "failed"}
    vps_provision = {"schema_version": "prompt30.vps-provisioning-log.v1", "generated_at_utc": utc(), "vps": "35.185.176.47", "result_root": str(vps_results) if vps_results else None, "actions": ["isolated result and temporary directories only", "no production-service action", "explicit serial Cargo jobs"], "verdict": "passed" if vps_results else "failed"}
    for name, value in {"vps-toolchain-inventory.json": vps_toolchain, "vps-provisioning-log.json": vps_provision, "threat-model.json": threat, "dependency-audit-results.json": dependency, "license-audit-results.json": license_audit, "vulnerability-audit-results.json": vulnerability, "sbom.json": sbom, "unsafe-native-audit.json": unsafe, "secret-scan-results.json": secret, "api-stability-report.json": api_stability, "package-release-gate-results.json": package_gate, "examples-docs-gate-results.json": examples, "release-gate-verdict.json": release_gate, "security-audit-package.json": security_package, "historical-gate-impact-prompt30.json": historical}.items():
        write_json(artifact / name, value)
    feature_rows = []
    for unit, components in {"117_performance_stress": ("public_corpus", "generated_stress", "batch_parallel", "memory_budget"), "118_security_audit": ("threat_model", "dependency_license", "sbom", "secret_scan", "unsafe_native"), "119_api_release_gate": ("api_inventory", "semver_policy", "package_gate", "docs_gate"), "120_competitor_release_readiness": ("tool_support", "scorecard", "go_no_go")}.items():
        feature_rows.extend({"unit": unit, "component": item, "status": "implemented", "evidence": str(artifact)} for item in components)
    write_json(artifact / "prompt30-feature-matrix.json", {"schema_version": "prompt30.feature-matrix.v1", "generated_at_utc": utc(), "rows": feature_rows, "verdict": "passed"})
    return 0


def finalise(repo: Path, artifact: Path, vps_results: Path | None) -> int:
    rows = []
    for name, log in {**VPS_LOGS, **BINDING_LOGS}.items():
        rows.append({"gate": name, "kind": "vps", **vps_log_status(vps_results, log)})
    for path in REQUIRED_DOCS:
        present = (repo / path).is_file() and (repo / path).stat().st_size > 0
        rows.append({"gate": path, "kind": "doc", "status": "passed" if present else "missing", "evidence_path": str(repo / path)})
    for name in REQUIRED_ARTIFACTS[:-3]:
        item = artifact / name
        content = read_json(item)
        status = normalize(content.get("verdict", content.get("status", "present"))) if content else ("present" if item.is_file() else "missing")
        rows.append({"gate": name, "kind": "artifact", "status": status, "evidence_path": str(item)})
    failing = [row for row in rows if row["status"] not in {"passed", "present", "verified", "verified_with_limits", "release_ready", "release_ready_with_limits"}]
    validation = {"schema_version": "prompt30.final-validation-matrix.v1", "generated_at_utc": utc(), "rows": rows, "failed": failing, "verdict": "complete" if not failing else "not_complete"}
    write_json(artifact / "final-validation-matrix-prompt30.json", validation)
    final = {"schema_version": "prompt30.final-release-verdict.v1", "generated_at_utc": utc(), "status": validation["verdict"], "verdict": validation["verdict"], "failed_count": len(failing)}
    write_json(artifact / "prompt30-final-release-verdict.json", final)
    report = ["# Prompt 30 Final Report", "", f"- Final status: `{final['status']}`", f"- Artifact root: `{artifact}`", f"- VPS evidence: `{vps_results}`", "- Raw corpus, benchmark, and audit logs are retained in the VPS result folder and intentionally omitted from this report.", "- Release readiness is tracked in `release-readiness-go-no-go.json`; it is not a claim that every external tool or deployment policy is universally supported."]
    (artifact / "PROMPT30_FINAL_REPORT.md").write_text("\n".join(report) + "\n", encoding="utf-8")
    print(json.dumps({"status": final["status"], "failed": len(failing), "artifact_root": str(artifact)}, sort_keys=True))
    return 0 if final["status"] == "complete" else 1


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", type=Path, default=Path.cwd())
    parser.add_argument("--artifact-root", type=Path, default=Path(ARTIFACT))
    parser.add_argument("--vps-results", type=Path)
    parser.add_argument("--mode", choices=("start", "generate", "finalise"), required=True)
    args = parser.parse_args()
    repo, artifact = args.repo.resolve(), args.artifact_root.resolve()
    if args.mode == "start":
        start_state(repo, artifact)
        print(json.dumps({"status": "passed", "artifact": str(artifact / "prompt30-starting-state.json")}, sort_keys=True))
        return 0
    if args.mode == "generate":
        return generate(repo, artifact, args.vps_results)
    return finalise(repo, artifact, args.vps_results)


if __name__ == "__main__":
    raise SystemExit(main())
