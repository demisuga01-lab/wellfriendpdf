# Coverage reports

`scripts/run_prompt29_coverage.py` records coverage tool support, coverage summary, and a low-coverage risk register.

When `cargo llvm-cov` is available, the runner executes a bounded workspace coverage summary. Otherwise it records exact fallback status without claiming measured coverage.

Coverage scope includes parser, repair, xref/object stream, filters/codecs, renderer entry points, writer/edit entry points, standards/signature entry points, and binding smoke where practical.

Artifacts:

- `coverage-tool-support-matrix.json`
- `coverage-summary.json`
- `coverage-low-coverage-risk-register.json`

The risk register identifies areas where line or region coverage is still not sufficient for final release claims. Prompt 29 coverage is a hardening signal, not proof that every real-world malformed PDF path has been exercised.
