# Prompt 10 Color Glyph Rendering

Prompt 10E closes the remaining safe COLRv1 color-glyph work with bounded
gradient, clip, and composite rendering plus exact mode, payload, or security
diagnostics for the rest.

## Implemented Rendering

COLR/CPAL v0 solid layered glyphs render through the vector glyph path. The
renderer preserves palette color, layer order, glyph transform, text matrix,
CTM, rise, scaling, graphics-state alpha, and clipping state.

COLR/CPAL v1 now supports the bounded vector paint subset:

- `PaintSolid`
- `PaintColrGlyph`
- `PaintTransform`
- `PaintTranslate`
- `PaintScale`
- `PaintRotate`
- `PaintSkew`
- `PaintLinearGradient`
- `PaintRadialGradient`
- `PaintSweepGradient`
- `PaintClip`
- `PaintClipBox`
- `PaintComposite` with `SourceOver` and the PDF blend modes already supported
  by Prompt 07/07B

Embedded bitmap color glyphs use the shared safe raster branch. PNG
`RasterGlyphImage` payloads and bounded bitmap strikes exposed by the font
parser are rendered with origin offsets, baseline placement, CTM/text transform,
graphics alpha, and clipping. `sbix` PNG strikes are implemented and covered by
the Prompt 10B and 10C synthetic sbix fixtures.

Prompt 10D adds `sbix` JPEG strike rendering through the existing bounded DCT
decoder, with CMYK JPEG converted through the existing color-space converter.
TIFF/PDF/mask/unknown `sbix` payload tags remain exact unsupported rows when no
safe decoder exists.

Prompt 10D also adds safe static SVG-in-OpenType rendering for simple paths,
basic shapes, finite transforms, fill/stroke, and opacity. It never executes
active SVG content.

## Exact Unsupported Rows

Unsupported COLRv1 composites are reported by mode name: `Clear`, `Source`,
`Destination`, `DestinationOver`, `SourceIn`, `DestinationIn`, `SourceOut`,
`DestinationOut`, `SourceAtop`, `DestinationAtop`, `Xor`, and `Plus`.
Moving-center radial gradients are implemented with a bounded deterministic
approximation until a full two-circle solver is added.

SVG-in-OpenType active or dynamic content remains blocked. Scripts, event
handlers, network/file/javascript URLs, `foreignObject`, animation, CSS imports,
remote fonts, external images, filters, masks, path bombs, and recursive
references are never executed or dereferenced.

Non-PNG or ambiguous color bitmap payloads are exact unsupported format rows:
CBDT/CBLC ambiguous compressed payloads and malformed strike tables, plus sbix
TIFF/PDF/mask and unknown `graphicType` payloads. Known unsupported color
payloads fail closed instead of silently falling back to monochrome outlines.

## Cache And Reports

Glyph cache keys include a color-glyph mode so monochrome outlines, COLR/CPAL
layers, raster color strikes, security-blocked SVG posture, and unsupported
bitmap payloads cannot alias.

Public feature reports expose the additive sections:

```text
prompt10b_color_glyph_cjk_rtl_fidelity_closure
prompt10c_color_glyph_hinting_cff_closure
prompt10d_full_colrv1_svg_color_glyph_closure
prompt10e_colrv1_gradient_clip_composite_closure
```
