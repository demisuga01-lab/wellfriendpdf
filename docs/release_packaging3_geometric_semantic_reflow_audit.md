# text reflow Geometric and Semantic Reflow Audit

## Continuation state

This work continues from the user-authorized dirty text reflow worktree. The
original clean editing transactions baseline was
`7b33a77e6da8321644734051afeaeaec59a196bc`; the captured dirty-worktree
manifest hash is recorded in
`target/text_reflow-geometric-semantic-reflow/text_reflow-starting-state.json`.

## Canonical extension points

| Requirement area | Canonical module | Audit classification | Current boundary |
| --- | --- | --- | --- |
| Operator provenance | `source_editing.rs` | canonical_complete | Provides source instruction identity and explicit edit-mode routing. |
| Scene, snapshots, transaction reports | `editing_transactions.rs`, `text_reflow.rs` | canonical_complete | Provides source-linked scene projections, transaction reports, executable replay undo, and bounded invalidation for text reflow transactions. |
| Unicode shaping and generated source rewrite | `advanced_editing.rs` | canonical_complete | Reuses canonical shaping, subset reconstruction, and source mutation. The preserve-per-run path retains supported mixed CMap/font/text-state runs for changed-length horizontal replacements. |
| Geometric region planner | `text_reflow.rs` | canonical_complete | Region, paragraph, preview, final dynamic layout, constraints, overflow, source rewrite, and typed refusal reports share one request model. |
| Semantic layout analysis | `text_reflow.rs`, `semantic.rs`, `text/semantic_model.rs`, `analysis/layout.rs` | canonical_complete | Projects the canonical semantic model into bounded runtime region graphs, typed semantic nodes, reading-order edges, cycle resolution, and supported explicit flow transactions. |
| Page-tree editing | `writer.rs`, `utilities.rs`, `authoring.rs` | canonical_complete | text reflow uses the catalog-preserving canonical append writer for explicit-policy one-page continuation and its inverse restores the stored preimage. |
| Bindings | `sdk.rs`, CLI, C ABI, Python, WASM, .NET, Java | canonical_complete | All surfaces route to the same SDK operations and have retained VPS runtime/package evidence. |
| Closeout artifacts | `scripts/text_reflow_generate_closeout_artifacts.py` | unsafe_or_ambiguous | The inherited generator marked unexecuted gates complete. It was changed to a non-completion posture and is not release evidence. |

## No duplicate architecture finding

The inherited module reuses source editing provenance, editing transactions scene/snapshot
reports, advanced editing shaping and incremental source mutation, the canonical
reader/writer, and the shared SDK/binding envelopes. It does not introduce a
second parser, writer, renderer, scene graph, semantic model, or transaction
engine.

## Implemented and tested during this continuation

`GeometricBlock` resolves exact source provenance before it rewrites actual
content instructions. Final UAX #14 candidates drive the writer; the operation
updates font resources, reopens the output, validates extraction and unaffected
streams, and stores an executable inverse. The default generated-Type0 path
and the bounded preserve-original-per-run path remain explicit policies and
never silently downgrade each other.

The Unicode pipeline keeps grapheme and shaping-cluster boundaries intact,
uses canonical bidi/shaping data for final advances, and provides deterministic
greedy preview plus bounded dynamic final layout. Pattern-backed language
hyphenation is explicit by BCP 47 tag, with unavailable languages returning a
typed result rather than an English fallback. Justification is bounded by the
selected script policy; unsupported source serialization, including a kashida
feature the writer cannot preserve, is a typed refusal.

`SemanticDocument` retains its mode through local reflow and supported explicit
next-region, next-column, existing-next-page, and append-only page-creation
flows. The runtime semantic graph uses canonical geometry, confidence, source
evidence, deterministic IDs, reading-order cycle removal, headings, lists,
captions, footnotes, and repeated header/footer candidates. Low-confidence or
ambiguous transitions require review or refuse before mutation.

## Documented typed limits

text reflow does not claim arbitrary tagged-content rewriting, generic object
movement, inferred page insertion, or repair of references that have no exact
source association. Those cases return a precise unsupported, ambiguous, or
review-required result with no partial mutation. document subsystems owns full
table/formula/OCR editing; document security owns final accessibility/tag-repair and
forensic-redaction closure.

## Executed closure evidence

The final VPS matrix contains serial workspace fmt/check/clippy/test, focused
text reflow fixtures, source editing/32 and writer/signature/tag historical impact,
all binding/package runtimes, qpdf/Poppler differential checks, bounded fuzz
build/smoke, performance, and hygiene. Every counted stage has a retained
exit code, duration, peak RSS, and log hash. The closure commit remains
forbidden only until those retained candidate results are reviewed and staged.
