# Prompt 10D Full Color Glyph Rendering Closure

Prompt 10D is a focused closure pass for the remaining Prompt 10 color-glyph
items. It does not reopen parser, codec, transparency, shadings, annotations,
OCG, progressive cache, CMM, prepress, or Prompt 11 work.

## Implemented In Prompt 10D

- Safe static SVG-in-OpenType subset rendering for `svg`, `g`, `path`, `rect`,
  `circle`, `ellipse`, `line`, `polyline`, and `polygon`.
- Static SVG fill, stroke, stroke width, opacity, fill opacity, stroke opacity,
  and finite `matrix`, `translate`, `scale`, `rotate`, `skewX`, and `skewY`
  transforms.
- SVGZ gzip decoding with a decompressed size cap.
- `sbix` JPEG payload rendering through the existing bounded DCT decoder.
- `sbix` PNG regression preservation and existing CBDT/CBLC safe bitmap paths.
- Additive public report section
  `prompt10d_full_colrv1_svg_color_glyph_closure`.

## Exact Unsupported Rows

COLRv1 gradient, clip, and non-SourceOver composite operators remain exact
operator-level rows:

- `PaintLinearGradient`
- `PaintRadialGradient`
- `PaintSweepGradient`
- `PaintClip`
- `PaintClipBox`
- non-`SourceOver` `PaintComposite` modes

These are not silently replaced by monochrome outlines. They fail closed with
operator names because the current `ttf-parser` painter callback path does not
provide a bounded glyph paint-surface model for gradients, clips, and isolated
composites.

SVG active or dynamic constructs remain security-blocked. TIFF/PDF/mask/unknown
`sbix` payload tags remain unsupported when no existing safe decoder is
available.

## Artifacts

All artifacts are under:

```text
target/prompt10-cjk-rtl-color-glyph-reference/
```

Primary Prompt 10D artifacts:

- `prompt10d-closure-audit.json`
- `colrv1-linear-gradient-matrix-prompt10d.json`
- `colrv1-radial-gradient-matrix-prompt10d.json`
- `colrv1-sweep-gradient-matrix-prompt10d.json`
- `colrv1-clip-matrix-prompt10d.json`
- `colrv1-composite-matrix-prompt10d.json`
- `svg-opentype-static-rendering-matrix-prompt10d.json`
- `svg-opentype-security-policy-prompt10d.json`
- `bitmap-color-glyph-nonpng-matrix-prompt10d.json`
- `multi-reference-render-results-prompt10d.json`
- `multi-reference-diff-metrics-prompt10d.json`
- `reference-disagreement-summary-prompt10d.json`
- `prompt10d-html-report/index.html`
