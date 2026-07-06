# Prompt 08 Fallback Policy

Prompt 08 removes these vague later buckets from the current renderer posture:

- `text_clipping/later`
- `shading/later`
- `pattern/later`

Remaining fallback or unsupported-reported reasons must be precise:

- `advanced_icc_device_link_multicolor_cmm`
- `type3_text_clip_outline_extraction`
- `missing_glyph_outline_for_text_clip`
- `type7_exact_tensor_interior_interpolation`
- malformed shading stream fail-closed
- malformed pattern step fail-closed
- pattern recursion cap reached

Prompt 08 does not hide these cases behind compatibility rendering. The public
feature report exposes the limits under
`prompt08_text_clipping_shading_patterns`, and the audit artifacts keep the
fixture-level classification.
