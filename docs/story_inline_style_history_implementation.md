# Causal inline-style history - source implementation and native materialization

This increment extends the causal story history to schema 5. It implements
stable rich-text intent, explicit conflicts, selective activity, browser review,
and linked-story PDF materialization across supported horizontal/vertical flow
and table cells.

## Identity and inheritance model

Inline formatting targets `StoryAtomRange` values, not mutable byte offsets.
Each range names Unicode scalar atoms from the immutable base paragraph or a
specific insertion operation. Text reflow, concurrent insertions and later
deletions therefore do not silently retarget an old style operation.

The governed fields are `preferred_font`, `font_size`, `rgb` and `shaping`.
Omitted fields inherit from the projected paragraph. An explicit `clear` restores
that inheritance. A field cannot be assigned and cleared by the same operation.
Selections must be non-empty UTF-8 ranges on grapheme boundaries; operations
store the resulting stable atom ranges after checking the exact selected text.

Text inserted by a schema-5 edit captures the observed resolved inline style.
Replacement text inherits the first selected atom. A caret insertion is
left-biased, except at paragraph start where it uses the right atom. An
unobserved concurrent insertion cannot accidentally inherit a remote mark.

## Causal conflict semantics

Each atom and field is a causal multi-value register. Causally superseded writes
disappear from the projection. Concurrent equal values converge. Concurrent
different values remain typed candidates and suppress the complete `merged`
draft. Resolution must:

- bind the exact history and inline-conflict hashes;
- choose exactly one field;
- target every currently conflicting atom for that paragraph and field; and
- observe every conflicting candidate in one successor operation.

Resolution operations are ordinary original operations. Same-replica selective
undo can therefore re-expose the conflict, after which publication is suppressed
until another exact resolution is recorded. Delete-versus-concurrent-inline-style
is treated as content intent in the paragraph-presence conflict model.

The projection emits canonical, non-overlapping visible UTF-8
`inline_style_runs`. It never serializes byte-offset marks as durable truth.

## Protocol and browser review

The retained-session protocol exposes both base-bound commands and Start/Resume
coordinator commands:

- `text_history_inline_style` / `history_inline_style`
- `text_history_resolve_inline_style` / `history_resolve_inline_style`

The browser history panel lets a reviewer select exact paragraph text, record a
JSON patch, inspect resolved runs and candidate operations, and resolve one
conflicting field over its complete atom set. Worker/client inputs are cloned
before queueing. These commands change only the logical draft; they publish no
PDF bytes and create no byte-level undo entry.

## Native checkpoint materialization

`StoryParagraph.inline_styles` carries canonical, sorted, non-overlapping
effective override spans, and story metadata reserves schema 5 for that model.
History projection populates those spans and seed schema 3 preserves a materialized
baseline across later epochs. The subsequent tab-stop increment uses seed schema
4 only when nondefault tab settings must also persist. Story layout converts each broken line
to a complete line-relative style partition. It intersects that partition with
resolved fallback-font spans, applies bidi visual ordering once to the combined
items, and uses the same shaped runs for prepared measurement and final PDF
emission. Per-run font size, colour, shaping settings, glyph advances and offsets
are retained; the complete logical line remains protected by `/ActualText`.

Font preparation resolves each effective inline segment against the approved
font pool, uses that segment's shaping settings for coverage, performs contextual
fallback when permitted and reports exact paragraph byte ranges. Saved metadata
is revalidated on load, including every inline UTF-8/grapheme boundary and field
budget. Every new frame stamps a canonical paint-model digest covering its
lines, style/font partitions, bidi context and decorations; checkpoint compares
that receipt after reopen, while the frame hash binds the emitted bytes. A
second shaped-run receipt binds the exact approved font programs and every
emitted glyph ID, CID, logical mapping, advance, offset, vertical orientation
and outline metric. The transaction retains each write receipt and requires the
same shaped-run digest after tagging, image, anchor and page-flow mutations.

- history prepare, join, edit, conflict resolution and export accept schema-5
  logical drafts, including after resuming a verified schema-2/3 seed;
- supported horizontal/vertical story preview/checkpoint paints and persists
  those drafts, including supported table cells,
  through the canonical source-owned frame writer;
- checkpoint reopen verifies the saved story, frame-owner content hashes and the
  complete paragraph/inline projection before the causal epoch resumes; and
- unsupported vertical typography constructs such as ruby/tate-chu-yoko retain
  the existing typed boundaries; uniform and inline-styled supported columns use
  the same run model.

Remaining work includes executable independent extraction/render evidence and
general source-object provenance outside these owned generated runs.
Merely storing marks in metadata is not accepted as materialization.

## Resource and verification boundary

Existing actor, operation, atom, byte and work budgets remain in force. Inline
targets are bounded and overlap-checked; conflicts are canonical and bounded.
Replica IDs remain host assertions, not authentication. Exported histories retain
deleted text and formatting intent and are not sanitizing redaction.

Regression sources cover stable targeting, inheritance, clearing, equal and
unequal concurrent values, exact resolution, selective undo, missing insertion
origins, schema enforcement, Start/Resume routing and native styled-run
preview/checkpoint/reopen retention.
They have not been executed. Only formatting, JavaScript syntax and whitespace
checks are permitted in this source-only phase. Compilation, tests, PDF workloads,
rendering, browser QA and corpus qualification remain pending.
