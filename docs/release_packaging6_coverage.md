# release validation Coverage

Workspace coverage was generated with `cargo llvm-cov` after fixing the
instrumentation-sensitive OCR timeout test.

Evidence:

- `target/release_validation-enterprise-validation/coverage-results.json`
- `target/release_validation-enterprise-validation/low-coverage-risk-register.json`

Coverage is used as release evidence with the risk register, not as a claim of
universal path exhaustion.
