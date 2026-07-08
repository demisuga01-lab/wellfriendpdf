# Prompt 10 COLRv1 Paint Graphs

Prompt 10C implements the safe COLRv1 subset that maps directly to existing
vector glyph painting. Prompt 10D preserves that behavior and keeps the
remaining paint operators as exact unsupported rows when a bounded glyph
paint-surface model is required.

## Implemented Operators

- `PaintSolid`
- `PaintColrGlyph`
- `PaintTransform`
- `PaintTranslate`
- `PaintScale`
- `PaintRotate`
- `PaintSkew`
- `PaintComposite` when the composite mode is `SourceOver`

Each painted layer preserves palette color, alpha, glyph transform, text matrix,
CTM, rise, scaling, and clipping context. The renderer applies finite-transform
checks and caps transform depth and paint layer count.

## Prompt 10D Exact Unsupported Operators

These operators are exact unsupported rows:

- `PaintLinearGradient`
- `PaintRadialGradient`
- `PaintSweepGradient`
- `PaintClip`
- `PaintClipBox`
- non-`SourceOver` `PaintComposite`

Unsupported operators fail closed with diagnostics and do not silently flatten
to a monochrome fallback. Gradients, clip stacks, and non-SourceOver composites
need a bounded COLRv1 paint tree/offscreen glyph surface before Oxide can reuse
the renderer's shading, clipping, and Prompt 07 blend machinery.

Evidence:

- `color-glyph-colrv1-matrix-prompt10c.json`
- `color-glyph-colrv1-reference-results-prompt10c.json`
- `colrv1-linear-gradient-matrix-prompt10d.json`
- `colrv1-radial-gradient-matrix-prompt10d.json`
- `colrv1-sweep-gradient-matrix-prompt10d.json`
- `colrv1-clip-matrix-prompt10d.json`
- `colrv1-composite-matrix-prompt10d.json`
