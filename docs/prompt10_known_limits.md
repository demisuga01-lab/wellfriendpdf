# Prompt 10 Known Limits

Prompt 10D narrows the remaining Prompt 10 limits to exact COLRv1 paint
operators, SVG security/static-subset boundaries, bitmap decoder availability,
or missing safe geometry exposure.

## Remaining Limits

- COLRv1 gradients and clip operators are not approximated:
  `PaintLinearGradient`, `PaintRadialGradient`, `PaintSweepGradient`,
  `PaintClip`, `PaintClipBox`, and non-`SourceOver` composites are exact
  unsupported operator rows.
- SVG-in-OpenType active or dynamic content is blocked. Scripts, event handlers,
  network/file/javascript URLs, `foreignObject`, animation, CSS imports, remote
  fonts, external images, filters, masks, path bombs, and recursive references
  are not executed or fetched.
- Safe static SVG path and shape glyphs render through Oxide's path painter.
  SVG gradients, `clipPath`, filters, masks, recursive `<use>`, CSS blocks, and
  URL paint servers remain exact unsupported/security rows.
- CBDT/CBLC non-PNG or ambiguous compressed payloads are unsupported unless the
  font parser exposes bounded safe bitmap metadata.
- sbix TIFF/PDF/mask and unknown `graphicType` payloads are exact unsupported
  format rows when no existing safe decoder is available.
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
- sbix JPEG color glyph rendering is implemented through the existing bounded
  DCT decoder.
- Safe static SVG-in-OpenType path and shape rendering is implemented.
- The shared bounded embedded-bitmap glyph path supports safe bitmap payloads
  exposed by the font parser.
- Korean and Hebrew rendered-page fixture gaps are closed.
- Prompt 10B, Prompt 10C, and Prompt 10D multi-reference audits have zero Oxide outlier
  failures and zero unclassified failures.
