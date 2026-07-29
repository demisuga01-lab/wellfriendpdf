# Colrv Svg Bitmap COLRv1, SVG, And Bitmap Closure Audit

Colrv Svg Bitmap classifies each remaining Color Glyph Hinting blocker with no broad
unsupported buckets.

Current status note: Colrv Gradient Composite supersedes the COLRv1 gradient and clip rows,
and Porterduff Radial Color Glyph supersedes the remaining Porter-Duff/Plus composite and
moving-center radial rows. This table remains historical evidence for Roadmap task
10D.

| Item | Status | Evidence |
| --- | --- | --- |
| COLRv1 `PaintLinearGradient` | `unsupported_reported_exotic_operator` | `colrv1-linear-gradient-matrix-colrv_svg_bitmap.json` |
| COLRv1 `PaintRadialGradient` | `unsupported_reported_exotic_operator` | `colrv1-radial-gradient-matrix-colrv_svg_bitmap.json` |
| COLRv1 `PaintSweepGradient` | `unsupported_reported_exotic_operator` | `colrv1-sweep-gradient-matrix-colrv_svg_bitmap.json` |
| COLRv1 `PaintClip` / `PaintClipBox` | `unsupported_reported_exotic_operator` | `colrv1-clip-matrix-colrv_svg_bitmap.json` |
| COLRv1 non-`SourceOver` composites | superseded by Porterduff Radial Color Glyph | `colrv1-composite-matrix-colrv_svg_bitmap.json` |
| SVG-in-OpenType safe static rendering | `implemented` | `svg-opentype-static-rendering-matrix-colrv_svg_bitmap.json` |
| SVG active/dynamic constructs | `unsupported_reported_security_policy` | `svg-opentype-security-policy-colrv_svg_bitmap.json` |
| CBDT/CBLC non-PNG exposed raw/gray/color strikes | `implemented_with_limits` | `bitmap-color-glyph-nonpng-matrix-colrv_svg_bitmap.json` |
| `sbix` JPEG payloads | `implemented` | `sbix-nonpng-results-colrv_svg_bitmap.json` |
| `sbix` TIFF/PDF/mask/unknown tags | `unsupported_reported_no_safe_decoder` | `bitmap-color-glyph-nonpng-matrix-colrv_svg_bitmap.json` |
| malformed bitmap payloads | `implemented_with_limits` | `bitmap-color-glyph-nonpng-matrix-colrv_svg_bitmap.json` |
| multi-reference audit | `implemented` | `reference-disagreement-summary-colrv_svg_bitmap.json` |
| public report parity | `implemented` | `public-feature-report-colrv_svg_bitmap.json` |

The machine-readable version is
`target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/colrv_svg_bitmap-closure-audit.json`.
