# Multilingual Color Glyphs CID-Keyed CFF Clipping

Color Glyph Hinting keeps the no-bbox-fake clipping rule. CID clipping is only claimed
when the renderer can build a real glyph path from encoded bytes through CMap,
CID/GID mapping, font data, FontMatrix, text matrix, CTM, rise, and scaling.

## Supported Path

Common CID text clipping remains supported where the font subsystem exposes the
glyph outline path, including the Identity-H TrueType CID path covered by the
earlier text-clipping campaign. CJK RTL Color Glyph Closeout/10C route available outlines by
resolved glyph id so supported vector glyphs contribute real clipping geometry.

## Color Glyph Hinting Narrowing

Color Glyph Hinting audits the exotic CFF closure surface as real geometry only:

- FDArray and FDSelect selection must be diagnosable.
- Local/global subr access and subr bias must be bounded.
- Charset/CID mapping and `CIDToGIDMap` interaction must be recorded.
- `defaultWidthX`, `nominalWidthX`, FontMatrix, text matrix, CTM, rise, and
  scaling must stay in the clipping transform chain.
- Charstring recursion and malformed data fail closed.

Unsupported rows are now limited to missing or unsafe charstring path geometry,
malformed subr recursion/depth, or unsupported FDSelect/FDArray exposure. The
diagnostic row records font object, subtype, CID, GID, FD index, and reason where
available. Bounding-box clipping is still forbidden as a substitute.

Evidence:

- `cid-keyed-cff-clipping-matrix-cjk_rtl_color_glyph_closeout.json`
- `cid-keyed-cff-reference-results-cjk_rtl_color_glyph_closeout.json`
- `cid-keyed-cff-clipping-matrix-color_glyph_hinting.json`
- `cid-keyed-cff-clipping-reference-results-color_glyph_hinting.json`
- `cjk_rtl_color_glyph_closeout-reference-disagreement-summary.json`
- `reference-disagreement-summary-color_glyph_hinting.json`
