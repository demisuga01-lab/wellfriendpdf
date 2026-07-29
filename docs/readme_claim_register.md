# README claim register

Every public README claim was narrowed to one of the allowed evidence classes.

| Claim ID | README claim | Evidence class | Source |
| --- | --- | --- | --- |
| `status.complete` | Current implementation status is complete. | validated_in_repository | `target/prompt36-enterprise-validation/final-release-verdict.json` |
| `posture.release_ready_with_limits` | Release posture is `release_ready_with_limits`. | validated_in_repository | `target/prompt36-enterprise-validation/final-release-verdict.json` |
| `memory.under_budget` | Prompt 36 stayed under the 32 GiB memory budget; max observed RSS was 6,618,920 KiB. | validated_in_repository | `target/prompt36-enterprise-validation/performance-results.json` |
| `fuzz.43_targets` | 43 fuzz targets built and smoke-ran at 64 runs per target. | validated_in_repository | `target/prompt36-enterprise-validation/fuzz-results.json` |
| `bindings.surface` | Rust/CLI/Python/C/WASM/.NET/Java Maven passed; Gradle has an exact VPS host limit. | validated_in_repository | `target/prompt36-enterprise-validation/binding-release-matrix.json` |
| `comparators.narrow_smoke` | The README direct comparator smoke passed 11/11 narrow operations. | measured_directly | `target/readme-rewrite/readme-direct-comparisons-qpdf-clean.json` |
| `commercial.docs_only` | Commercial SDK comparisons are documentation-only. | official_competitor_documentation | `docs/benchmarks/competitor-comparison.json` |
| `blocked.universal_claims` | Universal dynamic XFA, all-viewer appearance parity, and unavailable external-tool parity are blocked claims. | validated_in_repository | `target/prompt36-enterprise-validation/product-claim-matrix.json` |

Blocked public wording:

- universal Adobe parity;
- complete PDF support;
- fastest/best overall PDF engine;
- PDFium/MuPDF/Poppler superiority without equivalent direct tests;
- certification from internal validation alone;
- universal XFA conversion;
- all-viewer form/annotation appearance parity;
- perfect OCR, reading order, accessibility, or redaction.
