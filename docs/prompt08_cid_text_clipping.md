# Prompt 08 CID/CMap Text Clipping

Prompt 08B closes the common embedded CID/CMap clipping path for text rendering
modes `4`, `5`, `6`, and `7`.

Mapping path:

- Encoded bytes are decoded through the active CMap.
- The resulting CID is mapped through `/CIDToGIDMap` or the embedded font
  mapping used by the normal renderer.
- The glyph outline is extracted from the embedded TrueType/CFF-compatible font
  program when the current font subsystem exposes it.
- ToUnicode remains independent; clipping uses glyph geometry, not Unicode text.

Transform behavior:

- Glyph geometry preserves font size, horizontal scaling, text rise, glyph
  displacement, text matrix, line matrix behavior, and CTM.
- Identity-H fixtures are covered. Identity-V is not claimed as full vertical
  typography parity; vertical outline availability can be supported where the
  font subsystem maps the glyph, while exact CJK/RTL raster typography remains a
  later font-fidelity campaign.

Diagnostics:

- Missing outlines fail closed and report the font subtype plus CID/GID context
  when available.
- Missing or exotic font programs remain unsupported-reported rather than using
  a bounding box as clip geometry.

Evidence:

- `cargo test -p wellfriendpdf-engine --test prompt08b_type3_cid_tensor --jobs 1`
- `target/prompt08b-type3-cid-tensor/prompt08b-cid-clip-matrix.json`
- `target/prompt08b-type3-cid-tensor/prompt08b-reference-disagreement-summary.json`

The Prompt 08B audit has five CID/CMap clipping fixtures in the
`all_references_agree_wellfriendpdf_passes` classification and one missing-outline
fixture in `unsupported_reported_expected`.
