# Prompt 10E COLRv1 Gradient, Clip, And Composite Closure

Prompt 10E closes the remaining Prompt 10 color-glyph renderer blockers for
COLRv1 gradient, clip, and blend/composite behavior.

## Implemented

- `PaintLinearGradient` with bounded stop count, palette colors, alpha stops,
  finite coordinate checks, and pad/repeat/reflect handling.
- `PaintRadialGradient` with same-center circle handling and a bounded
  deterministic moving-center approximation.
- `PaintSweepGradient` with deterministic angular interpolation.
- `PaintClip` through real glyph-outline clip masks.
- `PaintClipBox` through transformed COLR ClipList boxes.
- Non-`SourceOver` blend composites that map to the existing Prompt 07 blend
  machinery: Multiply, Screen, Overlay, Darken, Lighten, ColorDodge, ColorBurn,
  HardLight, SoftLight, Difference, Exclusion, Hue, Saturation, Color, and
  Luminosity.
- Scheduler-bounded glyph paint surfaces for COLRv1 paint graph execution.

## Evidence

The Prompt 10E harness generates self-contained COLRv1 fonts/PDFs and compares
Oxide, Poppler, PDFium, and MuPDF output.

Primary artifacts:

- `prompt10e-closure-audit.json`
- `colrv1-glyph-paint-surface-model-prompt10e.json`
- `colrv1-gradient-matrix-prompt10e.json`
- `colrv1-gradient-reference-results-prompt10e.json`
- `colrv1-clip-stack-matrix-prompt10e.json`
- `colrv1-clip-reference-results-prompt10e.json`
- `colrv1-composite-surface-matrix-prompt10e.json`
- `colrv1-composite-reference-results-prompt10e.json`
- `colrv1-cache-scheduler-matrix-prompt10e.json`
- `multi-reference-render-results-prompt10e.json`
- `multi-reference-diff-metrics-prompt10e.json`
- `reference-disagreement-summary-prompt10e.json`
- `prompt10e-html-report/index.html`

Prompt 10E rendered 17 pages, classified 24 rendered/policy fixture rows, and
recorded 0 Oxide outlier failures and 0 unclassified failures.

## Exact Remaining Limits

- Porter-Duff `Clear`, `Source`, `Destination`, `DestinationOver`, `SourceIn`,
  `DestinationIn`, `SourceOut`, `DestinationOut`, `SourceAtop`,
  `DestinationAtop`, `Xor`, and `Plus` are exact unsupported composite-mode
  rows.
- Moving-center radial gradients use a bounded deterministic approximation
  until a full COLRv1 two-circle solver is added.
- COLRv1 glyph paint surfaces are scheduler-bounded full render buffers. Cropped
  glyph-space allocation remains an optimization, not a Prompt 10 correctness
  blocker.
