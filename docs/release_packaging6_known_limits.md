# release validation Known Limits

ReleaseValidation closes with `release-ready, with documented boundaries`.

Known limits:

- Gradle validation is host-limited by VPS Gradle 4.4.1. Java Maven passed.
- MuPDF, PDFium, veraPDF, and a configured PDFBox harness were unavailable.
- cargo-audit, cargo-deny, cargo-about, cargo-sbom, and cargo-semver-checks
  were unavailable, so cargo metadata inventories were generated instead.
- Product claims remain bounded to supported true-editing paths and exact
  typed-refusal behavior.

Evidence is in `target/release_validation-enterprise-validation/final-release-verdict.json`.
