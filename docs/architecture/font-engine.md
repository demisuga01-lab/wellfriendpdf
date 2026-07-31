# Font engine

Rendering keeps distinct PDF text identities: PDF code, CID, GID, Unicode
scalar, grapheme cluster, shaping cluster, and painted glyph occurrence. Those
identities are required for extraction, editing, redaction, reflow, undo, and
render validation.

Supported renderer paths include Standard fonts, embedded fonts, CID fonts,
Type 3 charprocs, colour glyph routes, CMap and ToUnicode mapping, glyph bitmap
caching, glyph outline fallback, and controlled font-substitution reports.

New/reflowed text uses the Prompt 32/33 shaping and font-subset systems. Existing
PDF text is not arbitrarily reshaped; the renderer respects the source text
state, glyph placement, writing mode, render mode, clipping mode, and resource
scope.

Unsupported font cases must preserve the input and report a typed evidence
record rather than substituting silently.
