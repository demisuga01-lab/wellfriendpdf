# Prompt 08B Type3/CID/Tensor Closure Audit

## Starting checkpoint

- Starting HEAD: `a403873`
- Starting commit: `Complete combined prompt 08 text clipping shading pattern parity`
- Starting worktree status: clean

## Prompt 08 known limits and reclassification

Copied Prompt 08 known limits:

- Type3 glyph clipping needs a Type3 outline/content-to-clip model.
- Missing font or glyph outlines cannot produce exact text clipping and are unsupported-reported.
- Exact Type 7 tensor patch interior interpolation remains future math work; streams are parsed and bounded today.
- Advanced ICC, device-link, multicolor, and prepress CMM parity remains later CMM work.
- Pattern execution uses bounded per-render tile loops rather than an unbounded global pattern cache.

Prompt 08B reclassification:

- `Type3 glyph clipping needs a Type3 outline/content-to-clip model`: Prompt 08B blocker.
- `Missing font or glyph outlines cannot produce exact text clipping`: Prompt 08B owns common CID/embedded outline cases; exotic unavailable outlines remain later font fidelity only when precisely unsupported-reported.
- `Exact Type 7 tensor patch interior interpolation`: Prompt 08B blocker.
- `Advanced ICC/device-link/multicolor/prepress CMM`: later advanced CMM/prepress.
- `Bounded per-render tile loops`: already resolved safety posture, not a Prompt 08B blocker.

## Current text clipping implementation path

- Text rendering mode is parsed by `GraphicsState::process` in `crates/engine/src/content/state.rs`.
- Page rendering dispatch handles `BT`, `ET`, `Tj`, `TJ`, `'`, and `"` in `RenderState::dispatch` in `crates/engine/src/render/page_renderer.rs`.
- `RenderState::render_text_string` decodes bytes through `render/text_decode.rs::decode_text_bytes`, retrieves font bytes, and calls `render_glyph_with_cache`.
- `RenderState::render_glyph_with_cache` extracts normal glyph outlines with `render/glyph_outline.rs`, accumulates Tr `4..=7` masks through `accumulate_text_clip`, and paints Tr `0/1/2/4/5/6`.
- `pending_text_clip: Option<ClipMask>` is reset at `BT`, unioned per glyph, and applied at `ET` by `apply_pending_text_clip`, which intersects with the active `PixelBuffer` clip.
- Existing `q`/`Q` clip preservation is handled by `clip_stack` and `PixelBuffer::restore_clip`.

## Current Type3 implementation path

- Type3 fonts are detected by `fonts/resolver.rs::detect_font_subtype`.
- Type3 fonts do not produce embedded font-program bytes through `FontRasterizer::extract_font_bytes`; charprocs are PDF content streams under the font dictionary's `/CharProcs`.
- `PageResources` stores page fonts, XObjects, color spaces, graphics states, patterns, and shadings, but not a dedicated Type3 charproc cache.
- The current text renderer does not interpret Type3 charprocs in paint or clip mode. Prompt 08 therefore logged missing outlines for Type3 clipping.
- A Prompt 08B Type3 path collector must decode the selected charproc stream, parse it with `ContentParser`, collect path construction and paint operators, apply `/FontMatrix`, text state, text matrix, rise, and CTM, and reject resource-heavy or non-path charprocs with precise diagnostics.

## Current CID/CMap implementation path

- `render/text_decode.rs::decode_type0_text` handles Type0 fonts.
- Encoded bytes are chunked using `FontResolver::code_size`.
- `FontResolver` resolves ToUnicode/predefined CMap text independently of glyph geometry.
- For embedded CID fonts, `fonts/cid.rs::cid_font_has_embedded_program` enables glyph-id rendering.
- CID to glyph id maps through `fonts/cid.rs::cid_to_gid`, honoring `/CIDToGIDMap /Identity` or stream maps.
- Outlines then use `render/glyph_outline.rs::extract_glyph_path_by_gid_var`, including sfnt and bare CID CFF fallback.
- Missing outline diagnostics are currently generic; Prompt 08B should add fixture/report evidence for common embedded CID clipping and keep exotic missing outlines unsupported-reported.

## Current Type 7 tensor-patch implementation path

- `render/shading.rs::ShadingRenderer::paint` dispatches ShadingTypes `6 | 7` to `paint_patch_mesh`.
- `paint_patch_mesh` decodes flags, points, corner colors, bit depths, decode arrays, and stream boundaries through `MeshDecode` and `BitReader`.
- `assemble_patch` currently drops Type 7 interior points `p13..p16` and returns a 12-point Coons patch.
- `render_coons_patch` uses fixed `10x10` subdivision and `coons_point` boundary evaluation with bilinear corner-color interpolation.
- Malformed/truncated streams stop rendering without panics, but exact tensor interior evaluation is not implemented.
- Prompt 08B must keep the same fail-closed stream posture while evaluating Type 7 with all 16 control points.

## Current multi-reference audit state

- Prompt 08 audit script: `scripts/prompt08_text_shading_patterns_audit.py`.
- Prompt 08 fixture count: 26.
- Prompt 08 artifacts live under `target/prompt08-text-shading-patterns/`.
- Prompt 08 classification counts:
  - `all_references_agree_wellfriendpdf_passes`: 19
  - `references_disagree_wellfriendpdf_within_cluster`: 3
  - `unsupported_reported_expected`: 3
  - `malformed_reference_failure`: 1
  - Wellfriend outlier failures: 0
- Poppler/PDFium/MuPDF availability is recorded in `target/prompt08-text-shading-patterns/reference-tool-manifest.json`.
- Prompt 08B must regenerate a separate 08B corpus, render results, visual metrics, fallback taxonomy, and HTML report rather than relying on Prompt 08 artifacts.
