# Multilingual Color Glyphs Reference Disagreement Policy

Colrv Gradient Composite keeps the multi-reference rule from Multilingual Color Glyphs: a single-reference
visual pass is not enough. Supported rendered fixtures are compared across
Wellfriend, Poppler, PDFium, and MuPDF, with per-page pairwise metrics and explicit
classification. Policy-only rows are allowed only when they are exact
operator/payload/security diagnostics and not visual claims.

## Classifications

- `all_references_agree_wellfriendpdf_pass`: Wellfriend and all available references are in
  the accepted visual cluster.
- `reference_disagreement_wellfriendpdf_inside_cluster`: references disagree, but Wellfriend
  is inside a reference cluster.
- `unsupported_reported_security_policy`: the feature is blocked for a specific
  security reason.
- `unsupported_reported_exotic_case`: the feature requires geometry or paint
  behavior not safely exposed by the current renderer.
- `unsupported_reported_exotic_format`: the feature is a named unsupported
  payload or paint operator.
- `implemented_with_limits`: the safe subset is implemented or classified and
  every remaining limit is exact.
- `blocked`: not acceptable for Colrv Svg Bitmap/10E closure.

## CJK RTL Color Glyph Closeout Outcome

The current summary is:

- `all_references_agree_wellfriendpdf_pass`: 3 pages
- `reference_disagreement_wellfriendpdf_inside_cluster`: 1 page
- `unsupported_reported_exotic_case_cid_keyed_cff_clip_geometry`: 1 page
- Wellfriend outlier failures: 0
- unclassified failures: 0

Evidence:

- `cjk_rtl_color_glyph_closeout-multi-reference-render-results.json`
- `cjk_rtl_color_glyph_closeout-multi-reference-diff-metrics.json`
- `cjk_rtl_color_glyph_closeout-reference-disagreement-summary.json`
- `cjk_rtl_color_glyph_closeout-html-report/index.html`

## Color Glyph Hinting Outcome

The current summary is:

- rendered pages: 5
- policy-only rows: 4
- `all_references_agree_wellfriendpdf_pass`: 4 pages
- `unsupported_reported_exotic_case_cid_keyed_cff_clip_geometry`: 1 page
- Wellfriend outlier failures: 0
- unclassified failures: 0

Evidence:

- `multi-reference-render-results-color_glyph_hinting.json`
- `multi-reference-diff-metrics-color_glyph_hinting.json`
- `reference-disagreement-summary-color_glyph_hinting.json`
- `color_glyph_hinting-html-report/index.html`

## Colrv Svg Bitmap Outcome

The current summary is:

- rendered pages: 4
- policy-only rows: 15
- Wellfriend outlier failures: 0
- unclassified failures: 0

Evidence:

- `multi-reference-render-results-colrv_svg_bitmap.json`
- `multi-reference-diff-metrics-colrv_svg_bitmap.json`
- `reference-disagreement-summary-colrv_svg_bitmap.json`
- `colrv_svg_bitmap-html-report/index.html`

## Colrv Gradient Composite Outcome

The current summary is:

- rendered pages: 17
- fixture rows including policy diagnostics: 24
- `all_references_agree_wellfriendpdf_pass`: 17 pages
- Wellfriend outlier failures: 0
- unclassified failures: 0

Evidence:

- `multi-reference-render-results-colrv_gradient_composite.json`
- `multi-reference-diff-metrics-colrv_gradient_composite.json`
- `reference-disagreement-summary-colrv_gradient_composite.json`
- `colrv_gradient_composite-html-report/index.html`

## Porterduff Radial Color Glyph Outcome

Porterduff Radial Color Glyph closes the final COLRv1 Porter-Duff/Plus and moving-center radial
gradient rows. The required audit posture remains unchanged:

- Wellfriend outlier failures: 0
- unclassified failures: 0
- every reference disagreement classified
- no broad unsupported COLRv1 composite rows remain

Evidence:

- `multi-reference-render-results-porterduff_radial_color_glyph.json`
- `multi-reference-diff-metrics-porterduff_radial_color_glyph.json`
- `reference-disagreement-summary-porterduff_radial_color_glyph.json`
- `porterduff_radial_color_glyph-html-report/index.html`
