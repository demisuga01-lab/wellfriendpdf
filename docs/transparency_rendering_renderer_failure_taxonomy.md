# Transparency Rendering Renderer Failure Taxonomy

Transparency Rendering uses the Reference Renderer multi-reference posture and adds transparency
specific classifications.

## Page Classifications

- `all_references_agree_wellfriendpdf_pass`: all references match each other and Wellfriend
  falls inside threshold against all three.
- `all_references_agree_wellfriendpdf_mismatch`: references agree, but Wellfriend is outside
  threshold.
- `references_disagree_wellfriendpdf_matches_poppler`: references disagree and Wellfriend
  matches Poppler within threshold.
- `references_disagree_wellfriendpdf_matches_pdfium`: references disagree and Wellfriend
  matches PDFium within threshold.
- `references_disagree_wellfriendpdf_matches_mupdf`: references disagree and Wellfriend
  matches MuPDF within threshold.
- `references_disagree_wellfriendpdf_between_references`: references disagree and Wellfriend
  matches more than one reference or lands between the reference outputs.
- `needs_manual_review`: transparency, blend, or soft-mask fixture where no
  pairwise classification is enough to determine the owner.
- `reference_tool_failure`: Poppler, PDFium, or MuPDF failed to render.
- `wellfriendpdf_render_failure`: Wellfriend failed to render.
- `dimension_mismatch`: outputs rendered at incompatible dimensions.

Transparency Closeout also writes normalized closure classifications:

- `all_references_agree_and_wellfriendpdf_passes`
- `all_references_agree_and_wellfriendpdf_mismatches`
- `references_disagree_and_wellfriendpdf_within_cluster`
- `references_disagree_and_wellfriendpdf_outlier`
- `malformed_or_reference_failure`
- `unsupported_reported`

## Ownership Categories

- `wellfriendpdf/bug`: references agree and Wellfriend differs.
- `reference/disagreement`: references visibly differ.
- `advanced_rendering/pattern_or_shading`: paint source belongs to the next roadmap task.
- `antialias/tolerance`: difference is limited to edge coverage.
- `unsupported/edge_case`: feature is acknowledged and bounded but not complete.
- `fixture/issue`: fixture generation is invalid or non-deterministic.

The generated JSON taxonomy is
`target/transparency_rendering-transparency-compositing/fallback-taxonomy.json`.
Transparency Closeout closure evidence is in
`target/transparency_rendering-transparency-compositing/transparency_closeout-reference-disagreement-summary.json`
and `target/transparency_rendering-transparency-compositing/transparency_closeout-transparency-matrix.json`.
