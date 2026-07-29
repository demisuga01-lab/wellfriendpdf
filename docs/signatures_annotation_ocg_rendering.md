# Annotation Ocg Rendering Signatures

Wellfriend parses signature dictionaries, validates `/ByteRange`, hashes the signed byte ranges, parses detached CMS/PKCS#7 `SignedData`, verifies supported RSA signatures, reports signer certificate details, and reports PAdES/LTV material.

## Report Status Fields

`SignatureReport.checks` separates:

- `byte_range_present`, `byte_range_well_formed`, `byte_range_in_bounds`, and `byte_range_non_overlapping`.
- `digest_matches` and `cms_verified`.
- `chain_verified`.
- `timestamp_present` and `timestamp_verified`.
- `ltv_material_present` and `ltv_verified`.
- `docmdp_evaluated` and `fieldmdp_evaluated`.

This avoids overclaiming: ByteRange validity is not CMS validation, CMS validation is not chain trust, and timestamp presence is not timestamp trust.

## Supported

- ByteRange validation and signed-byte digest construction.
- Detached CMS verification for supported RSA PKCS#1 v1.5 signatures.
- Trust anchors supplied by the caller.
- Incremental signing with a reserved `/Contents` placeholder.
- DSS/LTV material embedding and reporting.
- Timestamp token presence and parseability reporting.

## Bounded Limits

- ECDSA, EdDSA, and RSA-PSS are not implemented.
- Live OCSP/CRL/TSA fetching is not implemented.
- Timestamp imprint and TSA chain validation are not claimed.
- DocMDP/FieldMDP permission evaluation remains bounded; Annotation Ocg Rendering reports when permission evaluation is unavailable.
- Canonical full rewrite invalidates existing signed byte ranges by design.

## Tests

Existing signature tests cover valid, tampered, timestamp, DSS/LTV, revoked CRL, and incremental signing cases. Annotation Ocg Rendering adds check-bit assertions for ByteRange, digest, CMS, chain, timestamp, and LTV status separation.
