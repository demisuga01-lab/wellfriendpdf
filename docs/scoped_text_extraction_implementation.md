# Scoped text extraction - source increment, not qualification

Base revision: `27e62db3a1b84804339e65b6025273fd003b3736`, `main`.
This increment is part of the existing uncommitted candidate. It does not certify
that the candidate compiles or that the entire editing/rendering roadmap is done.

This report records the initial extraction increment. The subsequent
`semantic_stream_scope_implementation.md` report supersedes its page-only
semantic-bridge projection; ordinary Form ownership now reaches that bridge.

## Implemented in source

- Whole-page extraction executes nested and repeated Form occurrences. It uses
  the canonical tokenizer/parser, caches unique Form programs, and retains each
  invocation path separately from shared object identity.
- Explicit Form resources replace the current namespace; omitted/null legacy
  resources use the original page. A selected font remains object-bound across
  that namespace transition. Local Tf/ExtGState Font changes and q/Q restore the
  appropriate selection; returning from a Form restores the caller's state.
- Text position, rise, advance magnitude and effective size include both the
  text matrix and accumulated page/Form CTM. This fixes missing transforms; it
  does **not** make the legacy scalar chunk box an exact rotated glyph outline.
- Scoped extraction handles direct and named marked-content properties, with
  shallow property lookup (unrelated nested dictionaries are not metadata).
  Caller ActualText replaces descendant text once, including when the child's
  own ActualText is empty. Form-owned MCIDs retain their stream owner.
- The existing page-keyed tag bridge intentionally does not attach Form-local
  MCIDs to same-numbered page tags. New `ScopedTextChunk` data retains ownership
  for a subsequent stream-aware semantic bridge. Page wrappers can still own
  unmarked child content and caller ActualText.
- Engine page text, layout, geometric document modeling, table text input,
  classification, text-layer analysis and semantic/search chunk collection use
  the new traversal. The Form editor filters descendant Do operations before
  its fallible direct-text postcondition; it does not start validating unrelated
  descendant content as though it belonged to the selected Form's direct text.
- Root extraction uses strict token parsing, propagating lexical/stream errors.
  Full-document extraction now returns a failed-page/out-of-range error instead
  of silently dropping that page and returning a successful partial string.
- Form depth, invocation count, unique decoded bytes, executed operations,
  graphics/marked stack depth, emitted chunks and decoded text have explicit
  limits. Font resolvers and parsed Form programs are reused within traversal.
  Tj/TJ emission borrows source strings, avoids a whole-string code-vector copy,
  enforces output limits while decoding, and polls cancellation inside glyph/TJ
  loops. Rayon page workers install the captured cancellation token explicitly.

New Rust entry points are `TextCollector::collect_scoped`,
`ContentEngine::collect_page_scoped_text_chunks`, and the corresponding
`collect_page_scoped_text_chunks_with_limits` method. Invocation indexes are
extraction provenance, **not** persistent or revision-bound edit IDs.

The resource rules follow PDF Association's published ISO 32000-2 corrections:
[resource dictionaries](https://pdf-issues.pdfa.org/32000-2-2020/clause07.html#783-resource-dictionaries)
and [Form dictionaries](https://pdf-issues.pdfa.org/32000-2-2020/clause08.html#8102-form-dictionaries).

## Evidence boundary

Added 24 unexecuted regression functions in
`crates/engine/src/text/scoped_collector_tests.rs`: nested/root fallback,
explicit/empty/null/malformed scopes, inherited/local/ExtGState fonts, q/Q and
return restoration, repeated geometry/provenance, direct-only collection,
ActualText ownership, named properties, independent MCID namespaces, malformed
scopes/operators, recursion, decoder failures, cancellation, work/output limits,
inline images, invisible text, text-rise transforms and strict root token errors.

Only rustfmt source parsing/formatting and Git whitespace checks were run.
No compilation, type checking, test execution, PDF workload, rendering,
benchmark, commit, push or deployment was performed. These checks do not prove
Rust ownership/type correctness or any regression's runtime outcome.

## Still open

- The subsequent `semantic_stream_scope_implementation.md` increment adds
  ordinary Form MCR/ParentTree binding and a lossless semantic bridge. StmOwn,
  broader structure reconciliation and tagged Form cloning remain open; this
  does not claim full Form accessibility semantics.
- Exact transformed quads, clipping/optional-content-aware visible-only text,
  and non-text ActualText carriers such as outlined text/image-only spans.
  Extraction deliberately includes invisible OCR; it is not a pixel visibility
  or permanent-redaction oracle.
- Unresolvable font semantics and native font/codec behavior remain bounded by
  their own existing contracts. Traversal budgets are not a hard process-memory
  ceiling or an immediate cancellation guarantee inside every native routine.
- APs, patterns, masks and Type 3 glyph programs are not blindly treated as
  independent logical text; appropriate owner/semantic-specific handling remains
  separate to avoid duplicating characters and mask-only content.
- Broad SDK binding/corpus compatibility, source-edit/reopen checks using the
  changed extraction behavior, independent render/extraction comparison and
  performance measurement require the deferred executable phase.

The broader tracked implementation gaps still apply. This increment is not
evidence for universal PDF editing or superiority to Acrobat.
