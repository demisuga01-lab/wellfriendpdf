# Multilingual Color Glyphs Color Glyph Cache And Scheduler

Color-glyph rendering is kept separate from monochrome outline caching.

## Cache Posture

The glyph outline cache key includes a color-glyph mode so monochrome outlines,
COLR/CPAL, embedded bitmap strikes, SVG security-blocked glyphs, and known
unsupported bitmap payloads do not alias. COLRv1 paints are rendered from the
font table on each glyph invocation rather than cached as stale painted bitmaps,
so palette, gradient, clip, and composite changes are carried by the font bytes
and paint graph traversal.

Porterduff Radial Color Glyph records the final cache-key closure in
`colrv1-cache-key-porterduff_radial_color_glyph.json`.

## Scheduler Posture

COLRv1 paint graphs that need gradients, clips, or composites allocate a
transparent glyph paint surface through the renderer offscreen-surface scheduler
token path. Porter-Duff/Plus source paints also allocate scheduler-reserved
transparent source surfaces before compositing against the glyph-local backdrop.
Scheduler denial fails closed with a diagnostic before the surface is used.

The current implementation uses a scheduler-bounded render-sized surface and
clips paint loops to glyph/path bounds. Cropped glyph-space allocation remains a
future optimization.

Evidence:

- `colrv1-glyph-paint-surface-model-colrv_gradient_composite.json`
- `colrv1-cache-scheduler-matrix-colrv_gradient_composite.json`
- `colrv1-tile-band-progressive-equivalence-colrv_gradient_composite.json`
- `colrv1-determinism-report-colrv_gradient_composite.json`
- `colrv1-composite-scheduler-cache-porterduff_radial_color_glyph.json`
- `colrv1-cache-key-porterduff_radial_color_glyph.json`
- `colrv1-scheduler-memory-porterduff_radial_color_glyph.json`
- `colrv1-determinism-porterduff_radial_color_glyph.json`
