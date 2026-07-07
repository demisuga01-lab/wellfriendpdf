# Prompt 10 COLRv1 Paint Graphs

Prompt 10C implements the safe COLRv1 subset that maps directly to existing
vector glyph painting.

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

## Unsupported Operators

These operators are exact unsupported rows:

- `PaintLinearGradient`
- `PaintRadialGradient`
- `PaintSweepGradient`
- `PaintClip`
- `PaintClipBox`
- non-`SourceOver` `PaintComposite`

Unsupported operators fail closed with diagnostics and do not silently flatten
to a monochrome fallback.

Evidence:

- `color-glyph-colrv1-matrix-prompt10c.json`
- `color-glyph-colrv1-reference-results-prompt10c.json`
