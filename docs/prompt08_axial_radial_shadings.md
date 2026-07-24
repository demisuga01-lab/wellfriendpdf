# Prompt 08 Axial And Radial Shadings

Prompt 08 uses the shared shading evaluator for ShadingType `2` axial and
ShadingType `3` radial shadings.

Implemented behavior:

- `/Coords`, `/Domain`, `/Extend`, `/BBox`, CTM, and active clipping are handled.
- Function Types `0`, `2`, `3`, and bounded Type `4` evaluation route through
  `render/function.rs`.
- DeviceGray, DeviceRGB, and DeviceCMYK use the current deterministic color
  conversion model.
- Shadings are clipped by ordinary path clips and Prompt 08 text clips.

Tests:

- `cargo test -p wellfriendpdf-engine --test shadings --jobs 1`
- Prompt 08 audit fixtures: `axial_horizontal`, `axial_diagonal_extend`,
  `axial_transformed_clipped`, `radial_simple`, `radial_offset_extend`, and
  `radial_degenerate_reported`.

Artifacts:

- `target/prompt08-text-shading-patterns/axial-radial-shading-matrix.json`
- `target/prompt08-text-shading-patterns/visual-diff-metrics.json`

Remaining precise limit:

- Advanced ICC/device-link/multicolor CMM exactness remains later CMM work.
