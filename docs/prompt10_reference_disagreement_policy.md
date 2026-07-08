# Prompt 10 Reference Disagreement Policy

Prompt 10E keeps the multi-reference rule from Prompt 10: a single-reference
visual pass is not enough. Supported rendered fixtures are compared across
Oxide, Poppler, PDFium, and MuPDF, with per-page pairwise metrics and explicit
classification. Policy-only rows are allowed only when they are exact
operator/payload/security diagnostics and not visual claims.

## Classifications

- `all_references_agree_oxide_pass`: Oxide and all available references are in
  the accepted visual cluster.
- `reference_disagreement_oxide_inside_cluster`: references disagree, but Oxide
  is inside a reference cluster.
- `unsupported_reported_security_policy`: the feature is blocked for a specific
  security reason.
- `unsupported_reported_exotic_case`: the feature requires geometry or paint
  behavior not safely exposed by the current renderer.
- `unsupported_reported_exotic_format`: the feature is a named unsupported
  payload or paint operator.
- `implemented_with_limits`: the safe subset is implemented or classified and
  every remaining limit is exact.
- `blocked`: not acceptable for Prompt 10D/10E closure.

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

## Prompt 10C Outcome

The current summary is:

- rendered pages: 5
- policy-only rows: 4
- `all_references_agree_oxide_pass`: 4 pages
- `unsupported_reported_exotic_case_cid_keyed_cff_clip_geometry`: 1 page
- Oxide outlier failures: 0
- unclassified failures: 0

Evidence:

- `multi-reference-render-results-prompt10c.json`
- `multi-reference-diff-metrics-prompt10c.json`
- `reference-disagreement-summary-prompt10c.json`
- `prompt10c-html-report/index.html`

## Prompt 10D Outcome

The current summary is:

- rendered pages: 4
- policy-only rows: 15
- Oxide outlier failures: 0
- unclassified failures: 0

Evidence:

- `multi-reference-render-results-prompt10d.json`
- `multi-reference-diff-metrics-prompt10d.json`
- `reference-disagreement-summary-prompt10d.json`
- `prompt10d-html-report/index.html`

## Prompt 10E Outcome

The current summary is:

- rendered pages: 17
- fixture rows including policy diagnostics: 24
- `all_references_agree_oxide_pass`: 17 pages
- Oxide outlier failures: 0
- unclassified failures: 0

Evidence:

- `multi-reference-render-results-prompt10e.json`
- `multi-reference-diff-metrics-prompt10e.json`
- `reference-disagreement-summary-prompt10e.json`
- `prompt10e-html-report/index.html`
