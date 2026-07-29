# Incremental Signing Standards known limits

Incremental Signing Standards intentionally reports these boundaries without claiming conformance:

- PDF/A-4 rule execution and full veraPDF corpus parity are deferred to Crypto Standards Fuzz.
- PDF/UA human reading-order judgement is not mechanically certified.
- Deep DeviceN/Separation/overprint and older PDF/X transparency corpus parity are deferred to
  Crypto Standards Fuzz.
- qpdf is structural-only; pyHanko, veraPDF, and PDFBox are scoped to their actual available
  checks.
- WASM does not fake host filesystem, unrestricted network, OS trust-store, or external-signer
  integration.

All limits are represented by an exact status/evidence row and keep the affected report from a
false conformant result.
