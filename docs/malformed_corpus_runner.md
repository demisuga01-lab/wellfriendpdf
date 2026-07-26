# Malformed corpus runner

`scripts/run_malformed_corpus.py` is the Prompt 29 malformed-corpus runner.

Important options:

- `--repo` selects the repository root containing fixtures and the `wellfriendpdf` CLI.
- `--artifact-root` selects the target evidence directory.
- `--wellfriendpdf-bin` points at the VPS-built CLI binary.
- `--limit`, `--size-limit-bytes`, `--timeout-seconds`, and `--memory-mb` bound execution.

The runner writes the corpus manifest, JSONL per-file results, failure buckets, and survival scorecard into `target/prompt29-malformed-differential-coverage/`.

Structured parser/validator rejection is a clean result. Process crashes, unhandled exceptions, hangs, OOMs, and missing required diagnostics are findings. Missing optional external helpers, such as `pdfinfo`, are classified as unavailable instead of crashing the runner.

The runner avoids logging raw PDF bytes. Per-file output is JSONL with SHA-256 hashes and sanitized summaries.
