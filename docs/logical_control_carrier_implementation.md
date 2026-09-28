# Logical controls with no painted glyphs

Status: implemented in source, not compiled or runtime-qualified. This extends
the uncommitted `main` candidate based on
`27e62db3a1b84804339e65b6025273fd003b3736`. No build, typecheck, tests, PDF workload,
rendering, benchmark, deployment, commit or push was performed.

## Problem and supported contract

The configured shaper removes default-ignorable output after OpenType shaping.
A run consisting only of such controls can consequently have no painted glyphs.
An `/ActualText` wrapper around no text-showing content does not give the SDK's
extractor or source-selection model an actual character carrier. Existing
private carriers handled seven mandatory line separators, not these control-only
runs. Treating empty output as font coverage does not solve logical persistence.

`fonts/logical_carrier.rs` now defines a shared, stable carrier code alphabet.
It preserves the seven original separator codes and adds the default-ignorable
ranges removed by the pinned Rustybuzz 0.20.1 implementation. The source basis is
`src/hb/unicode.rs::is_default_ignorable` together with the already-enabled
`REMOVE_DEFAULT_IGNORABLES` flag. This deliberately is not an unversioned claim
about every Unicode default-ignorable character. In particular, unrelated
whitespace, tabs, NUL, Hangul fillers and code points outside that pinned policy
are not silently classified as disposable text.

The carrier uses a real empty TrueType space outline with zero PDF advance and
explicit ToUnicode mappings, including supplementary variation selectors.
It does not use invisible rendering mode to conceal the selected original text.
The selected old source codes are still removed by the existing source mutation.
Ordinary spaces and visible characters continue through normal coverage/shaping.
New characters in the carrier alphabet must retain prior code assignments when
the shaping dependency is upgraded.

## Source paths changed

- Story/table paint lines consisting entirely of supported logical controls or
  hard separators receive carrier operands inside their existing ActualText,
  paragraph, artifact and frame ownership. They need no paint-font glyphs.
  Only used control mappings, plus the original seven separators, are embedded.
  Existing story-owned resource retirement also applies to these fonts.
- Bounded and multi-run generated replacements use the same carrier mechanism,
  including source-order insertion ownership and existing scoped Form/appearance
  routing. Their reopened checks validate mappings and source operands.
- Clipping/tagged inline replacements without hard line separators receive
  explicit empty-outline glyphs inside the original BT/ET, rather than appending
  a second text object. The source render mode and endpoint restoration remain
  authoritative. Character spacing is not added between these logical controls.
  Their private font is excluded from ordinary fallback candidates. Repeated
  controls reuse stable CIDs and duplicate font definitions are removed from the
  staged font glyph list; finite text/glyph limits and cancellation remain.
- Authoring single-run and paragraph commands retain supported control-only
  content using the existing logical-carrier command and builtin font. This also
  applies to paragraph-based table/flow authoring. Standard-14 measurement treats
  a control-only run as zero advance, not as missing printable characters.
  Registered custom font identity checks for painted content are unchanged.

These are logical preservation changes. They do not claim a visible typography
improvement for content that intentionally has no painted glyphs.

## Stronger story output verification

The former verifier checked ToUnicode mappings and extracted ActualText. A wrong
but valid code sequence could pass both checks: ActualText could conceal that the
operand used a different valid carrier code.

The verifier now also scans the exact owned stream interval, decodes each actual
carrier operand, checks its expected logical owner and zero-advance text state,
and rejects missing or extra carrier operands. It then checks the extractor's
logical view and writing mode. A matching word elsewhere in the document or
unchanged ActualText is not sufficient. This uses the SDK's own parser/extractor;
it is not independent interoperability proof.

## Unexecuted regression source

Twelve new regression functions cover:

- Stable/injective code assignment, supplementary scalars, excluded visible and
  unsupported characters, bounded sparse alphabets and cancellation.
- Horizontal and both vertical story modes, extraction with ActualText removed,
  exact source selection, zero advances and tampering with a different valid CID.
- Empty-outline inline glyph plans.
- Standard and Unicode authoring, standalone controls, and control-only lines
  between painted paragraphs.
- Bounded, source-order inserted, generated and style-preserving replacements;
  reselect/edit/save/reopen; tagged/clipping replacement and following-text
  endpoint preservation with nonzero source character spacing.

Two existing story regressions were extended for control eligibility and repeated
rewrites/resource retirement. Source files are `fonts/logical_carrier_tests.rs`,
`advanced_story_carrier_tests.rs`, `advanced_generated_carrier_tests.rs` and
`authoring_layout_tests.rs`. None was executed. Rustfmt formatting/parser checks
and Git whitespace checks are the only automated checks in this increment.

## Remaining work

This does not provide arbitrary glyphless-font semantics, a Unicode-version-
independent default-ignorable policy, tab-stop layout or generalized multiline
OCR placement. The pre-existing inline hard-line restriction remains: moving
lines within source clipping/tagged objects requires an explicit layout contract.
For mixed painted/control text the shaping run and its logical ActualText retain
ownership; this increment does not introduce an independent selection rectangle
for every removed control or replace aggregate mappings with a universal caret
model. Bounded whole-replacement carriers likewise do not establish per-character
visual geometry for multiple blank lines.

Broader font/layout/tag/table/anchor handling, rendering and bindings remain in
`universal_editor_roadmap_tracking.md`. Current compilation, independent saved-PDF
validation, rendering, corpus and comparative Acrobat evidence are unexecuted.
The full roadmap is not complete.
