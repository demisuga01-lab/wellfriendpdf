# Annotation relationship lifecycle transactions

Source-only continuation on dirty `main`, base
`27e62db3a1b84804339e65b6025273fd003b3736`. No Cargo, compilation,
typechecking, tests, PDF workloads, rendering, benchmark, deployment, commit or
push ran. The full editor/rendering roadmap is still incomplete.

## Shared graph, not independent writer guesses

`annotation_relationships.rs` is now the relationship model used by XFDF export,
import validation and native story/group discovery. It resolves actual
page-owned source references using the shared annotation identity index.
Widget `/Parent` remains a field-tree relationship, never a popup edge.

The graph models IRT/RT replies, Popup ownership and optional popup Parent.
It supports parentless popups and ownership established by the owner's Popup
entry without an explicit Parent. It checks unique ownership, reciprocal
relationships, same-page references, valid markup targets, Group primaries and
reply cycles. The requirements and distinction between replies and Group
annotations follow the [Adobe PDF Reference, section 8.4](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.6.pdf).
The optional Parent and standalone popup cases are described in
[PDF Reference 1.7, table 8.34](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.7old.pdf).

Cycle checking uses a global completed-node set, so checking every member of a
long reply chain does not repeatedly walk its full ancestry. Graph, identity,
edge-byte and geometry budgets remain explicit; cancellation is cooperative.

## XFDF operations implemented in source

Before allocating output objects, `annotation_xfdf_relationships.rs` constructs
and validates the final graph after all explicit updates, creations and
deletions. This allows these operations to compose in one returned transaction:

- Create a reply or popup, including parents created in the same request.
- Reparent a popup while clearing the old owner's Popup entry and setting the
  new owner's entry. Multiple popups cannot silently replace one another in
  the same owner's single slot.
- Reparent replies or explicitly clear their IRT/RT relationship.
- Delete a popup and clear the surviving owner's backlink.
- Delete owners and dependents together, or reparent surviving dependents in
  the same transaction. There is no implicit cascading deletion.
- Move a closed relationship component across pages. An omitted dependent
  that would create a cross-page edge rejects before mutation. Moved source
  annotations retain their original relative Annots order rather than being
  sorted by opaque editing IDs.
- Detect conflicting duplicate import records and subtype changes instead of
  selecting whichever contradictory topology happened to be encountered first.

Normal safe-field merge preserves omitted reply fields. Explicit removal is
represented by additive record flags `clear_reply` and `detach_popup`, serialized
as namespace-qualified `wellfriendpdf:clear-reply="true"` and
`wellfriendpdf:detach-popup="true"`. Prefix aliases resolve by namespace URI;
invalid values and contradictory flags/targets reject. Replace policy treats
omitted reply fields as removal. Popup detachment remains explicit even under
Replace policy. New popups may be parentless.

Source IDs of surviving participants in the affected relationship components
are persisted. Implicit reciprocal/identity patches start with the original
source dictionaries; they do not feed other annotations through the sanitizing
XFDF field writer. Explicitly imported records retain the established action
sanitization policy. Creating a reply is not permission to strip its parent's
action dictionary.

The additive `relationship_transaction` report lists exact old/new relationship
targets, persisted source IDs, reciprocal-only participant updates, final graph
verification and whether tagged ownership was rebuilt. Reciprocal-only entries
may include unchanged relationship participants whose identity was persisted.
After save/reopen, verification follows actual output object references, page
membership, subtype, reply type, popup ownership and identities, not just strings
in the import report. It also checks the order of surviving indirect residents,
moved source annotations and new annotations. Unselected revision-scoped IDs are not mistaken for stable
persisted IDs after canonical renumbering.

## Tagged deletion and simultaneous movement

`tagged_annotation_deletion.rs` composes exact content-carrier deletion with
existing annotation page migration. It starts from staged migration edits when
both affect the same StructElem K array, so deletion cannot discard a preceding
page-reference change.

It removes OBJR entries for selected deleted annotations and MCR/OBJR carriers
for their exclusively owned tagged appearance scopes, including nested invoked
Forms. Scopes still used by surviving annotations or page content require
occurrence-specific semantic cloning and are rejected before publication.
The unpublished candidate then rebuilds ParentTree/IDTree against surviving
owners and validates the result. No intermediate candidate is returned.

Unrelated structural content and metadata remain. Empty semantic containers
are retained rather than guessing which roles or ancestors should disappear.
This is ownership repair, not a promise of semantic PDF/UA conformance.

## Boundaries still open

- Ordinary direct annotations and their source relationship components now use
  exact-occurrence promotion (`annotation_source_promotion.md`). Unique reachable
  widget/tag owners are supported by `annotation_owner_promotion.md`; ambiguous
  owners and direct structure ancestors remain open. Direct field ancestors now
  use `annotation_field_materialization.md`; unrelated source content remains.
- Existing malformed relationship graphs need a dedicated explicit repair
  workflow; export/import no longer silently drops unresolved source edges.
- Foreign-document import and duplicate persisted-ID remapping remain open.
- Shared tagged appearance cloning, arbitrary semantic subtree changes, role
  requirements, reference-based structures and complete standards qualification
  remain open. XFDF deletion cannot orphan an AcroForm widget; canonical field
  deletion is required.
- This work does not infer layout movement for linked stories or silently
  rewrite saved story approvals after an external annotation mutation.
- Action sanitization differs between explicitly imported records and native
  geometry/reciprocal-only updates. Existing actions are never executed.
- XFDF uses a full rewrite; tagged finalization may add an internal incremental
  revision. Neither path is advertised as sanitizing redaction or as preserving
  existing signatures. Encryption/profile coverage remains separately bounded.

## Evidence

Nine new unexecuted tests in `annotation_relationship_tests.rs` cover creation,
reparenting, action/opaque-field preservation on implicit owners, deletion,
cycles, Group constraints, cross-page closure/order, optional and absent Parent,
duplicate records, explicit flags, widget protection and a 400-node reply chain.
A further unexecuted regression in `tagged_structure.rs` combines deletion of
one annotated appearance and movement of another annotation within the same
semantic owner, checking surviving K and ownership tables after reopening.

Allowed checks were rustfmt formatting/parser checks and whitespace checking.
They passed, but they are not compiler, regression, interoperability or visual
evidence. VPS qualification and the full roadmap remain required.
