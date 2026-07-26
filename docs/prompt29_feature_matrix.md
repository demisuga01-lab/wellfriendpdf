# Prompt 29 feature matrix

The machine-readable matrix is `target/prompt29-malformed-differential-coverage/prompt29-feature-matrix.json`.

It maps each Prompt 29 unit to concrete evidence:

- Original 113: malformed corpus source inventory, manifest, runner output, failure buckets, and survival scorecard.
- Original 114: differential tool support, corpus manifest, run results, disagreement buckets, manual-review queue, and scale scorecard.
- Original 115: crash/hang/OOM inventories, minimized artifact records, triage results, and fixed-bug regression list.
- Original 116: coverage tool matrix, measured/fallback coverage status, sanitizer support, sanitizer run results, and low-coverage risk register.

Allowed statuses are `implemented`, `implemented_with_limits`, `verified`, `verified_with_limits`, `unavailable_external_tool`, `unavailable_external_corpus`, `unsupported_reported_exact`, `deferred_prompt30`, `blocked`, and `not_in_scope`.

At closure, Prompt 29-owned rows may not remain `blocked`. Unavailable external tools and unavailable corpora are recorded separately from passes.
