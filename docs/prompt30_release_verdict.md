# Prompt 30 release verdict

The final machine verdict is written to
`target/prompt30-release-readiness/prompt30-final-release-verdict.json` and is
exactly `complete` or `not_complete`.

Prompt 30 closes only after VPS-backed performance/stress, security, API/package,
competitor-scorecard, binding, workspace, documentation, and release-readiness gates
have explicit evidence; all required files exist; no real security or unclassified
crash/hang/OOM failure remains; and the exact closure commit leaves a clean worktree.
