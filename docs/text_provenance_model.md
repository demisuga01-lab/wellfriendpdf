# Text Provenance Model

Reference Renderer makes provenance visible below `TextChunk`. The semantic model can now
explain how characters and matches were recovered.

## Sources

`TextMappingSource` includes native PDF text, ActualText, ToUnicode,
embedded/predefined CMap, Encoding/Differences, glyph-name/AGL, font cmap,
Identity CID fallback, OCR, and unknown.

`TextProvenanceFlag` adds tagged MCID, StructTree role, ToUnicode, predefined
CMap, encoding differences, font cmap, Identity CID, ligature expansion,
hyphenation join, normalized whitespace, heuristic role, hidden/invisible, and
unknown/unmapped flags.

## Aggregation

- `TextSemanticChar` stores mapping source, flags, MCID, role, role source, quad,
  font, writing direction, and confidence.
- `TextSemanticSpan`, `Word`, `Line`, `Block`, and `TextSearchMatch` carry compact
  `TextProvenanceSummary` counts.
- Search matches report whether hidden text was included and which MCIDs/roles
  were matched.

## ActualText

ActualText replacement remains single-emission. Logical characters inherit a
source-quad strategy from the replaced marked content and are marked
`actual_text`; extraction does not duplicate visible glyph text.

## Limits

The resolver now reports common source categories. Some rare font cmap fallback
paths still surface through the generic enum slot until lower-level font events
are expanded further.

