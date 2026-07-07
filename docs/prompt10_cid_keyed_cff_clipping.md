# Prompt 10 CID-Keyed CFF Clipping

Prompt 10B keeps the no-bbox-fake clipping rule. CID clipping is only claimed
when the renderer can build a real glyph path from encoded bytes through CMap,
CID/GID mapping, font data, FontMatrix, text matrix, CTM, rise, and scaling.

## Supported Path

Common CID text clipping remains supported where the font subsystem exposes the
glyph outline path, including the Identity-H TrueType CID path covered by the
earlier text-clipping campaign. Prompt 10B also routes available outlines by
resolved glyph id so supported vector glyphs can contribute real clipping
geometry.

## Narrow Unsupported Case

Advanced real-world CID-keyed CFF clipping remains classified as
`unsupported_reported_exotic_case` when the embedded CFF CID charstring geometry
is not exposed to the renderer as a safe glyph path. The renderer fails closed
with feature diagnostics instead of approximating clipping with a bounding box.

Evidence:

- `cid-keyed-cff-clipping-matrix-prompt10b.json`
- `cid-keyed-cff-reference-results-prompt10b.json`
- `prompt10b-reference-disagreement-summary.json`
