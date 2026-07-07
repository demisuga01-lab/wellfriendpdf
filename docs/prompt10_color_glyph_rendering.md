# Prompt 10 Color Glyph Rendering

Prompt 10C closes the remaining color-glyph work with bounded rendering for the
safe cases and exact operator or payload diagnostics for the rest.

## Implemented Rendering

COLR/CPAL v0 solid layered glyphs render through the vector glyph path. The
renderer preserves palette color, layer order, glyph transform, text matrix,
CTM, rise, scaling, graphics-state alpha, and clipping state.

COLR/CPAL v1 now supports the bounded solid/vector subset:

- `PaintSolid`
- `PaintColrGlyph`
- `PaintTransform`
- `PaintTranslate`
- `PaintScale`
- `PaintRotate`
- `PaintSkew`
- `PaintComposite` with `SourceOver`

Embedded bitmap color glyphs use the shared safe raster branch. PNG
`RasterGlyphImage` payloads and bounded bitmap strikes exposed by the font
parser are rendered with origin offsets, baseline placement, CTM/text transform,
graphics alpha, and clipping. `sbix` PNG strikes are implemented and covered by
the Prompt 10B and 10C synthetic sbix fixtures.

## Exact Unsupported Rows

Unsupported COLRv1 operators are reported by operator name:
`PaintLinearGradient`, `PaintRadialGradient`, `PaintSweepGradient`, `PaintClip`,
`PaintClipBox`, and non-`SourceOver` `PaintComposite`.

SVG-in-OpenType is classified by the static-subset policy. Safe static
candidates are identified, but active or dynamic SVG remains blocked. Scripts,
event handlers, network/file/javascript URLs, `foreignObject`, animation, CSS
imports, remote fonts, external images, filters, masks, path bombs, and
recursive references are never executed or dereferenced.

Non-PNG or ambiguous color bitmap payloads are exact unsupported format rows:
CBDT/CBLC ambiguous compressed payloads and malformed strike tables, plus sbix
JPEG/TIFF/PDF/mask and unknown `graphicType` payloads. Known unsupported color
payloads fail closed instead of silently falling back to monochrome outlines.

## Cache And Reports

Glyph cache keys include a color-glyph mode so monochrome outlines, COLR/CPAL
layers, raster color strikes, security-blocked SVG posture, and unsupported
bitmap payloads cannot alias.

Public feature reports expose the additive sections:

```text
prompt10b_color_glyph_cjk_rtl_fidelity_closure
prompt10c_color_glyph_hinting_cff_closure
```
