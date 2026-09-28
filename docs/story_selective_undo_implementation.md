# Selective text-history undo and redo - source implementation

Continuation over `27e62db3a1b84804339e65b6025273fd003b3736` in the existing dirty
`main` checkout. This implements same-replica selective text, paragraph-style,
paragraph-structure and logical inline-style edit activity, not whole-document collaborative undo,
authenticated authorship or runtime-qualified convergence. The full roadmap
remains active.

## Semantics and implementation

An undo records a new immutable control event setting one original text,
paragraph-style, paragraph-structure or inline-style edit inactive. Redo records another setting that same edit active. An atomic grouped
request canonicalizes 1..=4096 unique original edits from the same replica,
requires one complete history hash and a uniform expected activity, then appends
one causally chained control per target or publishes nothing. Both routes use the
replica's existing contiguous sequence space, complete causal vector and canonical
successor logical clock. Controls may target only an earlier original edit of
their own replica in the same paragraph. They cannot target other controls or
carry an insertion, deletion or predecessor payload.

The latest control in that replica's causal sequence determines the edit's
activity; arrival order does not. Unique-operation collision checks reject two
different events with the same ID. Missing controls/targets/dependencies remain
in the log and prevent a partial draft. This does not use an inter-author
last-writer rule to discard another author's text.

The projection retains every original atom and predecessor edge. An atom is
visible precisely when its original insertion is active (base atoms are always
eligible) and no active deletion names that atom. Therefore:

- Undoing an insertion hides its atoms, not other edits anchored under them.
- Undoing a deletion removes only that deletion's effect. Another active deletion
  can still keep the same atom hidden.
- Redo does not manufacture copied characters with new identities and does not
  bypass deletions made by another replica.
- Undoing a replacement changes its insertion and deletion effects together.
- Malformed atom references are checked even when their edit is inactive.

`inactive_operations` and `suppressed_atom_count` expose activity separately from
active deletion counts. All operations, hidden text and tombstones remain retained;
neither this API nor whole-PDF undo is sanitizing redaction. Existing history,
atom, expansion-work and metadata limits remain in force. Activity computation
uses bounded index vectors, not a copied operation payload per character.

This is **selective operation-effect undo**, not a claim that every restored
paragraph matches the user's semantic intent. For example, a later replacement
can remain beside restored earlier wording. Concurrent combining characters can
also need human correction. Review the complete resulting draft before native
layout/approval/checkpoint. Undoing a paragraph-style resolution can deliberately
re-expose the concurrent candidate conflict; publication stays suppressed until
that conflict is resolved again. Undoing insertion hides its stable paragraph while
retaining descendant events; redo restores the same identity. Undoing inline
formatting can re-expose its per-atom field conflict. Inline native paint is
available for supported horizontal/vertical stories and table cells;
non-paragraph block operations use separate models.

## Compatibility, APIs and UI

Ordinary text-only histories retain schema 1 and their canonical serialization.
Adding a control requires history schema 2; schema-1 controls are rejected.
Schema-2 deltas may contain only part of the log. Canonical merge derives the
history version from the retained event set; a complete persisted schema-2
checkpoint always retains its controls. Old clients must reject, not ignore,
schema-2 events. The outer saved-story and sidecar formats are unchanged.

`story_text_history::set_operation_active` takes an exact history hash, replica
ID, original edit ID, expected activity and requested activity. A stale history
or activity preimage is rejected; a valid no-op adds no event. It changes only
logical history and creates no PDF undo entry.

`story_text_history::set_operations_active` applies the same compare-and-swap to
an explicit edit group. Duplicate IDs, mixed replicas, control-event targets,
missing edits and partially active groups fail before publication. Canonical
operation ordering makes the resulting causal chain independent of request order.

`linked_stories::history::set_operation_active` applies the same rule after
verifying the current saved model, source owners and epoch. The shared session
commands are `text_history_set_active` / `text_history_set_many_active` (original
base/history) and `history_set_active` / `history_set_many_active` (Start/Resume
source). C, Java, .NET, Python and WASM use the
same existing command dispatcher. Browser client methods snapshot queued inputs.
New activity requires a newly reviewed native history-preview receipt before
checkpoint; an old receipt cannot authorize a changed log even if text happens
to match.

The browser history panel lists the latest 200 own edits for a paragraph, accepts
an exact sequence for older edits, and accepts an explicit comma/space-separated
atomic group that may span paragraphs. Undo/redo buttons use the selected
replica's current verified activity. State lookups use a prepared set rather
than scanning every inactive operation for every displayed choice. Its live
paragraph field also collapses a typing burst after 750 ms of inactivity (or on
blur) into the smallest contiguous grapheme-safe replacement, so the native
history receives one immutable operation and selective undo cannot expose a
partial burst. DOM UTF-16/newline normalization is mapped back to exact source
UTF-8 bytes; retained source line endings are preserved and newly inserted
newlines use the nearest source convention. IME composition is never flushed
mid-composition, and the native 4 MiB paragraph/edit limit is enforced before
queueing. Paragraph, replica and history
changes are disabled until the pending burst is flushed or discarded. Other
hosts must provide equivalent coalescing or intentionally author one operation
per edit; the native core does not infer timing from nondeterministic clocks. No
server, network transport or identity provider is added.

Replica IDs are not authentication. Hosts must provision distinct replicas,
authorize incoming events and retain their latest log/counter across offline
sessions. Whole-PDF byte undo restores a past snapshot and must not be used to
reset/reuse an already-issued replica sequence. Rejoin the retained complete log
and use selective controls, or start a separately identified replica/epoch.
The panel retains local events and rejoins them on resume; arbitrary external
hosts must implement that lifecycle too. Equivocation is rejected at merge,
not magically prevented by reading a replica ID from JSON.

## Research context

[Stewen and Kleppmann's undo/redo paper](https://arxiv.org/abs/2404.11308)
distinguishes collaborative undo semantics and develops a register algorithm.
It is not a proof of this sequence implementation, and its register-restoration
semantics are not presented here as identical to selective text-effect activity.
[Yjs documents origin-scoped selective undo](https://docs.yjs.dev/api/undo-manager).
The source here does not embed Yjs or inherit its qualification. These references
inform the explicit scope and distinction from global snapshot rollback.

## Verification boundary

Regression functions are present, **not executed**: core cases, durable
native checkpoint cases, shared-protocol cases and fake-worker routing cases.
They specify independent overlapping deletions, remote descendants,
replacement coexistence, Unicode identities, no duplicate atoms on redo,
out-of-order controls, six merge orders, schema compatibility, invalid targets,
CAS/no-op/cancellation, atomic grouped undo/redo, mixed-replica/duplicate/partial-
preimage refusal, save/reopen, stale-preview rejection and grapheme/newline-safe
typing-burst coalescing. The fake worker
does not exercise WASM or browser behavior.

Only Rust formatting/syntax and Git whitespace checks were performed. No compiler,
build, typecheck, tests, PDF workload, rendering, browser QA, benchmark, deployment,
commit or push ran. Formal properties, executable convergence, native/binding
behavior, memory/cancellation and independent visual/text evidence remain pending.
