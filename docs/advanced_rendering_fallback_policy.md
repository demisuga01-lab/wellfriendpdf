# Advanced Rendering Fallback Policy

Advanced Rendering removes these vague later buckets from the current renderer posture:

- `text_clipping/later`
- `shading/later`
- `pattern/later`

Remaining fallback or unsupported-reported reasons must be precise:

- `advanced_icc_device_link_multicolor_cmm`
- `image_or_resource_only_Type3_charproc_fail_closed`
- `exotic_missing_glyph_outline_for_text_clip`
- malformed shading stream fail-closed
- malformed pattern step fail-closed
- malformed Type 7 tensor stream fail-closed
- excessive Type 7 patch-count cap reached
- pattern recursion cap reached

Advanced Rendering does not hide these cases behind compatibility rendering. The public
feature report exposes the limits under
`advanced_rendering_text_clipping_shading_patterns` and
`type3_cid_rendering_type3_cid_tensor_closure`, and the audit artifacts keep the
fixture-level classification.
