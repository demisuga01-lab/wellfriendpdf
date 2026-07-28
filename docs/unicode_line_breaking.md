# Unicode Line Breaking

## Implemented boundary

`wellfriendpdf_engine::line_break_text` is the canonical Prompt 33 candidate
pipeline for the currently supported single-region `GeometricBlock` path. It
uses `unicode-linebreak` 0.1.5 (Apache-2.0) for UAX #14 Unicode 15 default
break opportunities and `unicode-segmentation` for extended grapheme
boundaries. A candidate is selectable only when the UAX #14 position coincides
with a grapheme boundary.

The pipeline records the UTF-8 logical offset, grapheme index, source location,
line-break class, mandatory/optional/prohibited disposition, penalty,
soft-hyphen source state, visual insertion behavior, and extraction behavior.
It also resolves a `unicode-bidi` paragraph for each final line without
modifying logical source text.

Interactive preview uses deterministic greedy filling. Final selection uses
bounded dynamic programming over the same candidate set. Every selectable
candidate is shaped with Prompt 20's Rustybuzz-backed generated-font pipeline,
and its advance is measured in the same point scale used by the source writer.
The final line list is passed to `edit_advanced_text_pdf_with_layout`; the
canonical source writer therefore emits the selected lines rather than
independently recomputing breaks.

An over-wide unbreakable line is refused as `unresolved_overflow` before source
mutation. It is never clipped, painted over, or silently reduced in font size.

## Exact limits

- Candidate spans are capped at 2,048 per layout request. Exceeding that budget
  returns `resource_limit_exceeded`; it does not fall back to approximate
  character widths.
- Unicode default UAX #14 behavior is active. `hyphenation` 0.8.4 supplies
  audited Knuth-Liang candidates for `en-US` and `es`; locale fallback is
  explicit (`en-*` to `en-US`, `es-*` to `es`) and all other languages report
  `hyphenation_unavailable` rather than using English rules.
- Dictionary candidates are grapheme-safe and recorded with data/provider
  provenance. For the supported `en-US` and `es` dictionaries, the final
  writer can select one candidate per line: it paints a shaped end-of-line
  hyphen and gives that CID an empty ToUnicode mapping, preserving the logical
  requested text during extraction. Source soft hyphens remain recorded but
  are not selected because their source/extraction policy has not yet been
  serialized through this boundary.
- Vertical writing has a direction label and bounded candidate layout only; it
  is not an asserted full vertical-text serialization implementation.
- The optimizer does not yet enforce widow/orphan, keep-with-next, baseline
  grid, or script-specific justification constraints.

## Regression evidence

The Prompt 33 engine suite covers source-linked final-line serialization,
grapheme/bidi separation, and refusal of an over-wide non-breaking-space token.
VPS evidence for the continuation snapshot is retained under
`/home/demisuga01/wellpdf/results/prompt33-20260727T171233Z/`.
