# Malformed Coverage feature matrix

The machine-readable matrix is `target/malformed_coverage-malformed-differential-coverage/malformed_coverage-feature-matrix.json`.

It maps each Malformed Coverage unit to concrete evidence:

- Original 113: malformed corpus source inventory, manifest, runner output, failure buckets, and survival scorecard.
- Original 114: differential tool support, corpus manifest, run results, disagreement buckets, manual-review queue, and scale scorecard.
- Original 115: crash/hang/OOM inventories, minimized artifact records, triage results, and fixed-bug regression list.
- Original 116: coverage tool matrix, measured/fallback coverage status, sanitizer support, sanitizer run results, and low-coverage risk register.

Allowed statuses are `implemented`, `implemented_with_limits`, `verified`, `verified_with_limits`, `unavailable_external_tool`, `unavailable_external_corpus`, `unsupported_reported_exact`, `deferred_release_readiness_benchmark`, `blocked`, and `not_in_scope`.

At closure, Malformed Coverage-owned rows may not remain `blocked`. Unavailable external tools and unavailable corpora are recorded separately from passes.
