# Prompt 10 Color Glyph Cache And Scheduler

Color-glyph rendering is kept separate from monochrome outline caching.

## Cache Posture

The glyph outline cache key includes a color-glyph mode so monochrome outlines,
COLR/CPAL, embedded bitmap strikes, SVG security-blocked glyphs, and known
unsupported bitmap payloads do not alias. COLRv1 paints are rendered from the
font table on each glyph invocation rather than cached as stale painted bitmaps,
so palette, gradient, clip, and composite changes are carried by the font bytes
and paint graph traversal.

Prompt 10E records this in
`colrv1-cache-scheduler-matrix-prompt10e.json`.

## Scheduler Posture

COLRv1 paint graphs that need gradients, clips, or composites allocate a
transparent glyph paint surface through the renderer offscreen-surface scheduler
token path. Scheduler denial fails closed with a diagnostic before the surface
is used.

The current implementation uses a scheduler-bounded render-sized surface and
clips paint loops to glyph/path bounds. Cropped glyph-space allocation remains a
future optimization.

Evidence:

- `colrv1-glyph-paint-surface-model-prompt10e.json`
- `colrv1-cache-scheduler-matrix-prompt10e.json`
- `colrv1-tile-band-progressive-equivalence-prompt10e.json`
- `colrv1-determinism-report-prompt10e.json`
