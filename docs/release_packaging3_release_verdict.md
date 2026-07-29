# text reflow Release Verdict

## Scope

text reflow extends the source editing provenance/operator-editing and editing transactions scene/transaction/font stack. It owns GeometricBlock and SemanticDocument routing and does not create a second parser, scene graph, font engine, semantic model, writer or binding-specific reflow implementation.

## Executable boundary

The canonical engine provides provenance-resolved `GeometricBlock` source
rewriting and bounded `SemanticDocument` reflow. Both routes retain explicit
mode selection, use the source editing/32 transaction foundations, reopen the
serialized output, and expose executable inverse replay. The supported
SemanticDocument paths are source-linked local, next-region, LTR/RTL
next-column, existing-next-page, and explicit append-page continuation; each
has an exact eligibility, confidence, overflow, and no-change refusal path.

Mixed supported text-state runs retain their CMap/font selection and supported
text-state transitions. The UAX #14 candidate pipeline, BCP-47 pattern
hyphenation, shaped final metrics, bounded dynamic optimizer, justification,
constraint solver, dependency-linked movement, graph inference, semantic
types, reading-order cycle repair, reference-associated link movement, and
unaffected-content proof all execute through the canonical reflow path.

## Evidence status

The retained VPS evidence contains serial workspace format/check/clippy/test,
focused text reflow fixtures, source editing/32 and writer/signature/tag impact
filters, runtime/package coverage for every binding, qpdf/Poppler differential
checks, bounded fuzz build/smoke, performance, and repository hygiene. Each
counted stage has a saved exit code, duration, peak RSS, log location, and
hash. The candidate is pending only the single authorized closure commit,
push, synchronization check, and clean-worktree verification; no deployment
is part of text reflow.
## Known limits

document subsystems owns full table/formula/OCR edit engines. document security owns final
tagged-PDF/accessibility repair and forensic redaction closure. text reflow
refuses arbitrary tag rewriting, generic object movement, inferred page
insertion, and reference repair without an exact source association; low
confidence semantic reconstruction is never silently applied.
