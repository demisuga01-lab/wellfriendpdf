# Causal paragraph-structure history - source implementation, unqualified

This increment extends the revision-bound story history with stable paragraph
membership and position operations. It is source implementation only: no build,
test, PDF workload, render comparison, browser QA or binding execution was run.

## Schema and merge model

History schema 4 adds a mutually exclusive `paragraph_structure` payload. A
structure patch can write two causal registers for one stable paragraph ID:

- `present`: keep/restore (`true`) or logically delete (`false`);
- `position`: the predecessor paragraph ID, with `after: null` meaning the start.

The unique insertion operation additionally carries the complete immutable
`inserted_paragraph` preimage and must write `present: true` plus an initial
position. IDs already owned by the base or another insertion are rejected.
Text and paragraph-style operations can address an inserted paragraph only after
observing its insertion operation.

Causally later values supersede values they observed. Equal concurrent values
converge. Different concurrent values remain typed candidates and suppress the
complete draft. A delete that did not observe a concurrent text/style edit also
becomes an explicit presence conflict; it never silently drops that content.
Resolution must bind the exact canonical history and complete structure-conflict
hash, then replace every conflicting field for one paragraph in one successor.

Projection begins with immutable base order and every stable insertion identity.
Selected position registers are applied deterministically as single-item
remove-and-insert operations. Therefore moving one paragraph does not drag its old
successors. Inactive/deleted paragraphs remain logical anchors, and explicit
predecessor cycles suppress publication until one involved position is resolved.

## Durable native path

Seed schema 2 already retained text and every paragraph formatting/pagination
field. Reopen now reconstructs the immutable seed paragraph list instead of
requiring the saved projection to have the original count/order. The canonical
history is then replayed and compared with the fully saved paragraph projection.
Inserted, moved and deleted paragraphs therefore survive the existing native
preview, exact receipt, checkpoint, reopen and re-edit path; no parallel PDF
writer or DOM-only document is introduced.

Commands are `text_history_structure` and `history_structure`. Status exposes
`history_paragraph_structure_protocol_version: 1`. Browser client methods are
`structureTextHistory` and `structureHistory`. The supplied history component can
insert, move, delete, resolve presence/position conflicts and selectively
undo/redo original structure operations. A projection remains an unsaved draft
until normal native layout review and checkpoint approval.

## Boundaries

- Paragraph IDs and replica IDs are caller-provisioned, not authenticated.
- A logical delete is not sanitizing redaction; operation history and earlier
  incremental PDF revisions can retain the paragraph and its text.
- Table, figure, anchor, tag and source-owner invariants remain governed by the
  native story validator/checkpoint. A structural history operation does not
  grant authority to detach or rewrite those owners.
- This implements paragraph list operations, not inline rich-text ranges,
  arbitrary block types, authenticated transport or automatic distributed GC.
- A delta must be joined with its retained causal prefix; an isolated later
  operation cannot reconstruct an omitted insertion preimage.

## Unexecuted regression source

Core regression source covers single-item movement, stable insertion followed by
text editing, selective insertion undo/redo, both keep and delete resolutions for
concurrent delete-versus-content, selective undo that re-exposes the conflict,
equal/different concurrent positions, explicit-cycle resolution, legacy-seed
refusal and schema/conflict checks. A
shared-protocol regression covers insert, native checkpoint, reopen, move, delete,
second checkpoint and final resume. Browser fixtures cover request snapshots and
capability gating. These are specifications in source, not execution evidence.

The full editor/rendering roadmap and every executable/corpus qualification gate
remain active.
