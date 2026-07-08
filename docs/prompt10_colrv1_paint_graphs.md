# Prompt 10 COLRv1 Paint Graphs

Prompt 10C implemented the initial safe COLRv1 solid/transform subset. Prompt
10D preserved that behavior while SVG and bitmap color glyph closure proceeded.
Prompt 10E adds the bounded glyph paint-surface model needed for COLRv1
gradients, clips, and PDF blend-mode composites.

## Implemented Operators

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
- `PaintClip` through real glyph-outline clip masks
- `PaintClipBox` through COLR ClipList boxes
- `PaintComposite` for `SourceOver`, Multiply, Screen, Overlay, Darken,
  Lighten, ColorDodge, ColorBurn, HardLight, SoftLight, Difference, Exclusion,
  Hue, Saturation, Color, and Luminosity

Each painted layer preserves palette color, alpha, glyph transform, text matrix,
CTM, rise, scaling, and clipping context. The renderer applies finite-transform
checks and caps transform depth, paint layer count, and gradient stop count.

## Prompt 10E Exact Unsupported Modes

The remaining unsupported COLRv1 rows are exact composite-mode rows:

- `Clear`
- `Source`
- `Destination`
- `DestinationOver`
- `SourceIn`
- `DestinationIn`
- `SourceOut`
- `DestinationOut`
- `SourceAtop`
- `DestinationAtop`
- `Xor`
- `Plus`

Unsupported modes fail closed with diagnostics and do not silently flatten to a
monochrome fallback. These modes need source/backdrop ownership semantics that
are not equivalent to the existing Prompt 07 PDF blend-mode machinery.

Evidence:

- `color-glyph-colrv1-matrix-prompt10c.json`
- `color-glyph-colrv1-reference-results-prompt10c.json`
- `colrv1-gradient-matrix-prompt10e.json`
- `colrv1-gradient-reference-results-prompt10e.json`
- `colrv1-clip-stack-matrix-prompt10e.json`
- `colrv1-clip-reference-results-prompt10e.json`
- `colrv1-composite-surface-matrix-prompt10e.json`
- `colrv1-composite-reference-results-prompt10e.json`
