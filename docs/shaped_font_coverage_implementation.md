# Shaped-font coverage and exact-asset approval

Status: source implementation, not compiled or runtime-qualified. This extends
the dirty `main` candidate based on `27e62db3a1b84804339e65b6025273fd003b3736`.
No build, typecheck, test, PDF workload, rendering, benchmark or deployment was
run. No commit or push was made.

## Findings addressed

Nominal Unicode-to-glyph lookup is not a sufficient coverage test. A
GID-preserving subset can retain a cmap entry after removing its outline.
Conversely, OpenType normalization can produce a valid decomposed sequence even
when the precomposed scalar has no nominal cmap entry. The old advanced analysis,
substitution reports and approved reflow-asset resolver made different decisions
about these cases. Final generated-glyph conversion also did not repeat the
outline check after line shaping.

The earlier suspicion about joiner placeholders needs a narrower statement:
both horizontal and vertical shapers already set
`REMOVE_DEFAULT_IGNORABLES`. Inspection of the pinned Rustybuzz 0.20.1 source
(`hb/ot_shape.rs`, `hide_default_ignorables`) confirms that removal happens after
substitution/positioning. This increment does not claim to have newly added that
flag. Blank output still needs care: a real space and a combining mark can share
one source cluster, without making the space a missing visible glyph.

## Shared coverage implementation

`fonts/coverage.rs` supplies sorted UTF-8 missing-cluster offsets. The existing
`shaper::has_missing_glyphs` API delegates to it, so contextual fallback,
story/table measurement and authoring callers retain their existing interfaces.

- Validate every cluster boundary and finite advance/offset, including glyphs
  with outlines. Reject `.notdef` and out-of-font-range GIDs.
- Check the final shaped GID's outline instead of requiring every source scalar
  to have a nominal cmap entry. Cache outline presence per GID.
- Permit source-bound blank whitespace glyphs within a visible cluster, including
  the pinned normalizer's ordinary-space fallback for Unicode spacing scalars.
  This does not authorize an absent combining-mark outline or an arbitrary empty
  visible glyph. Hard separators do not confer that exception.
- Detect completely empty visible output and omitted visible prefixes rather
  than returning coverage success vacuously. Default-ignorable-only input may
  legitimately shape to no painted glyphs; logical PDF persistence is separate.
- Bound text/glyph counts and poll cancellation during cluster and outline work.

Advanced analysis now shapes hard lines with paragraph-derived bidi/joining
context in horizontal, RTL and vertical modes. Missing offsets and glyph
provenance refer to the original UTF-8 input. Final generated conversion repeats
coverage for the actual line, including the vertical route. Sorted cluster lookup
replaces a per-glyph scan of every cluster; glyph bounds are cached locally and
conversion polls cancellation. No timing or memory improvement was measured.

## Approval and reflow integration

Bundled and supplied-font reports use shaped coverage and advances. Additive
diagnostics disclose missing clusters, coverage errors and approval eligibility.
A scalar coverage ratio alone cannot approve a font: complete cluster coverage
and supported editable-outline embedding are required.

A supplied asset binds exact bytes. Its same-name bundled candidate is removed
from the ranked list, and only that supplied asset can enter `approved_candidates`
for this request. An invalid asset named `Helvetica` therefore cannot inherit
another font's approval. Other ranked fonts remain diagnostic alternatives;
selecting one requires a new request without the rejected bound asset. The
immutable request/plan continues to bind the actual program bytes.

Substitution ranking is a default-feature horizontal screen, not proof for every
writing mode or arbitrary feature set. The approved reflow resolver separately
checks its horizontal/RTL/vertical mode using the same outline rules and exact
bytes, and the final line writer checks again. Its former independent scalar-cmap
veto has been removed. Custom settings, intended appearance and embedding license
rights beyond the font's permission flags are not certified by these checks.

## Unexecuted regression source

Twenty-one regression functions were added across:

- `fonts/coverage_tests.rs`: retained subset holes, canonical decomposition,
  joiners/variation selectors, space-plus-mark clusters, malformed offsets,
  nonfinite geometry, `.notdef`, empty output/prefixes, separators and cancellation.
- `advanced_coverage_tests.rs`: three writing modes, actual final-line checks,
  hard-line provenance and a bounded replacement/save/reopen path.
- `universal_font_coverage_tests.rs`: exact-asset name collisions, malformed
  assets, decomposition, bundled diagnostics, denied planning/approval and a
  successful plan/approve/apply/reopen workflow.
- `text_reflow_font_coverage_tests.rs`: approved exact bytes, normalized output,
  subset holes and policy-name mismatch in all three reflow directions.

The test-only cmap fixture uses the existing sfnt serializer/checksum path while
retaining source outlines and layout tables. Neither the fixture nor any of these
test functions has been executed. Rustfmt parsed the integration files and
formatted the new files; that is not Rust type checking or a passing test result.

## Remaining boundaries

Coverage means the surviving shaped output has the expected supported outline
resources. It cannot prove the intended design of arbitrary font substitutions,
deliberate font-program deletions, pixel fidelity or Unicode semantics of
unmapped existing source codes. Color-only/bitmap fonts and unsupported exotic
programs remain separate work.

All-default-ignorable text can have no surviving glyphs. The subsequent
`logical_control_carrier_implementation.md` increment extends the hard-separator
carriers to supported standalone default ignorables in story, generated,
source-inline and authoring paths. It does not establish arbitrary glyphless-font
semantics or per-control visual selection geometry. A covered empty run alone
is not a claim of lossless logical PDF serialization. Single-baseline OCR runs
are also not a general multiline layout model. Authoring fallback/tab stops, broader typography, general
pagination/anchors/tables/tags, bindings and renderer closure remain in the full
roadmap. Independent build, save/reopen, render and corpus qualification remains
unexecuted. This is not universal-editor completion or an Acrobat comparison.
