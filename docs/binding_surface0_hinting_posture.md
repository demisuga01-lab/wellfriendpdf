# Multilingual Color Glyphs Hinting Posture

Color Glyph Hinting keeps the default renderer pure Rust and records a reference-cluster
acceptance proof instead of adding a native hinting dependency.

No native hinting backend is added, no native dependency is enabled silently, and
WASM/default builds remain portable. Feature reports expose the active posture
as `pure_rust_analytic_aa`; optional native hinting remains a future
feature-gated enhancement, not a Multilingual Color Glyphs blocker.

What the page renderer actually does with glyph outlines:

- Font hinting programs (TrueType bytecode, Type 1/CFF hints) are never
  executed. This matches Poppler, whose default is "no hinting".
- Filled glyphs from sfnt (TrueType and OpenType) and Type 1 programs, and
  glyphs drawn from fallback faces, get a light grid-fit
  (`GlyphHinting::light`) when the text is axis-aligned and 7 to 32 device
  pixels per em: the baseline moves by at most 0.35 px to a whole pixel and
  outline points within 0.18 px of a pixel edge snap to it. Outside that size
  range, for rotated or skewed text, for bare CFF (FontFile3) programs, and
  for stroked or clipping text, outlines are rasterized unmodified.
- `FontRasterizer::rasterize_glyph`, the standalone single-glyph helper, does
  not grid-fit at all.

Evidence:

- `hinting-posture-cjk_rtl_color_glyph_closeout.json`
- `hinting-posture-color_glyph_hinting.json`
- `cjk_rtl_color_glyph_closeout-multi-reference-diff-metrics.json`
- `cjk_rtl_color_glyph_closeout-reference-disagreement-summary.json`
- `multi-reference-diff-metrics-color_glyph_hinting.json`
- `reference-disagreement-summary-color_glyph_hinting.json`

The Color Glyph Hinting rendered corpus includes Korean, Hebrew, COLRv1, sbix, and
CID-keyed CFF regression rows. It records zero Wellfriend outlier failures and zero
unclassified failures under the pure-Rust posture.
