# Vertical text range editing

Prompt 20B range edits use the same logical range model for horizontal, RTL,
and vertical replacement requests. The selected source range must still be a
contiguous sequence of provenance-bearing decoded string operands in one page
content stream. The replacement may use `paragraph_reflow_vertical`, which
serializes deterministic generated Type0 text with Identity-V metrics, vertical
advances, and bounded column placement.

Vertical source selection is not inferred from x-coordinate sorting. Callers
must resolve a visual selection to one unambiguous logical range first. The
range report records source spans, logical offsets, writing mode, bidi
provenance, and exact diagnostics. Vertical clusters and columns are bounded by
the Prompt 20 glyph, line/column, and output-size caps.

Exact limits: existing arbitrary vertical PDF glyph streams are not blindly
reshaped; Type3 vertical editing, broken CMaps, partial-token ranges, and
cross-page selections fail closed. A caller-supplied font is required for CJK
glyph coverage outside the bundled font.
