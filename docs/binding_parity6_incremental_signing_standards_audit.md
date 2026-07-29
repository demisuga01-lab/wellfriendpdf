# Incremental Signing Standards — Incremental Signing & Standards (PDF/A, PDF/UA, PDF/X) Audit

Schema: `incremental_signing_standards.incremental-signing-standards.v1`

## Section 0 — Verified starting state (trust the repository, not the roadmap task)

Verified with `git rev-parse HEAD`, `git log`, `git status --short`, `git diff --check`,
`git diff --cached --check`, remote-tracking inspection, and a process inventory.

- HEAD: `291d8ea424b2e657629d3606fbf8a93b33999f98`
  ("Close roadmap closure 25 timestamp ltv mdp signature edits") — matches the expected
  Pades LTV Fuzz closure.
- Branch `main`; remote `origin` = `https://github.com/demisuga01-lab/wellfriend-parser.git`.
- Remote tracking: `main...origin/main` **[ahead 1]**. The Pades LTV Fuzz closure commit is
  **local only** (not pushed); `git branch -r --contains HEAD` is empty. Per the roadmap task,
  the local closure commit is **not** reset or discarded; Incremental Signing Standards continues from the
  local clean Pades LTV Fuzz closure.
- Worktree clean; both whitespace checks clean.
- No repo-local long-running cargo/rustc/wellfriendpdf/fuzz/java/dotnet/python processes.
- Host: Windows 11 (10.0.26200), x86_64. rustc stable 1.95.0 / nightly 1.98.0; cargo-fuzz
  0.13.1. Targets: wasm32, x86_64-pc-windows-msvc, x86_64-unknown-linux-gnu.

Machine-readable: `target/incremental_signing_standards-incremental-signing-standards/incremental_signing_standards-starting-state.json`.

## Section 0.1 - Recovery checkpoint superseding the original clean-start note

The standalone Incremental Signing Standards recovery run resumed from the same baseline commit,
`291d8ea424b2e657629d3606fbf8a93b33999f98`, but the worktree was no longer clean. The
dirty tree contained Incremental Signing Standards implementation, binding, documentation, and fuzz-target work.
That dirty state was preserved without reset, stash, clean, discarded files, push, deployment,
or Crypto Standards Fuzz work.

Recovery evidence was captured under
`target/incremental_signing_standards-incremental-signing-standards/`, including the resume start-state JSON and a
local recovery archive. Heavy verification was moved to the VPS runner under
`/home/demisuga01/wellpdf/tmp/incremental_signing_standards-completion-20260724T172523Z/`, with final evidence under
`/home/demisuga01/wellpdf/results/incremental_signing_standards-completion-20260724T172523Z/`.

The earlier "Worktree clean" bullet above describes the first clean Incremental Signing Standards planning
checkpoint only. It must not be read as the recovery or final closure state.

## Section 0.2 — Guarantees to preserve (no regression)

crypto writer closeout (pubsec/AES-GCM/PDF-MAC/key-provider/no-plaintext-on-auth-failure), Signature Validation Resume
(CMS/PKIX/AIA/OCSP/CRL/evidence replay/PAdES baseline), Pades LTV Fuzz (RFC 3161 timestamp,
DSS/VRI/LTV, DocMDP/FieldMDP, signature-preserving form-fill, fuzz configuration),
deterministic + incremental writer guarantees, signature-impact reporting, sanitizer/
security reports, binding parity, the 4 GiB memory-cap discipline, and no-fake-success.

## Baseline (what already exists vs. what Incremental Signing Standards must add)

Incremental Signing Standards is a **major uplift on an existing foundation**, not a green field:

Signing (already present in `signature.rs` / `secure_mutation.rs` / `pubsec.rs`):
- `sign_document` approval signing over a `/Contents` placeholder with `contents_placeholder`,
  `patch_contents_hex`, `patch_byte_range`, `build_detached_cms`.
- Full Signature Validation/25 post-hoc validation (`verify_signatures*`), RFC 3161, DSS/VRI/LTV.
- DocMDP/FieldMDP enforcement and signature-preserving append-only form-fill.
- `PubSecKeyProvider` key ingestion (PKCS#12/PKCS#8/PEM/DER).

Standards (already present in `standards.rs` / `compliance.rs` / `color_report.rs` /
`prepress.rs`):
- `validate_standards_profile` with `StandardsProfile {PdfA,PdfUa,PdfX,Security,All}` and a
  first-generation `ValidationRuleResult` (statuses Pass/Fail/Warn/NotApplicable).
- `validate_pdfa` (A-1B/2B/2A/3B/3A), `convert_to_pdfa`, `validate_pdfua` (best-effort).
- PDF/X output-intent + ICC + CMYK/DeviceN/spot/overprint checks (partial), Prepress CMM/13
  prepress infrastructure.

Incremental Signing Standards gaps to close (the actual work):
1. **Incremental signing engine**: external-signer callback API, explicit CMS insertion
   boundary type-set, RSA-PSS/ECDSA signing modes, certification (DocMDP-creation)
   signatures validated by the Pades LTV permission engine, document-timestamp creation
   status, retry-on-too-small placeholder, mandatory post-sign reopen+validate.
2. **Clause-mapped rule engines**: uplift PDF/A, PDF/UA, PDF/X to stable rule IDs + ISO
   clause refs + object/page/resource context + evidence path + the full Incremental Signing Standards status
   set (`pass`/`fail`/`warning`/`indeterminate`/`not_applicable`/`unsupported_reported_exact`/
   `deferred_crypto_standards_fuzz_corpus_parity`/`blocked_normative_dependency`).
3. **Shared report envelope + cross-profile conflict report** (one profile passing must not
   hide another failing).
4. **Positive + negative fixtures** per category; **external validator comparison**
   (veraPDF/qpdf/pyHanko/PDFBox) with disagreement classification.
5. **Binding surfaces** (Rust/CLI/Python/C ABI/WASM/.NET/Java) exposing real runtime ops.
6. **Adversarial matrix + Incremental Signing Standards fuzz targets** under the Pades LTV Fuzz 4 GiB low-memory
   posture; performance/memory/security audit; secret scan.
7. **Docs (12) + artifacts (~30) + full validation gates + one closure commit.**

## Scope & honesty note

This combines original roadmap units 101–104 and is a large, multi-workstream
implementation. It is being executed in staged increments (shared architecture →
incremental signing → PDF/A → PDF/UA → PDF/X → cross-profile → bindings → external
comparison → adversarial/fuzz/perf/security → docs/artifacts → closure), with real code,
tests, fixtures, and evidence at each stage. Full veraPDF-corpus parity and long fuzz
campaigns are explicitly deferred to Crypto Standards Fuzz. "Certification-grade" here means
clause-mapped, deterministic, fixture-backed, externally compared where available — not an
accredited certification claim.

## No deployment

No deployment performed; VPS untouched. This roadmap task ends at repository validation and, only
if explicitly instructed, a Git push.
