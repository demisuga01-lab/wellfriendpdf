# Prompt 36 Known Limits

Prompt36 closes with `release_ready_with_limits`.

Known limits:

- Gradle validation is host-limited by VPS Gradle 4.4.1. Java Maven passed.
- MuPDF, PDFium, veraPDF, and a configured PDFBox harness were unavailable.
- cargo-audit, cargo-deny, cargo-about, cargo-sbom, and cargo-semver-checks
  were unavailable, so cargo metadata inventories were generated instead.
- Product claims remain bounded to supported true-editing paths and exact
  typed-refusal behavior.

Evidence is in `target/prompt36-enterprise-validation/final-release-verdict.json`.
