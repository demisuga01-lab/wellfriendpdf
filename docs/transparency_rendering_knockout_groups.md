# Transparency Rendering Knockout Groups

Transparency Closeout changes knockout groups from a final flatten-time approximation to a
per-pixel initial-backdrop model inside the group buffer. When `/K true` is
active, the group offscreen surface keeps a copy of its initial backdrop. Each
subsequent interior paint uses that initial backdrop as the destination for
covered pixels, so later overlapping objects knock out earlier objects.

Implemented cases:

- Isolated knockout groups.
- Non-isolated groups through the existing parent-backdrop seed and removal
  path.
- Nested Form XObject knockout groups.
- Vector and image paints that ultimately call `PixelBuffer::blend_pixel`.
- Group state restoration for alpha, blend mode, CTM, clip, and soft mask.

Evidence:

- Unit test: `knockout_backdrop_prevents_interior_overlap_accumulation`.
- Fixtures: `knockout_overlap_exact`, `knockout_overlap_nested_form`.
- Audit: `target/transparency_rendering-transparency-compositing/transparency_closeout-transparency-matrix.json`.

Remaining later-owned limits are text clipping and Advanced Rendering pattern/shading
paints inside knockout groups.
