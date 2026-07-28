# Cross-Column Flow

The runtime graph materializes bounded `Column` nodes from canonical semantic
block x-projection clusters. It records `contains` and direction-aware
`next_column` edges without treating paint order as a column detector.

One narrow source-linked continuation is executable: an approved
`SemanticDocument` edit may split one provenance-resolved horizontal paragraph
into an explicit, disjoint `next_column` rectangle in the same reading band
that is proven empty by canonical semantic and scene geometry. The required
direction is explicit: LTR flows rightward, RTL flows leftward. Both fragments
are emitted in one positioned canonical source stream, so logical extraction
remains source-column then next-column; session undo restores the exact
preimage. Positioned RTL output also carries one standard PDF `/ActualText`
span for the logical paragraph: shaped CIDs retain visual RTL placement while
the canonical collector and accessibility consumers do not re-sort same-row
columns as LTR text.

This is not inferred multi-column reconstruction. Three-column ordering,
sidebars, figures, captions, lists, footnotes, vertical writing, and all
ambiguous or unlinked transitions refuse without moving source objects.
