# Rendering Architecture Wasm Packaging

Wasm Packaging closes the first renderer-architecture leftovers without claiming
PDFium/Poppler/MuPDF parity. The implementation keeps the existing immediate
renderer as the semantic compatibility engine, but the display-list path is no
longer vector-only: complex page content is carried as a typed replayable
compatibility run.

## Architecture

Current path:

`content stream -> DisplayList -> RenderDevice / compatibility run -> PixelBuffer`

Release Packaging normalized path fill/stroke/clip operations directly. Wasm Packaging adds
`DisplayOp::ContentRun` for page content that still depends on the existing
renderer semantics: text, image XObjects, inline images, Form XObjects, shadings,
patterns, soft masks, and transparency groups. This is a bridge, not a second
font/color/image implementation.

## Capability Matrix

| Content category | Immediate renderer | Display-list capture | Display-list replay | Wasm Packaging status |
| --- | --- | --- | --- | --- |
| Fill path | Supported | Normalized | Native CPU device | Done |
| Stroke path | Supported | Normalized | Native CPU device | Done |
| Clip path | Supported | Normalized | Native CPU device | Done |
| Text | Supported by current font renderer | `ContentRun` bridge, counted in stats | Existing renderer semantics | Done as bridge; Codec Boundary owns font fidelity |
| Image XObject | Supported through Binding Parity decode caps | `ContentRun` bridge, counted in stats | Existing renderer semantics | Done as bridge |
| Inline image | Supported through inline image decode | `ContentRun` bridge, counted in stats | Existing renderer semantics | Done as bridge |
| Form XObject | Supported with depth cap | `ContentRun` bridge | Existing renderer semantics | Done as bridge |
| Transparency groups | Supported for existing common cases | `ContentRun` bridge, transparency stats | Existing renderer semantics | Done as bridge; knockout remains approximate |
| Soft masks | Supported for current alpha/luminosity path | `ContentRun` bridge | Existing renderer semantics | Done as bridge |
| Blend modes | Existing formulas in `BlendMode`/`PixelBuffer` | State counted when `gs` uses alpha/BM | Existing renderer semantics | Done for current modes |
| Shadings | Existing bounded CPU shading renderer | `ContentRun` bridge, counted in stats | Existing renderer semantics | Done as bridge |
| Tiling patterns | Existing immediate pattern renderer with tile cap | `ContentRun` bridge | Existing renderer semantics | Done as bridge |
| Mesh/patch shadings | Existing shading module path | `ContentRun` bridge | Existing renderer semantics | Done as bridge |
| Tile rendering | Not previously exposed | Uses display-list replay then crops | Deterministic tile buffer | Done as bounded compatibility tile |
| Band rendering | Not previously exposed | Uses tile API | Deterministic vertical bands | Done as bounded compatibility bands |
| Progressive/cancel | Immediate cancellation existed | Display-list replay checks token | Clean `Cancelled` error | Done for cancellation; resumable progressive output deferred |
| Render cache | Not present for renderer tiles | `RenderCache` with byte budget | Tile cache hit/evict metrics | Done |

## Display-List Stats

`DisplayListStats` now records:

- native vector operation counts;
- text operation count;
- XObject and inline-image counts;
- shading, pattern, and transparency counts;
- compatibility run count and byte estimate;
- unsupported-operation count.

The byte estimate is intentionally approximate and exists for cache budgeting
and report/debug surfaces. It is not a serialized display-list size contract.

## Codec Boundary Font Integration Note

Codec Boundary keeps the Wasm Packaging compatibility-run text replay model, but removes
the duplicate raster text decoder from `page_renderer.rs`. Raster rendering,
SVG/vector text, and display-list replay now share
`render::text_decode::decode_text_bytes` for PDF font-code to glyph mapping.
Font-specific provider, shaping, cache, and diagnostics status is tracked in
[`font_subsystem_codec_boundary.md`](font_subsystem_codec_boundary.md).

## Tile, Band, Cache, And Cancellation

Wasm Packaging adds:

- `RenderTile`;
- `RenderCache`, `RenderCacheKey`, and `RenderCacheMetrics`;
- `ContentEngine::render_page_tile_with_mode`;
- `ContentEngine::render_page_bands_with_mode`;
- display-list cancellation via `render_display_list_cancellable_with_mode`.

The first tile/band implementation is compatibility-safe: it renders through
the display-list path and crops/splits the bounded page buffer. It does not yet
perform true viewport-only drawing for complex transparency groups. This avoids
changing compositing semantics while giving downstream callers a stable API and
metamorphic test surface.

## Tests Added

- text page display-list replay matches immediate pixels;
- image page display-list replay matches immediate pixels;
- vector replay remains pixel-identical to immediate rendering;
- full page equals stitched tiles;
- full page equals stitched bands;
- tile cache records hit/insert and respects byte budget;
- cache skips oversized entries;
- pre-cancelled display-list replay returns `Cancelled`.

Wasm Packaging also adds a bounded `display_list` fuzz target under
`fuzz/fuzz_targets`. It parses arbitrary content-stream bytes, builds a tiny
display list with empty resources, and replays only native-vector lists so the
target cannot allocate large surfaces or enter the broader font/image
compatibility renderer on arbitrary input.

## Reference Benchmark

Wasm Packaging ran a temporary capped manifest generated under
`target/wasm_packaging-renderer-slice-manifest.json`.

- Files: 50
- Visual pages compared: 42
- Categories: synthetic graphics, geometry, text, images, transparency, forms;
  real text, complex vector, forms, scanned, CJK, RTL, multi-column, JPEG2000;
  large files; hostile truncated and bad-filter files
- Reference: Poppler 26.02.0
- PDFium: not available, skipped
- DPI: 72
- Page cap: 1 page per file
- Weighted score: 90.45
- Visual pass: 83.33%
- Hostile crash-free/timeout-safe/memory-bounded: 100%
- Determinism: 5/5 sampled files stable

Known failures in this slice are not hidden: `function_based_shading.pdf`,
`IdentityToUnicodeMap_charCodeOf.pdf`, `ThuluthFeatures.pdf`, `bug_jpx.pdf`,
and `jp2k-resetprob.pdf` remain blockers. These mostly point to future font,
color/shading, and JPX fidelity work rather than the display-list seam itself.

Artifacts are under `target/wasm_packaging-renderer-benchmark/` and are not checked
in.

## Honest Remaining Limits

- Text is replayed through current renderer semantics; Codec Boundary owns shaping,
  fallback, Type0/CID/CMap, hinting, and glyph fidelity.
- Full ICC/DeviceN/Separation and overprint behavior remain Decode Scheduler work.
- Tile/band rendering currently crops/splits a bounded compatibility page render;
  it is not yet a true low-memory viewport-only renderer for every transparency
  case.
- Knockout transparency groups remain approximate where the immediate renderer
  already approximates them.
- Mesh/patch shading and JPX failures are visible in the reference slice and
  remain measured follow-up categories.
