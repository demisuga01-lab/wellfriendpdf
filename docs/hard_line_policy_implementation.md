# Shared hard-line shaping policy — source implementation, unqualified

This closes the specific separator-list divergence recorded in
`balanced_line_composition_implementation.md`. It does not complete typography,
pagination, PDF editing or the full roadmap.

## Source changes

`fonts/hard_break.rs` supplies one predicate using the existing Unicode line-break
data's BK/CR/LF/NL classes. It covers CR, LF, VT, FF, NEL, LINE SEPARATOR and
PARAGRAPH SEPARATOR. CRLF is one logical separator. Ordinary spaces, tabs,
nonbreaking spaces, word joiners and bidi controls are not stripped as hard
breaks. This policy is for generated Unicode, not PDF byte-token whitespace,
existing source glyph codes, or a license to normalize original PDF strings.

The borrowed, cancellable hard-line iterator retains exact logical byte ranges
and separate visible ranges. It preserves consecutive blank logical lines but
does not invent an extra final line after a trailing separator. It neither
clones the paragraph nor allocates a line list. Cancellation is polled every
1,024 source scalars. Canonical shaping callers retain their 4 MB input budget.

`ParagraphBidi::all_hard_lines` intersects these lines with the already analyzed
bidi paragraphs and supplies their original resolved levels. It suppresses a
phantom CRLF blank line when the bidi dependency represents CR and LF as separate
type-B paragraphs. Empty-input coverage still validates the font/settings; an
early coverage failure stops traversal. There is no unconditional bidi restart
at LS/VT/FF: a directional isolate can remain active across a line separator.

The shared policy now feeds:

- Horizontal paragraph shaping and vertical font coverage.
- Single-font line metrics, story/table metrics and retained paint-line levels.
- Generated horizontal/RTL/vertical reflow, explicit visual-layout validation,
  styled source-font replacement and final story emission.
- Joining-context stop boundaries and paragraph break-policy edge classification.
- Already-resolved horizontal/vertical line APIs and resolved authoring input
  validation. Those single-line APIs reject embedded hard separators; callers
  use paragraph layout for multiline text. Authoring rejects before appending a
  command, rather than leaving a bad command to fail later during serialization.

Logical line strings still include their original separators. Only font input
and visible glyph ranges omit the separator. Paragraph shaping returns an
aggregate glyph run with clusters offset into the original full string; it does
not itself position multiple baselines. Layout/emission own those positions.
Neither font coverage nor paint should require a font glyph for VT or FF.

The paragraph and resolved-line cache domains advance to version 4. Native
status and the matching worker require `line_shaping_context_version: 3`;
`line_break_policy_version` remains 2. Deploy the guard, worker and rebuilt WASM
together. No new request field or saved-story schema is introduced: request
text is already retained, and layout regenerates line state after reopening.

## Standards basis and important boundaries

Unicode distinguishes forced line breaks from bidi paragraph boundaries. In
particular, LS and FF have whitespace bidi behavior rather than unconditionally
resetting paragraph analysis. This is why vertical coverage now reuses paragraph
levels instead of calling bidi resolution separately on every split string.
[Unicode bidirectional algorithm](https://www.unicode.org/reports/tr9/tr9-51.html)

The Unicode line-break specification includes VT/FF in mandatory breaks and
keeps CRLF together. It also describes page/paragraph semantics beyond merely
breaking a line. The SDK's pinned line-break data remains Unicode 15; referring
to current standards does not establish full Unicode-version conformance.
[Unicode line breaking](https://www.unicode.org/reports/tr14/)

Remaining implementation limits include:

- **This low-level layer still does not create pages.** The subsequent
  `form_feed_pagination_implementation.md` gives U+000C physical-page ownership
  in linked-story and fresh-authoring flow APIs, including trailing blank pages,
  page parity and receipts. Page-local and table-cell APIs reject it where they
  cannot own the successor page. Section masters, headers/footers, numbering and
  footnotes remain separate layout work.
- **Blank-only logical carriers required further work.** That source gap is
  addressed for story/table hard-only lines by the subsequent
  `logical_line_carrier_implementation.md`: explicit zero-advance text codes,
  ToUnicode and owner-local output postconditions replace empty ActualText-only
  scopes. This follow-up is unexecuted; metadata alone remains insufficient
  evidence, and equivalent coverage of all legacy writers is not established.
- Inline clipping/tag-preserving source routes still enforce their one-line
  contract. The predicate now includes VT/FF instead of accidentally letting
  those controls evade the rule. General multiline source-scope migration,
  arbitrary fonts, global variable-frame pagination and broader renderer/editor
  work remain open.
- No independent viewer, extraction, accessibility or visual comparison has
  qualified this code. A flat shaping API is not a complete paragraph compositor,
  and a shared predicate is not proof that every PDF text route is universal.

## Verification boundary

Sixteen new regression functions are **unexecuted**: eleven hard-line/shaping
cases, three story/table cases, one generated-writer-plan case and one authoring
atomicity case. They cover all supported separators, UTF-8 ranges, CRLF, consecutive
breaks, significant spaces, original glyph-cluster offsets, bidi isolates,
joining termination, strict single-line inputs, empty coverage validation,
greedy/balanced measurement, cancellation, writing modes, retained visible bidi
lengths and checkpoint/reopen metadata. Existing break-policy and native/browser
capability regression source is updated too, without execution.

Allowed checks were source inspection, Rust parsing/formatting, JavaScript syntax
and Git whitespace checks. No Cargo, compiler, type check, test, PDF workload,
rendering, benchmark, browser/device QA, deployment, commit or push. The tests
that mention saving/reopening are code added for future execution, not evidence
that those operations ran. The full roadmap remains active.
