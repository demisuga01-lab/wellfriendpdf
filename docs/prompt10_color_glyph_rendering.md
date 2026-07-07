# Prompt 10 Color Glyph Rendering

Prompt 10B changes the color glyph posture from detection-only to bounded
rendering for safe OpenType color glyph formats.

## Implemented

COLR/CPAL v0 solid layered glyphs are rendered through the existing vector glyph
path. The renderer uses the font palette, preserves layer order, paints each
layer outline with its solid color, applies graphics-state alpha, and honors the
text matrix, CTM, rise, scaling, and clipping state.

Embedded bitmap color glyphs are supported through the shared raster color glyph
path. The renderer can use bounded PNG payloads, bounded premultiplied BGRA
payloads, and bounded grayscale/mono bitmap payloads exposed by the font parser.
The glyph image transform preserves strike ppem, origin offsets, baseline
placement, text transform, clipping, and graphics alpha.

sbix PNG glyphs are implemented and covered by the Prompt 10B synthetic sbix
fixture. The fixture includes a PNG strike, scaling, offset handling, and a
clipping interaction.

## Unsupported With Narrow Policy

COLR/CPAL v1 remains an exotic unsupported case unless the paint graph is
equivalent to the supported solid-layer COLRv0 path. Gradients, nested paint
graphs, transforms, and compositing are not silently approximated.

SVG-in-OpenType is security-blocked. SVG glyph documents are not executed, and
scripts, event handlers, external references, network fetches, foreignObject,
animation, and remote resources remain blocked. Future support must be a static
no-network subset.

Malformed, oversized, unsupported-image, or incomplete bitmap color glyph
payloads fail closed with diagnostics rather than falling back silently to a
monochrome approximation.

## Cache And Reports

Glyph cache keys include a color-glyph mode so monochrome outlines, COLR/CPAL
layers, raster color strikes, and security-blocked SVG posture cannot alias.

Public feature reports expose the additive section:

```text
prompt10b_color_glyph_cjk_rtl_fidelity_closure
```
