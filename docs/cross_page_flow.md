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

The canonical page-tree writer can insert one or more continuation pages
immediately after the edited page, including in the middle of the document.
Overflow lines are chunked by the proven per-page capacity and every baseline
is checked against the continuation region before success is reported. It rewrites the
owning `/Pages` `/Kids` array and ancestor counts, preserves existing indirect
page identities, and shifts number-tree page-label indexes at or after the
insertion boundary. Signatures still follow the selected signature policy, and
tagged structure, inferred insertion positions, associations from forms,
annotations, outlines, named destinations, or attachments to newly generated
text require an explicit semantic/object-graph operation rather than silent
guessing. Linked multi-paragraph movement and automatic pagination remain
governed reflow operations, not side effects of page insertion.
