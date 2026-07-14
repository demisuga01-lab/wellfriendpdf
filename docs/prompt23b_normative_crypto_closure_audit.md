# Prompt 23B Normative Crypto Closure Audit

Status: `resumed_from_blocker_source_gate_passed_implemented_with_limits`

Prompt 23B originally started from clean HEAD
`29c7d14f7e975b5f591f078c1fa502eb2531ca6f` and stopped at commit
`40556fb1f48cd1035f0767b78afbfe1c2034bb36` with
`blocked_normative_dependency`.

The prompt resumed from the existing blocker-evidence commit:

- `git status --short`: no entries
- `git rev-parse HEAD`: `40556fb1f48cd1035f0767b78afbfe1c2034bb36`
- top commit: `40556fb Record prompt 23B normative dependency blocker`

## Preserved Blocker Evidence

The original blocker was valid: the repository did not contain legally usable
local copies of ISO 32000-2:2020, ISO/TS 32003:2023, ISO/TS 32004:2024, or the
signature/CMS extension material needed to avoid guessing PDF-specific
cryptographic byte layouts.

No Prompt 23B cryptographic code was changed in that blocker commit.

## Resumed Source Verification

The required PDF-family documents are now available locally under
`E:\wellpdfsdk\PDFA\`. The directory is excluded by `.git/info/exclude`; the
standards PDFs are not committed or redistributed.

Verified local files and hashes are recorded in
`target/prompt23-writer-crypto/normative-source-manifest-prompt23b.json`.

The source gate now has enough PDF-specific information for implementation of:

- PubSec dictionary and recipient-location parsing from ISO 32000-2:2020.
- CMS EnvelopedData mapping as selected by the PDF PubSec profile.
- AES-GCM crypt-filter behavior from ISO/TS 32003:2023.
- Integrity-protection reporting boundaries from ISO/TS 32004:2024.

## Implementation Boundaries

Prompt 23B implementation must still distinguish:

- recipient identity matching from certificate trust-chain validation;
- authenticated AES-GCM object decrypt from document-level PDF MAC validation;
- decryption success from digital-signature or PAdES validity;
- production random nonce generation from deterministic test-vector fixtures;
- precise unsupported recipient/algorithm rows from silent downgrade.

The clause implementation matrix is in
`target/prompt23-writer-crypto/clause-implementation-matrix-prompt23b.json`.

## Implemented Since Resumption

- ISO/TS 32003 AESV4 Standard-handler object encryption/decryption.
- AES-GCM authentication failure returns no plaintext.
- `/Adobe.PubSec` parsing for scoped public-key security-handler dictionaries.
- CMS EnvelopedData parsing for KeyTransRecipientInfo.
- Issuer/serial and subject-key-identifier recipient matching.
- RSAES-PKCS1-v1_5 and default-parameter RSAES-OAEP key transport.
- PubSec file-key recovery for scoped `/adbe.pkcs7.s5` crypt-filter fixtures.
- Explicit in-memory Rust key-provider APIs.
- Explicit RSAES-OAEP SHA-1/SHA-256/SHA-384/SHA-512 MGF1 parameter validation for absent/default or empty pSource labels.
- PubSec full-rewrite writing/re-encryption for scoped `/adbe.pkcs7.s5` KeyTrans recipients.
- Recipient add/remove/replace by full rewrite with fresh seed/file-key material.
- CLI, Python, C ABI, .NET, and Java runtime wrappers for explicit PubSec key/certificate byte inputs, with managed password-provider and hardware-provider lifecycles still unsupported.

## Validation Snapshot

Passed in this pass:

- `cargo fmt --check`
- `git diff --check`
- `git diff --cached --check`
- `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings`
- `cargo test --workspace --all-targets --jobs 1`
- Focused PubSec, AES-GCM, and Prompt 23 Rust tests.
- WASM target check.
- Fuzz-bin compile check.
- C ABI runtime tests through workspace and Prompt 03 gate.
- Fresh Python wheel build/install plus Prompt 23B runtime smoke.
- .NET tests and pack.
- Java Maven package/runtime smoke.
- Java Gradle test/JAR/build/equivalence smoke.
- wasm-pack web/Node smoke.
- Prompt 03 release gate after binding report expectations were updated.

Not complete:

- PubSec encrypted incremental updates.
- PKCS #12/PFX password-provider APIs are implemented for bounded non-WASM PubSec provider loading of one unambiguous RSA certificate/private-key identity with explicit password bytes. Encrypted PKCS #8 DER/PEM RSA private-key loading is also implemented with explicit password bytes.
- ISO/TS 32004 PDF-MAC AttachedToSig extraction/binding and non-SHA256 profiles. Standalone AESV4 full-rewrite PDF-MAC generation and verification are implemented for the mapped PasswordRecipientInfo/pdfMacWrapKdf/AES-256-KW/HMAC-SHA256/SHA-256 profile; `valid` is not returned until CMS, key unwrap, HMAC, ByteRange, and digest verification succeed.
- Independent external implementation pass for every implemented crypto profile.
