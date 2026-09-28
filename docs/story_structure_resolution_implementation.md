# Reviewed structural merge resolution — source implementation

This increment continues the dirty `main` candidate over
`27e62db3a1b84804339e65b6025273fd003b3736`. It is not a compiled, executed or
production-qualified release. No build, test, PDF workload, renderer, browser,
type check, benchmark, deployment, commit or push was performed.

## Implemented behavior

The existing structural merger now supplies a typed review and a constrained
resolution route. It still merges snapshots against one exact logical story and
PDF revision; it does not merge arbitrary PDF byte edits or grant new authority.

- `review_story_structure` returns a provisional candidate, unplaced paragraph
  values, typed conflict targets, base/branch alternatives and a review hash.
  Paragraph IDs containing slashes are not parsed as patch paths. Branch arrival
  order does not change the canonical review.
- `resolve_story_structure` requires the current review hash and exactly one
  acknowledgment for every conflict. It accepts logical paragraphs and existing
  frame geometry, not a replacement source/permission model. An acknowledgment
  is a caller decision, not authentication or proof that a person reviewed it.
- Only fields or whole paragraphs covered by an actual conflict may change.
  Automatic, non-conflicting values and protected paragraph order are retained.
  Unknown identities and silent loss of unplaced, non-conflicting insertions are
  rejected. Membership/order conflicts allow explicit choices within their
  scope, not arbitrary source rebinding.
- Table text is no longer silently overwritten during structural merge by
  `synchronize_values`. An inconsistent typed-value projection is now a conflict.
  Resolution must agree with the retained table topology and values; the UI can
  explicitly propose recalculation for review. Editing the typed value/formula
  itself requires a separate table draft, not this merge route.
- Retained Figure captions and annotation anchor paragraphs must exist. Removing
  a dependency-owned paragraph requires restoring it here or separately
  replanning that object's ownership/deletion. Restoring a paragraph does not
  itself permit unrelated style or text changes.
- Derived tag mappings are rebuilt from the original base identities. A restored
  paragraph recovers its original owner even after provisional deletion or
  reordering; it does not inherit another paragraph's ordinal tag.
- Source bindings, source frame identities/order, font assets, images, tables,
  annotations, tags, page-creation/pruning options and signature policy remain
  inherited from the base. The ordinary native planner/writer still validates
  sources, fonts, geometry, layout and mutation policy before a save.

The legacy `merge_story_structure` API still returns no merged draft when
conflicted. A review candidate or resolution hash is not a native layout receipt.
The resolved request must pass `preview` and its exact `checkpoint` receipt.

## Native and browser integration

The retained-session JSON protocol adds `review_structure {request}` and
`resolve_structure {request,resolution}`. Both check the open PDF revision and
return data without modifying PDF bytes, undo history or layout approval.
Existing C, Java, .NET, Python and WASM generic session-command entry points
delegate to this same dispatcher; no new ABI symbol or separate writer exists.

The module worker/client expose `reviewStructure` and `resolveStructure`, taking
owned snapshots of queued arguments. The `wellfriend-story-structure-editor`
component is exported by the WASM package and embedded in the story editor.
It provides:

- base-bound branch/package import/export and native recomputation of reviews;
- paginated conflict cards with lazy text-only alternative displays;
- a logical-only draft editor, retained unplaced paragraphs, scoped alternative
  selection, explicit table recalculation and per-conflict acknowledgment;
- a separate complete-draft approval before adoption, followed by the host's
  existing native layout preview and save;
- host-draft preimage checking, refusal while causal history is attached, and
  explicit notices that structural saves may detach a persisted text epoch;
- resetting acknowledgments/approvals on imports, and confirmation before a new
  branch recomputes and replaces pending local decisions.

Packages preserve pending decisions only when their saved review hash matches a
new native review. They do not restore approval. They include source text and
supplied font assets, so hosts must handle them as document data; no transport,
storage service or authentication layer was added. Restored/new paragraphs are
appended in the editor for explicit order review, not automatically placed by
inferred semantics. The final native scope check remains authoritative.

## Bounds and cancellation

The existing 64-branch limit remains. Additional aggregate limits cover 200,000
paragraph snapshots, 16,000,000 source text bytes, 256 MiB font bytes, 4,096 base
frames, and bounded identities. Resolution allows at most 100,000 paragraphs,
4,000,000 text bytes and 4,096 frames/conflict acknowledgments. Reviews allow at
most 4,096 conflicts and 16 MiB of encoded alternatives. Session commands and UI
packages retain their separate 32 MiB transport limit.

Alternative lookup indexes paragraph/frame identities once per snapshot. Scalar
style alternatives do not clone entire paragraph text or source-bearing frames.
The merger serializes paragraph snapshots once per paragraph/branch for style
comparison. Cancellation is polled during merging, review and resolution, not
guaranteed inside every serialization call. These are source bounds, not measured
latency or memory results.

## Regression source and verification boundary

Fourteen new regression functions were added, **not executed**:

- twelve pure resolution cases covering conflict masks, ordering, insertions,
  stale/duplicate/missing acknowledgment, authority, table text, annotation
  dependencies, original tag ownership, cancellation and no-conflict equivalence;
- one native protocol case spanning review, resolve, no early publication,
  preview/checkpoint, reopen text and stale-revision refusal;
- one fake-worker client case covering queued snapshots and unchanged PDF bytes
  and undo history. A fake worker does not establish WASM/native/browser behavior.

Static review, Rust formatting and patch whitespace checks were performed. They
do not establish compilation, binding parity, convergence, visual preservation,
browser usability/accessibility or runtime correctness.

## Still open

This closes a report-only conflict workflow, not the entire editor roadmap.
Rich-text/structural causal operation histories and undo, automatic epoch
migration, arbitrary external rebasing, source-authority merging, general
dependent-object migration, production authentication/replica lifecycle, richer
visual conflict tools and all executable qualification remain separate work.
The full ledger is `universal_editor_roadmap_tracking.md`. Universal editing or
superiority over a named Acrobat version is not established.
