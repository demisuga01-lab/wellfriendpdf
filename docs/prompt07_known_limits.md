# Prompt 07 Known Limits

Prompt 07 establishes the native transparency/compositing foundation, but does
not claim complete PDF renderer parity.

## Remaining Limits

- Shadings and tiling patterns are owned by Prompt 08, including painting those
  sources into transparency groups.
- Group color spaces remain mostly device-space. Full ICC-managed group
  conversion is not claimed.
- Exact knockout behavior for every interior object-overlap case remains
  partial.
- Soft-mask matte/background edge cases remain partial.
- Offscreen surfaces are scheduler-bounded page-coordinate surfaces with BBox
  clipping; cropped coordinate surfaces remain a future memory optimization.
- CJK/RTL raster fidelity, color glyphs, advanced annotations, OCG/progressive
  behavior, and other later renderer categories remain outside this prompt.

## Public Surface

Rust, CLI, Python, C ABI, WASM, .NET, and Java all consume the shared
`feature_report_json` section named `prompt07_transparency_compositing`. The
report preserves the envelope version and documents implemented, partial, and
later-owned behavior.
