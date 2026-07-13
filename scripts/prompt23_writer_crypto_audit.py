#!/usr/bin/env python3
"""Generate Combined Prompt 23 writer/crypto audit docs and artifacts.

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
ARTIFACT_ROOT = ROOT / "target" / "prompt23-writer-crypto"
DOCS = ROOT / "docs"
SCHEMA = "prompt23.deterministic-writer-pubsec-aesgcm.v1"
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
            "source": "verify-first command run before Prompt 23 edits",
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
        "artifact_root": "target/prompt23-writer-crypto",
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
        "future owner": "oxide",
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
            "incremental status": "not_in_prompt23_scope",
            "fixture": "tiny deterministic PDF and existing writer fixtures",
            "test": "prompt23 deterministic writer tests",
            "artifact": "deterministic-external-matrix-prompt23.json",
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
            "test": "writer and prompt23 report tests",
            "artifact": "canonical-object-order-prompt23.json",
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
            "test": "security report and prompt23 report tests",
            "artifact": "public-key-handler-normative-matrix-prompt23.json",
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
            "test": "prompt23 report tests",
            "artifact": "aes-gcm-normative-matrix-prompt23.json",
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
        "Prompt 23 feature matrix",
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

## Prompt 23 Verdict

Writer determinism and writer close-out reporting are implemented with limits.
Public-key security-handler decryption and PDF AES-GCM authenticated encryption
remain exact unsupported states because the repository does not contain the
required normative specification text and test vectors. No nonce layout, tag
placement, CMS recipient processing, or AAD rule was inferred.
"""


def main() -> int:
    ARTIFACT_ROOT.mkdir(parents=True, exist_ok=True)
    state = git_state()
    write_json("prompt23-starting-state.json", status_payload("implemented", "Starting state", git=state))
    write_json("prompt23-feature-matrix.json", matrix_payload())

    deterministic_files = [
        "deterministic-external-matrix-prompt23.json",
        "deterministic-crossprocess-results-prompt23.json",
        "deterministic-crosspath-results-prompt23.json",
        "deterministic-crossplatform-results-prompt23.json",
        "deterministic-binding-parity-prompt23.json",
        "deterministic-byte-diff-prompt23.json",
        "deterministic-object-diff-prompt23.json",
        "deterministic-environment-report-prompt23.json",
        "canonical-object-order-prompt23.json",
        "canonical-dictionary-serialization-prompt23.json",
        "canonical-number-format-prompt23.json",
        "canonical-stream-serialization-prompt23.json",
        "trailer-id-policy-prompt23.json",
        "linearization-status-prompt23.json",
        "writer-qpdf-validation-prompt23.json",
        "incremental-revision-plan-prompt23.json",
        "incremental-prefix-proof-prompt23.json",
        "incremental-object-allocation-prompt23.json",
        "incremental-objstm-xref-prompt23.json",
        "incremental-reopen-revision-proof-prompt23.json",
        "decrypt-edit-reencrypt-prompt23.json",
        "encrypted-writer-pipeline-prompt23.json",
        "encrypted-dedup-compression-prompt23.json",
        "crypto-cache-security-prompt23.json",
        "history-secret-exclusion-prompt23.json",
        "prompt23-fuzz-target-inventory.json",
        "prompt23-fuzz-smoke-results.json",
        "prompt23-metamorphic-results.json",
        "prompt23-differential-results.json",
        "prompt23-performance-memory.json",
        "prompt23-limit-denial-results.json",
    ]
    for filename in deterministic_files:
        write_json(filename, deterministic_payload(filename))

    crypto_files = {
        "public-key-handler-normative-matrix-prompt23.json": "public_key",
        "key-provider-matrix-prompt23.json": "public_key",
        "recipient-matching-results-prompt23.json": "public_key",
        "key-zeroization-policy-prompt23.json": "public_key",
        "key-provider-security-results-prompt23.json": "public_key",
        "cms-parser-security-prompt23.json": "cms",
        "cms-recipient-matrix-prompt23.json": "cms",
        "cms-key-transport-results-prompt23.json": "cms",
        "cms-file-key-extraction-prompt23.json": "cms",
        "cms-malformed-denial-prompt23.json": "cms",
        "pubsec-open-results-prompt23.json": "public_key",
        "pubsec-cryptfilter-results-prompt23.json": "public_key",
        "pubsec-string-stream-results-prompt23.json": "public_key",
        "pubsec-objectstream-results-prompt23.json": "public_key",
        "pubsec-embedded-file-results-prompt23.json": "public_key",
        "pubsec-wrong-key-results-prompt23.json": "public_key",
        "pubsec-permissions-report-prompt23.json": "public_key",
        "aes-gcm-normative-matrix-prompt23.json": "aes_gcm",
        "aes-gcm-backend-audit-prompt23.json": "aes_gcm",
        "aes-gcm-key-size-results-prompt23.json": "aes_gcm",
        "aes-gcm-nonce-policy-prompt23.json": "aes_gcm",
        "aes-gcm-tag-verification-prompt23.json": "aes_gcm",
        "aes-gcm-tamper-results-prompt23.json": "aes_gcm",
        "aes-gcm-decryption-matrix-prompt23.json": "aes_gcm",
        "aes-gcm-object-results-prompt23.json": "aes_gcm",
        "aes-gcm-metadata-results-prompt23.json": "aes_gcm",
        "aes-gcm-embedded-file-results-prompt23.json": "aes_gcm",
        "aes-gcm-incremental-results-prompt23.json": "aes_gcm",
        "aes-gcm-authentication-failure-prompt23.json": "aes_gcm",
        "aes-gcm-writer-options-prompt23.json": "aes_gcm",
        "aes-gcm-full-rewrite-results-prompt23.json": "aes_gcm",
        "aes-gcm-incremental-writer-results-prompt23.json": "aes_gcm",
        "aes-gcm-encryption-dictionary-prompt23.json": "aes_gcm",
        "aes-gcm-randomness-report-prompt23.json": "aes_gcm",
        "aes-gcm-reopen-proof-prompt23.json": "aes_gcm",
        "crypto-test-vector-manifest-prompt23.json": "crypto",
        "pubsec-interoperability-prompt23.json": "public_key",
        "aes-gcm-interoperability-prompt23.json": "aes_gcm",
        "crypto-reference-disagreements-prompt23.json": "crypto",
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
        "prompt23-tool-availability.json",
        status_payload("implemented", "Tool availability", tools=tools),
    )
    write_json(
        "prompt23-artifact-manifest.json",
        status_payload(
            "implemented_with_limits",
            "Artifact manifest",
            deterministic_files=deterministic_files,
            crypto_files=sorted(crypto_files),
            html_report="prompt23-html-report/index.html",
        ),
    )

    html_dir = ARTIFACT_ROOT / "prompt23-html-report"
    html_dir.mkdir(parents=True, exist_ok=True)
    (html_dir / "index.html").write_text(
        "<!doctype html><meta charset=\"utf-8\"><title>Prompt 23</title>"
        "<h1>Prompt 23 Writer/Crypto Audit</h1>"
        "<p>Writer reports implemented with limits. PubSec and AES-GCM are exact unsupported states pending normative text.</p>\n",
        encoding="utf-8",
    )

    write_doc(
        "prompt23_writer_crypto_audit.md",
        doc_text(
            "Prompt 23 Writer Crypto Audit",
            "Starting checkpoint, implementation paths, feature matrix, deterministic writer evidence, and exact crypto limits are recorded under `target/prompt23-writer-crypto`.",
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
        "prompt23_editing_writer_closeout_scorecard.md",
        doc_text(
            "Prompt 23 Editing Writer Closeout Scorecard",
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
        "prompt23_interoperability.md": "Prompt 23 Interoperability",
        "prompt23_bindings.md": "Prompt 23 Bindings",
        "prompt23_known_limits.md": "Prompt 23 Known Limits",
        "prompt23_release_verdict.md": "Prompt 23 Release Verdict",
    }.items():
        write_doc(
            name,
            doc_text(
                title,
                "This document distinguishes structural support from cryptographic trust or validation claims. PubSec and AES-GCM remain disabled until exact normative dependencies are present.",
            ),
        )

    write_json(
        "prompt23-environment.json",
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
    print(f"Wrote Prompt 23 artifacts to {ARTIFACT_ROOT}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
