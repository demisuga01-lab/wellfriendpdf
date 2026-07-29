# Color Glyph Hinting Color Glyph, Hinting, and CFF Closure

Color Glyph Hinting closes the remaining strict Multilingual Color Glyphs blockers without reopening
unrelated renderer phases.

## Scope

Color Glyph Hinting covers:

- COLRv1 bounded paint graph rendering.
- SVG-in-OpenType static subset classification and security blocking.
- non-PNG color bitmap payload diagnostics.
- pure-Rust hinting reference-cluster proof.
- exotic CID-keyed CFF clipping diagnostics.
- Poppler/PDFium/MuPDF/Wellfriend audit for the Color Glyph Hinting corpus.

It does not start native CMM, prepress, standards, parser, codec, annotation,
OCG, shadings, patterns, progressive cache, or release-hardening work.

## Artifacts

All Color Glyph Hinting artifacts live under:

```text
target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/
```

Primary artifacts:

- `color_glyph_hinting-closure-audit.json`
- `color-glyph-colrv1-matrix-color_glyph_hinting.json`
- `color-glyph-colrv1-reference-results-color_glyph_hinting.json`
- `color-glyph-svg-static-subset-matrix-color_glyph_hinting.json`
- `color-glyph-svg-security-policy-color_glyph_hinting.json`
- `color-glyph-svg-reference-results-color_glyph_hinting.json`
- `color-glyph-bitmap-payload-matrix-color_glyph_hinting.json`
- `color-glyph-cbdt-cblc-results-color_glyph_hinting.json`
- `color-glyph-sbix-results-color_glyph_hinting.json`
- `hinting-posture-color_glyph_hinting.json`
- `cid-keyed-cff-clipping-matrix-color_glyph_hinting.json`
- `cid-keyed-cff-clipping-reference-results-color_glyph_hinting.json`
- `multi-reference-render-results-color_glyph_hinting.json`
- `multi-reference-diff-metrics-color_glyph_hinting.json`
- `reference-disagreement-summary-color_glyph_hinting.json`
- `color_glyph_hinting-html-report/index.html`

## Outcome

The Color Glyph Hinting harness records 5 rendered pages and 4 precise policy rows:

- COLRv1 supported subset.
- Korean hinting regression.
- Hebrew hinting regression.
- sbix PNG regression.
- CID-keyed CFF clipping regression.
- COLRv1 unsupported operator policy row.
- SVG static/security policy rows.
- bitmap non-PNG payload policy row.

The multi-reference audit records zero Wellfriend outlier failures and zero
unclassified failures.
