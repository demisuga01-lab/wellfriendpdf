# Causal paragraph-style history - source implementation, unqualified

This increment extends the durable story operation log from plain text to the
existing paragraph-level typography and pagination model. It produces native
linked-story requests that pass through the same preview, source writer,
checkpoint and reopen path. It is not an HTML overlay; inline spans use the
separate schema-5 styled-run path. This document does not claim arbitrary
structural CRDTs or runtime qualification.

## Data and convergence model

History schema 3 adds a mutually exclusive `paragraph_style` payload. A patch can
write the approved paragraph font alias, size, line height, device-RGB colour,
direction, keep/break rules, widow/orphan counts, spacing, OpenType settings and
line-break policy. All numeric and resource bounds are checked before an operation
enters the canonical log; the final combined paragraph is revalidated by the
linked-story engine before a draft is emitted.

Each paragraph field is an independent causal multi-value register:

- a later write supersedes only operations named by its complete causal context;
- concurrent writes to different fields merge;
- concurrent writes of the same value converge without a false conflict; and
- concurrent different values remain as typed candidates with operation IDs.

There is no arrival-order or cross-author last-writer winner. Any unresolved style
conflict suppresses the complete draft and therefore native preview/checkpoint.
Resolution is another immutable schema-3 operation that must bind the exact
history hash and exact complete conflict-report hash, omit a fabricated current
value, and replace every conflicting field for one paragraph. Its causal context
observes all current candidates. Newly arrived concurrent operations make the
approval stale rather than disappearing.

Existing schema-2 selective undo/redo controls apply to original text or style
operations. Disabling a resolution exposes its prior conflict candidates again;
disabling one conflicting writer can also leave the remaining value applicable.
Operation identities and actor sequences are never reused.

## Durable baseline

History seed schema 2 stores the exact pre-history paragraph style beside each
immutable paragraph ID/text. Reopen restores that baseline before replaying style
operations. Without this, undo after save would incorrectly use the already-styled
saved projection as its base. Legacy schema-1 saved seeds may continue text edits,
but style operations require an explicitly approved replacement epoch so the
missing original style is never guessed.
New checkpoints use seed schema 3, retaining the same paragraph-style baseline
plus canonical materialized inline spans. Seed schema 2 remains accepted for
older checkpoints and implies an empty inline baseline.
The subsequent tab-stop increment retains schema 3 for default-tab stories and
uses schema 4 when the style seed contains nondefault tab stops.

Checkpoint comparison now covers complete paragraph state, not only IDs/text.
The canonical story writer therefore must save the selected style, and resume must
reproduce it, before the lineage is accepted.

## APIs and browser review

The shared protocol adds `text_history_style` and `history_style`; status exposes
`history_paragraph_style_protocol_version: 1`. Browser client methods are
`styleTextHistory` and `styleHistory`. The supplied history component accepts a
typed JSON style patch, derives exact expected values from the current projection,
shows all conflict candidates and requires an exact all-conflicting-fields patch
when resolving one paragraph.

## Boundaries and evidence

This is paragraph-level formatting. Character-range marks, overlapping inline
styles and links/comments within text still need their own identity and merge
model. Stable paragraph insertion/deletion/reordering is implemented by the later
schema-4 structure increment; arbitrary block operations remain open. Actor
IDs and conflict approval are consistency data, not authentication.

Regression source covers disjoint concurrent writes, same-field conflicts,
explicit resolution, selective undo, native checkpoint/reopen/re-edit and browser
request snapshots. These fixtures were not executed. No compiler, tests, PDF
workload, rendering, browser QA, cross-binding execution, benchmark, deployment,
commit or push was run.
