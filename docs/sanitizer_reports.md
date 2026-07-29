# Sanitizer reports

`scripts/run_malformed_coverage_sanitizers.py` records sanitizer support and runs supported bounded sanitizer checks.

Malformed Coverage runs AddressSanitizer-backed cargo-fuzz smoke for the parser target where the current Linux/nightly/cargo-fuzz toolchain supports it. UBSan, MSan, and TSan support is recorded with exact availability and constraints.

Artifacts:

- `sanitizer-support-matrix.json`
- `sanitizer-run-results.json`
- `sanitizer-failure-triage.json`

No sanitizer finding may remain unclassified at closure. Raw stack traces, sanitizer dumps, and crash payloads are not copied into chat or public docs.
