# Cross-Column and Cross-Page Flow

## Scope

text reflow extends the source editing provenance/operator-editing and editing transactions scene/transaction/font stack. It owns GeometricBlock and SemanticDocument routing and does not create a second parser, scene graph, font engine, semantic model, writer or binding-specific reflow implementation.

## Actual implementation status

This document describes the intended text reflow boundary. It is not evidence
that every named capability is implemented. The current executable support is
limited to a provenance-resolved, bounded, single-region `GeometricBlock`
source rewrite. Unknown neighboring objects remain locked and a refusal leaves
the input bytes unchanged.

Semantic reconstruction and reading-order analysis are available with the
documented inference limits. One approved same-page `next_region` continuation
is executable for a single provenance-resolved paragraph: the target must be
an explicit, disjoint, below-source rectangle on the same untagged page and
must be proven empty by canonical semantic and scene geometry. Both fragments
are emitted through one positioned advanced editing source rewrite, preserving visual
and logical order; undo restores the exact preimage. One identical-box,
proven-empty, immediately following untagged page is also supported. These are
not inferred cross-column flow, arbitrary dependency movement, or reference
repair; all broader transitions refuse without mutation. A second narrow horizontal
cross-column exception accepts an explicit, rightward, same-reading-band,
semantic/scene-proven-empty `next_column` rectangle (rightward for LTR,
leftward for RTL) and serializes both
fragments in one positioned canonical source stream. It refuses RTL, inferred,
sidebar, three-column, figure, caption, list, footnote, and ambiguous cases.

There is a separately tested cross-page exception: with explicit SemanticDocument,
review approval, and page-creation policy, a single provenance-resolved
paragraph in a plain, untagged, one-page PDF may split at its final laid-out
line boundary and append one continuation page through the canonical
page-tree writer. The path refuses signed PDFs, rotations/non-zero boxes,
forms, annotations, outlines, page labels, named destinations, attachments,
and tagged structures because those require repair support it does not yet
have. `ReflowMutationSession` restores that output from an in-memory exact
preimage; it is not a general page-flow undo token.
## Evidence status

The only current continuation evidence is the focused Rust engine suite, a
focused C ABI owned-output test, and the corresponding serial VPS runs. These
are not a full workspace, differential, fuzz, performance, hygiene, or
binding-parity gate. No release verdict, closure commit, or deployment is
justified from this document.
## Known limits

document subsystems owns full table/formula/OCR edit engines. document security owns final tagged-PDF/accessibility repair and forensic redaction closure. text reflow reports low-confidence semantic reconstruction and broad page-flow limitations instead of treating inference as exact fact.
