# Prompt 26 finalization audit

## Checkpoint

- Baseline commit: `291d8ea424b2e657629d3606fbf8a93b33999f98` (`Close combined prompt 25 timestamp ltv mdp signature edits`).
- The worktree is intentionally dirty with the uncommitted Prompt 26 implementation; it was preserved without reset, stash, clean, or discarded files.
- `git diff --check` and `git diff --cached --check` passed during the finalization-start audit.
- No repository-local cargo, rustc, Oxide, fuzz, Java, .NET, Python, or test process was running at the checkpoint.

## VPS completion evidence

Final heavy verification ran on the VPS snapshot under
`/home/demisuga01/wellpdf/tmp/prompt26-completion-20260724T172523Z/repo`, with final
evidence under `/home/demisuga01/wellpdf/results/prompt26-completion-20260724T172523Z/`.

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
- Prompt 26 fuzz target build and `-runs=64` smoke for all required Prompt 26 fuzz
  targets under the 4 GiB Prompt 25B cgroup posture.
- External comparison evidence with qpdf and pyHanko library validation; veraPDF and
  PDFBox were unavailable and recorded as unavailable, not passed.
- Adversarial/tamper matrix, performance/memory probes, security audit, secret scan,
  docs/artifacts verification, and impacted historical gate reruns/focused equivalents.

The full workspace gate peaked below the 32 GiB WellPDF budget. The fuzz cgroup stayed
below the 4 GiB Prompt 25B cap; the highest per-target build RSS recorded was for the
initial fuzz target build and remained under the cap.

## Honest external-tool availability

qpdf was available and produced clean structural evidence for the CLI signed output.
pyHanko was available as a Python library and reported the self-signed test signature as
intact/valid but untrusted, which is expected for the generated test certificate. The pyHanko
console script, veraPDF, and PDFBox were unavailable and are not counted as passing
validators.

## Closure condition

At this document stage the only remaining closure condition is the local Git closure step:
stage only intended Prompt 26 files, pass `git diff --cached --check`, commit exactly
`Close combined prompt 26 incremental signing pdfa pdfua pdfx validation`, and verify the
final worktree is clean. No push, deployment, reset, stash, clean, discard, or VPS production
service action is part of Prompt 26.
