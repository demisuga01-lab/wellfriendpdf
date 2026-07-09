# Prompt 12 Prepress Known Limits

Prompt 12 intentionally does not claim:

- certification-grade PDF/X validation
- press-calibrated per-plate image export

Implemented with bounded limits:

- device-link profiles are detected and reported everywhere; native execution is
  limited to legal LittleCMS contexts with safe channel shapes.
- multicolor ICC profiles are inventoried; Prompt 12B adds the bounded
  n-channel representation, while unsupported safe-wrapper pixel formats remain
  precise fail-closed cases.
- fallback and WASM builds are preview/report-only for device-link and
  multicolor proofing.
- plate identity survives in the sampled n-channel separation framebuffer;
  Prompt 13 adds the bounded overprint/prepress close-out on top of this
  baseline.
- text/vector/stencil-image/shading/pattern plate samples are implemented for
  supported named Separation/DeviceN paths; nested Type3 resource programs and
  unsafe high-channel packed images remain exact limits.

No unsupported path should be silent. Unsupported profile classes, malformed
profiles, channel mismatch, excessive colorants, missing alternates, and unsafe
tint transforms must produce report rows or diagnostics.
