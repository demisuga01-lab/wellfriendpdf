# Prompt 10 COLRv1 Paint Graphs

Prompt 10C implemented the initial safe COLRv1 solid/transform subset. Prompt
10D preserved that behavior while SVG and bitmap color glyph closure proceeded.
Prompt 10E added the bounded glyph paint-surface model needed for COLRv1
gradients, clips, and PDF blend-mode composites. Prompt 10F closes the remaining
Porter-Duff/Plus composite modes and replaces the moving-center radial
approximation with an analytic two-circle solve.

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
- `PaintComposite` for `SourceOver`, Porter-Duff `Clear`, `Source`,
  `Destination`, `DestinationOver`, `SourceIn`, `DestinationIn`, `SourceOut`,
  `DestinationOut`, `SourceAtop`, `DestinationAtop`, `Xor`, additive `Plus`,
  and the PDF blend modes Multiply, Screen, Overlay, Darken, Lighten,
  ColorDodge, ColorBurn, HardLight, SoftLight, Difference, Exclusion, Hue,
  Saturation, Color, and Luminosity

Each painted layer preserves palette color, alpha, glyph transform, text matrix,
CTM, rise, scaling, and clipping context. The renderer applies finite-transform
checks and caps transform depth, paint layer count, and gradient stop count.

## Prompt 10F Closure

No Prompt 10 COLRv1 composite operator remains broadly unsupported. Porter-Duff
and `Plus` source paints render into scheduler-reserved transparent source
surfaces and are composited against the current glyph-local backdrop with exact
premultiplied-alpha equations.

Moving-center radial gradients solve the two-circle equation per covered pixel
and then apply the COLRv1 pad/repeat/reflect stop behavior.

Evidence:

- `color-glyph-colrv1-matrix-prompt10c.json`
- `color-glyph-colrv1-reference-results-prompt10c.json`
- `colrv1-gradient-matrix-prompt10e.json`
- `colrv1-gradient-reference-results-prompt10e.json`
- `colrv1-clip-stack-matrix-prompt10e.json`
- `colrv1-clip-reference-results-prompt10e.json`
- `colrv1-composite-surface-matrix-prompt10e.json`
- `colrv1-composite-reference-results-prompt10e.json`
- `colrv1-porterduff-composite-matrix-prompt10f.json`
- `colrv1-porterduff-composite-reference-results-prompt10f.json`
- `colrv1-exact-radial-gradient-matrix-prompt10f.json`
- `colrv1-exact-radial-gradient-reference-results-prompt10f.json`
