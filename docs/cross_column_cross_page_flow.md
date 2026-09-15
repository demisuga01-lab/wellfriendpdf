# Cross-Column and Cross-Page Flow

## Scope

text reflow extends the source editing provenance/operator-editing and editing transactions scene/transaction/font stack. It owns GeometricBlock and SemanticDocument routing and does not create a second parser, scene graph, font engine, semantic model, writer or binding-specific reflow implementation.

## Actual implementation status

This document describes the current source implementation boundary; it is not
runtime or corpus evidence. Provenance-resolved `GeometricBlock` and approved
`SemanticDocument` operations use the canonical source rewriter. Unknown
neighboring objects remain locked and a denied plan leaves the input bytes
unchanged.

Semantic reconstruction and reading-order analysis are available with the
documented inference limits. One approved same-page `next_region` continuation
is executable for a single provenance-resolved paragraph: the target must be
an explicit, disjoint, below-source rectangle on the same untagged page and
must be proven empty by canonical semantic and scene geometry. Both fragments
are emitted through one positioned advanced-editing source rewrite, preserving
visual and logical order; undo restores the exact preimage. One identical-box,
proven-empty, immediately following untagged page is also supported. These are
not inferred cross-column flow, arbitrary dependency movement, or reference
repair. A bounded horizontal cross-column route accepts an explicit,
same-reading-band, semantic/scene-proven-empty `next_column` rectangle
(rightward for LTR and leftward for RTL) and serializes both fragments in one
positioned canonical source stream. Inferred sidebars, multi-column topology,
figures, captions, lists, footnotes, and ambiguous ownership require an
explicit accepted semantic model rather than geometry-only guessing.

With explicit SemanticDocument review approval and page-creation policy, a
provenance-resolved paragraph may split at its final laid-out line boundary and
insert as many line-capacity-bounded continuation pages as required immediately
after the edited page through the canonical page-tree writer. Every line width
and baseline is checked before the fit result is committed. Existing indirect
page identities are retained and page-label number-tree indexes are shifted at
each insertion boundary. Signed documents,
tagged relationships, rotations/non-zero boxes, and associations from forms,
annotations, outlines, named destinations, or attachments are handled only
when the requested security and semantic/object-graph policies make their
meaning explicit; they are never guessed. `ReflowMutationSession` restores the
output from an in-memory exact preimage; it is not a general page-flow undo
token.
## Evidence status

No current build, test, render, differential, fuzz, performance, validator, or
binding-parity run was performed for this source change. Those checks are
deliberately deferred to the planned VPS qualification campaign. No release
verdict, closure commit, or deployment is justified from this document.
## Known limits

document subsystems owns full table/formula/OCR edit engines. document security owns final tagged-PDF/accessibility repair and forensic redaction closure. text reflow reports low-confidence semantic reconstruction and broad page-flow limitations instead of treating inference as exact fact.
