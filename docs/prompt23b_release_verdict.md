# Prompt 23B Release Verdict

Status: `complete`

Prompt 23B resumed from blocker-evidence checkpoint
`40556fb1f48cd1035f0767b78afbfe1c2034bb36` and preserves the dirty
implementation worktree. The local ISO PDFs remain outside Git under
`E:\wellpdfsdk\PDFA\`; committed materials record only identifiers, editions,
hashes, clause references, and derived implementation behavior.

Final closure evidence:

- Normative source manifest and clause matrix are present under
  `target/prompt23-writer-crypto/`.
- Scoped `/Adobe.PubSec` `/adbe.pkcs7.s5` PubSec create, decrypt, full-rewrite
  re-encrypt, recipient add/remove/replace, and removed-recipient denial are
  implemented for supported KeyTrans RSA recipient profiles.
- ISO/TS 32003 AESV4 AES-GCM encrypt/decrypt uses the mapped 12-byte IV,
  ciphertext, 16-byte tag, nil AAD posture, nonce tracking, and no plaintext
  release before authentication succeeds.
- ISO/TS 32004 standalone PDF-MAC create/verify is implemented for the mapped
  AESV4 full-rewrite profile using CMS AuthenticatedData,
  PasswordRecipientInfo, HKDF-SHA256, AES-256-KW, HMAC-SHA256, deterministic
  ByteRange placeholder patching, and false-valid denial on tamper.
- Encrypted PKCS #8 and PKCS #12/PFX provider paths are implemented with
  explicit password input, bounded parsing, wrong-password rejection, and
  secret-free reports.
- Rust, CLI, Python, C ABI, .NET, Java Maven, Java Gradle, and constrained WASM
  surfaces expose runtime or exact unsupported posture for the Prompt 23B
  closure features.
- The bounded crypto fuzz-smoke timeout was diagnosed as cold cargo-fuzz build
  time exceeding the old outer timeout, not a fuzz target hang. The harness now
  records per-target progress, enforces build/run/global limits, terminates
  process trees on timeout, and emits machine-readable results.
- Prompt 04 through Prompt 22B historical gates were rerun individually through
  the repository's accepted gate scripts and all 34 gates passed.
- Independent interoperability was executed where available. qpdf 12.3.2
  rejects the ISO/TS 32003 R7/V6 encryption dictionary as unsupported, so it is
  not counted as an AESV4/PDF-MAC pass. Poppler rendered decrypted output, and
  Java JCA primitive vectors passed for AES-GCM, AES-KW, HMAC, and HKDF.

Final validation artifacts:

- `target/prompt23-writer-crypto/prompt23b-final-validation-summary.json`
- `target/prompt23-writer-crypto/prompt23b-final-release-verdict.json`
- `target/prompt23-writer-crypto/prompt23b-final-security-verdict.json`
- `target/prompt23-writer-crypto/prompt23b-final-interoperability-verdict.json`
- `target/prompt23-writer-crypto/prompt23b-final-fuzz-verdict.json`
- `target/prompt23-writer-crypto/prompt23b-final-historical-gates-verdict.json`

Prompt 24 may begin after the final closure commit is present and
`git status --short` is empty.
