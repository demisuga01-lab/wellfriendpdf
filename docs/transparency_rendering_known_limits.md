# Transparency Rendering Known Limits

Transparency Rendering plus Transparency Closeout establishes the native transparency/compositing
foundation. Transparency Closeout closes the Transparency Rendering-owned partial rows for `alpha_image`,
common image soft-mask `/Matte`, ExtGState soft-mask `/BC`, DeviceGray/RGB/CMYK
luminosity masks, common DeviceGray/RGB/CMYK transparency-group fixtures, and
interior knockout overlap for supported vector/Form group cases.

## Remaining Limits

- Shadings and tiling patterns are owned by Advanced Rendering, including painting those
  sources into transparency groups.
- Text clipping is later renderer work, including text used inside knockout
  groups.
- Advanced ICC/device-link/multicolor CMM parity remains unsupported-reported
  rather than claimed. Transparency Closeout covers common DeviceGray, DeviceRGB, and
  DeviceCMYK paths with the current deterministic color converter.
- Offscreen surfaces are scheduler-bounded page-coordinate surfaces with BBox
  clipping; cropped coordinate surfaces remain a future memory optimization.
- CJK/RTL raster fidelity, color glyphs, advanced annotations, OCG/progressive
  behavior, and other later renderer categories remain outside this roadmap task.

## Public Surface

Rust, CLI, Python, C ABI, WASM, .NET, and Java all consume the shared
`feature_report_json` sections named `transparency_rendering_transparency_compositing` and
`transparency_closeout_transparency_closure`. The report preserves the envelope version and
documents implemented, unsupported-reported, and later-owned behavior.
