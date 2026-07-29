# Malformed Coverage malformed/differential/coverage audit

Malformed Coverage starts from pushed Fuzz Campaign baseline `77a09bbaca506e52e56fb2ac4d3b55c2703b5bfa` (`Close roadmap closure 28 long codec renderer writer fuzz safedocs`) and covers Original 113 through 116 for Wellfriend PDF SDK.

The roadmap task-owned work is:

- real malformed-corpus acquisition/provenance and bounded execution;
- at-scale differential comparison against independent tools when available;
- crash, hang, OOM, and sanitizer triage/minimization;
- coverage and sanitizer reporting;
- final workspace, binding/package, historical, memory-budget, and secret-scan evidence.

The authoritative machine-readable evidence is generated under `target/malformed_coverage-malformed-differential-coverage/`. Heavy execution is performed on VPS `35.185.176.47` under `/home/demisuga01/wellpdf/results/malformed_coverage-<timestamp>/`.

Raw malformed payloads, sanitizer dumps, fuzz artifacts, and long logs are retained in result folders only. Public docs and chat summaries use sanitized statuses, artifact paths, and hashes.

This audit does not claim universal PDF safety, full world-corpus coverage, final release readiness, or full parity with every permissive parser. Those boundaries are recorded as Release Readiness Benchmark release-hardening work where applicable.
