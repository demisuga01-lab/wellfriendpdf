# Reflow Transactions and Undo

## Scope

text reflow extends the source editing provenance/operator-editing and editing transactions scene/transaction/font stack. It owns GeometricBlock and SemanticDocument routing and does not create a second parser, scene graph, font engine, semantic model, writer or binding-specific reflow implementation.

## Actual implementation status

This document describes the intended text reflow boundary. It is not evidence
that every named capability is implemented. The current executable support is
limited to a provenance-resolved, bounded, single-region `GeometricBlock`
source rewrite. Unknown neighboring objects remain locked and a refusal leaves
the input bytes unchanged.

Supported source-linked geometric reflows now have an executable
`ReflowMutationSession` undo path. Incremental revisions validate the edited
fingerprint, truncate to the recorded byte boundary, verify the preimage hash,
reopen the restored PDF, and verify page count before changing session state.
For the separately tested plain-document page-creation boundary, the session
retains the exact preimage in memory and restores it atomically after the same
fingerprint and reopen checks. The focused engine tests prove byte-exact
restoration for both boundaries.

This is not a persisted cross-process undo token. General source-linked
downstream movement remains limited: one explicitly dependency-linked,
same-page path object may move through the canonical advanced editing vector mutator
only when its scene identity, collision-free target, ownership, and interactive
document restrictions are proven. One caller-associated same-page `/Link` may
also move when its exact source rectangle, relationship, and delta are supplied;
the transaction preserves its action/destination and moves `/Rect` plus existing
`/QuadPoints`. Broad object movement, destination/annotation tag repair, and
full binding parity remain unavailable. Explicit page creation
is limited to one provenance-resolved SemanticDocument paragraph in one plain
untagged one-page PDF, with explicit approval and policy; any catalog or
interactive structure that would need repair is refused before mutation.
## Evidence status

The only current continuation evidence is the focused Rust engine suite, a
focused C ABI owned-output test, and the corresponding serial VPS runs. These
are not a full workspace, differential, fuzz, performance, hygiene, or
binding-parity gate. No release verdict, closure commit, or deployment is
justified from this document.
## Known limits

document subsystems owns full table/formula/OCR edit engines. document security owns final tagged-PDF/accessibility repair and forensic redaction closure. text reflow reports low-confidence semantic reconstruction and broad page-flow limitations instead of treating inference as exact fact.
