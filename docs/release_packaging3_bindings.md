# text reflow Bindings

## Scope

text reflow extends the source editing provenance/operator-editing and editing transactions scene/transaction/font stack. It owns GeometricBlock and SemanticDocument routing and does not create a second parser, scene graph, font engine, semantic model, writer or binding-specific reflow implementation.

## Executable implementation status

All public surfaces route the same bounded, provenance-resolved geometric and
semantic reflow engine. One additionally tested, source-linked movement path can
move up to eight pairwise-disjoint same-page vector paths through the canonical advanced editing
 mutator when the request supplies both its stable identity and an approved
 dependency edge. Unknown neighboring objects remain locked and a refusal
 leaves the input bytes unchanged.

Rust, CLI, Python, C ABI, WASM, .NET, and Java route text reflow analysis,
preview, geometric apply, semantic apply, operation reports, and output bytes
through the same serialized engine request. Therefore the explicit
`next_region`, direction-aware horizontal `next_column`, and
`downstream_vector_moves` request fields are available without a
binding-specific layout implementation. `downstream_vector_moves` is bounded
to eight same-page source-resolved paths and rejects forms without an explicit
advanced editing shared-Form policy, annotations, clipping/marked/optional-content
paths, unknown relationships, and any target collision. The focused C ABI
owned-output test exercises the shared output route; it does not yet prove
movement parity for every binding.

The same serialized request supports up to eight `downstream_link_moves` entries for a
source-associated same-page `/Link`: callers provide `annotation_index`, the
exact `expected_rect`, an approved relationship/dependency edge, and `dx`/`dy`.
The engine preserves `/A` or `/Dest`, updates `/Rect` and existing `/QuadPoints`,
and rejects stale or non-Link annotations. This is a narrow shared-engine
surface, not generic annotation movement or durable cross-binding undo.

`layout_constraints` is part of the same serialized request. It accepts at
most 64 exact resolved-region constraints with `constraint_id`, `variable`,
`relation`, `value`, and `priority`. The canonical engine accepts the metric
and priority vocabulary in `reflow_constraint_solver.md`; malformed or
non-finite values refuse before mutation, a conflicting required constraint
refuses, and an unmet soft constraint remains visible in the report. Rust,
C ABI, Python, Java, .NET, and WASM all accept that request JSON; focused C
ABI, fresh-wheel Python, and Maven Java runtime tests exercise it.

The documented narrow semantic target-flow and page-creation exceptions are
available through those surfaces. `undo_reflow` is also exposed across Rust,
CLI, Python, C ABI, WASM, .NET, and Java: it replays the request against the
immutable preimage, compares the supplied output bytes exactly, and then
executes `ReflowMutationSession`'s atomic inverse. It returns
`stale_snapshot_conflict` rather than accepting a mismatched output buffer.
This is an executable public undo for the supported bounded operations, not a
general transaction-token protocol.

Rust exposes `query_overflow`, `query_constraints`, `query_confidence`, and
`validate_reflow_output`; the CLI exposes `overflow-report`,
`reflow-constraints`, `reflow-confidence`, and `reflow-validate`. Python, C ABI, WASM, .NET,
and Java expose the same three non-mutating query reports and explicit-output
validation through the canonical SDK functions. Validation takes the original
document plus caller-owned output bytes; it does not mutate either document.
Reference repair beyond existing source-linked Link rectangles remains a typed
unsupported boundary. Runtime/package parity is validated for the exposed
bounded operations; it does not imply support for those excluded repairs.

The CLI’s `--font-policy preserve_original_per_run` selects the bounded
source-style serializer; `rebuild_subset_or_generated_type0` remains its
default. Unsupported source semantics are reported/refused by the shared Rust
engine rather than being normalized by the CLI.
## Evidence status

Retained VPS evidence includes focused Rust, C ABI, CLI, fresh isolated Python
wheel, WASM target/runtime, .NET test/pack, Maven test/package/runtime, and
Gradle test/build/class-equivalence gates, together with serial workspace,
differential, fuzz, performance, and hygiene stages. Each binding invokes the
same engine request/report route and is bounded by the same typed refusals.
## Known limits

document subsystems owns full table/formula/OCR edit engines. document security owns final tagged-PDF/accessibility repair and forensic redaction closure. text reflow reports low-confidence semantic reconstruction and broad page-flow limitations instead of treating inference as exact fact.
