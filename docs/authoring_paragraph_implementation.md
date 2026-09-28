# Authoring paragraph/font parity - source implementation, not qualification

This follows `generated_logical_carrier_implementation.md`. It changes the
separate fresh-document authoring path, not imported-document story ownership.
The full editor/rendering roadmap remains active. No compiler, build, test,
PDF workload, rendering, benchmark, deployment, commit or push was run.

## Source changes

Previously `PdfPageBuilder::text_width` measured a custom font using the bundled
Unicode font, while serialization used the registered custom bytes. Paragraph,
table and flow helpers also collapsed spaces, discarded blank paragraphs and
reshaped each wrapped line without its paragraph context. Raw Unicode drawing
could flatten several hard lines onto one baseline.

`authoring_layout.rs` now supplies one final-line plan to page paragraphs, table
cells, flow paragraphs and lists. It uses the existing `PreparedParagraph`
indexes and line-breaking algorithm, with paragraph-derived bidi and joining
context. Unicode measurement uses the actual selected font, final shaping and
outline/offset-aware metrics. The same font bytes, resolved line context and
default shaping settings feed PDF emission. Alignment includes measured left
overhang. Standard-14 text continues to use its existing simple-encoding metric
model; it is not silently converted into a different shaped font.

Custom font data is shared through immutable `Arc` assets and a copy-on-write
document registry. Existing pages receive registrations; document clones share
font bytes without making one clone's later registrations mutate another.
Drawing commands retain the selected custom asset. Font planning rejects a page
transferred into a document whose numerically identical font handle refers to
different bytes, even after registry refresh. A standalone page without that
document's registered assets no longer substitutes bundled metrics for them.

Logical and visible line strings remain separate. Spaces are not normalized.
Hard separators remain in per-line ActualText; blank-only lines emit real
Type0 source codes mapped to a checked empty TrueType space outline in the
built-in Unicode font. These carrier CIDs have exact separator ToUnicode values
and zero widths. The authoring width-array writer now preserves planned zero
widths rather than treating zero as a missing-metric sentinel. CRLF remains two
scalars, and a trailing separator does not invent an additional empty line.
The carrier is ordinary zero-outline text, not an invisible OCR layer.

Font planning reuses parsed metrics per font and already planned shaped runs.
Cancellation is checked during paragraph preparation, validation, flow, row and
font planning. Existing paragraph text/line budgets are retained, and a bounded
line result cannot silently truncate the requested text.

## API behavior and transaction changes

- `draw_text`, its single-run aliases and `text_width` require a single line.
  Use `draw_paragraph` for mandatory separators. Invalid/non-finite sizes and
  positions are rejected before commands are appended.
- `wrap_text` and `draw_paragraph` return visible lines, including empty lines
  and uncollapsed spaces. Those return values omit separator scalars; the
  internal plan and written logical text retain them. They are not a logical
  round-trip interchange format.
- Page paragraphs stage all commands before appending. Failed table drawing
  restores the prior command count. Flow paragraphs, lists and tables restore
  page counts, command counts, active page and cursor on error or cancellation.
  This rollback is append-only; it does not clone all fonts/images/documents.
- Flow/table top positioning uses measured ascenders and reserves at least the
  measured ascent/descent extent or requested line height. Lists allow marker
  width to enlarge the hanging indent. Page `draw_paragraph` retains its explicit
  first-baseline anchor and user-selected baseline interval.
- Invalid derived dimensions, nonpositive line heights, undeclared extra table
  cells and non-finite table geometry are rejected. An unsplittable authored row
  plus repeated header that exceeds usable page height is rejected instead of
  being painted outside the page. The first header reserves room for its first
  row rather than being left alone at a page bottom. The subsequent
  `authored_table_pagination_implementation.md` increment adds line-boundary
  row fragmentation; explicit `KeepTogether` retains this refusal contract.
- The subsequent `form_feed_pagination_implementation.md` makes U+000C advance
  `FlowDocument` to a later physical page and adds next/odd/even page APIs.
  `PdfPageBuilder::draw_paragraph` now rejects U+000C because a page-local
  builder cannot own or create its successor. Authored table cells reject it
  until row-level page-break semantics can own the complete grid transaction.
- The subsequent `tab_stop_layout_implementation.md` makes U+0009 an exact
  positioned field separator rather than a shaped glyph or guessed spaces.
  Paragraph, fallback, flow/list/note/table/field and tagged-owner paths share
  left/right/center/decimal stops, bounded default continuation and zero-advance
  logical tab carriers without renumbering older carrier codes.

These are intentional corrections to previously lossy or mismatched behavior.
Callers that passed multiline text to a single-run API or depended on collapsed
spaces must use the paragraph API or normalize input explicitly.

## Unexecuted regression source

Fifteen functions in `authoring_layout_tests.rs` cover actual custom-font
measurement/emission, late registration, clone sharing, foreign-page font
identity, missing page assets, paragraph-derived Arabic/mixed-direction lines,
all hard separators and blank-only carriers through save/reopen, zero-width
outline/CMap plans, uncollapsed spaces, overwide graphemes, invalid geometry,
cancellation, flow rollback, cross-page blanks and authored table behavior.

Only Rust formatting/parser checks, source inspection and Git whitespace checks
were run. These are not type checking, compilation, executed regressions,
independent PDF extraction, pixels or interoperability evidence.

## Remaining roadmap

Explicit multi-font authoring fallback is now extended by
`authoring_font_fallback_implementation.md`; that source increment remains
unqualified. Authored text-row fragmentation is extended by
`authored_table_pagination_implementation.md`. Still open are vertical/ruby
typography, user-defined tab leaders/locale-inferred decimal tokens, arbitrary
glyphless-font semantics, accessibility
tagging, section-master/header/footer/numbering/footnote pagination or broad
font-program support. Default shaping settings are used here; story-level
language/features and composition policies are not exposed as authoring options.
Standard-14 outline fidelity remains dependent on the consumer's available font.
The subsequent `shaped_font_coverage_implementation.md` increment confirms that
joiner/default-ignorable removal already existed in both shapers. It unifies
cluster/outline coverage, distinguishes source-bound blank spacing from missing
subset outlines, and corrects the approval/reflow cmap-only gates. Arbitrary
glyphless logical persistence and intended font-program semantics remain open;
coverage is not proof of all joiner/font cases or visual fidelity. Supported
standalone logical controls are now persisted by the subsequent
`logical_control_carrier_implementation.md` increment; arbitrary font-program
semantics and per-control selection geometry are still not established.

The changes do not establish exact text selection geometry for every ActualText
span or independently validate a saved PDF. Broader imported-object preservation,
renderer limits, binding/browser behavior and all VPS/build/corpus qualification
remain as tracked in `universal_editor_roadmap_tracking.md`. This is not a
universal-editor or better-than-Acrobat completion claim.
