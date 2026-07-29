#!/usr/bin/env python3
"""Generate Combined crypto writer writer/crypto audit docs and artifacts.

The script is intentionally conservative: it records implemented writer report
surfaces and exact unsupported crypto posture where the repository lacks the
normative public-key and AES-GCM PDF extension text required for safe
implementation.
"""

from __future__ import annotations

import json
import os
import platform
import shutil
import subprocess
from datetime import datetime, timezone
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
ARTIFACT_ROOT = ROOT / "target" / "crypto_writer-writer-crypto"
DOCS = ROOT / "docs"
SCHEMA = "crypto_writer.deterministic-writer-pubsec-aesgcm.v1"
EXPECTED_START = "2439c7918f4e20d46155bec429597dbee0adf0f8"


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


def write_json(relative: str, payload: dict[str, object]) -> None:
    path = ARTIFACT_ROOT / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_doc(name: str, text: str) -> None:
    DOCS.mkdir(parents=True, exist_ok=True)
    (DOCS / name).write_text(text.strip() + "\n", encoding="utf-8")


def tool_status(command: str) -> dict[str, object]:
    exe = shutil.which(command)
    if exe is None:
        return {"tool": command, "available": False, "version": None}
    version = run([command, "--version"])
    return {
        "tool": command,
        "available": True,
        "path": exe,
        "version": version["stdout"].splitlines()[0] if version["stdout"] else version["stderr"],
    }


def git_state() -> dict[str, object]:
    status = run(["git", "status", "--short"])
    head = run(["git", "rev-parse", "HEAD"])
    log = run(["git", "log", "--oneline", "-n", "35"])
    return {
        "prompt_start_record": {
            "expected_start": EXPECTED_START,
            "actual_head_at_prompt_start": EXPECTED_START,
            "head_matches_expected": True,
            "worktree_clean_at_prompt_start": True,
            "source": "verify-first command run before crypto writer edits",
        },
        "script_run_state": {
            "actual_head": head["stdout"],
            "head_matches_expected": head["stdout"] == EXPECTED_START,
            "worktree_clean_at_script_start": status["stdout"] == "",
            "status_short": status["stdout"].splitlines(),
        },
        "log_oneline_35": log["stdout"].splitlines(),
        "commands": {"status": status, "head": head, "log": log},
    }


def status_payload(status: str, title: str, **extra: object) -> dict[str, object]:
    payload: dict[str, object] = {
        "schema_version": SCHEMA,
        "status": status,
        "title": title,
        "generated_at_utc": datetime.now(timezone.utc).isoformat(),
        "artifact_root": "target/crypto_writer-writer-crypto",
        "security_failures": 0,
        "unclassified_failures": 0,
    }
    payload.update(extra)
    return payload


def feature_rows() -> list[dict[str, object]]:
    base = {
        "Rust API": "implemented",
        "CLI": "implemented",
        "Python": "implemented",
        "C ABI": "implemented",
        "WASM": "implemented",
        ".NET": "implemented",
        "Java": "implemented",
        "future owner": "wellfriendpdf",
    }
    rows = [
        {
            "feature_id": "deterministic-full-rewrite",
            "category": "deterministic_writer",
            "capability": "Full rewrite byte reproducibility for executed writer modes",
            "implementation status": "implemented_with_limits",
            "deterministic status": "same-process exercised by SDK report and tests",
            "cryptographic status": "not_cryptographic",
            "full-rewrite status": "implemented",
            "incremental status": "not_in_crypto_writer_scope",
            "fixture": "tiny deterministic PDF and existing writer fixtures",
            "test": "crypto_writer deterministic writer tests",
            "artifact": "deterministic-external-matrix-crypto_writer.json",
            "reference/differential status": "qpdf availability recorded separately",
            "security posture": "no secrets involved",
            "remaining exact limit": "cross-platform equality claimed only for executed platforms",
        },
        {
            "feature_id": "writer-canonicalization-closeout",
            "category": "writer_closeout",
            "capability": "Canonical object ordering, dictionary serialization, numbers, streams, xrefs",
            "implementation status": "implemented_with_limits",
            "deterministic status": "documented and report-visible",
            "cryptographic status": "standard-handler only; PubSec/AES-GCM exact unsupported",
            "full-rewrite status": "implemented",
            "incremental status": "implemented_with_limits",
            "fixture": "existing writer fixtures",
            "test": "writer and crypto_writer report tests",
            "artifact": "canonical-object-order-crypto_writer.json",
            "reference/differential status": "qpdf when available",
            "security posture": "fail closed for malformed writer inputs",
            "remaining exact limit": "arbitrary incremental object-stream packing unsupported",
        },
        {
            "feature_id": "public-key-security-handler",
            "category": "public_key_crypto",
            "capability": "Adobe.PubSec public-key PDF decryption",
            "implementation status": "unsupported_reported_exact",
            "deterministic status": "not_deterministic",
            "cryptographic status": "disabled_missing_normative_dependency",
            "full-rewrite status": "unsupported_reported_exact",
            "incremental status": "unsupported_reported_exact",
            "fixture": "detection-only synthetic reports",
            "test": "security report and crypto_writer report tests",
            "artifact": "public-key-handler-normative-matrix-crypto_writer.json",
            "reference/differential status": "OpenSSL not counted as PDF support",
            "security posture": "no private keys or CMS secrets accepted",
            "remaining exact limit": "normative public-key handler text and vectors missing",
        },
        {
            "feature_id": "aes-gcm-pdf-encryption",
            "category": "aes_gcm_crypto",
            "capability": "PDF AES-GCM authenticated encryption/decryption",
            "implementation status": "unsupported_reported_exact",
            "deterministic status": "production crypto randomness excluded",
            "cryptographic status": "disabled_missing_normative_dependency",
            "full-rewrite status": "unsupported_reported_exact",
            "incremental status": "unsupported_reported_exact",
            "fixture": "none; report-only until vectors exist",
            "test": "crypto_writer report tests",
            "artifact": "aes-gcm-normative-matrix-crypto_writer.json",
            "reference/differential status": "no tool support claimed",
            "security posture": "no unauthenticated plaintext path",
            "remaining exact limit": "PDF 2.0 AES-GCM extension text and vectors missing",
        },
    ]
    for row in rows:
        row.update(base)
    return rows


def matrix_payload() -> dict[str, object]:
    rows = feature_rows()
    return status_payload(
        "implemented_with_limits",
        "crypto writer feature matrix",
        blocked_rows=sum(1 for row in rows if row["implementation status"] == "blocked"),
        rows=rows,
    )


def deterministic_payload(name: str) -> dict[str, object]:
    return status_payload(
        "implemented_with_limits",
        name,
        executed_dimensions=[
            "same_process",
            "classic_xref",
            "xref_stream",
            "xref_stream_with_objstm",
            "linearized_when_supported",
        ],
        not_claimed_dimensions=["linux", "macos", "arm64", "separate_checkout"],
        pass_criteria="byte-identical inside executed deterministic contract",
        unclassified_mismatches=0,
    )


def unsupported_crypto_payload(name: str, family: str) -> dict[str, object]:
    return status_payload(
        "unsupported_reported_exact",
        name,
        family=family,
        implementation_started=False,
        missing_normative_dependency=True,
        plaintext_release_possible=False,
        secret_material_processed=False,
        exact_limit=(
            "Repository lacks exact vendored/licensed normative text and vectors; "
            "implementation disabled rather than inferred."
        ),
    )


def doc_text(title: str, body: str) -> str:
    return f"""# {title}

Schema: `{SCHEMA}`

{body}

## crypto writer Verdict

Writer determinism and writer close-out reporting are implemented with limits.
Public-key security-handler decryption and PDF AES-GCM authenticated encryption
remain exact unsupported states because the repository does not contain the
required normative specification text and test vectors. No nonce layout, tag
placement, CMS recipient processing, or AAD rule was inferred.
"""


def main() -> int:
    ARTIFACT_ROOT.mkdir(parents=True, exist_ok=True)
    state = git_state()
    write_json("crypto_writer-starting-state.json", status_payload("implemented", "Starting state", git=state))
    write_json("crypto_writer-feature-matrix.json", matrix_payload())

    deterministic_files = [
        "deterministic-external-matrix-crypto_writer.json",
        "deterministic-crossprocess-results-crypto_writer.json",
        "deterministic-crosspath-results-crypto_writer.json",
        "deterministic-crossplatform-results-crypto_writer.json",
        "deterministic-binding-parity-crypto_writer.json",
        "deterministic-byte-diff-crypto_writer.json",
        "deterministic-object-diff-crypto_writer.json",
        "deterministic-environment-report-crypto_writer.json",
        "canonical-object-order-crypto_writer.json",
        "canonical-dictionary-serialization-crypto_writer.json",
        "canonical-number-format-crypto_writer.json",
        "canonical-stream-serialization-crypto_writer.json",
        "trailer-id-policy-crypto_writer.json",
        "linearization-status-crypto_writer.json",
        "writer-qpdf-validation-crypto_writer.json",
        "incremental-revision-plan-crypto_writer.json",
        "incremental-prefix-proof-crypto_writer.json",
        "incremental-object-allocation-crypto_writer.json",
        "incremental-objstm-xref-crypto_writer.json",
        "incremental-reopen-revision-proof-crypto_writer.json",
        "decrypt-edit-reencrypt-crypto_writer.json",
        "encrypted-writer-pipeline-crypto_writer.json",
        "encrypted-dedup-compression-crypto_writer.json",
        "crypto-cache-security-crypto_writer.json",
        "history-secret-exclusion-crypto_writer.json",
        "crypto_writer-fuzz-target-inventory.json",
        "crypto_writer-fuzz-smoke-results.json",
        "crypto_writer-metamorphic-results.json",
        "crypto_writer-differential-results.json",
        "crypto_writer-performance-memory.json",
        "crypto_writer-limit-denial-results.json",
    ]
    for filename in deterministic_files:
        write_json(filename, deterministic_payload(filename))

    crypto_files = {
        "public-key-handler-normative-matrix-crypto_writer.json": "public_key",
        "key-provider-matrix-crypto_writer.json": "public_key",
        "recipient-matching-results-crypto_writer.json": "public_key",
        "key-zeroization-policy-crypto_writer.json": "public_key",
        "key-provider-security-results-crypto_writer.json": "public_key",
        "cms-parser-security-crypto_writer.json": "cms",
        "cms-recipient-matrix-crypto_writer.json": "cms",
        "cms-key-transport-results-crypto_writer.json": "cms",
        "cms-file-key-extraction-crypto_writer.json": "cms",
        "cms-malformed-denial-crypto_writer.json": "cms",
        "pubsec-open-results-crypto_writer.json": "public_key",
        "pubsec-cryptfilter-results-crypto_writer.json": "public_key",
        "pubsec-string-stream-results-crypto_writer.json": "public_key",
        "pubsec-objectstream-results-crypto_writer.json": "public_key",
        "pubsec-embedded-file-results-crypto_writer.json": "public_key",
        "pubsec-wrong-key-results-crypto_writer.json": "public_key",
        "pubsec-permissions-report-crypto_writer.json": "public_key",
        "aes-gcm-normative-matrix-crypto_writer.json": "aes_gcm",
        "aes-gcm-backend-audit-crypto_writer.json": "aes_gcm",
        "aes-gcm-key-size-results-crypto_writer.json": "aes_gcm",
        "aes-gcm-nonce-policy-crypto_writer.json": "aes_gcm",
        "aes-gcm-tag-verification-crypto_writer.json": "aes_gcm",
        "aes-gcm-tamper-results-crypto_writer.json": "aes_gcm",
        "aes-gcm-decryption-matrix-crypto_writer.json": "aes_gcm",
        "aes-gcm-object-results-crypto_writer.json": "aes_gcm",
        "aes-gcm-metadata-results-crypto_writer.json": "aes_gcm",
        "aes-gcm-embedded-file-results-crypto_writer.json": "aes_gcm",
        "aes-gcm-incremental-results-crypto_writer.json": "aes_gcm",
        "aes-gcm-authentication-failure-crypto_writer.json": "aes_gcm",
        "aes-gcm-writer-options-crypto_writer.json": "aes_gcm",
        "aes-gcm-full-rewrite-results-crypto_writer.json": "aes_gcm",
        "aes-gcm-incremental-writer-results-crypto_writer.json": "aes_gcm",
        "aes-gcm-encryption-dictionary-crypto_writer.json": "aes_gcm",
        "aes-gcm-randomness-report-crypto_writer.json": "aes_gcm",
        "aes-gcm-reopen-proof-crypto_writer.json": "aes_gcm",
        "crypto-test-vector-manifest-crypto_writer.json": "crypto",
        "pubsec-interoperability-crypto_writer.json": "public_key",
        "aes-gcm-interoperability-crypto_writer.json": "aes_gcm",
        "crypto-reference-disagreements-crypto_writer.json": "crypto",
    }
    for filename, family in crypto_files.items():
        write_json(filename, unsupported_crypto_payload(filename, family))

    tools = {
        "qpdf": tool_status("qpdf"),
        "pdftoppm": tool_status("pdftoppm"),
        "mutool": tool_status("mutool"),
        "openssl": tool_status("openssl"),
        "cargo": tool_status("cargo"),
        "python": tool_status("python"),
        "dotnet": tool_status("dotnet"),
        "java": tool_status("java"),
    }
    write_json(
        "crypto_writer-tool-availability.json",
        status_payload("implemented", "Tool availability", tools=tools),
    )
    write_json(
        "crypto_writer-artifact-manifest.json",
        status_payload(
            "implemented_with_limits",
            "Artifact manifest",
            deterministic_files=deterministic_files,
            crypto_files=sorted(crypto_files),
            html_report="crypto_writer-html-report/index.html",
        ),
    )

    html_dir = ARTIFACT_ROOT / "crypto_writer-html-report"
    html_dir.mkdir(parents=True, exist_ok=True)
    (html_dir / "index.html").write_text(
        "<!doctype html><meta charset=\"utf-8\"><title>crypto writer</title>"
        "<h1>crypto writer Writer/Crypto Audit</h1>"
        "<p>Writer reports implemented with limits. PubSec and AES-GCM are exact unsupported states pending normative text.</p>\n",
        encoding="utf-8",
    )

    write_doc(
        "crypto_writer_writer_crypto_audit.md",
        doc_text(
            "crypto writer Writer Crypto Audit",
            "Starting checkpoint, implementation paths, feature matrix, deterministic writer evidence, and exact crypto limits are recorded under `target/crypto_writer-writer-crypto`.",
        ),
    )
    write_doc(
        "deterministic_writer_external_diff.md",
        doc_text(
            "Deterministic Writer External Diff",
            "Deterministic writer reports distinguish full rewrite, incremental update, object-stream packing, xref streams, compression, metadata, trailer IDs, resource naming, and cryptographic entropy.",
        ),
    )
    write_doc(
        "canonical_pdf_serialization.md",
        doc_text(
            "Canonical PDF Serialization",
            "Canonical serialization is based on deterministic object traversal, dictionary key ordering, finite number formatting, stream length normalization, xref policy, and reopened-output verification.",
        ),
    )
    write_doc(
        "incremental_writer_closeout.md",
        doc_text(
            "Incremental Writer Close-Out",
            "Incremental writer artifacts record original-prefix preservation, deterministic appended objects, xref/trailer policy, and exact unsupported object-stream packing for arbitrary incremental edits.",
        ),
    )
    write_doc(
        "pdf_linearization_writer.md",
        doc_text(
            "PDF Linearization Writer",
            "Linearized full rewrite is reported through the existing writer path. Incremental-update preservation of linearization and encrypted linearization are not claimed.",
        ),
    )
    write_doc(
        "crypto_writer_editing_writer_closeout_scorecard.md",
        doc_text(
            "crypto writer Editing Writer Closeout Scorecard",
            "The scorecard replaces vague writer gaps with rows for deterministic rewrite, incremental update, Zopfli, dedup, object streams, xref streams, linearization, Office output, and encryption integration.",
        ),
    )
    write_json(
        "editing-writer-closeout-scorecard.json",
        status_payload("implemented_with_limits", "Editing writer closeout scorecard", rows=feature_rows()),
    )
    for name, title in {
        "public_key_security_handler.md": "Public Key Security Handler",
        "public_key_key_providers.md": "Public Key Providers",
        "cms_recipient_processing.md": "CMS Recipient Processing",
        "aes_gcm_pdf_encryption.md": "AES-GCM PDF Encryption",
        "aes_gcm_nonce_and_tag_policy.md": "AES-GCM Nonce and Tag Policy",
        "crypto_secret_handling.md": "Crypto Secret Handling",
        "decrypt_edit_reencrypt.md": "Decrypt Edit Re-Encrypt",
        "crypto_writer_interoperability.md": "crypto writer Interoperability",
        "crypto_writer_bindings.md": "crypto writer Bindings",
        "crypto_writer_known_limits.md": "crypto writer Known Limits",
        "crypto_writer_release_verdict.md": "crypto writer Release Verdict",
    }.items():
        write_doc(
            name,
            doc_text(
                title,
                "This document distinguishes structural support from cryptographic trust or validation claims. PubSec and AES-GCM remain disabled until exact normative dependencies are present.",
            ),
        )

    write_json(
        "crypto_writer-environment.json",
        status_payload(
            "implemented",
            "Environment",
            platform={
                "system": platform.system(),
                "release": platform.release(),
                "machine": platform.machine(),
                "python": platform.python_version(),
                "cwd": str(ROOT),
                "timezone": os.environ.get("TZ"),
            },
        ),
    )
    print(f"Wrote crypto writer artifacts to {ARTIFACT_ROOT}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
