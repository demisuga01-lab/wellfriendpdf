# Transparency Rendering Soft-Mask Luminosity

Luminosity soft masks render their `/G` Form XObject into a scheduler-admitted
offscreen surface. Transparency Closeout treats the rendered mask surface as the output of
the current color pipeline, then derives alpha using Rec.601 RGB luminance.

Supported common paths:

- DeviceGray mask groups.
- DeviceRGB mask groups.
- DeviceCMYK mask groups, converted by the current deterministic CMYK-to-RGB
  fallback before luminosity extraction.
- `/TR` transfer functions after luminosity extraction when the shared function
  evaluator can build a lookup table.
- `/BC` backdrop arrays for alpha and luminosity masks in common device spaces.

Unsupported-reported CMM work:

- Exact ICCBased luminosity parity.
- Exact CalGray/CalRGB profile behavior beyond the current converter.
- Device-link and multicolor prepress workflows.

Evidence:

- `softmask_luminosity_devicegray`
- `softmask_luminosity_devicergb`
- `softmask_luminosity_devicecmyk`
- `target/transparency_rendering-transparency-compositing/transparency_closeout-render-results.json`
