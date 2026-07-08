# Prompt 10D COLRv1, SVG, And Bitmap Closure Audit

Prompt 10D classifies each remaining Prompt 10C blocker with no broad
unsupported buckets.

Current status note: Prompt 10E supersedes the COLRv1 gradient and clip rows,
and Prompt 10F supersedes the remaining Porter-Duff/Plus composite and
moving-center radial rows. This table remains historical evidence for Prompt
10D.

| Item | Status | Evidence |
| --- | --- | --- |
| COLRv1 `PaintLinearGradient` | `unsupported_reported_exotic_operator` | `colrv1-linear-gradient-matrix-prompt10d.json` |
| COLRv1 `PaintRadialGradient` | `unsupported_reported_exotic_operator` | `colrv1-radial-gradient-matrix-prompt10d.json` |
| COLRv1 `PaintSweepGradient` | `unsupported_reported_exotic_operator` | `colrv1-sweep-gradient-matrix-prompt10d.json` |
| COLRv1 `PaintClip` / `PaintClipBox` | `unsupported_reported_exotic_operator` | `colrv1-clip-matrix-prompt10d.json` |
| COLRv1 non-`SourceOver` composites | superseded by Prompt 10F | `colrv1-composite-matrix-prompt10d.json` |
| SVG-in-OpenType safe static rendering | `implemented` | `svg-opentype-static-rendering-matrix-prompt10d.json` |
| SVG active/dynamic constructs | `unsupported_reported_security_policy` | `svg-opentype-security-policy-prompt10d.json` |
| CBDT/CBLC non-PNG exposed raw/gray/color strikes | `implemented_with_limits` | `bitmap-color-glyph-nonpng-matrix-prompt10d.json` |
| `sbix` JPEG payloads | `implemented` | `sbix-nonpng-results-prompt10d.json` |
| `sbix` TIFF/PDF/mask/unknown tags | `unsupported_reported_no_safe_decoder` | `bitmap-color-glyph-nonpng-matrix-prompt10d.json` |
| malformed bitmap payloads | `implemented_with_limits` | `bitmap-color-glyph-nonpng-matrix-prompt10d.json` |
| multi-reference audit | `implemented` | `reference-disagreement-summary-prompt10d.json` |
| public report parity | `implemented` | `public-feature-report-prompt10d.json` |

The machine-readable version is
`target/prompt10-cjk-rtl-color-glyph-reference/prompt10d-closure-audit.json`.
