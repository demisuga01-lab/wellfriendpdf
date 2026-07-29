# Colrv Gradient Composite COLRv1 Gradient, Clip, And Composite Closure Audit

Colrv Gradient Composite classifies the remaining Colrv Svg Bitmap blockers with no `blocked` rows.

| Blocker | Status | Evidence |
| --- | --- | --- |
| `PaintLinearGradient` | `implemented` | `colrv1-gradient-matrix-colrv_gradient_composite.json` |
| `PaintRadialGradient` | `implemented_with_limits` | `colrv1-gradient-limit-diagnostics-colrv_gradient_composite.json` |
| `PaintSweepGradient` | `implemented` | `colrv1-gradient-matrix-colrv_gradient_composite.json` |
| `PaintClip` | `implemented` | `colrv1-clip-stack-matrix-colrv_gradient_composite.json` |
| `PaintClipBox` | `implemented` | `colrv1-clip-stack-matrix-colrv_gradient_composite.json` |
| non-`SourceOver` `PaintComposite` | `implemented_with_limits` | `colrv1-composite-surface-matrix-colrv_gradient_composite.json` |
| isolated glyph paint surfaces | `implemented_with_limits` | `colrv1-glyph-paint-surface-model-colrv_gradient_composite.json` |
| glyph paint clip stack | `implemented` | `colrv1-clip-stack-matrix-colrv_gradient_composite.json` |
| glyph paint cache posture | `implemented_with_limits` | `colrv1-cache-scheduler-matrix-colrv_gradient_composite.json` |
| scheduler admission | `implemented` | `colrv1-glyph-paint-surface-model-colrv_gradient_composite.json` |
| malformed/deep/cyclic COLRv1 graphs | `unsupported_reported_security_or_safety_policy` | `colrv1-gradient-limit-diagnostics-colrv_gradient_composite.json` |
| multi-reference audit | `implemented` | `reference-disagreement-summary-colrv_gradient_composite.json` |
| public report parity | `implemented` | `public-feature-report-colrv_gradient_composite.json` |

The machine-readable audit is
`target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/colrv_gradient_composite-closure-audit.json`.
