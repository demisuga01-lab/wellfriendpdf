# Prompt 10C Color Glyph, Hinting, and CFF Closure

Prompt 10C closes the remaining strict Prompt 10 blockers without reopening
unrelated renderer phases.

## Scope

Prompt 10C covers:

- COLRv1 bounded paint graph rendering.
- SVG-in-OpenType static subset classification and security blocking.
- non-PNG color bitmap payload diagnostics.
- pure-Rust hinting reference-cluster proof.
- exotic CID-keyed CFF clipping diagnostics.
- Poppler/PDFium/MuPDF/Wellfriend audit for the Prompt 10C corpus.

It does not start native CMM, prepress, standards, parser, codec, annotation,
OCG, shadings, patterns, progressive cache, or release-hardening work.

## Artifacts

All Prompt 10C artifacts live under:

```text
target/prompt10-cjk-rtl-color-glyph-reference/
```

Primary artifacts:

- `prompt10c-closure-audit.json`
- `color-glyph-colrv1-matrix-prompt10c.json`
- `color-glyph-colrv1-reference-results-prompt10c.json`
- `color-glyph-svg-static-subset-matrix-prompt10c.json`
- `color-glyph-svg-security-policy-prompt10c.json`
- `color-glyph-svg-reference-results-prompt10c.json`
- `color-glyph-bitmap-payload-matrix-prompt10c.json`
- `color-glyph-cbdt-cblc-results-prompt10c.json`
- `color-glyph-sbix-results-prompt10c.json`
- `hinting-posture-prompt10c.json`
- `cid-keyed-cff-clipping-matrix-prompt10c.json`
- `cid-keyed-cff-clipping-reference-results-prompt10c.json`
- `multi-reference-render-results-prompt10c.json`
- `multi-reference-diff-metrics-prompt10c.json`
- `reference-disagreement-summary-prompt10c.json`
- `prompt10c-html-report/index.html`

## Outcome

The Prompt 10C harness records 5 rendered pages and 4 precise policy rows:

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
