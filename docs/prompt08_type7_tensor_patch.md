# Prompt 08 Type 7 Tensor Patch Shading

Prompt 08B replaces the Prompt 08 Type 7 boundary-only posture with tensor
product interior evaluation.

Implemented behavior:

- Type 7 stream decoding reads flags, 16 control points, corner color samples,
  coordinate/component bit widths, and decode arrays.
- Flagged continuation patches reuse the prior patch edge and read the required
  new tensor controls.
- Interior points are evaluated with bicubic Bernstein basis functions over the
  4x4 tensor grid, not by a Type 6 Coons approximation.
- Subdivision is deterministic and curvature-scaled: flat patches use fewer
  cells, curved interiors get more cells, and all output is bounded.
- Rasterization keeps the Prompt 08 clipping, CTM, BBox, transparency, and
  current device-color conversion posture.

Limits and fail-closed behavior:

- Type 7 patch streams cap at 4096 patches.
- Truncated streams, invalid decode arrays, impossible bit depths, non-finite
  coordinates, and runaway subdivision fail closed with diagnostics.
- DeviceGray, DeviceRGB, and DeviceCMYK stay within the current renderer color
  model. Advanced ICC/device-link/multicolor exactness remains later CMM work.

Evidence:

- `cargo test -p wellfriendpdf-engine --test shadings --jobs 1`
- `cargo test -p wellfriendpdf-engine render::shading::tests --jobs 1`
- `cargo test -p wellfriendpdf-engine --test prompt08b_type3_cid_tensor --jobs 1`
- `target/prompt08b-type3-cid-tensor/prompt08b-type7-tensor-matrix.json`
- `target/prompt08b-type3-cid-tensor/prompt08b-reference-disagreement-summary.json`

Prompt 08B includes smooth, curved-interior, clipped, transformed,
multi-patch, transparency-group, truncated-stream, and excessive-patch fixtures.
All six valid Type 7 tensor fixtures are classified as
`all_references_agree_wellfriendpdf_passes`; malformed/limit rows are
`unsupported_reported_expected`.
