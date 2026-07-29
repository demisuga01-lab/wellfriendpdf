# Prompt 36 Fuzz And Sanitizers

All listed cargo-fuzz targets built under nightly and ran a bounded smoke
campaign on the VPS.

Evidence:

- `target/prompt36-enterprise-validation/fuzz-target-inventory.json`
- `target/prompt36-enterprise-validation/fuzz-results.json`
- `target/prompt36-enterprise-validation/sanitizer-results.json`

ASAN coverage comes through cargo-fuzz. Other sanitizer combinations are
classified exactly when unavailable or not configured on the VPS.
