# Causal logical text histories — source implementation, unqualified

This increment continues the dirty candidate based on
`27e62db3a1b84804339e65b6025273fd003b3736`. It implements text-history exchange and
review; it does not complete the entire editor/collaboration roadmap.

## Model and algorithm

`story_text_history` adds an immutable PDF/story-bound operation set beside the
existing conservative snapshot/structural merges. It uses insertion predecessor
identities, descending logical-clock sibling order, depth-first projection and
retained deletion markers, following the
[RGA model documented by Inria's Concordant project](https://concordant.gitlabpages.inria.fr/software/c-crdtlib/c-crdtlib.crdt/-r-g-a/index.html).
This is a new implementation requiring qualification, not a reuse of that
project's verified binary or a proof of this code.

Each replacement records its actor/sequence identity, complete observed version
vector, canonical successor logical clock, paragraph, predecessor, compact
deleted-atom ranges and an inserted scalar chain. Base and inserted text have
distinct identities. Concurrent inserted runs remain present; deletion targets
only named observed atoms. Deleted anchors remain available to later descendants.
Concurrent incompatible replacements can therefore retain both proposed words:
the review must choose the intended wording. No last-writer winner silently
discards an author's insertion.

The canonical merge sorts unique operation identities and rejects identity
reuse with different payloads, missing/changed bases, noncausal references,
cross-paragraph references, bad offsets, non-transitive contexts and inflated
clocks. Duplicate JSON context keys are rejected. Missing dependencies remain in
the returned canonical history but suppress the entire projected draft; they
are not interpreted as deletion or silently discarded. A partial history reports
only contiguous received actor prefixes, so synchronization does not advertise
unreceived sequence gaps. Delta export uses receiver prefixes without deleting
the sender's retained history.
The subsequent schema-2 extension adds same-replica activity controls for selective
and atomic grouped undo/redo while retaining original atom identities. Schema-1 text histories keep
their serialization. See `story_selective_undo_implementation.md` for the precise
visibility rules, concurrency semantics and whole-PDF undo distinction.

Local text selection uses exact UTF-8 preimages and grapheme boundaries. Atom
identity uses Unicode scalar offsets. This does not guarantee semantic spelling
or grapheme intent after arbitrary concurrent combining-character operations.
Inline formatting is not flattened into mutable text offsets: the
[Peritext research](https://www.inkandswitch.com/peritext/) explains why rich-text
intent needs additional rules beyond a plain sequence CRDT. The subsequent
schema-5 layer uses stable atom-targeted field registers, insertion inheritance,
typed conflicts and exact resolution. It does not claim Peritext's complete
semantics; see `story_inline_style_history_implementation.md`.

The subsequent schema-3 extension adds causal **paragraph-level** style and
pagination registers with explicit multi-value conflicts. It uses the existing
native paragraph model and does not pretend that paragraph attributes solve
overlapping inline marks. See `story_paragraph_style_history_implementation.md`.

## Native save path and bindings

Rust entry points are `new_history`, `merge_histories`, `edit_history` and
`export_delta`. The shared retained-session protocol exposes:

| Command | Fields | Effect |
|---|---|---|
| `text_history_new` | `base` | Empty history and unchanged logical projection |
| `text_history_merge` | `base`, `histories` | Canonical union, dependency report and optional full draft |
| `text_history_edit` | `base`, `history`, `edit` | Compare-and-swap edit against the exact history hash |
| `text_history_style` | `base`, `history`, `edit` | Exact-history/conflict-bound paragraph-style write or explicit resolution |
| `text_history_structure` | `base`, `history`, `edit` | Stable paragraph insert/move/delete or exact structure resolution |
| `text_history_inline_style` | `base`, `history`, `edit` | Grapheme-safe selection converted to stable atom-targeted field writes |
| `text_history_resolve_inline_style` | `base`, `history`, `resolution` | Exact per-field replacement over every conflicting atom |
| `text_history_delta` | `base`, `history`, `peer` | Operations beyond receiver actor prefixes |
| `text_history_set_active` | `base`, `history`, `change` | Exact-history/activity-bound selective undo or redo |
| `text_history_set_many_active` | `base`, `history`, `change` | Atomic activity change for 1..=4096 unique original edits from one replica |

Every session command checks the base's PDF revision. These commands publish no
PDF bytes, grant no permissions and create no undo entry. A reviewed `merged`
request still passes through the existing native layout preview, exact receipt,
approval and checkpoint writer. It is not a DOM overlay or a second PDF writer.

The generic C/Java/.NET/Python/WASM command envelope reaches the same dispatcher;
no new C ABI symbol is introduced. Browser worker/client methods clone queued
inputs. An exported `wellfriend-story-history-editor` component is embedded in
the story editor: start from a draft, record text edits, import/merge a portable
history, inspect the projection and explicitly offer it as an unsaved draft.
The component can also debounce a live paragraph draft into one grapheme-safe
history operation per typing burst, preserving exact source UTF-8 offsets and
source newline conventions instead of deriving PDF edits from DOM indexes.
The host compares the expected draft before accepting it; unrelated local edits
are not overwritten. The component retains histories when PDF revisions change.
Files are imported/exported only on the user's action; no collaboration server,
network upload or executable PDF action is introduced.

## Resource, history and authority boundaries

- At most 256 actors/branches, 100,000 operations, one million scalar atoms,
  16 MiB canonical operation JSON, 32 MiB incoming operation JSON, and eight
  million counted expansion/causal work units. Individual operation text is at
  most four million bytes. Missing-dependency reports are bounded to 4,096 IDs.
- Logical clocks/sequences fit exact JavaScript integers. IDs are bounded ASCII
  identifiers, **not authenticated identities**. Hosts must provision unique
  replicas and authorize imported operations/transport separately.
- Arena indexes avoid storing an actor string per character; base scalar counts
  are prepared once, and traversal is iterative. Reconciliation still rebuilds
  the logical projection and is not an incremental per-keystroke performance
  claim. Cancellation is cooperative; sorting/serialization need further latency
  qualification.
- Exported histories contain deleted text and may include supplied font assets
  in their base story. They are not sanitizing redaction. Font permissions remain
  the host's responsibility. There is no implicit tombstone garbage collection.
  The durable coordinator now supports an explicit, exact-frontier, plan-approved
  replacement epoch that retires the current logical log. It does not erase old
  incremental-PDF bytes, authenticate acknowledgements or let old replicas rejoin;
  see `story_history_compaction_implementation.md`.
- An epoch is bound to its original PDF and complete story fingerprint. The
  original `text_history_*` API refuses direct replay on a later revision. The
  subsequent `history_*` coordinator now persists the seed/log and resumes through
  verified saved owners, preserving the epoch across approved native checkpoints;
  see `story_history_checkpoint_implementation.md`. This is not arbitrary external
  rebasing. Same-replica selective and atomic grouped text undo/redo plus browser
  typing-burst coalescing are now implemented in source;
  paragraph-level styles, stable paragraph insert/move/delete history and
  schema-5 inline rich-text intent with their selective undo are now included.
  Supported horizontal/vertical stories and table cells materialize those runs
  through native preview/checkpoint/reopen. Arbitrary
  block operation history, authenticated transport and typed table-value
  coordination remain work.

## Evidence

Regression functions were added, **not executed**: core cases for
ordering, join permutations, idempotence, tombstones, concurrent replacement,
out-of-order dependencies, delta rejoin, serialization, clocks/equivocation,
Unicode/preimages, malformed references, duplicate keys, cancellation and no-op
history; one complete native protocol/preview/checkpoint/stale-epoch case; and
fake-worker snapshot/no-publication cases. The fake worker is not WASM, and
source assertions are not runtime evidence or a formal convergence proof.

Rust formatting/syntax and Git whitespace checks are the only local checks.
No compiler, build, typecheck, tests, PDF workloads, rendering, browser QA,
benchmarks, binding execution, deployment, commit or push was run. The full
roadmap remains active and neither universality nor Acrobat superiority is proven.
