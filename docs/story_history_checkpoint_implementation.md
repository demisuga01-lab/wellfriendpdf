# Durable causal story checkpoints - source implementation

Continuation in the dirty `main` working tree over
`27e62db3a1b84804339e65b6025273fd003b3736`. This adds a save/reopen coordinator
around the existing plain-text history and native story writer. It does not
establish a universally editable PDF model or qualified collaborative product.

## Implemented path

`linked_stories::history` stores an optional causal checkpoint in the existing
`WellfriendStories` metadata stream, not in a second document store. The immutable
seed holds original paragraph IDs/text and the original PDF/story fingerprints;
physical source ranges and font bytes are not copied into the seed. Each checkpoint
records the canonical log/hash, seed hash, raw saved-model hash, parent checkpoint
hash, generation and input revision hash. The final output hash is returned rather
than embedded in itself. These hashes are consistency/CAS checks, not signatures,
authenticated authorship, tamper-proof provenance or mathematical PDF proof.

Two explicit source kinds are supported:

- `start`: a current-revision base and its causal history. Replacing a stored epoch
  requires `replace_epoch: true`; import never grants that decision implicitly.
- `resume`: current PDF hash, story ID, expected stored-checkpoint hash and incoming
  history. The coordinator verifies the persisted model and canonical log, then
  calls the existing loader to verify native frame/image/annotation ownership and
  rebind current page locations. It does not replay old byte offsets or select a
  visible duplicate by matching its text.

Resume unions all persisted events with incoming operations. Omitting operations
cannot erase saved history. Original scalar identities and actor sequences remain
valid across native checkpoints. Missing dependencies remain retained but prevent
layout preview/publication. The core's Unicode, causal and resource limits remain.
Stored model, seed and history together have a 16 MiB decoded metadata budget.
Resume reuses its opened document and verified projection instead of reopening
and reconciling that same input a second time. It still inventories source owners;
no measured latency or memory improvement is claimed.

Native layout preview binds the normalized source, history hash and existing
layout receipt. Checkpoint verifies those exact values again, writes the story
privately through the canonical writer, attaches causal metadata, reopens and
verifies the combined result, then publishes once. Undo/redo restore exact byte
preimages, including causal metadata. This is **one session publication**, not a
claim of a single incremental PDF revision; the existing writer can stage several
revisions. Output bytes/history can retain deleted wording and are not redaction.

Ordinary saves retain the causal record but do not advance it. If the raw saved
model changes outside this coordinator, resume reports a detached history. Hosts
must explicitly reconcile or start a new approved epoch. An unrelated PDF revision
can be discovered anew when saved model/owners still verify; the old Resume source
cannot automatically follow it. Modified or duplicated native markers fail closed.

## Integrations

Twelve commands (`history_resume`, `history_prepare`, `history_join`, `history_edit`,
`history_style`,
`history_delta`, `history_set_active`, `history_set_many_active`,
`history_preview`, `history_checkpoint`,
`history_compaction_plan`, `history_compaction_apply`) use the retained native
JSON session shared by C, Java, .NET, Python and WASM. The older `text_history_*`
commands deliberately retain their original-revision gate.

The browser worker separates preparation from publication. The client snapshots
queued inputs, keeps bounded byte undo, and recovers published state after worker
cancellation. The history panel supports saved-epoch resume, same-epoch local-log
rejoin, explicit new-epoch replacement and versioned sidecars. The host editor
ties an adopted draft to its causal source and routes preview/save appropriately;
draft edits outside that source require explicit detachment. Opening another PDF
clears that host association. No server, network transport or identity provider
is created by this increment.

## Source review and unexecuted regressions

Regression functions were added, **not executed**, across the native coordinator,
shared protocol and fake-worker client. They specify
save/reopen/remote join, actor continuation, exact undo, ordinary-edit detachment,
stale source/approval, incomplete deltas, persisted-event retention, explicit epoch
replacement, unrelated revisions, metadata corruption, cancellation, page insertion
and changed-source rejection. Compaction fixtures additionally specify exact
frontier/CAS approval, a new empty epoch, stale-epoch rejection and truthful bounded
session undo reporting. The fake worker is not an executed WASM binding.

Static inspection corrected a metadata decode-limit integer type mismatch and
clears a previous history receipt before starting another history preview. Rust
formatting/syntax and Git whitespace checks were performed; these are not compiler
or behavior evidence. No build, typecheck, tests, PDF workload, rendering, browser
QA, binding execution, benchmarks, deployment, commit or push was run.

## Still open

Inline rich-text and non-paragraph structural operation histories and undo, general
external-owner rebasing, conflict-resolution UI for those domains, transport/auth,
automatic/authenticated distributed garbage collection, and typed table-value
intent are not implemented here. Explicit complete-frontier logical compaction now
creates a plan-hash-approved replacement epoch; it does not sanitize historical PDF
bytes or prove replica acknowledgement. Font,
tag, layout, visual-object and scan limitations in the full roadmap still apply.
All convergence, native/binding, independent extraction/render, resource/cancellation
and save/reopen qualification remains pending. The full objective remains active.

Follow-up: `story_selective_undo_implementation.md` adds same-replica selective
and atomic grouped plain-text undo/redo through this coordinator. Durable
commands `history_set_active` and `history_set_many_active` cover one or an
explicit 1..=4096-operation group. Paragraph style and stable paragraph-list
activity are also covered; inline rich-text and non-paragraph block undo remain.

Follow-up: `story_history_compaction_implementation.md` documents the explicit
new-epoch transaction, browser review flow and its non-redaction boundary.
`story_paragraph_style_history_implementation.md` adds a seed-preserving causal
paragraph-style path with explicit concurrent-value conflicts.
`story_paragraph_structure_history_implementation.md` adds seed-preserving stable
paragraph insertion, movement, deletion, conflict resolution and selective undo.
