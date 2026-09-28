# Tagged ownership and annotation transactions — source implementation

Date: 2026-09-16. Candidate remains dirty over main
`27e62db3a1b84804339e65b6025273fd003b3736`.

No compiler, build, test, PDF workload, renderer, benchmark or deployment was
executed. `rustfmt` parsed/formatted the touched Rust sources, and
`git diff --check` checked tracked patch whitespace. Those checks do not prove
type correctness, executable behavior, conformance or visual fidelity.

## Implemented paths

`engine::tagged_structure` is the common structural ownership writer/checker.
DocumentSecurity's `RepairTaggedStructure` with parent rebuilding,
`RebuildParentTree`, and `RepairAfterMutation` now use it. The former repair
assigned every page key zero and filled one array with all structure elements;
that implementation has been removed.

The new path:

- Traverses the existing `/K` hierarchy, retaining its order, role/attribute
  metadata and opaque fields. It repairs `/P` links and materializes direct
  structure elements/root as indirect objects.
- Builds independent MCID ownership maps for page content and each referenced
  Form/appearance stream. Sparse MCID arrays contain null holes. Whole-object
  `/OBJR` references produce direct parent-element references, not arrays.
- Reuses unique valid parent keys; resolves duplicate/missing keys; writes a
  bounded, balanced ParentTree and a consistent ParentTreeNextKey.
- Rebuilds IDTree from reachable structure-element byte-string IDs. Duplicate
  IDs, shared/cyclic structure nodes and conflicting ownership are not guessed.
- Uses the canonical content parser for real MCIDs, including named property
  resources, dictionary literals, inline-image isolation and nested Forms.
  Annotation N/R/D appearance/state graphs and their invoked Forms are checked.
- Checks annotation page membership, `/P`, `/OBJR /Pg`, and appearance
  `/MCR /Pg /StmOwn` relationships. Dangling owners, stale keys, missing MCIDs,
  duplicate MCIDs and mismatched actual content withhold output.
- Bounds recursion, node counts, MCID-array allocation and decoded bytes, and
  polls the current cancellation scope. Page lookup uses an indexed map.
- Reopens private output and compares reconstructed ownership with the actual
  ParentTree/IDTree before returning bytes. Lookup trees must have correct
  key ordering, subtree limits and nonoverlapping child ranges.

`ParentTreeReport.conformance_certified` stays false. This is not an automated
semantic-role, reading-order, alternative-text or PDF/UA certification engine.

## Annotation move integration

The XFDF import writer previously updated an existing annotation's `/P` without
removing it from the old page's Annots or adding it to the destination. It also
read Annots only as a direct array and sorted existing references by number.

The import path now resolves indirect arrays before canonical rewriting, retains
existing annotation order, removes moved/deleted occurrences, appends selected
moves/creations to their destination, and verifies actual saved membership in a
single page pass. Ambiguous duplicate source annotation IDs are rejected.

Cross-page annotation transactions stage `/OBJR` and tagged appearance `/MCR`
page changes alongside `/P` and both Annots arrays. Explicit child `/Pg` entries
are changed; ancestor `/Pg` values used by unrelated children are preserved.
The logical hierarchy/order remains unchanged by a geometric move.

This staging is wired into both canonical XFDF import and the incremental story
annotation mover. Move/resize preserves existing appearances. Both annotation
appearance-regeneration entry points guard against discarding tagged appearance
programs without replacement ownership; saved tagged outputs are checked.

Shared tagged appearance occurrences still require explicit cloning/semantic
ownership before moving one occurrence. Tagged annotation deletion/flattening
without a corresponding structure removal is rejected by output validation.
These remain implementation boundaries, not newly claimed universal coverage.

## Regression source added, not executed

`tagged_structure.rs` contains cases for:

- distinct page/Form namespaces, reused old keys, sparse MCIDs and OBJR entries;
- direct element promotion and rebuilt ID lookup;
- duplicate IDs, duplicate owner nodes, dangling MCIDs and orphan annotations;
- incorrect number-tree limits and reversed child ordering;
- cross-page annotation movement with both OBJR and tagged appearance MCR;
- shared tagged appearances requiring occurrence isolation;
- cancellation;
- indirect Annots arrays and preservation of existing annotation painting order.

No passing test result is claimed. VPS qualification must compile the current
candidate, run these cases and independently inspect/extract/render real outputs.

## Full roadmap remains open

The subsequent `tagged_story_implementation.md` increment connects approved
page-owned paragraph leaves to the story transaction, including MCIDs, MCRs,
ParentTree, sibling order and save/reopen identities. Arbitrary inline semantic
subtrees and general tagged repagination are still not implemented. Other open implementation
areas include general tables/anchors, vertical/exotic typography, safe generated
page pruning, further native session bindings, production UI integration and
renderer/scan breadth. See `universal_editor_roadmap_tracking.md`.

The structural checker is bounded to page contents, invoked Forms, annotation
appearance graphs and explicit MCR streams. Patterns, Type3 charprocs, optional
content semantics, extension-specific relationships, arbitrary structure `/Ref`
graphs, semantic tag correctness and complete standards validation need further
work. Unknown child/owner semantics are not silently reconstructed.

## Primary references

- [Adobe PDF Reference 1.4](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.4.pdf): structure parent keys, MCID-indexed arrays and ParentTree.
- [Adobe PDF Reference 1.6](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.6.pdf): MCR stream, page and owning-object relationships.
- [PDF Association logical structure objects](https://pdfa.org/download-area/cheat-sheets/LogicalStructureObjects.pdf): structure children, IDs and lookup relationships.
