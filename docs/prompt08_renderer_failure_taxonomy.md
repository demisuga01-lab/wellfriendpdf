# Prompt 08 Renderer Failure Taxonomy

Prompt 08 and Prompt 08B remove the vague renderer buckets that previously
covered text clipping, shadings, mesh/patch shadings, and tiling patterns.

Prompt 08B audit classifications:

- `all_references_agree_oxide_passes`: Poppler, PDFium, and MuPDF agree and
  Oxide is within the reference cluster.
- `references_disagree_oxide_within_cluster`: references disagree and Oxide is
  within the measured cluster.
- `oxide_outlier_failure`: Oxide differs from the usable reference cluster.
- `unsupported_reported_expected`: the fixture is malformed, limit-oriented, or
  exposes a reference/feature limitation that is expected and documented.
- `malformed_reference_failure`: one or more reference engines cannot produce a
  usable artifact for a malformed fixture.
- `blocked_environment`: a required renderer or audit tool is unavailable.

Prompt 08B removed vague buckets:

- `type3_text_clip_outline_extraction`
- `missing_glyph_outline_for_common_cid_text_clip`
- `type7_exact_tensor_interior_interpolation`

Remaining precise limits:

- `advanced_icc_device_link_multicolor_cmm`
- `exotic_font_outline_absence_unsupported_reported`
- `unsafe_recursive_type3_or_pattern_resource_bomb_fail_closed`
- `cropped_coordinate_offscreen_optimization`

Artifacts:

- `target/prompt08b-type3-cid-tensor/prompt08b-fallback-taxonomy.json`
- `target/prompt08b-type3-cid-tensor/prompt08b-reference-disagreement-summary.json`
- `target/prompt08b-type3-cid-tensor/prompt08b-html-report/index.html`
