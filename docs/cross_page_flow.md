# Cross-Page Flow

One narrow existing-page path exists: an approved `SemanticDocument` reflow
may split a single provenance-resolved paragraph into an identical-box,
provably empty region on the immediately following untagged page. Canonical
semantic text geometry proves the target empty; non-text scene objects remain
locked; the first page is rewritten and the continuation is inserted through
the canonical range/source writer. Undo restores the exact incremental
preimage. This existing-page path is distinct from same-page `next_region`
flow, which serializes both fragments in one positioned canonical source
stream to preserve logical extraction order.

A separately narrow path may append one continuation page through the
canonical page-tree writer for a one-page PDF with a direct root `/Pages`
`/Kids` array. It refuses signatures, tagged structure, non-zero/rotated page
boxes, inferred insertion positions, and any non-append operation. The writer
preserves the existing catalog/object graph, including forms, annotations,
outlines, named destinations, page labels, and attachments; existing page
references retain their copied identity. It does not infer associations from
those objects to newly generated continuation text, and general insertion,
retargeting, linked multi-paragraph movement, reference repair, and general
pagination remain unavailable.
