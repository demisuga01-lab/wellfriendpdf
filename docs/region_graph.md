# Region Graph

text reflow materializes a bounded graph from the existing Native Renderer semantic
model and editing transactions scene graph. Nodes include page regions, blocks,
paragraphs/list items, lines, words, glyphs, and image/path figure candidates.
Edges include `contains`, `list_parent`, `next_reading`, `next_page`,
`heading_for`, and `caption_of`, each with source evidence and confidence.

Graph IDs are deterministic. Every runtime semantic-layout report now includes
`region_graph_invariants`: unique node/edge IDs, finite non-empty bounds,
no dangling edges, an explicit edge-count limit, and the analyzed page set.
When more than one deterministic ensemble member supports the same
source/target/relationship triple, the runtime keeps one canonical edge and
retains the lower-priority evidence as an alternative; duplicate edge IDs are
not serialized.
For a `GeometricBlock` request the canonical scene projection is restricted to
the selected page and the report records that all other pages were reused
without full-page reanalysis. A `SemanticDocument` request deliberately uses
document scope so repeated headers/footers and page-flow candidates can be
evaluated; that wider scope is explicit in the report rather than a silent
mode upgrade.

This is bounded analysis invalidation, not broad cross-column application.
Inferred cross-column stories, multi-paragraph source insertion, and general
semantic graph mutation remain exact refusals until they have a source-linked
transaction and unaffected-content proof.
