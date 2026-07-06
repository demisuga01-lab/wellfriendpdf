# Prompt 08 Mesh And Patch Shadings

Prompt 08 covers the native mesh and patch shading families used by common PDFs.

Implemented behavior:

- ShadingType `4` free-form Gouraud meshes parse flags, coordinates, decode
  arrays, and vertex colors.
- ShadingType `5` lattice Gouraud meshes parse `/VerticesPerRow` and construct
  deterministic triangles.
- ShadingType `6` Coons patches use bounded subdivision and triangle rastering.
- ShadingType `7` tensor streams are parsed and bounded; the current renderer
  uses the shared Coons boundary tessellation and records exact tensor interior
  interpolation as a known limit.
- Truncated or malformed streams fail closed without panics or unbounded
  allocation.

Tests:

- `cargo test -p oxide-engine --test shadings --jobs 1`

Artifacts:

- `target/prompt08-text-shading-patterns/mesh-patch-shading-matrix.json`
- `target/prompt08-text-shading-patterns/memory-scheduler-report.json`

Remaining precise limits:

- Exact Type 7 tensor interior interpolation.
- Advanced color-management parity for non-device color spaces.
