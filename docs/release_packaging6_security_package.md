# release validation Security Package

The security package includes repository hygiene, dependency inventory, license
inventory, unsafe/native inventory, SBOM metadata, redaction residual evidence,
and sanitizer evidence.

Evidence:

- `target/release_validation-enterprise-validation/security-package.json`
- `target/release_validation-enterprise-validation/sbom.json`
- `target/release_validation-enterprise-validation/dependency-audit.json`
- `target/release_validation-enterprise-validation/license-audit.json`
- `target/release_validation-enterprise-validation/unsafe-native-audit.json`

Specialized audit helpers unavailable on the VPS are classified exactly.
