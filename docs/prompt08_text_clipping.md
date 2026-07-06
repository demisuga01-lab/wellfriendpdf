# Prompt 08 Text Clipping

Prompt 08 implements text rendering modes `4`, `5`, `6`, and `7` for fonts whose
glyph outlines can be extracted by the current font subsystem.

Behavior:

- Glyph outlines are collected during `BT`/`ET` text-object processing.
- The accumulated glyph mask is unioned per text object and intersected with the
  active clip at `ET`.
- Existing `q`/`Q` clip restoration remains authoritative because text clipping
  uses the same `ClipMask` stack as path clipping.
- Modes `4`, `5`, and `6` still perform their fill/stroke paint before the clip
  affects later operations. Mode `7` is clip-only.

Covered interactions:

- Subsequent path fill.
- Image XObject.
- Form XObject.
- Axial shading.
- Colored tiling pattern.

Tests:

- `cargo test -p oxide-engine --test prompt08_text_clip --jobs 1`

Artifacts:

- `target/prompt08-text-shading-patterns/text-clipping-matrix.json`
- `target/prompt08-text-shading-patterns/multi-reference-render-results.json`
- `target/prompt08-text-shading-patterns/html-report/index.html`

Remaining precise limits:

- Type3 glyph outline extraction is unsupported-reported.
- Fonts or glyphs without extractable outlines are unsupported-reported.
- CID/CJK clipping is limited by available outline extraction; no full CJK
  parity claim is made beyond corpus evidence.
