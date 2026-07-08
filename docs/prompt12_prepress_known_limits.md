# Prompt 12 Prepress Known Limits

Prompt 12 intentionally does not claim:

- certification-grade PDF/X validation
- full overprint simulation
- arbitrary n-channel multicolor ICC transform output
- press-calibrated per-plate image export
- complete image, shading, pattern, and text-outline plate writing

Implemented with bounded limits:

- device-link profiles are detected and reported everywhere; native execution is
  limited to legal LittleCMS contexts with safe channel shapes.
- multicolor ICC profiles are inventoried; high-channel transforms are
  unsupported/report-only until a safe renderer pixel format exists.
- fallback and WASM builds are preview/report-only for device-link and
  multicolor proofing.
- plate identity survives in the sparse framebuffer, but overprint compositing
  correctness is Prompt 13.

No unsupported path should be silent. Unsupported profile classes, malformed
profiles, channel mismatch, excessive colorants, missing alternates, and unsafe
tint transforms must produce report rows or diagnostics.
