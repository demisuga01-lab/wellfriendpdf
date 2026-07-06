# Prompt 08 Known Limits

Prompt 08 closes the native renderer ownership for text clipping, common
shadings, mesh/patch shadings, and tiling patterns. The remaining limits are
bounded and named.

Known limits:

- Type3 glyph clipping needs a Type3 outline/content-to-clip model.
- Missing font or glyph outlines cannot produce exact text clipping and are
  unsupported-reported.
- Exact Type 7 tensor patch interior interpolation remains future math work;
  streams are parsed and bounded today.
- Advanced ICC, device-link, multicolor, and prepress CMM parity remains later
  CMM work.
- Pattern execution uses bounded per-render tile loops rather than an unbounded
  global pattern cache.

Combined Prompt 09 can begin only if it accepts these named limits and does not
reopen Prompt 08 as a vague shading/pattern/text-clip bucket.
