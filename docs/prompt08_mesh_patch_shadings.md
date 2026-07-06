# Prompt 08 Mesh And Patch Shadings

Prompt 08 covers the native mesh and patch shading families used by common PDFs.

Implemented behavior:

- ShadingType `4` free-form Gouraud meshes parse flags, coordinates, decode
  arrays, and vertex colors.
- ShadingType `5` lattice Gouraud meshes parse `/VerticesPerRow` and construct
  deterministic triangles.
- ShadingType `6` Coons patches use bounded subdivision and triangle rastering.
- ShadingType `7` tensor patches parse all 16 control points and use bicubic
  tensor-product interior evaluation with deterministic curvature-scaled
  subdivision.
- Truncated or malformed streams fail closed without panics or unbounded
  allocation.

Tests:

- `cargo test -p oxide-engine --test shadings --jobs 1`
- `cargo test -p oxide-engine render::shading::tests --jobs 1`
- `cargo test -p oxide-engine --test prompt08b_type3_cid_tensor --jobs 1`

Artifacts:

- `target/prompt08-text-shading-patterns/mesh-patch-shading-matrix.json`
- `target/prompt08-text-shading-patterns/memory-scheduler-report.json`
- `target/prompt08b-type3-cid-tensor/prompt08b-type7-tensor-matrix.json`
- `target/prompt08b-type3-cid-tensor/prompt08b-reference-disagreement-summary.json`

Remaining precise limits:

- Advanced color-management parity for non-device color spaces.
- The Prompt 08B tensor fixtures are device-color fixtures; ICC/device-link and
  multicolor CMM exactness remains later advanced CMM/prepress work.
