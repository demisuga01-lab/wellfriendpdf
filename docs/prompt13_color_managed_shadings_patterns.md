# Prompt 13 Color-Managed Shadings and Patterns

Shadings and tiling patterns stay on the existing renderer color path instead
of introducing a second prepress architecture. Their colors flow through
`ColorSpaceHandler`, named Separation/DeviceN tint transforms, ICC/Cal/Lab
handling where supported, and the Prompt 12B separation framebuffer.

Shadings:

- axial and radial shadings use the renderer color interpolation path.
- mesh and patch shadings use the Prompt 08/08B geometry path, with color
  conversion reported through the CMM/prepress layer.
- Separation and DeviceN shading colors write plate contributions where the
  shading color space is representable.

Patterns:

- colored tiling patterns preserve pattern resource color spaces.
- uncolored tiling patterns preserve the caller color space.
- pattern matrix, cell identity, plate visibility, and overprint state
  participate in the Prompt 13 cache/equivalence artifacts.
- recursive patterns remain bounded by existing recursion and tile caps.

Fallback/WASM behavior remains preview-only for native proofing features. Native
LittleCMS is used only when the feature build and profile shape are legal.
