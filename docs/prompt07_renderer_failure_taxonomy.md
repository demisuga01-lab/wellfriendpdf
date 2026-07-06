# Prompt 07 Renderer Failure Taxonomy

Prompt 07 uses the Prompt 06B multi-reference posture and adds transparency
specific classifications.

## Page Classifications

- `all_references_agree_oxide_pass`: all references match each other and Oxide
  falls inside threshold against all three.
- `all_references_agree_oxide_mismatch`: references agree, but Oxide is outside
  threshold.
- `references_disagree_oxide_matches_poppler`: references disagree and Oxide
  matches Poppler within threshold.
- `references_disagree_oxide_matches_pdfium`: references disagree and Oxide
  matches PDFium within threshold.
- `references_disagree_oxide_matches_mupdf`: references disagree and Oxide
  matches MuPDF within threshold.
- `references_disagree_oxide_between_references`: references disagree and Oxide
  matches more than one reference or lands between the reference outputs.
- `needs_manual_review`: transparency, blend, or soft-mask fixture where no
  pairwise classification is enough to determine the owner.
- `reference_tool_failure`: Poppler, PDFium, or MuPDF failed to render.
- `oxide_render_failure`: Oxide failed to render.
- `dimension_mismatch`: outputs rendered at incompatible dimensions.

Prompt 07B also writes normalized closure classifications:

- `all_references_agree_and_oxide_passes`
- `all_references_agree_and_oxide_mismatches`
- `references_disagree_and_oxide_within_cluster`
- `references_disagree_and_oxide_outlier`
- `malformed_or_reference_failure`
- `unsupported_reported`

## Ownership Categories

- `oxide/bug`: references agree and Oxide differs.
- `reference/disagreement`: references visibly differ.
- `prompt08/pattern_or_shading`: paint source belongs to the next prompt.
- `antialias/tolerance`: difference is limited to edge coverage.
- `unsupported/edge_case`: feature is acknowledged and bounded but not complete.
- `fixture/issue`: fixture generation is invalid or non-deterministic.

The generated JSON taxonomy is
`target/prompt07-transparency-compositing/fallback-taxonomy.json`.
Prompt 07B closure evidence is in
`target/prompt07-transparency-compositing/prompt07b-reference-disagreement-summary.json`
and `target/prompt07-transparency-compositing/prompt07b-transparency-matrix.json`.
