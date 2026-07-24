# Prompt 06 Renderer Failure Taxonomy

The machine-readable taxonomy is written to
`target/prompt06-renderer-native-replay/failure-taxonomy.json`.

Reference execution categories:

- `missing_binary`: the reference executable was not found.
- `reference_execution_failure`: the reference process exited non-zero.
- `render_timeout`: the reference or Wellfriend render exceeded its timeout.
- `blank_output`: a reference command succeeded but produced no usable output.
- `malformed_input_rejection`: the input is rejected before comparison.

Comparison categories:

- `reference_disagreement`: available references disagree with each other.
- `wellfriendpdf_mismatch`: Wellfriend output differs from the agreed reference result.
- `unsupported_comparison`: no usable reference output exists.
- `visual_pass_with_compatibility_fallback`: visual comparison can proceed, but
  the display-list report still contains measured fallback.
- `native_replay_audited`: Wellfriend rendered through the native replay path for the
  covered operation category.

Fallback reasons:

- `unsupported_operator_shading`
- `unsupported_operator_pattern`
- `unsupported_graphics_state`
- `unsupported_xobject_subtype`
- `malformed_content`
- `safety_limit_exceeded`

Future renderer prompts should add categories only when they are needed for a
new failure mode. They should not merge reference failures, Wellfriend mismatches,
and unsupported comparisons into one generic bucket.

Prompt 06B writes the multi-reference taxonomy to
`target/prompt06-renderer-native-replay/renderer-parity-taxonomy-prompt06b.json`.
It adds page-level classifications for the three-reference audit:

- `all_references_agree_wellfriendpdf_pass`
- `all_references_agree_wellfriendpdf_mismatch`
- `references_disagree_wellfriendpdf_matches_poppler`
- `references_disagree_wellfriendpdf_matches_pdfium`
- `references_disagree_wellfriendpdf_matches_mupdf`
- `references_disagree_wellfriendpdf_between_references`
- `reference_tool_failure`
- `wellfriendpdf_render_failure`
- `dimension_mismatch`
- `needs_manual_review`

Prompt 08 updates the current renderer gap posture: text clipping, common
shadings, mesh/patch shadings, and tiling patterns are no longer vague
`later_owned` buckets. Prompt 08B closes the common Type3/CID text-clipping and
Type 7 tensor-interior leftovers. Current remaining limits must use precise
categories such as `advanced_icc_device_link_multicolor_cmm`,
`image_or_resource_only_Type3_charproc_fail_closed`,
`exotic_missing_glyph_outline_for_text_clip`, malformed stream fail-closed, or
bounded recursion/patch/tile caps.
