# Prompt 10 Reference Disagreement Policy

Prompt 10B keeps the multi-reference rule from Prompt 10: a single-reference
visual pass is not enough. Supported rendered fixtures are compared across
Oxide, Poppler, PDFium, and MuPDF, with per-page pairwise metrics and explicit
classification.

## Classifications

- `all_references_agree_oxide_pass`: Oxide and all available references are in
  the accepted visual cluster.
- `reference_disagreement_oxide_inside_cluster`: references disagree, but Oxide
  is inside a reference cluster.
- `unsupported_reported_security_policy`: the feature is blocked for a specific
  security reason.
- `unsupported_reported_exotic_case`: the feature requires geometry or paint
  behavior not safely exposed by the current renderer.
- `partial_blocker`: not acceptable for Prompt 10B closure.

## Prompt 10B Outcome

The current summary is:

- `all_references_agree_oxide_pass`: 3 pages
- `reference_disagreement_oxide_inside_cluster`: 1 page
- `unsupported_reported_exotic_case_cid_keyed_cff_clip_geometry`: 1 page
- Oxide outlier failures: 0
- unclassified failures: 0

Evidence:

- `prompt10b-multi-reference-render-results.json`
- `prompt10b-multi-reference-diff-metrics.json`
- `prompt10b-reference-disagreement-summary.json`
- `prompt10b-html-report/index.html`
