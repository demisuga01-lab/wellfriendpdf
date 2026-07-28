# Semantic Types and Relationships

## Scope

Prompt 33 extends the Prompt 31 provenance/operator-editing and Prompt 32 scene/transaction/font stack. It owns GeometricBlock and SemanticDocument routing and does not create a second parser, scene graph, font engine, semantic model, writer or binding-specific reflow implementation.

## Actual implementation status

Prompt 33 materializes canonical Prompt 06 semantic roles as runtime region
nodes for headings, lists/list items, captions, headers, footnote bodies, and
sidebars. Prompt 32 image/path scene occurrences are materialized as bounded
figure candidates. A caption receives a deterministic, confidence-scored
`caption_of` edge only to its nearest same-page figure candidate; competing
interpretations remain alternatives. A heading receives a bounded
`heading_for` edge to the next paragraph or list item in deterministic reading
order. Existing list blocks own `list_item` nodes through `list_parent` edges.
When the conservative canonical block role remains body text but its canonical
paragraph text has an enumerated or bullet label, Prompt 33 materializes one
bounded inferred `List` parent and preserves the body-block alternative. The
same canonical label evidence promotes a paragraph-level Figure/Fig./Table
caption and gives it a nearest-figure `caption_of` edge; it does not turn
arbitrary nearby prose into a caption.

These are semantic graph facts and inferences, not permission to move an
unlinked object. A bounded footnote association is implemented when an inline
superscript or symbol marker and one bottom-page body share an exact label;
the resulting `footnote_of` edge retains its evidence. Multiple body matches
are alternatives requiring review, and the current low-confidence marker node
cannot silently authorize a semantic edit. Figure/table source rewrite,
header/footer artifact removal, and broad semantic reflow are still exact
refusals outside the narrow approved paragraph boundary. Repeated header/footer
detection is implemented as a deterministic artifact candidate classification;
it does not delete or move those nodes.
## Evidence status

The semantic graph regression suite proves no dangling graph edges, stable IDs,
cycle resolution, duplicate-evidence edge merging, deterministic
nearest-figure association, and an owned heading/list/caption/figure fixture.
Full type accuracy metrics and cross-document corpora remain required closure
gates.
## Known limits

Prompt 34 owns full table/formula/OCR edit engines. Prompt 35 owns final tagged-PDF/accessibility repair and forensic redaction closure. Prompt 33 reports low-confidence semantic reconstruction and broad page-flow limitations instead of treating inference as exact fact.
