# release validation Performance And Stress

Performance and stress evidence is collected from timed VPS stages, including
workspace tests, coverage, fuzz smoke, resource-limit tests, parallelism tests,
and package gates.

Evidence:

- `target/release_validation-enterprise-validation/performance-results.json`
- `target/release_validation-enterprise-validation/stress-results.json`

The maximum observed ReleaseValidation RSS stayed below the 32 GiB budget.
