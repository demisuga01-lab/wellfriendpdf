# Malformed Coverage low-coverage risks

The low-coverage risk register is generated in `target/malformed_coverage-malformed-differential-coverage/coverage-low-coverage-risk-register.json`.

Risk areas that remain release-relevant after Malformed Coverage include:

- rare xref/object-stream repair combinations;
- malformed font and renderer paths that need larger differential corpora;
- writer/edit mutation paths that require future stress/performance gates;
- signature and standards edge cases that depend on external trust/corpus breadth;
- bindings native-lifetime edge cases outside smoke coverage.

Malformed Coverage records risk rather than claiming exhaustive coverage of every real-world malformed PDF pattern.

Release Readiness Benchmark owns final release stress, final security package/SBOM, API freeze, release checklist, and broader go/no-go scoring. Malformed Coverage may defer those exact areas only after crash/hang/OOM/security failures found in Malformed Coverage are fixed or classified.
