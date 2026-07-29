# Annotation Ocg Rendering Enterprise Trust Scorecard

| Area | Status | Notes |
|---|---|---|
| Memory safety | Implemented | Engine forbids unsafe code. |
| AES-256 standard encryption | Implemented | Tested in Annotation Ocg Rendering and earlier structural tests. |
| Public-key encryption | Diagnostic only | Detected and reported; certificate decryption unsupported. |
| Permissions | Implemented/reporting | Viewer-enforced policy note included. |
| CMS signatures | Implemented subset | RSA/CMS detached verification; status fields separated. |
| PAdES/DSS/LTV | Partial | Material reported; live revocation/TSA trust not claimed. |
| Incremental signing | Implemented subset | RSA signing with placeholder ByteRange fill. |
| Signature preservation | Partial | Append-only LTV preserves bytes; DocMDP/FieldMDP bounded. |
| Sanitizer | Implemented | Strict/balanced/preserve-visual policies with rescan. |
| PDF/A validation | Supported subset | No certification claim. |
| PDF/UA validation | Supported subset | No certification claim. |
| PDF/X validation | Supported subset | OutputIntent/active content/TrimBox subset. |
| Arlington integration | Implemented | Generated tables included; predicates bounded. |
| Fuzzing | Implemented infrastructure | Long runs remain CI/release work. |
| Differential testing | Availability-aware | qpdf/Poppler/MuPDF/PDFium/PDFBox/veraPDF optional. |
| Deterministic canonical output | Implemented | Full rewrite deterministic; signatures invalidated. |
| Multilingual Color Glyphs readiness | Ready with limits | Multilingual Color Glyphs should package and benchmark, not invent trust claims. |
