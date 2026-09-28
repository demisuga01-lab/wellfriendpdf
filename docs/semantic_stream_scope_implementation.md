# Stream-owned semantic binding - source increment, not qualification

Base: `main`, `27e62db3a1b84804339e65b6025273fd003b3736`, with the existing
uncommitted roadmap candidate preserved. No claim of roadmap completion.

## Implemented source behavior

The previous extraction increment retained Form MCIDs but deliberately projected
them away at the page-only semantic bridge. This increment carries the owner
through that bridge instead:

- A marked-content identity includes its stream reference and MCID, with the
  page kept by the containing page/model. Page content has no extra stream key;
  explicit MCR references to one of that page's Contents streams normalize to
  the same page namespace. Nested/repeated Forms keep their own namespace.
- Forward structure parsing resolves `/Stm`, validates explicit `/Pg` and MCID
  fields, and uses `(page, stream, MCID)` lookup. A malformed explicit page does
  not fall back to an inherited page. Cyclic/over-budget structure traversal
  returns an error rather than a successful partial tree.
- ParentTree recovery looks up `/StructParents` on the actual Form. Page and
  Form scopes cannot borrow one another's integer-key mappings. Shared Forms
  rendered on multiple pages remain one container; different containers using
  one StructParents key are conflicts.
- Indirect ParentTree arrays are resolved. Duplicate number-tree keys no longer
  produce a last-wins role assignment. Reciprocal `/K` references must identify
  the selected page/stream/MCID; a child's structural content is not treated as
  its parent's own content. Missing reciprocal evidence is reported as repaired,
  not spec-derived. Repaired/orphan/conflicting evidence survives conversion to
  the semantic tree and does not become a high-confidence authored tag.
- Semantic characters, words, spans, lines, blocks and search hits expose
  `marked_content` identities. Duplicate forward mappings are not first-wins;
  conflicting role binding is withheld. Geometric deduplication does not merge
  different marked-content namespaces merely because their text/positions match.
- RAG source spans/citations retain these identities. Cross-page RAG chunks add
  the page to each identity, and nonempty scope-qualified membership participates
  in the chunk hash. Reflow source evidence also carries it. The legacy integer
  arrays remain a **lossy summary**, not an authoritative source selector.
- A legacy redaction selector with only an MCID still means a page MCID; it does
  not start selecting same-numbered Form content. This is not implementation or
  qualification of occurrence-specific tagged Form redaction.
- A structure element spanning multiple extracted pages no longer receives a
  single union bbox on an arbitrary page. Selected-page projection retains only
  selected MCID entries and can identify the one resolved page. Text from
  different pages is not geometrically sorted into one shared coordinate plane.

The namespace rule follows the PDF Association's published correction to
[finding structure elements from content items](https://pdf-issues.pdfa.org/32000-2-2020/clause14.html#14754-finding-structure-elements-from-content-items).

## API effects

- `MarkedTextChunk` adds `mcid_owner`; `SemanticMcid` and `TextStructureEntry`
  add `stream`. `MarkedContentId` is the public scope-qualified identity.
- `collect_page_marked_text_chunks` now preserves Form ownership. The explicitly
  lossy `ScopedTextChunk::into_page_marked` remains available to a page-only
  consumer; normal semantic extraction uses `into_marked`.
- Semantic output adds optional recovery provenance and `marked_content` fields.
  RAG JSON defaults the added collections to empty when reading older payloads.
  Rust callers constructing these public structs directly must add the new
  fields; unexecuted binding compatibility is not assumed from serialization.

## Evidence

Added **26 unexecuted regression functions**: 23 in `semantic_scope_tests.rs`
and three in `advanced_rag.rs`. They cover page/Form ID collisions, roles/search,
ParentTree recovery, indirect arrays, repeated/cross-page Forms, page-stream
normalization, missing/conflicting/duplicate mappings, reciprocal ownership,
scope-aware deduplication, explicit malformed contexts, cycles/cancellation,
recovery evidence, RAG serialization/citations and scope-bound hashes.
The previous extraction regression was updated for the now-lossless bridge.

Only rustfmt source parsing/formatting and Git whitespace checks were performed.
No compiler, type checker, builds, tests, PDF workloads, rendering, benchmarks,
commits, pushes or deployments were run. Source parsing does not prove that
these changes compile or that any regression passes.

## Remaining implementation and qualification

- The subsequent `appearance_text_scope_implementation.md` adds annotation
  `/StmOwn` extraction/binding. Other stream-owner contexts and OBJR-to-text ownership,
  non-text ActualText carriers and other non-page programs require their actual
  owner/execution context. They are not guessed as page or ordinary Form content.
- Full forward-tree/ParentTree reconciliation, arbitrary malformed-graph repair,
  namespace/role-map inheritance and partial structural ActualText handling are
  not made complete by this increment.
- Stream-owned tagged clone/move transactions, caller ActualText migration and
  occurrence-specific on-page editing remain separate from reading these tags.
- Scalar text boxes are still not exact transformed glyph quads or visibility
  proofs. Multi-page structure geometry needs a per-page representation across
  every consuming tool; withholding an invalid one-page bbox is not that feature.
- Current compilation, every changed binding, realistic tagged PDF save/reopen,
  independent extraction/rendering, performance and the full corpus gates remain
  unexecuted. The broader roadmap remains active and incomplete.
