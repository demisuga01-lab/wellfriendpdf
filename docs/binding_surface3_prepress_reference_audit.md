# Prepress Proofing Prepress Reference Audit

The reference audit compares Wellfriend default/fallback and native-feature posture
against target-local Poppler, PDFium, and MuPDF tools where those tools are
available.

Policy:

- Wellfriend outlier failures must be zero.
- unclassified failures must be zero.
- Missing tools are `unavailable_exact`, not passed.
- Reference renderer disagreements around overprint flattening, spot preview,
  DeviceN flattening, or transparency are classified in
  `prepress-reference-disagreement-summary-prepress_proofing.json`.
- Wellfriend plate hashes are treated as internal prepress evidence; RGB previews are
  reference comparison evidence only.
