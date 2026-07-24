# Prompt 10 Hinting Posture

Prompt 10C keeps the default renderer pure Rust and records a reference-cluster
acceptance proof instead of adding a native hinting dependency.

No native hinting backend is added, no native dependency is enabled silently, and
WASM/default builds remain portable. Feature reports expose the active posture
as `pure_rust_analytic_aa`; optional native hinting remains a future
feature-gated enhancement, not a Prompt 10 blocker.

Evidence:

- `hinting-posture-prompt10b.json`
- `hinting-posture-prompt10c.json`
- `prompt10b-multi-reference-diff-metrics.json`
- `prompt10b-reference-disagreement-summary.json`
- `multi-reference-diff-metrics-prompt10c.json`
- `reference-disagreement-summary-prompt10c.json`

The Prompt 10C rendered corpus includes Korean, Hebrew, COLRv1, sbix, and
CID-keyed CFF regression rows. It records zero Wellfriend outlier failures and zero
unclassified failures under the pure-Rust posture.
