# RTL logical and visual range mapping

Prompt 20B records logical Unicode offsets, source PDF string-token provenance, and `BidiRunProvenance` visual run order. A visual selection is accepted only after a caller resolves it to one unambiguous logical span; duplicate extracted text is not selected by nearest x-coordinate.

Existing PDF codes/CIDs/GIDs remain source provenance. Only inserted Unicode is shaped. Bidi controls, malformed mappings, and missing glyphs are fail-closed diagnostics.

Prompt 20B range mapping records logical offsets before visual ordering. RTL
selection spans must be supplied as logical ranges or as visual selections that
the caller has already resolved to one unambiguous logical range. Oxide does
not sort glyphs by x coordinate to infer text order.

For newly generated replacement text, bidi runs, shaped clusters, glyph order,
and source-run provenance are recorded in the range/reflow reports. Existing
PDF glyph streams are preserved as provenance until the selected token-boundary
range is rewritten. Ambiguous duplicate text, broken CMaps, missing source
provenance, Type3 content, and cross-page ranges fail closed.
