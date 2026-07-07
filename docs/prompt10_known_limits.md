# Prompt 10 Known Limits

Prompt 10C narrows the remaining Prompt 10 limits to exact operators, security
boundaries, or missing safe geometry exposure.

## Remaining Limits

- COLRv1 gradients and clip operators are not approximated:
  `PaintLinearGradient`, `PaintRadialGradient`, `PaintSweepGradient`,
  `PaintClip`, `PaintClipBox`, and non-`SourceOver` composites are exact
  unsupported operator rows.
- SVG-in-OpenType active or dynamic content is blocked. Scripts, event handlers,
  network/file/javascript URLs, `foreignObject`, animation, CSS imports, remote
  fonts, external images, filters, masks, path bombs, and recursive references
  are not executed or fetched.
- Safe static SVG candidates are classified but not routed through a general SVG
  rendering engine in Prompt 10C.
- CBDT/CBLC non-PNG or ambiguous compressed payloads are unsupported unless the
  font parser exposes bounded safe bitmap metadata.
- sbix JPEG/TIFF/PDF/mask and unknown `graphicType` payloads are exact
  unsupported format rows.
- CID-keyed CFF clipping remains unsupported only when safe charstring-derived
  outline geometry is unavailable or malformed. Bounding-box clipping is never
  used as a substitute.
- Optional native hinting remains a future feature-gated enhancement, not a
  default runtime dependency.

## Not Limits

- COLR/CPAL v0 solid layered glyph rendering is implemented.
- COLRv1 `PaintSolid`, `PaintColrGlyph`, transform operators, and `SourceOver`
  composition are implemented with depth and layer caps.
- sbix PNG color glyph rendering is implemented.
- The shared bounded embedded-bitmap glyph path supports safe bitmap payloads
  exposed by the font parser.
- Korean and Hebrew rendered-page fixture gaps are closed.
- Prompt 10B and Prompt 10C multi-reference audits have zero Oxide outlier
  failures and zero unclassified failures.
