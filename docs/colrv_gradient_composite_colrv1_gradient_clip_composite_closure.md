# Colrv Gradient Composite COLRv1 Gradient, Clip, And Composite Closure

Colrv Gradient Composite closes the remaining Multilingual Color Glyphs color-glyph renderer blockers for
COLRv1 gradient, clip, and blend/composite behavior.

Current status note: Porterduff Radial Color Glyph supersedes the Colrv Gradient Composite remaining limits by
adding Porter-Duff/Plus composite rendering and exact moving-center radial
gradient solving.

## Implemented

- `PaintLinearGradient` with bounded stop count, palette colors, alpha stops,
  finite coordinate checks, and pad/repeat/reflect handling.
- `PaintRadialGradient` with same-center circle handling and a bounded
  deterministic moving-center fallback that Porterduff Radial Color Glyph replaces with the exact
  two-circle solver.
- `PaintSweepGradient` with deterministic angular interpolation.
- `PaintClip` through real glyph-outline clip masks.
- `PaintClipBox` through transformed COLR ClipList boxes.
- Non-`SourceOver` blend composites that map to the existing Transparency Rendering blend
  machinery: Multiply, Screen, Overlay, Darken, Lighten, ColorDodge, ColorBurn,
  HardLight, SoftLight, Difference, Exclusion, Hue, Saturation, Color, and
  Luminosity.
- Scheduler-bounded glyph paint surfaces for COLRv1 paint graph execution.

## Evidence

The Colrv Gradient Composite harness generates self-contained COLRv1 fonts/PDFs and compares
Wellfriend, Poppler, PDFium, and MuPDF output.

Primary artifacts:

- `colrv_gradient_composite-closure-audit.json`
- `colrv1-glyph-paint-surface-model-colrv_gradient_composite.json`
- `colrv1-gradient-matrix-colrv_gradient_composite.json`
- `colrv1-gradient-reference-results-colrv_gradient_composite.json`
- `colrv1-clip-stack-matrix-colrv_gradient_composite.json`
- `colrv1-clip-reference-results-colrv_gradient_composite.json`
- `colrv1-composite-surface-matrix-colrv_gradient_composite.json`
- `colrv1-composite-reference-results-colrv_gradient_composite.json`
- `colrv1-cache-scheduler-matrix-colrv_gradient_composite.json`
- `multi-reference-render-results-colrv_gradient_composite.json`
- `multi-reference-diff-metrics-colrv_gradient_composite.json`
- `reference-disagreement-summary-colrv_gradient_composite.json`
- `colrv_gradient_composite-html-report/index.html`

Colrv Gradient Composite rendered 17 pages, classified 24 rendered/policy fixture rows, and
recorded 0 Wellfriend outlier failures and 0 unclassified failures.

## Superseded Colrv Gradient Composite Limits

- Porter-Duff `Clear`, `Source`, `Destination`, `DestinationOver`, `SourceIn`,
  `DestinationIn`, `SourceOut`, `DestinationOut`, `SourceAtop`,
  `DestinationAtop`, `Xor`, and `Plus` were Colrv Gradient Composite exact mode rows and are
  implemented by Porterduff Radial Color Glyph.
- Moving-center radial gradients used a bounded deterministic Colrv Gradient Composite
  fallback and are implemented by the Porterduff Radial Color Glyph two-circle solver.
- COLRv1 glyph paint surfaces are scheduler-bounded full render buffers. Cropped
  glyph-space allocation remains an optimization, not a Multilingual Color Glyphs correctness
  blocker.
