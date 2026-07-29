# Advanced Rendering Text Clipping

Advanced Rendering implements text rendering modes `4`, `5`, `6`, and `7`. Type3 CID Rendering
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

- `cargo test -p wellfriendpdf-engine --test advanced_rendering_text_clip --jobs 1`
- `cargo test -p wellfriendpdf-engine --test type3_cid_rendering_type3_cid_tensor --jobs 1`

Artifacts:

- `target/advanced_rendering-text-shading-patterns/text-clipping-matrix.json`
- `target/advanced_rendering-text-shading-patterns/multi-reference-render-results.json`
- `target/advanced_rendering-text-shading-patterns/html-report/index.html`
- `target/type3_cid_rendering-type3-cid-tensor/type3_cid_rendering-type3-clip-matrix.json`
- `target/type3_cid_rendering-type3-cid-tensor/type3_cid_rendering-cid-clip-matrix.json`

Remaining precise limits:

- Type3 charprocs whose visible shape is only images, shadings, patterns, text,
  or unsafe nested resources fail closed with diagnostics.
- Fonts or glyphs without extractable outlines are unsupported-reported.
- Full CJK/RTL raster typography parity remains outside Type3 CID Rendering; the closure
  only covers clipping outline availability for common embedded CID fonts.
