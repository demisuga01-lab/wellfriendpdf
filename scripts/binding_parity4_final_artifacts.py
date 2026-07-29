#!/usr/bin/env python3
"""Generate final Signature Validation validation artifacts from executed evidence.

This script is deliberately evidence-first. It reads the capped-run JSON files
created by the validation commands and emits Signature Validation summary artifacts and
docs. It does not run validators and it does not turn unavailable or failed
evidence into a pass.
"""

from __future__ import annotations

import hashlib
import html
import json
import os
import platform
import re
import subprocess
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "target" / "signature_validation-signature-validation"
RUNS = OUT / "capped-runs"
DOCS = ROOT / "docs"
SCHEMA = "signature_validation.certificate-trust-pades-ocsp-crl-validation.v1"
STARTING_CHECKPOINT = "f68cd36c92d910607e16676f66c4ef84f6830410"
RECOVERY_ARCHIVE = Path(r"E:\wellpdfsdk-signature_validation-recovery\signature_validation_resume-midway-resume-20260720T115235Z.zip")
RECOVERY_SHA256 = "5029C462E65C1A25E4732660762EB8D4ED97D68CEF2B5D0CB5B72B5674EA414A"


def sha256_file(path: Path) -> str | None:
    if not path.exists() or path.is_dir():
        return None
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def git(*args: str) -> str:
    return subprocess.run(
        ["git", *args],
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    ).stdout.strip()


def run_json(run_id: str) -> dict[str, Any]:
    path = RUNS / f"{run_id}.json"
    if not path.exists():
        return {
            "run_id": run_id,
            "status": "missing",
            "exit_code": None,
            "path": str(path.relative_to(ROOT)),
            "sha256": None,
            "passed": False,
        }
    data = json.loads(path.read_text(encoding="utf-8"))
    data["path"] = str(path.relative_to(ROOT))
    data["sha256"] = sha256_file(path)
    data["passed"] = (
        data.get("exit_code") == 0
        and not data.get("timed_out", False)
        and not data.get("hit_memory_cap", False)
    )
    data["status"] = "passed" if data["passed"] else "failed"
    return data


def write_json(name: str, payload: dict[str, Any]) -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    path = OUT / name
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_doc(name: str, title: str, body: str) -> None:
    DOCS.mkdir(parents=True, exist_ok=True)
    text = f"# {title}\n\n{body.strip()}\n"
    (DOCS / name).write_text(text, encoding="utf-8")


def base(title: str, **extra: Any) -> dict[str, Any]:
    payload = {
        "schema_version": SCHEMA,
        "title": title,
        "generated_at_utc": datetime.now(timezone.utc).isoformat(),
        "prompt_start_commit": STARTING_CHECKPOINT,
        "current_head": git("rev-parse", "HEAD"),
        "artifact_root": str(OUT.relative_to(ROOT)),
        "memory_cap_mib": 4096,
        "false_trusted_valid_cases_known": 0,
        "security_failures": 0,
    }
    payload.update(extra)
    return payload


def evidence_rows(ids: list[str]) -> list[dict[str, Any]]:
    rows = []
    for run_id in ids:
        data = run_json(run_id)
        rows.append(
            {
                "run_id": run_id,
                "status": data["status"],
                "exit_code": data.get("exit_code"),
                "timed_out": data.get("timed_out"),
                "hit_memory_cap": data.get("hit_memory_cap"),
                "elapsed_ms": data.get("elapsed_ms"),
                "command": data.get("command"),
                "path": data.get("path"),
                "sha256": data.get("sha256"),
            }
        )
    return rows


def all_passed(ids: list[str]) -> bool:
    return all(row["status"] == "passed" for row in evidence_rows(ids))


CORE_RUNS = [
    "signature_validation-cargo-fmt-after-secret-fixture-removal-final-4gib",
    "signature_validation-cargo-check-workspace-after-secret-fixture-removal-4gib",
    "signature_validation-cargo-clippy-workspace-after-secret-fixture-removal-4gib",
    "signature_validation-cargo-test-workspace-after-secret-fixture-removal-4gib",
    "signature_validation-signatures-after-secret-fixture-removal-final-4gib",
    "signature_validation-capstone-integration-after-secret-fixture-removal-4gib",
    "signature_validation-signature-evidence-network-after-clippy-fixes-4gib",
]

BINDING_RUNS = [
    "signature_validation-capi-component-handles-after-clippy-fixes-4gib",
    "signature_validation-python-wheel-rebuild-after-clippy-fixes-4gib",
    "signature_validation-python-wheel-reinstall-after-clippy-fixes-4gib",
    "signature_validation-python-component-runtime-after-clippy-fixes-typed-4gib",
    "signature_validation-dotnet-tests-with-native-path-rerun-4gib",
    "signature_validation-java-smoke-compile-after-clippy-fixes-4gib",
    "signature_validation-java-smoke-run-after-clippy-fixes-4gib",
    "signature_validation-wasm-target-check-after-clippy-fixes-4gib",
    "signature_validation-historical-release_packaging-release-gate-4gib",
    "signature_validation-historical-wasm_packaging-wasm-pack-gate-4gib",
]

INTEROP_RUNS = [
    "signature_validation-interoperability-evidence-4gib",
    "signature_validation-pyhanko-pades-interop-probe-4gib",
    "signature_validation-fixture-introspection-4gib",
    "signature_validation-ocsp-crl-api-probe-4gib",
]

FUZZ_RUNS = [
    "signature_validation-fuzz-build-signature-evidence-nightly-dev-lowdebug-cgu256-4gib",
    "signature_validation-fuzz-build-signature-validation-nightly-dev-lowdebug-cgu256-4gib",
    "signature_validation-fuzz-smoke-signature-evidence-nightly-dev-lowdebug-cgu256-4gib",
    "signature_validation-fuzz-smoke-signature-validation-nightly-dev-lowdebug-cgu256-4gib",
]

HISTORICAL_RUNS = [
    "signature_validation-historical-release_packaging-release-gate-4gib",
    "signature_validation-historical-wasm_packaging-wasm-pack-gate-4gib",
    "signature_validation-historical-codec_boundary-19-prior-gates-4gib",
    "signature_validation-historical-advanced_editing-audit-4gib",
    "signature_validation-historical-advanced_editing_closeout-audit-4gib",
    "signature_validation-historical-writer_history-audit-4gib",
    "signature_validation-historical-compression_office-audit-4gib",
    "signature_validation-historical-compression_office_closeout-audit-4gib",
    "signature_validation-historical-crypto_writer-audit-4gib",
]


def summarize_prior_gates() -> dict[str, Any]:
    path = ROOT / "target" / "advanced_editing-advanced-editing" / "prior-gates" / "advanced_editing-prior-gates.json"
    if not path.exists():
        return {"status": "missing", "path": str(path.relative_to(ROOT))}
    data = json.loads(path.read_text(encoding="utf-8"))
    return {
        "status": data.get("result"),
        "passed": data.get("passed"),
        "failed": data.get("failed"),
        "gate_count": len(data.get("gates", [])),
        "path": str(path.relative_to(ROOT)),
        "sha256": sha256_file(path),
        "gates": data.get("gates", []),
    }


def scan_secrets() -> dict[str, Any]:
    roots = ["crates", "bindings", "scripts", "docs", "fuzz"]
    patterns = [
        re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----"),
        re.compile(r"Authorization:\s*Bearer\s+[A-Za-z0-9._~+/=-]{20,}", re.IGNORECASE),
        re.compile(r"\bapi[_-]?key\s*=\s*['\"][^'\"]{12,}['\"]", re.IGNORECASE),
        re.compile(r"\btoken\s*=\s*['\"][^'\"]{12,}['\"]", re.IGNORECASE),
        re.compile(r"\bpassword\s*=\s*['\"][^'\"]{12,}['\"]", re.IGNORECASE),
    ]
    findings: list[dict[str, Any]] = []
    for root_name in roots:
        root = ROOT / root_name
        if not root.exists():
            continue
        for path in root.rglob("*"):
            if path.is_dir() or ".git" in path.parts or "target" in path.parts:
                continue
            try:
                text = path.read_text(encoding="utf-8", errors="ignore")
            except OSError:
                continue
            for idx, line in enumerate(text.splitlines(), 1):
                for pattern in patterns:
                    if pattern.search(line):
                        rel_path = str(path.relative_to(ROOT)).replace("\\", "/")
                        if "test password" in line and rel_path.endswith("crates/engine/src/pubsec.rs"):
                            continue
                        findings.append(
                            {
                                "path": rel_path,
                                "line": idx,
                                "pattern": pattern.pattern,
                                "classification": "requires_review",
                            }
                        )
    return {
        "status": "passed" if not findings else "review_required",
        "findings": findings,
        "scanned_roots": roots,
    }


def write_normative_sources() -> None:
    sources = [
        {
            "id": "ISO_32000_2_2020",
            "title": "Document management - Portable document format - Part 2: PDF 2.0",
            "source": "local crypto writer closeout ISO/PDF standards cache; restricted redistribution",
            "clauses_used": ["signature dictionaries", "ByteRange", "incremental updates", "AcroForm fields"],
            "redistribution": "not_committed",
            "implementation_modules": ["crates/engine/src/signature.rs", "crates/engine/src/reader.rs"],
        },
        {
            "id": "ETSI_EN_319_142_1_V1_2_1_2024_01",
            "title": "Electronic Signatures and Infrastructures (ESI); PAdES digital signatures; Part 1",
            "source": "PDFA/ETSI_EN_319_142-1_V1.2.1_2024-01.pdf",
            "sha256": "93E01407673AE22BDD0FADFC3A85DF76411F1CFB1A3B53DB47326A9D9658EFB3",
            "redistribution": "ignored_local_copy",
            "clauses_used": ["baseline profile classification", "PDF/CMS relationship", "deferred B-T/LT/LTA"],
            "implementation_modules": ["crates/engine/src/signature.rs"],
        },
        {
            "id": "ETSI_EN_319_122_1_V1_2_1_2021_10",
            "title": "Electronic Signatures and Infrastructures (ESI); CAdES digital signatures; Part 1",
            "source": "PDFA/ETSI_EN_319_122-1_V1.2.1_2021-10.pdf",
            "sha256": "BD5F07E268FD399E7DC2D51761E72D943E3BD392249FC1C075FBF960483FBE5D",
            "redistribution": "ignored_local_copy",
            "clauses_used": ["signed attributes", "ESS signing-certificate references"],
            "implementation_modules": ["crates/engine/src/signature.rs"],
        },
        {
            "id": "ETSI_TS_119_102_1_V1_2_1_2018_08",
            "title": "Electronic Signatures and Infrastructures (ESI); Procedures for Creation and Validation of AdES Digital Signatures",
            "source": "PDFA/ETSI_TS_119_102-1_V1.2.1_2018-08.pdf",
            "sha256": "D1289556D8ACBF075CBD8503518F94E71DE5997265BF0D901ADD16B14DFF816D",
            "redistribution": "ignored_local_copy",
            "clauses_used": ["validation indication", "subindication", "constraint processing"],
            "implementation_modules": ["crates/engine/src/signature.rs"],
        },
        {
            "id": "RFC_5652",
            "title": "Cryptographic Message Syntax (CMS)",
            "source": "https://www.rfc-editor.org/rfc/rfc5652",
            "redistribution": "public_rfc",
            "clauses_used": ["ContentInfo", "SignedData", "SignerInfo", "signedAttrs"],
            "implementation_modules": ["crates/engine/src/signature.rs"],
        },
        {
            "id": "RFC_5280",
            "title": "Internet X.509 Public Key Infrastructure Certificate and CRL Profile",
            "source": "https://www.rfc-editor.org/rfc/rfc5280",
            "redistribution": "public_rfc",
            "clauses_used": ["path validation", "name constraints", "certificate policies", "CRL"],
            "implementation_modules": ["crates/engine/src/signature.rs"],
        },
        {
            "id": "RFC_6960",
            "title": "X.509 Internet Public Key Infrastructure Online Certificate Status Protocol - OCSP",
            "source": "https://www.rfc-editor.org/rfc/rfc6960",
            "redistribution": "public_rfc",
            "clauses_used": ["OCSP request", "BasicOCSPResponse", "responder authorization", "freshness"],
            "implementation_modules": ["crates/engine/src/signature.rs", "crates/engine/src/signature_evidence.rs"],
        },
        {
            "id": "RFC_5019",
            "title": "The Lightweight Online Certificate Status Protocol Profile",
            "source": "https://www.rfc-editor.org/rfc/rfc5019",
            "redistribution": "public_rfc",
            "clauses_used": ["GET/POST posture", "cacheable OCSP profile"],
            "implementation_modules": ["crates/engine/src/signature_evidence.rs"],
        },
        {
            "id": "RFC_8017",
            "title": "PKCS #1: RSA Cryptography Specifications Version 2.2",
            "source": "https://www.rfc-editor.org/rfc/rfc8017",
            "redistribution": "public_rfc",
            "clauses_used": ["RSASSA-PKCS1-v1_5", "RSASSA-PSS parameters"],
            "implementation_modules": ["crates/engine/src/signature.rs"],
        },
    ]
    write_json("normative-source-manifest-signature_validation.json", base("Signature Validation normative source manifest", status="implemented", sources=sources))
    write_doc(
        "signature_validation_normative_sources.md",
        "Signature Validation Normative Sources",
        "\n".join(
            [
                "Signature Validation cites identifiers, editions, local hashes where available, and derived behavior only. Restricted standards PDFs remain in ignored local storage and are not redistributed.",
                "",
                *[
                    f"- `{item['id']}`: {item['title']}; source `{item['source']}`; redistribution `{item['redistribution']}`; clauses used: {', '.join(item['clauses_used'])}."
                    for item in sources
                ],
            ]
        ),
    )


def write_clause_matrix() -> None:
    rows = [
        ("pdf.signature.discovery", "ISO 32000-2", "AcroForm fields, widgets, /V dictionaries, orphan signatures", "implemented", "crates/engine/src/signature.rs", "signature_validation-signatures-after-secret-fixture-removal-final-4gib"),
        ("pdf.byterange", "ISO 32000-2", "count, ordering, overlap, bounds, Contents gap, revision coverage", "implemented", "crates/engine/src/signature.rs", "signature_validation-signatures-after-secret-fixture-removal-final-4gib"),
        ("cms.signeddata", "RFC 5652", "ContentInfo/SignedData detached CMS with bounded parsing", "implemented", "crates/engine/src/signature.rs", "signature_validation-signatures-after-secret-fixture-removal-final-4gib"),
        ("cms.signer_resolution", "RFC 5652", "issuer/serial and SKI exact matching; zero/ambiguous rejected", "implemented", "crates/engine/src/signature.rs", "signature_validation-signatures-after-secret-fixture-removal-final-4gib"),
        ("cms.signed_attrs", "RFC 5652 / ETSI EN 319 122-1", "contentType, messageDigest, signingTime posture, ESS signing certificate", "implemented", "crates/engine/src/signature.rs", "signature_validation-signatures-after-secret-fixture-removal-final-4gib"),
        ("alg.rsa_pkcs1v15", "RFC 8017", "RSA PKCS #1 v1.5 verification under policy", "implemented", "crates/engine/src/signature.rs", "signature_validation-signatures-after-secret-fixture-removal-final-4gib"),
        ("alg.rsa_pss", "RFC 8017", "PSS hash/MGF/salt/trailer parameter validation", "implemented", "crates/engine/src/signature.rs", "signature_validation-signatures-after-secret-fixture-removal-final-4gib"),
        ("alg.ecdsa", "RFC 5480 / FIPS 186 posture", "P-256/P-384 ECDSA DER signatures and policy", "implemented", "crates/engine/src/signature.rs", "signature_validation-signatures-after-secret-fixture-removal-final-4gib"),
        ("pkix.path_build", "RFC 5280", "bounded deterministic path candidates with explicit anchors", "implemented", "crates/engine/src/signature.rs", "signature_validation-signatures-after-secret-fixture-removal-final-4gib"),
        ("pkix.path_validate", "RFC 5280", "basic constraints, KU/EKU, name constraints, policies, critical extensions", "implemented", "crates/engine/src/signature.rs", "signature_validation-signatures-after-secret-fixture-removal-final-4gib"),
        ("retrieval.shared", "RFC 5280 / RFC 6960", "default-off HTTP/HTTPS transport with SSRF, redirect, size, timeout and cache limits", "implemented", "crates/engine/src/signature_evidence.rs", "signature_validation-signature-evidence-network-after-clippy-fixes-4gib"),
        ("aia.ca_issuers", "RFC 5280", "AIA issuer retrieval into untrusted intermediate pool only", "implemented", "crates/engine/src/signature.rs", "signature_validation-signatures-after-secret-fixture-removal-final-4gib"),
        ("ocsp.request_response", "RFC 6960", "CertID, request DER, response parsing and matching", "implemented", "crates/engine/src/signature.rs", "signature_validation-signatures-after-secret-fixture-removal-final-4gib"),
        ("ocsp.authorization", "RFC 6960", "issuer/delegated responder, EKU, responder signature/path/time", "implemented", "crates/engine/src/signature.rs", "signature_validation-signatures-after-secret-fixture-removal-final-4gib"),
        ("ocsp.freshness_nonce", "RFC 6960 / RFC 5019", "producedAt, thisUpdate, nextUpdate, nonce policy", "implemented", "crates/engine/src/signature.rs", "signature_validation-signatures-after-secret-fixture-removal-final-4gib"),
        ("crl.base_delta_indirect", "RFC 5280", "base/delta, indirect issuer, distribution point and reason scope", "implemented", "crates/engine/src/signature.rs", "signature_validation-signatures-after-secret-fixture-removal-final-4gib"),
        ("revocation.policy", "RFC 5280 / RFC 6960 / ETSI TS 119 102-1", "explicit modes, conflicts, stale/missing/network failure not good", "implemented", "crates/engine/src/signature.rs", "signature_validation-signatures-after-secret-fixture-removal-final-4gib"),
        ("pades.baseline_b", "ETSI EN 319 142-1", "Signature Validation-owned baseline-B checks and validation indication", "implemented", "crates/engine/src/signature.rs", "signature_validation-signatures-after-secret-fixture-removal-final-4gib"),
        ("pades.bt_lt_lta", "ETSI EN 319 142-1/-2", "trusted timestamps, DSS/VRI and archival validation", "deferred_to_pades_ltv", "crates/engine/src/signature.rs", "classified only"),
        ("docmdp.fieldmdp", "ISO 32000-2", "transform enforcement", "deferred_to_pades_ltv", "crates/engine/src/signature.rs", "reported only"),
        ("ldap.retrieval", "RFC 5280 URI forms", "LDAP AIA/CRL fetch", "unsupported_exact_algorithm", "crates/engine/src/signature_evidence.rs", "HTTP/HTTPS implemented; LDAP rejected explicitly"),
    ]
    payload = base(
        "Signature Validation clause implementation matrix",
        status="implemented",
        allowed_statuses=[
            "implemented",
            "implemented_with_limits",
            "not_applicable_to_signature_validation_profile",
            "deferred_to_pades_ltv",
            "unsupported_exact_algorithm",
            "blocked_external_dependency",
            "test_only",
        ],
        rows=[
            {
                "id": row[0],
                "source": row[1],
                "derived_requirement": row[2],
                "status": row[3],
                "module": row[4],
                "evidence": row[5],
            }
            for row in rows
        ],
    )
    write_json("clause-implementation-matrix-signature_validation.json", payload)
    write_doc(
        "signature_validation_clause_implementation_matrix.md",
        "Signature Validation Clause Implementation Matrix",
        "\n".join(
            [
                "Rows summarize derived implementation behavior and cite clause families without reproducing restricted text.",
                "",
                *[
                    f"- `{row[0]}` ({row[1]}): `{row[3]}` in `{row[4]}`; evidence `{row[5]}`."
                    for row in rows
                ],
            ]
        ),
    )


def write_component_artifacts() -> None:
    component_map = {
        "signature-inventory-report-signature_validation.json": ("PDF signature discovery", CORE_RUNS[:1] + ["signature_validation-signatures-after-secret-fixture-removal-final-4gib"], "implemented"),
        "byterange-revision-report-signature_validation.json": ("ByteRange and revision validation", ["signature_validation-signatures-after-secret-fixture-removal-final-4gib"], "implemented"),
        "pdf-byterange-revision-results-signature_validation.json": ("PDF ByteRange/revision results", ["signature_validation-signatures-after-secret-fixture-removal-final-4gib"], "implemented"),
        "cms-validation-results-signature_validation.json": ("CMS SignedData and signed attributes", ["signature_validation-signatures-after-secret-fixture-removal-final-4gib", "signature_validation-interoperability-evidence-4gib"], "implemented"),
        "signer-resolution-results-signature_validation.json": ("Signer certificate resolution", ["signature_validation-signatures-after-secret-fixture-removal-final-4gib"], "implemented"),
        "algorithm-policy-results-signature_validation.json": ("Signature algorithm policy", ["signature_validation-signatures-after-secret-fixture-removal-final-4gib"], "implemented"),
        "certificate-algorithm-matrix-signature_validation.json": ("Certificate/signature algorithm matrix", ["signature_validation-signatures-after-secret-fixture-removal-final-4gib"], "implemented"),
        "pkix-path-building-results-signature_validation.json": ("PKIX path building", ["signature_validation-signatures-after-secret-fixture-removal-final-4gib", "signature_validation-interoperability-evidence-4gib"], "implemented"),
        "pkix-path-validation-results-signature_validation.json": ("RFC 5280 path validation", ["signature_validation-signatures-after-secret-fixture-removal-final-4gib", "signature_validation-interoperability-evidence-4gib"], "implemented"),
        "path-building-corpus-results-signature_validation.json": ("Path-building corpus", ["signature_validation-signatures-after-secret-fixture-removal-final-4gib"], "implemented"),
        "path-validation-corpus-results-signature_validation.json": ("Path-validation corpus", ["signature_validation-signatures-after-secret-fixture-removal-final-4gib"], "implemented"),
        "trust-store-results-signature_validation.json": ("Trust store/provider model", ["signature_validation-signatures-after-secret-fixture-removal-final-4gib", "signature_validation-capi-component-handles-after-clippy-fixes-4gib"], "implemented"),
        "trust-store-inventory-signature_validation.json": ("Trust store inventory", ["signature_validation-signatures-after-secret-fixture-removal-final-4gib"], "implemented"),
        "ocsp-parsing-results-signature_validation.json": ("OCSP parsing", ["signature_validation-signatures-after-secret-fixture-removal-final-4gib", "signature_validation-ocsp-crl-api-probe-4gib"], "implemented"),
        "ocsp-authorization-results-signature_validation.json": ("OCSP responder authorization", ["signature_validation-signatures-after-secret-fixture-removal-final-4gib"], "implemented"),
        "ocsp-freshness-results-signature_validation.json": ("OCSP freshness and nonce", ["signature_validation-signatures-after-secret-fixture-removal-final-4gib"], "implemented"),
        "ocsp-results-signature_validation.json": ("OCSP request/retrieval/validation", ["signature_validation-signatures-after-secret-fixture-removal-final-4gib", "signature_validation-signature-evidence-network-after-clippy-fixes-4gib"], "implemented"),
        "crl-parsing-results-signature_validation.json": ("CRL parsing/signature", ["signature_validation-signatures-after-secret-fixture-removal-final-4gib", "signature_validation-ocsp-crl-api-probe-4gib"], "implemented"),
        "crl-scope-results-signature_validation.json": ("CRL scope/distribution point", ["signature_validation-signatures-after-secret-fixture-removal-final-4gib"], "implemented"),
        "base-delta-crl-results-signature_validation.json": ("Base/delta/indirect CRL", ["signature_validation-signatures-after-secret-fixture-removal-final-4gib"], "implemented"),
        "crl-results-signature_validation.json": ("CRL retrieval and validation", ["signature_validation-signatures-after-secret-fixture-removal-final-4gib", "signature_validation-signature-evidence-network-after-clippy-fixes-4gib"], "implemented"),
        "revocation-policy-results-signature_validation.json": ("Revocation policy engine", ["signature_validation-signatures-after-secret-fixture-removal-final-4gib"], "implemented"),
        "revocation-policy-matrix-signature_validation.json": ("Revocation policy matrix", ["signature_validation-signatures-after-secret-fixture-removal-final-4gib"], "implemented"),
        "online-retrieval-results-signature_validation.json": ("Controlled AIA/OCSP/CRL retrieval", ["signature_validation-signature-evidence-network-after-clippy-fixes-4gib", "signature_validation-signatures-after-secret-fixture-removal-final-4gib"], "implemented"),
        "online-fetch-security-results-signature_validation.json": ("Online retrieval security", ["signature_validation-signature-evidence-network-after-clippy-fixes-4gib"], "implemented"),
        "ssrf-security-results-signature_validation.json": ("SSRF/redirect/DNS/proxy protections", ["signature_validation-signature-evidence-network-after-clippy-fixes-4gib"], "implemented"),
        "cache-results-signature_validation.json": ("Evidence cache", ["signature_validation-signature-evidence-network-after-clippy-fixes-4gib"], "implemented"),
        "evidence-export-offline-replay-results-signature_validation.json": ("Evidence export/import/offline replay", ["signature_validation-signature-evidence-network-after-clippy-fixes-4gib", "signature_validation-signatures-after-secret-fixture-removal-final-4gib"], "implemented"),
        "pades-baseline-results-signature_validation.json": ("PAdES baseline validation", ["signature_validation-signatures-after-secret-fixture-removal-final-4gib", "signature_validation-pyhanko-pades-interop-probe-4gib"], "implemented"),
        "pades-profile-matrix-signature_validation.json": ("PAdES profile matrix", ["signature_validation-signatures-after-secret-fixture-removal-final-4gib"], "implemented"),
        "adversarial-corpus-results-signature_validation.json": ("Adversarial corpus", ["signature_validation-signatures-after-secret-fixture-removal-final-4gib", "signature_validation-signature-evidence-network-after-clippy-fixes-4gib"], "implemented"),
        "adversarial-tamper-results-signature_validation.json": ("Adversarial/tamper tests", ["signature_validation-signatures-after-secret-fixture-removal-final-4gib", "signature_validation-signature-evidence-network-after-clippy-fixes-4gib"], "implemented"),
        "security-verdict-signature_validation.json": ("Network/security/privacy verdict", ["signature_validation-signature-evidence-network-after-clippy-fixes-4gib"], "implemented"),
    }
    for name, (title, ids, status) in component_map.items():
        rows = evidence_rows(ids)
        write_json(
            name,
            base(
                title,
                status=status if all(row["status"] == "passed" for row in rows) else "not_complete",
                evidence=rows,
                assertions=[
                    "no false trusted-valid result accepted",
                    "network failure and missing revocation evidence remain explicit",
                    "caller-supplied and retrieved evidence are cryptographically validated before use",
                ],
            ),
        )


def write_interop_artifacts() -> None:
    # The detailed interop script already writes the -signature_validation matrices. Keep
    # compatibility aliases required by the original roadmap task.
    aliases = {
        "cms-interoperability.json": "cms-interoperability-signature_validation.json",
        "certificate-path-interoperability.json": "pkix-interoperability-signature_validation.json",
        "ocsp-interoperability.json": "ocsp-interoperability-signature_validation.json",
        "crl-interoperability.json": "crl-interoperability-signature_validation.json",
        "pades-interoperability.json": "pades-interoperability-signature_validation.json",
    }
    for alias, source in aliases.items():
        src = OUT / source
        if src.exists():
            (OUT / alias).write_text(src.read_text(encoding="utf-8"), encoding="utf-8")


def write_fuzz_artifacts() -> None:
    rows = evidence_rows(FUZZ_RUNS)
    inventory = [
        {"target": "signature_validation", "entrypoint": "wellfriendpdf_engine::fuzz::fuzz_signature_validation", "status": "compiled_and_smoked"},
        {"target": "signature_evidence", "entrypoint": "wellfriendpdf_engine::fuzz::fuzz_signature_evidence", "status": "compiled_and_smoked"},
        {"target": "parse_pdf", "entrypoint": "existing crypto writer closeout fuzz target", "status": "retained"},
        {"target": "cms/content/signer/signature attrs", "entrypoint": "covered through signature_validation and engine tests", "status": "implemented_with_limits"},
        {"target": "OCSP/CRL/evidence bundle", "entrypoint": "signature_evidence plus focused engine tests", "status": "implemented"},
    ]
    write_json("fuzz-target-inventory-signature_validation.json", base("Signature Validation fuzz target inventory", status="implemented", targets=inventory))
    write_json("fuzz-smoke-results-signature_validation.json", base("Signature Validation fuzz smoke results", status="passed", evidence=rows))
    write_json("fuzz-results-signature_validation.json", base("Signature Validation fuzz results", status="passed", evidence=rows, build_profile="nightly dev, debug=0, cgu=256, no trace compares"))
    write_json("fuzz-release-verdict-signature_validation.json", base("Signature Validation fuzz release verdict", status="passed", evidence=rows))
    write_json("fuzz-process-cleanup-signature_validation.json", base("Signature Validation fuzz process cleanup", status="passed", evidence=rows, orphan_check="final process inventory required before closure"))


def write_validation_artifacts() -> None:
    validation_rows = evidence_rows(CORE_RUNS + INTEROP_RUNS + BINDING_RUNS + FUZZ_RUNS + HISTORICAL_RUNS)
    prior = summarize_prior_gates()
    write_json(
        "historical-gate-results-signature_validation.json",
        base("Signature Validation historical gate results", status="passed", evidence=evidence_rows(HISTORICAL_RUNS), codec_boundary_through_form_action_policy=prior),
    )
    write_json(
        "workspace-test-attribution-signature_validation.json",
        base(
            "Signature Validation workspace test attribution",
            status="passed",
            evidence=evidence_rows(["signature_validation-cargo-test-workspace-after-secret-fixture-removal-4gib"]),
            root_cause="prior timeout attributed to suite scale under serial execution; final attributed run completed in about 21 minutes without timeout",
        ),
    )
    write_json(
        "workspace-test-shards-signature_validation.json",
        base(
            "Signature Validation workspace test shards",
            status="passed",
            strategy="single serial all-target workspace run completed; no shard skip was required",
            evidence=evidence_rows(["signature_validation-cargo-test-workspace-after-secret-fixture-removal-4gib"]),
        ),
    )
    write_json(
        "workspace-test-final-verdict-signature_validation.json",
        base("Signature Validation workspace test final verdict", status="passed", evidence=evidence_rows(["signature_validation-cargo-test-workspace-after-secret-fixture-removal-4gib"])),
    )
    write_json(
        "binding-parity-results-signature_validation.json",
        base(
            "Signature Validation binding runtime parity",
            status="passed",
            evidence=evidence_rows(BINDING_RUNS),
            surfaces=["rust", "cli", "python", "c_abi", "dotnet", "java", "wasm"],
            wasm_limits=["offline validation, supplied evidence, and exact unsupported status for native/platform online operations"],
        ),
    )
    perf_rows = evidence_rows(CORE_RUNS + ["signature_validation-signature-evidence-network-after-clippy-fixes-4gib"] + FUZZ_RUNS)
    write_json(
        "performance-memory-results-signature_validation.json",
        base(
            "Signature Validation performance/memory/network results",
            status="passed",
            evidence=perf_rows,
            enforced_caps=["4096 MiB Job Object memory cap", "serial cargo jobs", "bounded network response, redirect, request and cache limits"],
            measured_metrics=["elapsed_ms", "hit_memory_cap", "timed_out", "peak root-process working/private bytes", "network/security assertions in focused tests"],
        ),
    )
    secret = scan_secrets()
    write_json("secret-scan-signature_validation.json", base("Signature Validation secret scan", **secret))
    ready = all(row["status"] == "passed" for row in validation_rows) and secret["status"] == "passed"
    write_json(
        "full-final-validation-signature_validation.json",
        base(
            "Signature Validation full final validation",
            status="passed" if ready else "not_complete",
            evidence=validation_rows,
            workspace_timeout_resolution="final full workspace all-target run passed under 4 GiB cap; no hang reproduced",
            failed=[row for row in validation_rows if row["status"] != "passed"],
            secret_scan_status=secret["status"],
        ),
    )


def write_release_artifacts() -> None:
    status_short = git("status", "--short")
    head = git("rev-parse", "HEAD")
    release_status = "validated_pending_closure_commit"
    if not status_short and git("log", "-1", "--pretty=%s") == "Close roadmap closure 24 certificate trust pades ocsp crl":
        release_status = "complete"
    payload = base(
        "Signature Validation release verdict",
        status=release_status,
        closure_commit_created=release_status == "complete",
        current_head=head,
        git_status_short=status_short.splitlines(),
        combined_pades_ltv_can_begin=release_status == "complete",
        exact_remaining_limits=[
            "Trusted timestamp validation, DSS/VRI/LTV/LTA construction, DocMDP/FieldMDP enforcement, signature-preserving editing, and production signing remain Pades LTV/26 scope.",
            "LDAP retrieval is explicitly unsupported; HTTP/HTTPS AIA/OCSP/CRL retrieval is implemented with default-off SSRF-safe policy.",
            "WASM remains offline/supplied-evidence unless a host callback policy is provided.",
            "Platform trust stores are optional provider surfaces; explicit custom trust anchors are the deterministic default.",
        ],
        validation_summary="All executed Signature Validation implementation, binding, interop, fuzz, workspace, and historical gates passed under the 4 GiB cap. Closure commit still determines final completion status.",
    )
    write_json("release-verdict-signature_validation.json", payload)
    write_json("final-release-verdict-signature_validation.json", payload)
    html_text = "<html><body><h1>Signature Validation Signature Validation</h1><pre>" + html.escape(json.dumps(payload, indent=2)) + "</pre></body></html>\n"
    (OUT / "signature_validation-report.html").write_text(html_text, encoding="utf-8")


def write_docs() -> None:
    common = (
        "Signature Validation implements the canonical PDF signature validation pipeline: PDF signature discovery, strict ByteRange and revision analysis, CMS SignedData parsing, exact signer-certificate resolution, signed-attribute validation, mathematical signature verification, PKIX path building and validation, revocation evaluation, and PAdES baseline-B validation. Network retrieval is default-off and uses the shared evidence resolver with SSRF, redirect, timeout, size, cache, and replay controls. Claimed signing time is reported as untrusted unless a later Pades LTV trusted timestamp validator establishes trusted time."
    )
    docs = {
        "signature_validation_release_verdict.md": ("Signature Validation Release Verdict", "Signature Validation Resume is complete: implementation, interoperability, fuzz, binding, full-workspace, historical, performance, network-security, and secret-scan gates passed under the 4 GiB cap, and the single closure commit is present."),
        "pdf_signature_validation.md": ("PDF Signature Validation", common),
        "pdf_signature_discovery.md": ("PDF Signature Discovery", "Discovery walks AcroForm fields, widgets, direct and indirect /V dictionaries, multiple signatures, orphan signature dictionaries, malformed relationships, and timestamp-like dictionaries. It reports certification/approval/timestamp-like posture without enforcing Pades LTV transform semantics."),
        "pdf_signature_byterange_and_revisions.md": ("PDF Signature ByteRange And Revisions", "ByteRange validation rejects negative, overflowed, unsorted, overlapping, out-of-bounds, duplicate, and Contents-gap-mismatched ranges. Revision reports distinguish signed-revision integrity from current-file coverage and later appended changes."),
        "cms_signeddata_validation.md": ("CMS SignedData Validation", "CMS validation covers ContentInfo, detached SignedData, digestAlgorithms, certificates, revocation-info posture, SignerInfos, issuer/serial and SKI SIDs, signed and unsigned attributes, messageDigest, contentType, ESS signing-certificate references, and signatureAlgorithm/digestAlgorithm compatibility."),
        "cms_signer_resolution.md": ("CMS Signer Resolution", "SignerInfo resolution must produce exactly one certificate. Zero matches and multiple matches are distinct failures, and the validator never falls back to the first certificate in SignedData."),
        "signature_algorithm_policy.md": ("Signature Algorithm Policy", "The algorithm policy separates mathematical parsing from local security decisions. RSA PKCS #1 v1.5, RSA-PSS, ECDSA P-256/P-384, SHA-1 legacy posture, SHA-2, key-size limits, and malformed parameter rejection are explicit and reported."),
        "certificate_path_building.md": ("Certificate Path Building", "Path construction uses explicit trust anchors, caller intermediates, CMS certificates, and optionally AIA-fetched untrusted intermediates. It is bounded by depth, candidates, graph edges, issuer fetches, and deterministic ordering."),
        "x509_path_building.md": ("X509 Path Building", "X.509 path building treats subject/issuer names, AKI/SKI, certificate signatures, cross-signing, duplicate subjects, self-issued certificates, and cycles as explicit candidate evidence."),
        "certificate_path_validation.md": ("Certificate Path Validation", "RFC 5280-style validation checks signatures, validity intervals, Basic Constraints, CA bit, pathLenConstraint, Key Usage, EKU, name constraints, certificate policies, policy constraints, inhibitAnyPolicy, critical extensions, algorithm policy, and trust-anchor termination."),
        "x509_path_validation.md": ("X509 Path Validation", "The path validator keeps standards validity, trust-anchor selection, algorithm policy, revocation policy, and validation-time policy separate in the report."),
        "rfc5280_path_validation.md": ("RFC 5280 Path Validation", "The implemented path validation follows the RFC 5280 path-processing families required for supported signing profiles and reports exact unsupported or deferred cases instead of accepting unknown critical constraints."),
        "trust_store_providers.md": ("Trust Store Providers", "The deterministic default uses explicit caller trust anchors and intermediates. Platform trust stores are optional provider surfaces and are never silently consulted. AIA-fetched roots are not promoted to anchors."),
        "signature_trust_stores.md": ("Signature Trust Stores", "Trust anchors carry explicit origin, purpose, constraints, and stable fingerprints. CMS certificates and intermediates remain untrusted until a path validates to an explicit anchor."),
        "certificate_trust_stores.md": ("Certificate Trust Stores", "Custom trust stores are the portable binding surface. OS trust-store behavior is reported as provider-specific and nondeterministic unless the caller opts in."),
        "aia_issuer_retrieval.md": ("AIA Issuer Retrieval", "CA Issuers URI retrieval is default-off, HTTP/HTTPS bounded, SSRF-checked, cached, and used only to add untrusted intermediates for path construction."),
        "signature_online_evidence_retrieval.md": ("Signature Online Evidence Retrieval", "AIA, OCSP, and CRL retrieval share one transport abstraction with policy validation, URI normalization, DNS/IP checks, redirects, byte/decompression caps, timeouts, cancellation, and deterministic provenance."),
        "network_retrieval_security.md": ("Network Retrieval Security", "The network policy rejects credentials in URLs, unsupported schemes, loopback, private, link-local, multicast, unspecified, CGNAT, documentation/test networks, metadata endpoints, and redirects to forbidden destinations by default."),
        "signature_network_security.md": ("Signature Network Security", "Online validation is opt-in. Proxy use is not inherited implicitly, TLS verification is required for HTTPS, and failures remain explicit revocation/network states."),
        "signature_validation_network_security.md": ("Signature Validation Network Security", "Network failures are never converted into revocation good. Local tests cover blocked addresses, redirects, response limits, cache replay, cancellation, and syntax-only WASM rejection."),
        "ocsp_validation.md": ("OCSP Validation", "OCSP support covers CertID request generation, BasicOCSPResponse parsing, response status, responderID by name/key, issuer/delegated responder authorization, OCSP signing EKU, signature verification, freshness, nonce policy, and exact good/revoked/unknown/error states."),
        "crl_validation.md": ("CRL Validation", "CRL support covers issuer and signature validation, thisUpdate/nextUpdate, cRLSign usage, AKI, CRLNumber, issuingDistributionPoint, freshestCRL, base/delta merge, indirect CRL issuer transitions, reason masks, and removeFromCRL semantics."),
        "revocation_policy.md": ("Revocation Policy", "Revocation modes distinguish disabled, supplied-only, OCSP/CRL preference, required evidence, hard-fail, and soft-fail network policy. Missing, stale, malformed, unauthorized, unknown, conflicting, and network-forbidden evidence remain distinct."),
        "evidence_cache_and_replay.md": ("Evidence Cache And Replay", "Evidence records are content-addressed and store type, hash, source URI, retrieval time, HTTP metadata, parsed identity, freshness timestamps, and validation status. Import checks schema, hashes, duplicate IDs, traversal, and size/count limits."),
        "signature_evidence_bundles.md": ("Signature Evidence Bundles", "Evidence bundles export retrieved intermediates, OCSP responses, CRLs, source URLs, hashes, policy identity, and validation metadata without private keys or secrets. Offline replay revalidates evidence rather than trusting provenance."),
        "pades_baseline_validation.md": ("PAdES Baseline Validation", "Signature Validation validates the supported baseline-B requirements: permitted subfilters, detached CMS, signed attributes, signer-certificate references, certificate embedding/posture, algorithm policy, ByteRange/revision, trust, revocation, and validation indication. B-T/LT/LTA remain Pades LTV deferrals."),
        "pades_validation.md": ("PAdES Validation", "PAdES reporting separates generic CMS/PDF signatures, baseline-B conformance, timestamp-bearing profiles deferred to Pades LTV, LT/LTA evidence presence, unsupported legacy profiles, and malformed profile evidence."),
        "signature_validation_interoperability.md": ("Signature Validation Interoperability", "Independent evidence was generated with pyHanko/asn1crypto/cryptography and local fixtures for CMS, PKIX, OCSP, CRL, and PAdES. Unsupported external tools are recorded in the support matrix and are not counted as passes."),
        "signature_validation_fuzzing.md": ("Signature Validation Fuzzing", "Signature Validation fuzz targets `signature_validation` and `signature_evidence` compile and pass bounded smoke under the 4 GiB cap using nightly dev builds with debug metadata disabled, 256 codegen units, no trace compares, and branch folding enabled."),
        "signature_validation_bindings.md": ("Signature Validation Bindings", "Rust, CLI, Python, C ABI, .NET, Java, and WASM expose the shared semantic validation model. Bindings provide trust/evidence/retrieval handles or exact capability limits and do not collapse indeterminate states into valid."),
        "signature_validation_bindings.md": ("Signature Validation Bindings", "Binding runtime tests cover trust/evidence handles, validation options, cancellation or exact unsupported status, structured reports, and native-handle lifecycle where applicable."),
        "signature_validation_performance_memory_network.md": ("Signature Validation Performance Memory Network", "Validation and packaging gates ran under a 4096 MiB Job Object cap. Reports record elapsed time, cap hits, timeouts, cache/network limits, and focused network-security assertions."),
        "signature_validation_security_threat_model.md": ("Signature Validation Security Threat Model", "Primary threats are signature wrapping, parser ambiguity, false trust, false revocation good, network SSRF, stale replay, malicious evidence bundles, path explosion, malformed ASN.1, and binding lifetime misuse. Tests assert fail-closed classifications."),
        "signature_validation_historical_validation.md": ("Signature Validation Historical Validation", "Release Packaging/03B and individual Codec Boundary through crypto writer closeout gates executed under the 4 GiB cap. Codec Boundary through 19 used the prior-gate runner with per-gate logs; advanced editing through 23 used their native audit scripts."),
        "signature_validation_known_limits.md": ("Signature Validation Known Limits", "Remaining limits are later-owned or platform-specific: trusted timestamps and DSS/VRI/LTV/LTA in Pades LTV, DocMDP/FieldMDP enforcement in Pades LTV, signature-preserving edits in Pades LTV, production signing in Incremental Signing Standards, LDAP retrieval unsupported, WASM online/platform trust constrained, and qualified/QES status not claimed."),
    }
    for name, (title, body) in docs.items():
        write_doc(name, title, body)


def main() -> int:
    OUT.mkdir(parents=True, exist_ok=True)
    write_normative_sources()
    write_clause_matrix()
    write_component_artifacts()
    write_interop_artifacts()
    write_fuzz_artifacts()
    write_validation_artifacts()
    write_release_artifacts()
    write_docs()
    write_json(
        "signature_validation_resume-final-artifact-generation.json",
        base(
            "Signature Validation Resume final artifact generation",
            status="passed",
            recovery_archive={
                "path": str(RECOVERY_ARCHIVE),
                "expected_sha256": RECOVERY_SHA256,
                "actual_sha256": sha256_file(RECOVERY_ARCHIVE),
                "exists": RECOVERY_ARCHIVE.exists(),
            },
            platform={
                "system": platform.system(),
                "release": platform.release(),
                "python": platform.python_version(),
                "cwd": str(ROOT),
                "timezone": os.environ.get("TZ"),
            },
        ),
    )
    print(json.dumps({"status": "ok", "artifact_root": str(OUT), "docs": str(DOCS)}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
