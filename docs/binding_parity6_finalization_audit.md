# Incremental Signing Standards finalization audit

## Checkpoint

- Baseline commit: `291d8ea424b2e657629d3606fbf8a93b33999f98` (`Close roadmap closure 25 timestamp ltv mdp signature edits`).
- The worktree is intentionally dirty with the uncommitted Incremental Signing Standards implementation; it was preserved without reset, stash, clean, or discarded files.
- `git diff --check` and `git diff --cached --check` passed during the finalization-start audit.
- No repository-local cargo, rustc, Wellfriend, fuzz, Java, .NET, Python, or test process was running at the checkpoint.

## VPS completion evidence

Final heavy verification ran on the VPS snapshot under
`/home/demisuga01/wellpdf/tmp/incremental_signing_standards-completion-20260724T172523Z/repo`, with final
evidence under `/home/demisuga01/wellpdf/results/incremental_signing_standards-completion-20260724T172523Z/`.

The following gates passed on the VPS:

- Focused engine standards tests, focused signing tests, and engine clippy.
- CLI check, CLI clippy, runtime smoke for PDF/A, PDF/UA, PDF/X, validate-all,
  placeholder planning, signing, prefix preservation, and qpdf structural check of the
  CLI signed output.
- Fresh Python wheel build/install and `pytest` runtime tests.
- C ABI runtime/ownership/header tests.
- WASM `wasm32-unknown-unknown` check and `wasm-pack` web build.
- .NET test and pack through the C ABI/native layer.
- Java Maven test/package and Gradle test/build through the native layer.
- Full workspace `cargo fmt --all --check`, `cargo check --workspace --all-targets`,
  `cargo clippy --workspace --all-targets -- -D warnings`, and
  `cargo test --workspace --all-targets`, each with `--jobs 1` where applicable.
- Incremental Signing Standards fuzz target build and `-runs=64` smoke for all required Incremental Signing Standards fuzz
  targets under the 4 GiB Pades LTV Fuzz cgroup posture.
- External comparison evidence with qpdf and pyHanko library validation; veraPDF and
  PDFBox were unavailable and recorded as unavailable, not passed.
- Adversarial/tamper matrix, performance/memory probes, security audit, secret scan,
  docs/artifacts verification, and impacted historical gate reruns/focused equivalents.

The full workspace gate peaked below the 32 GiB Wellfriend PDF SDK budget. The fuzz cgroup stayed
below the 4 GiB Pades LTV Fuzz cap; the highest per-target build RSS recorded was for the
initial fuzz target build and remained under the cap.

## Honest external-tool availability

qpdf was available and produced clean structural evidence for the CLI signed output.
pyHanko was available as a Python library and reported the self-signed test signature as
intact/valid but untrusted, which is expected for the generated test certificate. The pyHanko
console script, veraPDF, and PDFBox were unavailable and are not counted as passing
validators.

## Closure condition

At this document stage the only remaining closure condition is the local Git closure step:
stage only intended Incremental Signing Standards files, pass `git diff --cached --check`, commit exactly
`Close roadmap closure 26 incremental signing pdfa pdfua pdfx validation`, and verify the
final worktree is clean. No push, deployment, reset, stash, clean, discard, or VPS production
service action is part of Incremental Signing Standards.
