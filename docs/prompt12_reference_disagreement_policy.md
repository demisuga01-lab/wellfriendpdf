# Prompt 12 Reference Disagreement Policy

Prompt 12 uses reference renderers for visual RGB preview checks where that is
meaningful:

- Wellfriend default/fallback
- Wellfriend `native-cmm-lcms2`
- Poppler
- PDFium
- MuPDF

Reference renderers often flatten Separation and DeviceN color into RGB preview
and do not expose Wellfriend's internal plate framebuffer. Those differences are
classified, not treated as automatic failures.

The audit policy:

- when references agree on visual preview and Wellfriend is an outlier, classify it
  as an Wellfriend outlier.
- when references disagree because spot/DeviceN flattening policy differs,
  classify the disagreement and use Wellfriend internal plate artifacts as plate
  evidence.
- unavailable local reference tools are recorded as unavailable tooling and do
  not count as a pass.
- unclassified failures must be zero for Prompt 12 closure.
