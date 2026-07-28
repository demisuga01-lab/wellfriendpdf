# Semantic Layout Reconstruction

## Scope

Prompt 33 extends the Prompt 31 provenance/operator-editing and Prompt 32 scene/transaction/font stack. It owns GeometricBlock and SemanticDocument routing and does not create a second parser, scene graph, font engine, semantic model, writer or binding-specific reflow implementation.

## Executable implementation status

The runtime reconstruction path reuses the Prompt 06 semantic model and
canonical layout analysis rather than a second semantic tree. It is available
to `SemanticDocument` planning and application, with source-provenance,
confidence, revision, and transaction links retained for every supported
node. Unknown neighboring content stays locked and every ambiguous selection
returns a typed no-change result.

Semantic reconstruction now uses the existing Prompt 06 semantic text model
and canonical XY-cut layout: bounded runtime page-region, role-labelled
block/list/caption/header/footer, paragraph, line, word, and glyph nodes carry
PDF user-space geometry, role/MCID/structure evidence, revision IDs, and
source-occurrence links. Its precedence graph resolves a cycle by removing the
lowest-confidence edge with a deterministic edge-ID tie-break.

Repeated running headers and footers have a second deterministic evidence path:
matching normalized block text (with page-number digits collapsed) must recur
on at least two pages in the same top or bottom page band. Those runtime nodes
retain the evidence and are excluded from body reading order; a one-page or
positionally inconsistent match remains body content rather than an artifact.

SemanticDocument application is deliberately source-linked: one exact
paragraph is selected by deterministic source-text identity plus Prompt 31
provenance after explicit confidence approval. Its supported target-flow
adapters are local, next-region, direction-aware next-column,
existing-next-page, and explicit catalog-preserving append-page continuation.
Unrelated paragraphs do not invalidate the local selection; duplicate,
partial, inferred, or low-confidence selections return a typed refusal. The
engine does not claim generic object movement or reference repair without an
exact source association.
## Evidence status

Retained VPS evidence covers the focused runtime fixtures, serial workspace
format/check/clippy/test, all binding/package runtimes, qpdf/Poppler
differential checks, fuzz, performance, hygiene, and historical impact gates.
The evidence validates the documented supported boundary; it does not turn a
typed unsupported case into a successful semantic mutation.
## Known limits

Prompt 34 owns full table/formula/OCR edit engines. Prompt 35 owns final tagged-PDF/accessibility repair and forensic redaction closure. Prompt 33 reports low-confidence semantic reconstruction and broad page-flow limitations instead of treating inference as exact fact.
