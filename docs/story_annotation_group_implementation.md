# Linked-story annotation groups: source implementation

Implemented in the dirty `main` candidate based on
`27e62db3a1b84804339e65b6025273fd003b3736`. This is not a build or runtime
qualification report. No Cargo, compilation, tests, PDF workloads, rendering,
benchmarks, browser execution, commit, push or deployment ran for this change.

## Implemented transaction

- Source discovery builds bounded connected components using `Popup`, popup
  `Parent`, and `IRT`/`RT`. Widget `Parent` remains field ownership, not a graph
  edge. Replies, review-state carriers and explicit `RT /Group` subordinates
  remain original PDF annotation dictionaries.
- An anchor must carry the exact `group` receipt returned by discovery:
  topology hash, complete sorted member IDs/geometry hashes/source pages and
  relative `Annots` paint order. Omitting the receipt never authorizes implicit
  movement of replies. Overlapping anchors and ownership by another saved story
  reject the transaction.
- Unnamed annotations, including ordinary unnamed popups/replies, receive a
  revision-and-object-bound discovery ID. Only approved members are stamped
  with a private `WFStoryAnnotationID` before canonical page insertion. This
  leaves their `/NM` absent and preserves identity across object renumbering,
  incremental editing, page pruning, save/reopen and subsequent story edits.
  The staging step verifies selected dictionaries against the original input.
  The follow-up identity increment also stamps named members, disambiguates
  repeated page-local names and supports explicit destination-name repair; see
  `story_annotation_identity_implementation.md`.
- Preview expands the selection into one movement per member. All members
  translate by the same delta into the same destination page; offsets between
  members remain unchanged. Geometry, page rotation and UserUnit are checked
  both during preview and by the internal writer.
- One incremental batch changes selected geometry, `/P`, both pages' `Annots`
  arrays and supported tagged OBJR/MCR page ownership. Original appearances,
  actions, field owners, reply references/types, review-state metadata and popup
  open state are retained. Within-page movement keeps `Annots` order. Cross-page
  arrivals preserve source-page order after unchanged destination annotations;
  request-ID sorting never becomes the group's paint order.
- Reopening verifies every member's page/rectangle and the unchanged topology
  and relative paint order. Saved story metadata stores updated receipts and
  rejects later external group changes. Group members participate individually
  in continuation-page vacancy and dependency checks; removal of a page still
  requires the existing final reference/content checks.
- Saved source page ordinals rebind after other page-tree transactions; they are
  not mistaken for stable object identity. Such rebinding cannot change approved
  member IDs, geometry, topology or relative annotation order.
- The shared session JSON protocol exposes these source and request fields to
  Rust, WASM, C, Java, .NET and Python. No new native ABI entry point was needed.
  The browser component now lists sources and complete groups, requires explicit
  approval, attaches a group to a paragraph, edits offsets, and unlinks the
  association without deleting annotation objects. Saved output remains native
  PDF content, not an HTML export.

## Standards basis and limits

The relationship model follows ISO 32000-1 section 12.5.6 and Tables 170/183:
`IRT` annotations belong on the same page; `RT /Group` refers to a primary
annotation; a popup's `Parent` is optional while `Popup` establishes its owner.
See the [Adobe-hosted PDF specification](https://opensource.adobe.com/dc-acrobat-sdk-docs/standards/pdfstandards/pdf/PDF32000_2008.pdf).

Current explicit boundaries remain:

- Page-owned, indirect annotation dictionaries with normalized nonempty finite
  rectangles; unique editing IDs; supported translation geometry only. Repeated
  page-local names now have distinct source IDs; duplicate persisted IDs require
  explicit import remapping. Direct or
  malformed annotation dictionaries are not generalized by this change.
- Connected members must start on one page and fit the destination CropBox
  after translation, with matching page rotation/UserUnit. This does not add
  rotation, resize, clamping, popup layout inference or conversion between page
  coordinate systems.
- Malformed/missing targets, reply cycles, inconsistent popup ownership and
  unsupported reply types are refused. The optional popup `Parent` is supported.
  Invalid relationship graphs found by discovery fail closed.
- Extended `Path`, measurement and `ExData` geometry and shared tagged
  appearances still require separate handling. Actions referring to other page
  locations are preserved, not heuristically rewritten.
- At most 100,000 inventoried annotations, 256 members/component, 262,144
  expanded receipt memberships, 16 MiB of identity bytes and 32 MiB estimated
  receipt payload. Existing geometry budgets and cooperative cancellation apply.
- Incremental edits/page removal retain historical bytes; this is not redaction.

## Validation source and evidence boundary

Five new regression functions in `story_annotation_group_tests.rs` cover:

1. Unnamed popup/reply movement, unchanged dictionaries, relative paint order
   and identity through canonical page insertion.
2. Missing/partial approval, overlapping anchors, partial moves and split deltas.
3. Story growth/contraction, continuation-page pruning, save/reopen/re-edit,
   cancellation and exact undo/redo bytes.
4. External topology, geometry and paint-order changes, cross-story ownership,
   and harmless page-ordinal rebinding after an unrelated page insertion.
5. Dangling/conflicting/cyclic relationships and a valid omitted popup Parent.

A sixth regression in `tagged_structure.rs` covers a group with two tagged OBJR
owners, an unnamed popup and an unnamed reply moving in one batch.
These are **unexecuted regression source**, not passed tests. Only rustfmt
formatting/parser checks, JavaScript syntax checking and whitespace checking
were used. Type correctness, PDF interoperability, visual behavior, tagged
output and browser interactions still require the authorized VPS campaign.

This closes the blanket popup/reply refusal for the declared linked-story path;
it does not close the full editor/rendering roadmap or support a universal or
better-than-Acrobat claim.
