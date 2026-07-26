# Prompt 29 low-coverage risks

The low-coverage risk register is generated in `target/prompt29-malformed-differential-coverage/coverage-low-coverage-risk-register.json`.

Risk areas that remain release-relevant after Prompt 29 include:

- rare xref/object-stream repair combinations;
- malformed font and renderer paths that need larger differential corpora;
- writer/edit mutation paths that require future stress/performance gates;
- signature and standards edge cases that depend on external trust/corpus breadth;
- bindings native-lifetime edge cases outside smoke coverage.

Prompt 29 records risk rather than claiming exhaustive coverage of every real-world malformed PDF pattern.

Prompt 30 owns final release stress, final security package/SBOM, API freeze, release checklist, and broader go/no-go scoring. Prompt 29 may defer those exact areas only after crash/hang/OOM/security failures found in Prompt 29 are fixed or classified.
