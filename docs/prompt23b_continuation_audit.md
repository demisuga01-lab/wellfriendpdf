# Prompt 23B Continuation Audit

Recorded: 2026-07-14

Status: `not_complete`

Starting checkpoint for this continuation: `40556fb1f48cd1035f0767b78afbfe1c2034bb36`.

This continuation preserves the intentionally dirty Prompt 23B worktree. No reset, restore, stash, clean, revert, Prompt 23C split, Prompt 24 start, or partial commit was performed.

## Required Preserve Pass

- `git status --short` was inspected and showed the existing Prompt 23B dirty implementation across engine, CLI, C ABI, Python, .NET, Java, docs, and target artifacts.
- `git diff --stat` was inspected and showed 113 changed tracked files plus untracked Prompt 23B implementation/docs.
- `git diff --check` passed with line-ending warnings only.
- `git diff --cached --check` passed.
- `git diff` was captured to `target/prompt23-writer-crypto/prompt23b-current-dirty.diff`.
- `git rev-parse HEAD` returned `40556fb1f48cd1035f0767b78afbfe1c2034bb36`.
- `git log --oneline -n 25` was inspected and confirms the current HEAD is the Prompt 23B blocker-evidence commit.

## Existing Implementation Preserved

- Local ISO/PDF-family source manifest and clause matrix are present under `target/prompt23-writer-crypto/`.
- PubSec parsing, KeyTrans recipient handling, issuer/serial and SKI matching, RSAES-PKCS1-v1_5, default/explicit RSAES-OAEP empty-label cases, AES-CBC CMS content handling, seed/permissions parsing, and file-key derivation are implemented in `crates/engine/src/pubsec.rs`.
- Scoped `/Adobe.PubSec` `/adbe.pkcs7.s5` open/decrypt and full-rewrite writer/re-encryption are wired through engine, CLI, SDK, Python, C ABI, .NET, and Java surfaces.
- ISO/TS 32003 AESV4 object encryption/decryption is implemented in `crates/engine/src/crypto.rs`, `crates/engine/src/reader.rs`, and `crates/engine/src/writer.rs`.
- PDF-MAC structure discovery, supported-token verification, and AESV4 standalone writer creation are implemented in `crates/engine/src/pdf_mac.rs` for standalone AuthCode tokens using PasswordRecipientInfo, pdfMacWrapKdf, AES-256-KW, HMAC-SHA256, SHA-256, and ByteRange digest checks. It does not return `valid` from structure-only inspection.
- Existing validation artifacts record passed focused Rust tests, workspace tests, C ABI, Python, .NET, Java Maven/Gradle, WASM, fuzz-bin compile, and Prompt 23 audit regeneration.

## Exact Remaining Blockers

- ISO/TS 32004 PDF-MAC AttachedToSig signature binding and non-SHA256 profiles are not implemented.
- PDF-MAC PasswordRecipientInfo, HKDF-SHA256, AES-256 key wrap, HMAC-SHA256 validation, covered-byte digest comparison, and AESV4 full-rewrite writer placeholder/ByteRange patching are implemented for the mapped standalone profile when the document file key is available.
- Encrypted PKCS #8 provider APIs are implemented for explicit DER/PEM RSA private keys with password input. PKCS #12/PFX provider APIs are implemented with bounded non-WASM extraction for one unambiguous RSA certificate/private-key identity; wrong passwords, malformed containers, duplicate matches, non-RSA keys, and WASM PFX extraction remain exact unsupported/error cases.
- Independent PDF implementation interoperability is not claimed for every implemented PubSec/AESV4/PDF-MAC profile.
- Bounded cargo-fuzz smoke for `crypto` timed out and is recorded as not passed; fuzz-bin compile passed.
- Individual Prompt 04 through Prompt 22B historical gates have not all been rerun in this continuation.

## Security Posture

- Private keys, passwords, recovered file keys, recipient seed payloads, and MAC keys must not be serialized in reports.
- AESV4 plaintext is returned only after AEAD authentication succeeds.
- PDF-MAC document-integrity validity must not be claimed without CMS AuthenticatedData, key recovery, ByteRange, digest, and HMAC verification. Standalone AESV4 writer creation is implemented with placeholder/ByteRange patching for the mapped profile.
- Public-key recipient matching is not certificate trust validation.
