# Advanced Rendering Renderer Failure Taxonomy

Advanced Rendering and Type3 CID Rendering remove the vague renderer buckets that previously
covered text clipping, shadings, mesh/patch shadings, and tiling patterns.

Type3 CID Rendering audit classifications:

- `all_references_agree_wellfriendpdf_passes`: Poppler, PDFium, and MuPDF agree and
  Wellfriend is within the reference cluster.
- `references_disagree_wellfriendpdf_within_cluster`: references disagree and Wellfriend is
  within the measured cluster.
- `wellfriendpdf_outlier_failure`: Wellfriend differs from the usable reference cluster.
- `unsupported_reported_expected`: the fixture is malformed, limit-oriented, or
  exposes a reference/feature limitation that is expected and documented.
- `malformed_reference_failure`: one or more reference engines cannot produce a
  usable artifact for a malformed fixture.
- `blocked_environment`: a required renderer or audit tool is unavailable.

Type3 CID Rendering removed vague buckets:

- `type3_text_clip_outline_extraction`
- `missing_glyph_outline_for_common_cid_text_clip`
- `type7_exact_tensor_interior_interpolation`

Remaining precise limits:

- `advanced_icc_device_link_multicolor_cmm`
- `exotic_font_outline_absence_unsupported_reported`
- `unsafe_recursive_type3_or_pattern_resource_bomb_fail_closed`
- `cropped_coordinate_offscreen_optimization`

Artifacts:

- `target/type3_cid_rendering-type3-cid-tensor/type3_cid_rendering-fallback-taxonomy.json`
- `target/type3_cid_rendering-type3-cid-tensor/type3_cid_rendering-reference-disagreement-summary.json`
- `target/type3_cid_rendering-type3-cid-tensor/type3_cid_rendering-html-report/index.html`
