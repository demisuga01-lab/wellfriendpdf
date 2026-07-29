# Real-world malformed corpus

The malformed corpus pipeline accepts public/user corpus roots, repository fixtures, committed fuzz seeds, and generated compact malformed PDFs. Each file receives a SHA-256, size, provenance classification, retention policy, category tags, and expected clean-failure behavior.

The Malformed Coverage VPS run uses deterministic ordering, per-file timeouts, explicit operation selection, and process isolation through the CLI. The required operation set is parser/open diagnostics, repair diagnostics, extraction smoke, render smoke where supported, and standards/signature smoke where relevant.

Large or copyrighted corpora are not committed. If a public malformed corpus is unavailable on the VPS, the result is recorded as `unavailable_external_corpus` and the repository/generated fallback corpus is run without claiming full public-corpus coverage.

Closure evidence:

- `malformed-corpus-source-inventory.json`
- `malformed-corpus-manifest.json`
- `malformed-corpus-run-results.json`
- `malformed-corpus-failure-buckets.json`
- `malformed-corpus-survival-scorecard.json`

The survival scorecard must show zero unclassified panic, hang, OOM, sanitizer failure, or process death.
