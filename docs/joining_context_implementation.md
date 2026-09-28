# Paragraph joining context — source implementation, not qualified

This increment addresses a remaining discrepancy between paragraph-aware bidi
layout and line-local glyph shaping. Previously a chosen line break discarded
all characters on the other side. A wrapped Arabic word could therefore use
isolated/final/initial forms different from its unbroken joining context, even
though its bidi levels and logical selection remained correct.

The follow-up `joining_synopsis_implementation.md` extends the raw context with
the nearest nontransparent scalar beyond a long mark run and supersedes the
five-scalar joining limitation below for the pinned state machine. It also bumps
the native/browser capability to version 2.

## Implemented source paths

- `fonts/shaping_context.rs` represents non-emitting `before`/`after` neighbours.
  Each edge is bounded to **five Unicode scalars / twenty UTF-8 bytes**, matching
  `CONTEXT_LENGTH` in the pinned Rustybuzz 0.20.1 buffer. No complete paragraph
  string is cloned into every line or font run. UTF-8 boundaries are checked.
- `ParagraphBidi::line` now returns this context with its L1-adjusted levels.
  Forced-break classes BK/CR/LF/NL stop context propagation. The paragraph
  lookup uses the ordered range index instead of rescanning all previous hard
  paragraphs for every line request.
- `LineBidi::slice` composes the inner run's neighbours with the outer context.
  Font, bidi, script and vertical-orientation itemization retain logical context
  without resetting paragraph direction or putting neighbouring characters into
  the shaped glyph buffer.
- Horizontal and upright/sideways vertical shaping apply the context after
  adding the selected text to the Rustybuzz buffer. Final glyph clusters remain
  local to that text. Context does not become extra glyphs, ActualText or
  ToUnicode entries. Existing glyph advances and offsets remain authoritative.
- Shape-cache keys include both context edges with length framing and a new
  key-domain version. The authoring font-plan key already includes `LineBidi`;
  its derived hash/equality now also distinguishes context. Identical text in
  different joining contexts cannot alias those entries.
- Default and explicit-feature paragraph shaping share one resolved-line
  implementation. Paragraph bidi analysis is retained once per input, rather
  than rebuilt for every hard paragraph. Explicit shaping settings also retain
  automatically detected paragraph direction rather than reporting LTR for an
  auto-detected RTL paragraph.
- Prepared story/table line measurement and final story emission already share
  `LineBidi`; they now carry the same context into both paths. Shared authoring's
  `draw_text_resolved` retains it and rejects malformed context before appending
  a command. Existing reflow paths retaining `LineBidi` inherit the same change.

## API and compatibility

`LineBidi` adds optional serialized `context: { before, after }`. Empty context
is omitted; older JSON with only levels and `rtl` reads as an isolated fragment.
Rust callers constructing the public struct must initialize the added field;
in-repository literals were updated. Arbitrary externally supplied oversized or
hard-break-containing context is rejected, not silently truncated on use.
Automatically derived edges are bounded before they are stored.

Native retained-session status now reports `line_shaping_context_version: 3`
after the shared hard-line update, in
addition to `line_break_policy_version: 2` after the balanced-composition update.
The browser worker requires both;
deploy its modules and generated WASM together. Other hosts should check native
status when this behavior is required. Saved-story request schemas are unchanged
by this increment: context is derived from current paragraph text during layout,
not retained as stale neighbouring text in the story request. Preview lines and
authoring plans do retain their exact context.

## Design basis and limits

[CSS Text 3, line breaking details](https://www.w3.org/TR/css-text-3/#line-break-details)
requires contextual joining forms to survive soft wrapping inside words.
[HarfBuzz's buffer documentation](https://harfbuzz.github.io/harfbuzz-hb-buffer.html#hb-buffer-add-codepoints)
recommends supplying surrounding paragraph context for shaped subranges. The
implementation also follows the actual installed Rustybuzz buffer and Arabic
joining-state code, not an assumed API contract.

This is **not full contextual-typesetting closure**:

- The raw library context still contains only five scalars per edge. The
  subsequent joining synopsis preserves the actual nearest nontransparent
  scalar beyond long transparent runs for the pinned joining state machine.
  General font context outside that model remains unqualified; see the follow-up
  report for its dependency pins, source-data license and limitations.
- Context affects the library's contextual analysis; it does not shape a whole
  word and split arbitrary ligature/conjunct clusters afterward. Grapheme-safe
  wrapping is not a proof that every possible font's shaping cluster is safe at
  every permitted break. General script/font constraints remain open. In
  particular, HarfBuzz's unsafe-to-break flag requires reshaping, not blanket
  rejection; final-line shaping already reshapes these paths.
- No complete hyphenation, script dictionary, ruby, tate-chu-yoko, variable-font
  instancing or global pagination algorithm is added here.
- Callers supplying an isolated `LineBidi`/visual line without parent text cannot
  obtain missing neighbours automatically. Original source-glyph editing is not
  silently re-shaped; the changes concern generated text with an available
  paragraph model.
- The short context strings are present in preview/authoring data. Those records
  are not a sanitizing-redaction product. Existing source/revision/privacy
  policies still apply.

## Verification boundary

Eighteen new regression functions are **unexecuted**: thirteen shaping-context
cases, two authoring cases and three story/table integration cases. They cover
UTF-8 bounds, nested slicing, hard breaks, legacy JSON, context-aware cache keys,
Arabic forms, font changes, vertical orientation, bounded measurement, shared
entrypoints, malformed input, cancellation, CID/Unicode isolation, preview
serialization and edit/checkpoint/reopen. Existing native/browser capability
tests are extended, also unexecuted.

Direct Rustybuzz expected values test context plumbing but are not an independent
renderer oracle. Only Rust parser/formatting, JavaScript syntax and Git whitespace
checks are allowed in this phase. No compiler, type check, test, PDF workload,
render, benchmark, browser/device QA, deployment, commit or push was performed.
Exact-build tests, multilingual/long-mark/conjunct corpora, independent pixels,
extraction and cross-binding save/reopen remain VPS qualification work. The
complete editor/rendering roadmap remains active.
