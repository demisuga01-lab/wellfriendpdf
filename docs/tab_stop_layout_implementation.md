# Tab-stop layout - source implementation, not qualification

This increment implements deterministic paragraph tab stops in the shared
authoring and linked-story layout paths. It is source work only. No compiler,
test, PDF workload, renderer, browser, binding, benchmark, commit, push or
deployment was run.

## Contract

U+0009 is layout state, not a glyph and not a guessed run of spaces. A paragraph
can declare up to 256 strictly increasing, positive, finite explicit stops plus
a positive default interval. Explicit stops support left, right, center and
single-character decimal alignment plus an optional caller-supplied exact token
of up to 16 Unicode scalars / 64 UTF-8 bytes. A logical line is bounded to 4096
tabs.
Every explicit stop may also request a dot, dash or solid leader across the
resolved unoccupied gap and/or a perpendicular bar rule at the stop. These are
painted artifacts: they never become Unicode text, structure content or search
results. Default continuation stops never inherit decoration implicitly.

The planner returns exact UTF-8 source ranges for every field. It measures each
field through the same selected font, fallback, shaping, bidi and inline-style
path used for final emission. A right, center or decimal stop that would overlap
the preceding field is skipped; the next usable explicit stop is tried. If no
explicit stop can accept the field, layout advances to the next non-overlapping
default stop. Non-finite measurements and arithmetic overflow fail closed.
Tab discovery enforces its limit while scanning rather than after an unbounded
offset allocation. Each field width is measured once per plan; repeated decimal
tokens reuse one cached prefix measurement while candidate stops are tried.
The shared line breaker removes soft/emergency opportunities inside a tabbed
hard line. A positioned row therefore fits as one row or is rejected; it never
wraps a later field back to inline origin zero. Callers can insert an explicit
hard separator when a new row is intended.

## Integrated paths

- `ParagraphStyle::tab_stops` drives page paragraphs, flow paragraphs, lists,
  authored table cells, deferred body fields and notes/endnotes.
- Single-run `draw_text`/`text_width` reject U+0009 because they have no
  paragraph tab-stop contract; tabs are never passed to their font shapers.
- `StoryParagraph::tab_stops` drives horizontal and vertical linked stories and
  shared story table cells. Tab fields retain paragraph-derived bidi context,
  exact font/style partitions and full-line logical `/ActualText`.
- Saved stories require schema 6 when text contains a tab or undecorated tab
  settings are nondefault, schema 7 when a leader or bar is present, and schema
  8 when a multi-character decimal token is present. Causal paragraph-style
  history captures, patches, restores and validates tab settings. Durable seeds
  with nondefault undecorated settings require seed schema 4; decorated settings
  require seed schema 5; multi-character decimal settings require seed schema
  6; default-tab stories retain schema 3 compatibility. The retained-session
  status exposes `tab_stop_layout_version: 3`, `saved_story_schema_max: 8` and
  `history_seed_schema_max: 6`.
- Fresh authoring emits positioned child runs under one logical text group.
  Tagged/owned authoring keeps exact field provenance, assigns non-painting tab
  carriers to the correct semantic owner and positions visible fields without
  reshaping tabs. Leader and bar paths are enclosed in artifact marked content
  and use the row's paint colour without becoming owned logical children.
- U+0009 has a zero-advance logical-carrier code allocated after the historical
  separator/default-ignorable code space. Existing carrier codes are unchanged.

The paint model and shape receipt bind each story field's source range, origin,
width, leading pad, font/style selection and shaped glyph provenance. Reopened
story transactions therefore cannot silently turn tabs into spaces or accept a
different field placement under an old receipt.

## Deliberate boundaries

This is not a complete desktop-publishing tab system. It does not implement
locale inference, arbitrary tab-stop expressions, user-defined leader glyphs,
or tab-driven table construction. Decimal alignment uses either the legacy
single visible character or the caller-supplied exact bounded token; it never
guesses from document language. Oversize field plans are rejected instead of
painting beyond the approved line extent.

It also does not prove visual correctness. Regression source covers alignment,
default-stop fallback, artifact-only leaders/bars, logical extraction, semantic
ownership, exact story fields and schema selection, but remains unexecuted.
Build, save/reopen, independent extraction/rendering, binding, browser,
performance and corpus qualification remain required by
`universal_editor_roadmap_tracking.md`.
