# Advanced Rendering Known Limits

Advanced Rendering closes the native renderer ownership for text clipping, common
shadings, mesh/patch shadings, and tiling patterns. Type3 CID Rendering closes the
strict-scope Type3/CID clipping and Type 7 tensor-patch caveats. The remaining
limits are bounded and named.

Known limits:

- Type3 charprocs whose visible shape is image-only, shading-only,
  pattern-only, text-only, or resource-heavy cannot be converted into a safe
  clip path and fail closed with diagnostics.
- Missing or exotic font/glyph outlines cannot produce exact text clipping and
  are unsupported-reported with font/CID/GID context where available.
- Advanced ICC, device-link, multicolor, and prepress CMM parity remains later
  CMM work.
- Pattern execution uses bounded per-render tile loops rather than an unbounded
  global pattern cache.
- Cropped-coordinate offscreen surfaces remain a performance optimization, not
  a Advanced Rendering correctness blocker.

roadmap closure 09 can begin with these named limits and should not reopen
Advanced Rendering as a vague shading/pattern/text-clip bucket.
