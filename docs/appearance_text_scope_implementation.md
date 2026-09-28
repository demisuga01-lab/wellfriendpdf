# Annotation appearance text and ownership — source implementation

Status: implemented source increment, unqualified. The full roadmap is still
incomplete. Worktree base is `27e62db3a1b84804339e65b6025273fd003b3736`, branch
`main`; all accumulated candidate changes remain uncommitted.

## What changed

- The renderer and text extractor share source normal-appearance selection.
  Explicit unavailable states do not select a different state's text. The
  existing renderer's missing-AS fallback policy is retained; this is not a
  claim that every malformed state dictionary has a unique intended state.
- Shared appearance placement transforms all four BBox corners with Matrix,
  computes their upright bounds, and maps those bounds to Rect. Matrix is then
  composed exactly once. The former renderer path fitted the untransformed
  bounds, which misplaced translated/rotated/scaled appearances.
- `ContentEngine::collect_page_scoped_text_chunks_including_appearances` executes
  existing selected normal appearance streams with fresh graphics/text state,
  owner-local Resources or original-page fallback, and nested Form traversal.
  Page and appearance work share decode/operation/output/invocation budgets and
  cancellation. Unique streams share decoded programs; occurrences stay distinct.
- `ScopedTextChunk.appearance` records annotation object, annotation-array index
  and root appearance stream. Form invocation paths remain genuine Do paths;
  no fictitious root Do index is introduced. Local nested Form MCIDs keep their
  own namespace. An outer appearance MCID/ActualText may own descendant glyphs.
- `MarkedContentId.stream_owner`, `SemanticMcid.stream_owner`, structure entries,
  and ParentTree recovered nodes preserve annotation ownership alongside stream
  and MCID. JSON readers of MarkedContentId/recovery nodes accept older payloads
  without the new optional field. Rust struct literal callers must provide the
  new fields; this is a source API change.
- Forward semantic extraction validates explicit annotation StmOwn against page
  membership and its actual N/R/D appearance graph. Inactive appearance references
  may remain valid structure references but contribute no selected-state text.
  Omitted StmOwn is inferred only for one extracted stream-owner namespace.
  Competing namespaces are not arbitrarily combined.
- ParentTree recovery carries the same ownership through reciprocal K matching.
  Its element/node IDs distinguish shared appearances on different annotations.
  Per-transaction indexes cache page membership, owner states and stream-owner
  multiplicity, with work limits and cancellation.
- Page-only extraction/search retains its original source scope. Structure
  extraction uses the new appearance-aware bridge. Geometry-only structure
  redaction rejects appearance-owned selections instead of certifying that
  modifying underlying page content removed an annotation's text.

## Evidence and validation boundary

18 new regression functions are source only: 15 appearance/semantic cases, two
matrix/degenerate-geometry cases and one selector-scope case. They cover owner
binding, shared appearances, missing/explicit states, inactive references, nested
Forms/ActualText, resource fallback, page coordinates, limits, cancellation,
recursive invocation, unique decoding, and JSON compatibility.

Rustfmt formatting/parser checks and `git diff --check` passed. No Cargo,
compiler/typechecking, builds, tests, PDFs, rendering, benchmarks, native bindings,
deployment, commit or push were run for this increment. Source checks do not prove
that the candidate compiles or that any regression passes.

## Still open

- The subsequent `appearance_text_editing_implementation.md` adds revision-bound
  selected-occurrence source editing for existing untagged non-widget normal
  appearances and nested Forms. Broader appearance-aware search, redaction,
  governed universal planning and native bindings remain open. This extraction
  API itself is not an editor.
- Direct annotation/appearance dictionaries cannot supply stable indirect StmOwn
  identity here; source extraction reports them rather than inventing references.
  Existing writer-side promotion remains a separate transaction.
- Non-annotation StmOwn contexts, OBJR logical text, shared tagged appearance
  mutation, full forward/ParentTree cross-validation and malformed-graph repair.
- This is logical source text, not rendered visibility. Hidden/NoView, optional
  content, clipping, opacity, screen zoom/rotation flags and synthesized field
  appearances are not evaluated by this extractor. Glyph CharProcs, patterns and
  soft masks are not independently interpreted as document text.
- Exact rotated glyph quads remain a separate text-geometry task. The corrected
  placement establishes the source transform, not pixel or layout equivalence.
- Current compilation, tests, bindings, independent renderer/extractor comparison,
  real-document corpus and performance evidence remain pending as requested.

Reference: ISO 32000 appearance mapping (§12.5.5) and MCR ownership (§14.7.4.2),
available in [Adobe's published ISO 32000-2 draft](https://developer.adobe.com/document-services/docs/assets/5b15559b96303194340b99820d3a70fa/PDF_ISO_32000-2.pdf).
This is implementation guidance, not a conformance or Adobe-superiority claim.
