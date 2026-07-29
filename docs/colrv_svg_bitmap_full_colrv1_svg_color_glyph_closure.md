# Colrv Svg Bitmap Full Color Glyph Rendering Closure

Colrv Svg Bitmap is a focused closure pass for the remaining Multilingual Color Glyphs color-glyph
items. It does not reopen parser, codec, transparency, shadings, annotations,
OCG, progressive cache, CMM, prepress, or Renderer Fuzz CMM work.

## Implemented In Colrv Svg Bitmap

- Safe static SVG-in-OpenType subset rendering for `svg`, `g`, `path`, `rect`,
  `circle`, `ellipse`, `line`, `polyline`, and `polygon`.
- Static SVG fill, stroke, stroke width, opacity, fill opacity, stroke opacity,
  and finite `matrix`, `translate`, `scale`, `rotate`, `skewX`, and `skewY`
  transforms.
- SVGZ gzip decoding with a decompressed size cap.
- `sbix` JPEG payload rendering through the existing bounded DCT decoder.
- `sbix` PNG regression preservation and existing CBDT/CBLC safe bitmap paths.
- Additive public report section
  `colrv_svg_bitmap_full_colrv1_svg_color_glyph_closure`.

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
target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/
```

Primary Colrv Svg Bitmap artifacts:

- `colrv_svg_bitmap-closure-audit.json`
- `colrv1-linear-gradient-matrix-colrv_svg_bitmap.json`
- `colrv1-radial-gradient-matrix-colrv_svg_bitmap.json`
- `colrv1-sweep-gradient-matrix-colrv_svg_bitmap.json`
- `colrv1-clip-matrix-colrv_svg_bitmap.json`
- `colrv1-composite-matrix-colrv_svg_bitmap.json`
- `svg-opentype-static-rendering-matrix-colrv_svg_bitmap.json`
- `svg-opentype-security-policy-colrv_svg_bitmap.json`
- `bitmap-color-glyph-nonpng-matrix-colrv_svg_bitmap.json`
- `multi-reference-render-results-colrv_svg_bitmap.json`
- `multi-reference-diff-metrics-colrv_svg_bitmap.json`
- `reference-disagreement-summary-colrv_svg_bitmap.json`
- `colrv_svg_bitmap-html-report/index.html`
