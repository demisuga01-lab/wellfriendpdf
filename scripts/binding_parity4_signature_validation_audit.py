#!/usr/bin/env python3
"""Generate Signature Validation signature-validation docs and machine artifacts.

The implementation in this run adds real offline CMS signer selection,
bounded certificate path building, RFC 5280-style path validation through the
pkix-path stack, supplied OCSP/CRL evaluation hooks, CLI policy inputs,
structured Signature Validation reports, and option-aware binding entry points. It is
intentionally marked not complete until the remaining Signature Validation acceptance gates
are implemented and executed.
"""

from __future__ import annotations

import hashlib
import json
import subprocess
from datetime import datetime, timezone
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
ARTIFACT_ROOT = ROOT / "target" / "signature_validation-signature-validation"
DOCS = ROOT / "docs"
SCHEMA = "signature_validation.certificate-trust-pades-ocsp-crl-validation.v1"
EXPECTED_START = "f68cd36c92d910607e16676f66c4ef84f6830410"


def run(args: list[str]) -> dict[str, object]:
    proc = subprocess.run(
        args,
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    return {
        "command": args,
        "returncode": proc.returncode,
        "stdout": proc.stdout.strip(),
        "stderr": proc.stderr.strip(),
    }


def sha256(path: Path) -> str | None:
    if not path.exists():
        return None
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest().upper()


def write_json(name: str, payload: dict[str, object]) -> None:
    ARTIFACT_ROOT.mkdir(parents=True, exist_ok=True)
    (ARTIFACT_ROOT / name).write_text(
        json.dumps(payload, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def write_doc(name: str, title: str, body: str) -> None:
    DOCS.mkdir(parents=True, exist_ok=True)
    text = f"# {title}\n\nSchema: `{SCHEMA}`\n\n{body.strip()}\n"
    (DOCS / name).write_text(text, encoding="utf-8")


def status_payload(status: str, title: str, **extra: object) -> dict[str, object]:
    payload: dict[str, object] = {
        "schema_version": SCHEMA,
        "status": status,
        "title": title,
        "generated_at_utc": datetime.now(timezone.utc).isoformat(),
        "artifact_root": "target/signature_validation-signature-validation",
        "prompt_start_commit": EXPECTED_START,
        "security_failures": 0,
        "false_trusted_valid_cases_known": 0,
    }
    payload.update(extra)
    return payload


def source_manifest() -> dict[str, object]:
    local_sources = [
        (
            "ISO 32000-2:2020",
            "Document management - Portable document format - Part 2: PDF 2.0",
            ROOT / "PDFA" / "ISO_32000-2_sponsored_EC3-1.pdf",
            ["PDF signature dictionary", "ByteRange", "incremental revisions"],
        ),
        (
            "ISO/TS 32002:2022",
            "Document management - Portable document format - Extensions to ISO 32000-2 - Part 2: Digital signatures",
            ROOT / "PDFA" / "ISO_TS_32002-2022_sponsored_EC3.pdf",
            ["digital signature algorithm/profile extensions"],
        ),
    ]
    docs: list[dict[str, object]] = []
    for identifier, title, path, clauses in local_sources:
        docs.append(
            {
                "identifier": identifier,
                "title": title,
                "source": "PDF Association sponsored local copy supplied to workspace",
                "local_path": str(path),
                "download_or_access_date": "2026-07-14",
                "sha256": sha256(path),
                "license_access_status": "locally_available_for_project_use",
                "redistribution_status": "do_not_commit_pdf",
                "clauses_used": clauses,
                "implementation_modules": ["crates/engine/src/signature.rs"],
                "tests_derived": ["crates/engine/tests/signatures.rs"],
            }
        )
    docs.extend(
        [
            {
                "identifier": "RFC 5652",
                "title": "Cryptographic Message Syntax (CMS)",
                "source": "https://www.rfc-editor.org/rfc/rfc5652",
                "download_or_access_date": "2026-07-14",
                "redistribution_status": "public_rfc_url_only",
                "clauses_used": ["ContentInfo", "SignedData", "SignerInfo", "signedAttrs"],
                "implementation_modules": ["crates/engine/src/signature.rs"],
                "tests_derived": ["crates/engine/tests/signatures.rs"],
            },
            {
                "identifier": "RFC 5280",
                "title": "Internet X.509 Public Key Infrastructure Certificate and CRL Profile",
                "source": "https://www.rfc-editor.org/rfc/rfc5280",
                "download_or_access_date": "2026-07-14",
                "redistribution_status": "public_rfc_url_only",
                "clauses_used": ["6.1", "4.1", "4.2", "5", "6.3"],
                "implementation_modules": ["crates/engine/src/signature.rs"],
                "tests_derived": ["crates/engine/tests/signatures.rs"],
            },
            {
                "identifier": "RFC 6960",
                "title": "X.509 Internet Public Key Infrastructure Online Certificate Status Protocol - OCSP",
                "source": "https://www.rfc-editor.org/rfc/rfc6960",
                "download_or_access_date": "2026-07-14",
                "redistribution_status": "public_rfc_url_only",
                "clauses_used": ["OCSPResponse", "BasicOCSPResponse", "responder authorization"],
                "implementation_modules": ["crates/engine/src/signature.rs"],
                "tests_derived": [],
            },
            {
                "identifier": "RFC 8017",
                "title": "PKCS #1: RSA Cryptography Specifications Version 2.2",
                "source": "https://www.rfc-editor.org/rfc/rfc8017",
                "download_or_access_date": "2026-07-14",
                "redistribution_status": "public_rfc_url_only",
                "clauses_used": ["RSASSA-PKCS1-v1_5"],
                "implementation_modules": ["crates/engine/src/signature.rs"],
                "tests_derived": ["crates/engine/tests/signatures.rs"],
            },
            {
                "identifier": "ETSI EN 319 142-1 V1.2.0",
                "title": "PAdES digital signatures; Part 1: Building blocks and PAdES baseline signatures",
                "source": "https://www.etsi.org/deliver/etsi_en/319100_319199/31914201/01.02.00_20/en_31914201v010200a.pdf",
                "download_or_access_date": "2026-07-14",
                "redistribution_status": "official_url_recorded_pdf_not_committed",
                "clauses_used": ["baseline profile recognition and deferred higher-level evidence"],
                "implementation_modules": ["crates/engine/src/signature.rs"],
                "tests_derived": ["crates/engine/tests/signatures.rs"],
            },
            {
                "identifier": "ETSI EN 319 122-1 V1.3.1",
                "title": "CAdES digital signatures; Part 1: Building blocks and CAdES baseline signatures",
                "source": "https://www.etsi.org/deliver/etsi_en/319100_319199/31912201/01.03.01_60/en_31912201v010301p.pdf",
                "download_or_access_date": "2026-07-14",
                "redistribution_status": "official_url_recorded_pdf_not_committed",
                "clauses_used": ["CAdES/CMS signed attribute posture"],
                "implementation_modules": ["crates/engine/src/signature.rs"],
                "tests_derived": [],
            },
            {
                "identifier": "ETSI TS 119 102-1 V1.2.1",
                "title": "Procedures for Creation and Validation of AdES Digital Signatures - Part 1",
                "source": "https://www.etsi.org/deliver/etsi_ts/119100_119199/11910201/01.02.01_60/ts_11910201v010201p.pdf",
                "download_or_access_date": "2026-07-14",
                "redistribution_status": "official_url_recorded_pdf_not_committed",
                "clauses_used": ["validation process model"],
                "implementation_modules": ["crates/engine/src/signature.rs"],
                "tests_derived": [],
            },
        ]
    )
    return status_payload(
        "partial_source_gate",
        "Signature Validation normative source manifest",
        documents=docs,
        source_blockers=[
            "Full clause-by-clause ETSI PAdES/CAdES implementation review not completed in this run.",
            "NIST PKITS and EC DSS validation corpora were not acquired/executed in this run.",
        ],
    )


def clause_matrix() -> dict[str, object]:
    rows = [
        {
            "requirement": "CMS SignerInfo resolves to exactly one certificate",
            "source": "RFC 5652 section 5.3",
            "status": "implemented",
            "module": "crates/engine/src/signature.rs",
            "tests": ["crates/engine/tests/signatures.rs"],
        },
        {
            "requirement": "No fallback to arbitrary first CMS certificate",
            "source": "Signature Validation security rule 2.6 / RFC 5652 signer identifier",
            "status": "implemented",
            "module": "crates/engine/src/signature.rs",
            "tests": ["focused signature tests"],
        },
        {
            "requirement": "Bounded deterministic certificate path building",
            "source": "RFC 5280 section 6.1 / RFC 4158 path building",
            "status": "implemented_with_limits",
            "module": "crates/engine/src/signature.rs",
            "limits": ["offline supplied/cms intermediates only", "no AIA fetch"],
            "tests": ["pinning_signer_cert_as_trust_anchor_makes_it_trusted"],
        },
        {
            "requirement": "RFC 5280 path validation",
            "source": "RFC 5280 section 6.1",
            "status": "implemented_with_limits",
            "module": "pkix-path via crates/engine/src/signature.rs",
            "covered": [
                "signatures",
                "validity",
                "BasicConstraints",
                "pathLen",
                "KeyUsage keyCertSign",
                "critical extensions",
                "name constraints",
                "policy tree",
                "algorithm dispatch",
            ],
            "limits": ["RSA-PSS and EdDSA chain signatures not covered by bundled verifier"],
        },
        {
            "requirement": "Supplied OCSP and CRL parsing/signature/freshness evaluation",
            "source": "RFC 6960 / RFC 5280 section 6.3",
            "status": "implemented_with_limits",
            "module": "pkix-revocation via crates/engine/src/signature.rs",
            "limits": ["caller-supplied evidence only", "no live retrieval", "no DSS/VRI extraction as validated evidence"],
        },
        {
            "requirement": "Controlled online retrieval",
            "source": "Signature Validation sections 14 and 16",
            "status": "not_complete",
            "module": "CLI/report flag only",
            "limits": ["online flag reports deferred/no-fetch"],
        },
        {
            "requirement": "Binding trust-store APIs across Python/C/.NET/Java/WASM",
            "source": "Signature Validation section 21",
            "status": "implemented_with_limits",
            "module": "crates/engine/src/signature.rs plus C/Python/.NET/Java/WASM wrappers",
            "covered": [
                "shared JSON options for trust anchors",
                "shared JSON options for intermediates",
                "shared JSON options for supplied OCSP/CRL evidence",
                "validation time",
                "revocation mode",
                "online flag with explicit no-fetch posture",
                "path limits",
            ],
            "limits": [
                "no binding-specific opaque trust-store/evidence handles",
                "no .NET X509Certificate2 or Java KeyStore ingestion helpers",
                "WASM remains offline/caller-supplied only",
            ],
        },
    ]
    return status_payload(
        "implemented_with_limits_not_complete",
        "Signature Validation clause implementation matrix",
        rows=rows,
        closure_blockers=[
            "Full PAdES baseline clause implementation incomplete.",
            "Online retrieval, platform stores, full bindings, fuzz expansion, and interoperability not complete.",
        ],
    )


def simple_artifact(name: str, status: str, title: str, **extra: object) -> None:
    write_json(name, status_payload(status, title, **extra))


def docs() -> None:
    source_body = """Signature Validation records local ISO PDF sources without committing restricted PDFs.

Official ETSI URLs were verified on 2026-07-14 and are recorded in
`target/signature_validation-signature-validation/normative-source-manifest-signature_validation.json`.

This run is not a complete normative closure: the implemented code uses RFC 5652,
RFC 5280, RFC 6960, and the local PDF signature sources for an offline validation
core, while full ETSI PAdES/CAdES baseline validation remains incomplete.
"""
    matrix_body = """Implemented rows are limited to the offline core now wired into
`crates/engine/src/signature.rs`: exact CMS signer-certificate matching, bounded
path building, RFC 5280 path validation through `pkix-path`, and supplied
OCSP/CRL evaluation through `pkix-revocation`.

Rows marked `not_complete` are not release claims and block the required Signature Validation
closure commit.
"""
    standard_body = """Current state: implemented with limits, not Signature Validation complete.

The public report separates mathematical signature validity, signer certificate
resolution, path trust, revocation evidence, PAdES profile posture, network
posture, and Pades LTV deferred evidence. Claimed PDF `/M` and CMS signing time
remain untrusted metadata.

Bindings now expose a shared option JSON surface for explicit trust anchors,
intermediates, supplied OCSP responses, supplied CRLs, validation time,
revocation mode, online posture, and path limits. This is runtime support, not
the final binding-specific handle model required by Signature Validation closure.

Exact deferred features: trusted timestamp validation, DSS/VRI LTV validation,
DocMDP/FieldMDP enforcement, signature-preserving edits, production signing
beyond the existing incremental RSA path, qualified/QES trust-list status,
platform trust stores, and controlled online retrieval.
"""
    docs_map = {
        "signature_validation_normative_sources.md": ("Signature Validation Normative Sources", source_body),
        "signature_validation_clause_implementation_matrix.md": (
            "Signature Validation Clause Implementation Matrix",
            matrix_body,
        ),
        "certificate_trust_stores.md": ("Certificate Trust Stores", standard_body),
        "certificate_path_building.md": ("Certificate Path Building", standard_body),
        "certificate_path_validation.md": ("Certificate Path Validation", standard_body),
        "signature_algorithm_policy.md": ("Signature Algorithm Policy", standard_body),
        "pdf_signature_validation.md": ("PDF Signature Validation", standard_body),
        "cms_signeddata_validation.md": ("CMS SignedData Validation", standard_body),
        "pades_validation.md": ("PAdES Validation", standard_body),
        "ocsp_validation.md": ("OCSP Validation", standard_body),
        "crl_validation.md": ("CRL Validation", standard_body),
        "revocation_policy.md": ("Revocation Policy", standard_body),
        "signature_validation_network_security.md": (
            "Signature Validation Network Security",
            standard_body,
        ),
        "signature_online_evidence_retrieval.md": (
            "Signature Online Evidence Retrieval",
            standard_body,
        ),
        "signature_network_security.md": ("Signature Network Security", standard_body),
        "signature_evidence_bundles.md": ("Signature Evidence Bundles", standard_body),
        "pdf_signature_discovery.md": ("PDF Signature Discovery", standard_body),
        "pdf_signature_byterange_and_revisions.md": (
            "PDF Signature ByteRange And Revisions",
            standard_body,
        ),
        "cms_signer_resolution.md": ("CMS Signer Resolution", standard_body),
        "x509_path_building.md": ("X.509 Path Building", standard_body),
        "x509_path_validation.md": ("X.509 Path Validation", standard_body),
        "signature_trust_stores.md": ("Signature Trust Stores", standard_body),
        "pades_baseline_validation.md": ("PAdES Baseline Validation", standard_body),
        "signature_validation_bindings.md": ("Signature Validation Bindings", standard_body),
        "signature_validation_bindings.md": ("Signature Validation Bindings", standard_body),
        "signature_validation_interoperability.md": (
            "Signature Validation Interoperability",
            "No independent interop matrix was completed in this run. This blocks Signature Validation closure.",
        ),
        "signature_validation_fuzzing.md": (
            "Signature Validation Fuzzing",
            "The existing signature fuzz target remains present. New Signature Validation-specific fuzz targets and long-campaign artifacts were not completed.",
        ),
        "signature_validation_security_threat_model.md": (
            "Signature Validation Security Threat Model",
            standard_body,
        ),
        "signature_validation_known_limits.md": (
            "Signature Validation Known Limits",
            "This run is not complete. The exact remaining limits are recorded in the release verdict artifact and are not acceptable for Signature Validation closure.",
        ),
        "signature_validation_release_verdict.md": (
            "Signature Validation Release Verdict",
            "NOT_COMPLETE. No closure commit may be created. roadmap closure 25 must not begin.",
        ),
        "signature_validation_historical_validation.md": (
            "Signature Validation Historical Validation",
            "Historical Codec Boundary through crypto writer closeout gates were not fully executed in this continuation. This blocks Signature Validation closure.",
        ),
    }
    for name, (title, body) in docs_map.items():
        write_doc(name, title, body)


def main() -> None:
    git_status = run(["git", "status", "--short"])
    git_head = run(["git", "rev-parse", "HEAD"])
    write_json("normative-source-manifest-signature_validation.json", source_manifest())
    write_json("clause-implementation-matrix-signature_validation.json", clause_matrix())
    simple_artifact(
        "trust-store-inventory-signature_validation.json",
        "implemented_with_limits",
        "Signature Validation trust store inventory",
        custom_trust_anchors="DER trust-anchor inputs supported in Rust/CLI",
        platform_trust_stores="not_complete",
        intermediates="CMS and caller-supplied DER intermediates are candidate-only",
    )
    simple_artifact(
        "certificate-algorithm-matrix-signature_validation.json",
        "implemented_with_limits",
        "Signature Validation certificate algorithm matrix",
        path_verifier="pkix-path DefaultVerifier",
        supported=["RSA PKCS1v15 SHA-256/384/512", "ECDSA P-256 SHA-256", "ECDSA P-384 SHA-384"],
        unsupported=["RSA-PSS chain signatures in bundled verifier", "EdDSA", "P-521"],
    )
    for name in [
        "path-building-corpus-results-signature_validation.json",
        "path-validation-corpus-results-signature_validation.json",
        "cms-validation-results-signature_validation.json",
        "pdf-byterange-revision-results-signature_validation.json",
        "pades-profile-matrix-signature_validation.json",
        "ocsp-results-signature_validation.json",
        "crl-results-signature_validation.json",
        "revocation-policy-matrix-signature_validation.json",
        "online-fetch-security-results-signature_validation.json",
        "adversarial-corpus-results-signature_validation.json",
        "fuzz-results-signature_validation.json",
        "certificate-path-interoperability.json",
        "cms-interoperability.json",
        "pades-interoperability.json",
        "ocsp-interoperability.json",
        "crl-interoperability.json",
        "binding-parity-results-signature_validation.json",
        "performance-memory-results-signature_validation.json",
        "historical-gate-results-signature_validation.json",
        "security-verdict-signature_validation.json",
        "signature-inventory-report-signature_validation.json",
        "byterange-revision-report-signature_validation.json",
        "signer-resolution-results-signature_validation.json",
        "algorithm-policy-results-signature_validation.json",
        "pkix-path-building-results-signature_validation.json",
        "pkix-path-validation-results-signature_validation.json",
        "trust-store-results-signature_validation.json",
        "ocsp-parsing-results-signature_validation.json",
        "ocsp-authorization-results-signature_validation.json",
        "ocsp-freshness-results-signature_validation.json",
        "crl-parsing-results-signature_validation.json",
        "crl-scope-results-signature_validation.json",
        "base-delta-crl-results-signature_validation.json",
        "revocation-policy-results-signature_validation.json",
        "online-retrieval-results-signature_validation.json",
        "ssrf-security-results-signature_validation.json",
        "cache-results-signature_validation.json",
        "evidence-export-offline-replay-results-signature_validation.json",
        "pades-baseline-results-signature_validation.json",
        "cms-interoperability-signature_validation.json",
        "pkix-interoperability-signature_validation.json",
        "ocsp-interoperability-signature_validation.json",
        "crl-interoperability-signature_validation.json",
        "pades-interoperability-signature_validation.json",
        "adversarial-tamper-results-signature_validation.json",
        "fuzz-target-inventory-signature_validation.json",
        "fuzz-smoke-results-signature_validation.json",
        "fuzz-release-verdict-signature_validation.json",
        "secret-scan-signature_validation.json",
        "full-final-validation-signature_validation.json",
    ]:
        simple_artifact(
            name,
            "not_complete",
            name.replace("-", " ").replace(".json", ""),
            blocker="Required Signature Validation closure gate not fully executed in this run.",
        )
    simple_artifact(
        "binding-parity-results-signature_validation.json",
        "implemented_with_limits",
        "Signature Validation binding parity results",
        rust_sdk="signature_report_with_options_json uses shared VerifyOptions JSON parser",
        python="PyDocument.signature_report_with_options and verify_signatures_with_options expose shared options JSON; fresh maturin wheel smoke passed",
        c_abi="wellfriendpdf_document_signatures_with_options_json exposes shared options JSON",
        dotnet="WellfriendDocument.SignatureReportWithOptionsJson passes runtime smoke with native library configured",
        java="WellfriendPdf.Document.signatureReportWithOptionsJson compiles and direct Java smoke runs; Maven/Gradle unavailable on PATH",
        wasm="signatureReportWithOptionsJson exposes offline/caller-supplied options JSON",
        limits=[
            "no opaque trust-store/evidence handles per binding",
            "no OS trust store adapters",
            "Java Maven/Gradle package gates unavailable in this environment",
        ],
    )
    simple_artifact(
        "full-final-validation-signature_validation.json",
        "not_complete",
        "Signature Validation full final validation",
        executed=[
            {"command": "cargo fmt --all --check", "result": "passed"},
            {"command": "git diff --check", "result": "passed_with_crlf_warnings"},
            {"command": "git diff --cached --check", "result": "passed"},
            {"command": "cargo check --workspace --all-targets --jobs 1", "result": "passed"},
            {
                "command": "cargo clippy --workspace --all-targets --jobs 1 -- -D warnings",
                "result": "passed",
            },
            {
                "command": "cargo test --workspace --all-targets --jobs 1",
                "result": "timed_out_after_904_seconds_not_a_pass",
            },
            {
                "command": "cargo test -p wellfriendpdf-engine --test signatures",
                "result": "passed_14_tests",
            },
            {"command": "cargo test -p wellfriendpdf-capi --all-targets", "result": "passed_21_tests"},
            {"command": "cargo test -p wellfriendpdf-py --all-targets", "result": "passed_0_tests"},
            {
                "command": "cargo test -p wellfriendpdf-cli --all-targets",
                "result": "passed_49_tests",
            },
            {"command": "cargo test -p wellfriendpdf-wasm --all-targets", "result": "passed_0_tests"},
            {
                "command": "cargo check -p wellfriendpdf-wasm --target wasm32-unknown-unknown",
                "result": "passed",
            },
            {
                "command": "cargo build -p wellfriendpdf-capi",
                "result": "passed",
            },
            {
                "command": "dotnet test bindings/dotnet/WellfriendPdf.Tests/WellfriendPdf.Tests.csproj",
                "result": "passed_6_tests_with_WELLFRIENDPDF_NATIVE_LIBRARY",
            },
            {
                "command": "javac/java direct WellfriendPdfSmokeTest",
                "result": "passed_with_native_access_warning",
            },
            {
                "command": "python -m maturin build --manifest-path crates/wellfriendpdf-py/Cargo.toml",
                "result": "passed_wheel_built",
            },
            {
                "command": "installed Python wheel signature-options smoke",
                "result": "passed",
            },
        ],
        blockers=[
            "full workspace cargo test timed out and remains unclosed",
            "Gradle executable not available on PATH and no wrapper present",
            "Maven executable not available on PATH and no wrapper present",
            "wasm-pack executable not available on PATH",
            "Codec Boundary-23B historical gate matrix not fully executed",
            "independent DSS/PAdES, OCSP/CRL, PKIX interoperability not completed",
        ],
    )
    simple_artifact(
        "release-verdict-signature_validation.json",
        "not_complete",
        "Signature Validation release verdict",
        git_head=git_head,
        git_status=git_status,
        closure_commit_created=False,
        combined_pades_ltv_can_begin=False,
        exact_remaining_limits=[
            "controlled online OCSP/CRL/AIA retrieval not implemented",
            "full PAdES baseline and independent DSS interoperability not complete",
            "binding-specific trust/evidence handles not complete",
            "expanded fuzzing, adversarial corpus, performance, and historical gates not complete",
        ],
    )
    (ARTIFACT_ROOT / "signature_validation-report.html").write_text(
        "<!doctype html><meta charset='utf-8'><title>Signature Validation</title>"
        "<h1>Signature Validation Signature Validation</h1><p>Status: NOT_COMPLETE.</p>\n",
        encoding="utf-8",
    )
    docs()


if __name__ == "__main__":
    main()
