# Source-inline position preservation (unexecuted implementation)

Status: source code added on the existing dirty candidate. No Cargo command,
compiler, test, PDF workload, rendering, benchmark, deployment or push was run.
This increment is not completion of the full editor/rendering roadmap.

## Implemented paths

`advanced_inline_text.rs` extends the canonical editing token scanner with
font-resolved text positions. It does not introduce a second content grammar.
The multi-run edit path supplies page resources and retains a font-resolver cache
and position state across `/Contents` members.

- Tracks both the text line matrix and the accumulated two-axis displacement.
  Handles BT/ET, Tm, Td/TD, TL, T*, Tj, TJ, quote/double-quote, numeric-only TJ
  arrays and graphics-state restoration. Tc/Tw/Tz and W/W2 use the same shared
  source-displacement function as destructive text removal. Incomplete two-byte
  character codes and non-finite metrics are not silently padded or guessed.
- Generated inline replacement now emits each glyph at its shaped position in
  the original source text object. Horizontal offsets/advances and vertical
  rotations, offsets and cross-axis advances are retained. Generated writing
  mode is explicit, not inferred incorrectly from the source font. Changing
  writing modes selects the shaped route rather than retaining the old CMap.
- Restores the original Tlm, then positions Tm with numeric-only TJ arrays in
  the required axes, using resolved source/generated fonts at neutral size and
  scale. Restores source typography/paint before the untouched suffix. It does
  not divide by/invert the source matrix, add clipping text, retain removed
  glyph codes, append the replacement above later painting, or add q/Q wrappers.
- Source text rise is applied in the source axes, not rotated a second time
  with a sideways glyph. The shared vertical column writer and outline-bounds
  calculation now use that same convention.
- Generated per-glyph spans record a private `WFTextBasisV1` layout-origin hint
  and the corresponding emitted glyph matrix. The scanner accepts it only when
  its finite, bounded array matches the actual source text matrix. This avoids
  treating an already-applied glyph rotation/offset as a new layout baseline
  during repeat editing. Mismatched/malformed hints reject the selected edit.
  The spans use ActualText=null and add neither logical text nor MCID owners.
  They are consistency metadata, not authenticated provenance or a signature.
- Cancellation is polled during source metrics and glyph emission. Cancellation
  errors propagate rather than becoming unsupported-position fallbacks.

Unknown font history prevents a guessed cursor. A later explicit Tm/BT can
establish a new origin. The subsequent `ext_gstate_font_implementation.md`
increment resolves ExtGState-only indirect font objects, tracks gs selections and
materializes aliases when source restoration emits Tf. Transparency-only
ExtGStates do not unnecessarily invalidate font positions. Unresolvable fonts do.

## Specification basis

The [ISO 32000-2 text errata](https://pdf-issues.pdfa.org/32000-2-2020/clause09.html)
clarifies horizontal scaling, T* using Td rather than TD, and q/Q additionally
saving Tm/Tlm inside text objects. The tracker follows that interpretation for
existing content; generated output restores matrices explicitly instead of
relying on newly introduced in-text q/Q behavior. Older readers still require
interoperability qualification.

## Regression source and evidence boundary

Fourteen new, unexecuted tests in `advanced_inline_text_tests.rs` cover:

- mixed-axis/cross-stream positions and numeric TJ;
- quote/double-quote, leading, Td/TD and q/Q;
- unavailable fonts, ExtGState fonts and zero-size/zero-scale tracking;
- all four text-show forms and all eight rendering modes;
- rotation/offset/rise composition, contextual advances and writing-mode changes;
- singular source matrices and incomplete character codes;
- edit/save/reopen/edit again and unchanged page-content stream order;
- repeated rotated-glyph geometry and stale layout-origin metadata.

Tests inspecting matrices do not prove pixel correctness or clipping fidelity.
The end-to-end test source has not executed. Only rustfmt parsing/format checks,
source inspection and whitespace checks were performed.

The subsequent `vertical_story_implementation.md` adds uniform vertical story
pagination/fallback in source, without executable qualification.
Remaining work includes general source font/CMap semantics and occurrence-specific Form
integration, complete ruby/tate-chu-yoko/kinsoku behavior and the other items in
`universal_editor_roadmap_tracking.md`. Existing PDFs generated before the layout
origin marker have no retroactive proof separating glyph rotation from baseline
rotation. Incremental editing is still not historical-byte sanitization.
