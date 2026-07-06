# Prompt 08 Text Clipping

Prompt 08 implements text rendering modes `4`, `5`, `6`, and `7`. Prompt 08B
closes the common Type3 and CID/CMap outline gaps without using bounding-box
fake clipping.

Behavior:

- Glyph outlines are collected during `BT`/`ET` text-object processing.
- The accumulated glyph mask is unioned per text object and intersected with the
  active clip at `ET`.
- Existing `q`/`Q` clip restoration remains authoritative because text clipping
  uses the same `ClipMask` stack as path clipping.
- Modes `4`, `5`, and `6` still perform their fill/stroke paint before the clip
  affects later operations. Mode `7` is clip-only.
- Type3 fonts collect supported charproc path geometry in glyph space, transform
  it through FontMatrix, text state, text matrix, and CTM, and fail closed for
  image-only or resource-heavy charprocs.
- CID/CMap fonts follow the text-rendering mapping path from encoded bytes to
  CID to CIDToGIDMap or embedded glyph ID before outline extraction.

Covered interactions:

- Subsequent path fill.
- Image XObject.
- Form XObject.
- Axial shading.
- Colored tiling pattern.

Tests:

- `cargo test -p oxide-engine --test prompt08_text_clip --jobs 1`
- `cargo test -p oxide-engine --test prompt08b_type3_cid_tensor --jobs 1`

Artifacts:

- `target/prompt08-text-shading-patterns/text-clipping-matrix.json`
- `target/prompt08-text-shading-patterns/multi-reference-render-results.json`
- `target/prompt08-text-shading-patterns/html-report/index.html`
- `target/prompt08b-type3-cid-tensor/prompt08b-type3-clip-matrix.json`
- `target/prompt08b-type3-cid-tensor/prompt08b-cid-clip-matrix.json`

Remaining precise limits:

- Type3 charprocs whose visible shape is only images, shadings, patterns, text,
  or unsafe nested resources fail closed with diagnostics.
- Fonts or glyphs without extractable outlines are unsupported-reported.
- Full CJK/RTL raster typography parity remains outside Prompt 08B; the closure
  only covers clipping outline availability for common embedded CID fonts.
