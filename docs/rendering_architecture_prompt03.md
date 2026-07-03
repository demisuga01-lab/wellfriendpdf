# Prompt 03 Rendering Architecture

Prompt 03 moves Oxide's renderer toward a display-list and device-based
architecture without claiming PDFium, Poppler, or MuPDF parity. The existing
renderer already contains several high-fidelity pieces from earlier work; this
pass adds the missing replayable vector display-list seam and documents the
current renderer state honestly.

## Starting Inventory

Current rendering path before Prompt 03:

- Page content streams are parsed into `ContentOperation` values, then rendered
  immediately by `crates/engine/src/render/page_renderer.rs`.
- The renderer has a PDF graphics-state stack for `q`, `Q`, `cm`, path
  construction, path painting, clipping, line style, color, `gs`, XObjects,
  shadings, inline images, and text operators.
- `crates/engine/src/render/path.rs` owns reusable path geometry, adaptive cubic
  flattening, analytic coverage-based fill, stroke generation, joins, caps,
  miters, dashes, nonzero fill, and even-odd fill.
- `crates/engine/src/render/buffer.rs` owns `PixelBuffer`, clip masks, alpha
  masks, source-over compositing, all PDF blend modes, transparency group
  flattening helpers, and `RenderMode::{Compat, HighQuality}`.
- `crates/engine/src/render/shading.rs` supports function-based, axial, radial,
  Gouraud mesh, and patch mesh shadings through bounded CPU painting.
- `crates/engine/src/render/image_painter.rs` paints decoded images through the
  Prompt 02 central decode/cap layer.
- `crates/engine/src/render/svg.rs` and `postscript.rs` already have vector
  export fallback logic but were not a general reusable display-list pipeline.
- Rendering is page-bounded, guarded by `max_render_pixels()`, and supports
  cooperative cancellation in the immediate renderer.

Key architectural gap before Prompt 03:

- There was no replayable page display list with a render-device abstraction.
  Supported drawing operations were executed directly into a `PixelBuffer`,
  making future CPU/GPU/printer/debug backends harder to add.

## Added Architecture

Prompt 03 adds `crates/engine/src/render/display_list.rs`.

The new module contains:

- `DisplayList`: replayable page drawing program plus support status,
  unsupported-operation reasons, approximate memory size, and operation stats.
- `DisplayOp`: normalized save, restore, clip, fill-path, and stroke-path
  operations.
- `DrawState`: CTM, fill/stroke colors, blend mode, line width, cap, join,
  miter limit, and dash state captured at paint time.
- `DisplayListStats`: operation, path, clip, save/restore, unsupported, segment,
  and max-stack-depth counters.
- `RenderDevice`: device abstraction for save/restore, clip, fill path, and
  stroke path.
- `CpuRenderDevice`: first concrete device, backed by `PixelBuffer` and the
  existing `PathPainter` rasterizer.

Public Rust entry points:

- `ContentEngine::build_page_display_list(page, dpi)`
- `ContentEngine::render_page_display_list_with_mode(page, dpi, mode)`
- `PageRenderer::build_display_list(...)`
- `PageRenderer::render_page_display_list_with_mode(...)`

The default `render_page` path remains the immediate renderer. That is
intentional: the current display-list subset does not yet replay annotations,
text, images, shadings, patterns, Form XObjects, or soft masks. Automatically
switching vector-compatible pages would skip annotations on some files. Instead,
Prompt 03 exposes an explicit real replay path and keeps the proven default
behavior stable.

## Display-List Scope

Implemented:

- Save/restore state operations.
- CTM and page viewport capture.
- Clip path replay with nonzero and even-odd fill rules.
- Path fill replay.
- Path stroke replay.
- Line width, line cap, line join, miter limit, dash array, and dash phase.
- DeviceGray, DeviceRGB, and DeviceCMYK simple color states.
- Blend mode and alpha state when no soft mask is involved.
- Approximate display-list memory accounting.
- Unsupported operation diagnostics at capture time.

Conservative fallback:

- Text/glyph operations mark the display list unsupported.
- Image XObjects, Form XObjects, inline images, shadings, patterns, soft masks,
  and unknown operators mark the display list unsupported.
- Named and pattern color spaces mark the display list unsupported because they
  require resource resolution beyond the path-only replay subset.

## Render Device

`RenderDevice` is intentionally small in this pass. It is enough to make a real
CPU replay path and to keep future backends feasible:

- A debug/inspection device can count operations without raster output.
- A future GPU/printer/SVG device can implement the same path primitives first.
- Text/image/group operations can be added to `DisplayOp` without changing
  parser or decode layers.

`CpuRenderDevice` reuses the same `PixelBuffer`, `ClipMask`, and `PathPainter`
as the immediate renderer. This keeps vector replay pixel-equivalent on the
supported subset and avoids introducing a second rasterizer.

## Tests Added

New unit coverage:

- Captures and replays a simple fill.
- Captures save/restore, clip, and stroke operations.
- Captures stroke color.
- Marks text operations unsupported.
- Builds a page display list for a vector-only PDF and renders it through CPU
  replay.
- Verifies display-list replay matches the immediate renderer pixel-for-pixel
  on a vector page using fill, clip, and dashed stroke operations.
- Verifies a text fixture is reported as unsupported and stays on the immediate
  renderer fallback path.

Focused command:

```powershell
cargo test -p oxide-engine display_list --lib
```

Result during Prompt 03 implementation: 7 passed.

## Reference Measurement

Baseline before Prompt 03 edits:

```powershell
python renderer-benchmark\scripts\renderer_benchmark.py `
  --manifest renderer-benchmark\corpus\manifest.json `
  --oxide-bin target\debug\oxide.exe `
  --dpi 72 --timeout-sec 20 --max-memory-mb 1024 `
  --max-pages-per-file 1 --limit 5 --determinism-sample 2 `
  --threshold-profile renderer `
  --output-dir target\prompt03-baseline-benchmark
```

After Prompt 03 display-list architecture:

```powershell
python renderer-benchmark\scripts\renderer_benchmark.py `
  --manifest renderer-benchmark\corpus\manifest.json `
  --oxide-bin target\debug\oxide.exe `
  --dpi 72 --timeout-sec 20 --max-memory-mb 1024 `
  --max-pages-per-file 1 --limit 5 --determinism-sample 2 `
  --threshold-profile renderer `
  --output-dir target\prompt03-after-benchmark
```

Measured result on this small vector smoke slice:

| metric | baseline | after |
| --- | ---: | ---: |
| files | 5 | 5 |
| visual pages compared | 5 | 5 |
| weighted score | 75.0 | 75.0 |
| visual pass | 100.0% | 100.0% |
| determinism | 2/2 stable | 2/2 stable |
| PDFium | skipped | skipped |

This is not a broad fidelity claim. It proves no visual regression on a small
Poppler-backed vector smoke slice while the default renderer remains unchanged.
The new architecture is measured by display-list replay equivalence on vector
pages. The prior broad renderer campaign remains documented in
`docs/rendering_fidelity_baseline.md`.

## Support Table

| Feature | Status | Tests | Reference comparison | Remaining gaps |
| --- | --- | --- | --- | --- |
| Display list | DONE for vector subset | Unit and page replay tests | Small Poppler smoke no regression | Text/images/groups/patterns not captured |
| Render device abstraction | DONE for CPU paths | CPU replay tests | Same pixels as immediate vector render | Future GPU/printer/debug devices not implemented |
| Bezier flattening | Already present | Existing `path` tests | Covered by existing render benchmark | Prompt 03 did not change algorithm |
| Stroke caps/joins/miters/dashes | Already present | Existing path tests plus display-list dash replay | Smoke no regression | Round joins/caps remain existing approximation quality |
| Nonzero/even-odd fill | Already present | Existing path/clip tests | Smoke no regression | No change in this pass |
| Anti-aliasing | Already analytic coverage | Existing path and render-quality tests | Existing Phase 7 benchmark | No analytic text hinting expansion here |
| Clipping | DONE in display-list subset | Clip replay and immediate tests | Smoke no regression | Complex clip plus groups still immediate-only |
| Transparency alpha | Already present in immediate renderer; display-list simple alpha only | Existing transparency tests | Existing Phase 7 benchmark | Soft-mask replay not captured |
| Blend modes | Already present in `PixelBuffer` | Existing blend tests | Existing Phase 7 benchmark | Group semantics remain immediate-only |
| Soft masks | Immediate renderer only | Existing SMask tests | Existing Phase 7 benchmark | Display-list soft masks deferred |
| Isolated groups | Immediate renderer only | Existing transparency tests | Existing Phase 7 benchmark | Display-list group ops deferred |
| Knockout groups | Approximate immediate renderer support | Existing focused tests/log diagnostics | Existing Phase 7 benchmark | Per-element knockout interior remains approximated |
| Axial shading | Already present | Existing shading tests | Existing Phase 7 benchmark | Display-list shading op deferred |
| Radial shading | Already present | Existing shading tests | Existing Phase 7 benchmark | Display-list shading op deferred |
| Mesh/patch shadings | Already present with bounded CPU painting | Existing shading tests | Existing Phase 7 benchmark | Display-list mesh op deferred |
| Tiling patterns | Immediate renderer support | Existing pattern tests | Existing Phase 7 benchmark | Display-list pattern op deferred |
| Tile rendering | DEFERRED WITH REASON | Not added | Not run | Needs group/tile surface rules before public API |
| Band rendering | DEFERRED WITH REASON | Not added | Not run | Same transparency-group constraint as tile rendering |
| Progressive/cancel | Existing immediate cancellation | Existing resource-limit/cancel tests | Not benchmarked here | Display-list replay cancellation points not public yet |
| Render cache | DEFERRED WITH REASON | Not added | Not run | Display-list cache needs document cache policy in next renderer pass |
| Display-list cache | DEFERRED WITH REASON | Not added | Not run | Added memory estimate and stats first |
| Metamorphic tests | DONE for vector replay | Pixel-for-pixel immediate vs replay | N/A | Scale/tile/band metamorphic tests still deferred |

## Memory And Safety

The new display-list path:

- Does not read files or streams directly.
- Uses parsed page operations supplied by `ContentEngine`.
- Does not bypass Prompt 02 decoding.
- Does not allocate page-sized offscreen buffers until explicit CPU replay.
- Records approximate operation memory for future cache budgeting.
- Keeps unsupported content on the existing immediate renderer.

The existing renderer memory guard remains `max_render_pixels()` before page
buffer allocation. The broad 2 GB large-render evidence from the previous
renderer campaign remains in `docs/rendering_fidelity_baseline.md`; Prompt 03's
new code did not add a full-document or full-file rendering path.

## CPU Performance Direction

Prompt 03 establishes the seam needed for Blend2D-style future optimization:

- High-level drawing operations are separated from the concrete CPU device.
- Blend mode, pixel buffer mode, source path, CTM, clip, and stroke state are
  explicit replay inputs.
- The CPU device remains safe Rust and reuses existing tested raster loops.
- Future optimized composite loops, safe SIMD kernels, tile devices, or a GPU
  backend can be added behind the `RenderDevice` trait.

No JIT pipeline was added. That is deliberate: the current engine prioritizes
stable Rust, WASM portability, and audited safety. Runtime code generation would
be a separate optional backend decision after profiling shows the replay device
is a bottleneck.

## Known Limits

Exact remaining limits after Prompt 03:

- Display-list replay covers vector paths only: save/restore, CTM, clipping,
  fill, stroke, line style, dash, simple colors, alpha, and blend mode.
- Text rendering still depends on the immediate renderer and current font
  subsystem; Prompt 04 should add display-list glyph/run operations.
- Image XObjects and inline images still render through the immediate renderer;
  a future display-list op should carry decoded-image references under Prompt 02
  decode limits.
- Form XObjects and transparency groups still render through immediate nested
  `RenderState`; display-list group begin/end ops are designed but not emitted.
- Soft masks are not replayed through the display list.
- Shadings and tiling patterns are not replayed through the display list even
  though the immediate renderer supports many of them.
- Tile and band rendering are not exposed yet because transparency group
  allocation and clip propagation need device-level rules first.
- Display-list caching is not enabled yet; this pass added stats and memory
  estimates needed to budget it safely.
- The small Poppler smoke is not a Tier-3 fidelity claim.

## Prompt 03B Update

Prompt 03B keeps the Prompt 03 vector-native display-list path and broadens the
architecture with a conservative compatibility-run bridge. Pages that contain
text, image XObjects, inline images, Form XObjects, shadings, patterns, or
transparency operators are now represented as typed `ContentRun` display-list
operations instead of disappearing into an opaque fallback. Replay of those runs
uses the existing immediate renderer semantics, so this closes the architectural
replay gap without moving font shaping or color management work into Prompt 03B.

The renderer now also exposes bounded tile and band rendering helpers,
pre-cancelled display-list replay checks, and a byte-accounted `RenderCache` for
tile-sized `PixelBuffer` entries. Tile and band tests verify stitched output
matches full-page output for supported pages, and cache tests cover hits,
eviction, oversized-entry skips, disabled budgets, and deterministic output.

Prompt 03B does not claim mature-renderer parity. Its expanded Poppler-backed
slice compared 50 files with 42 visual pages and reached a weighted score of
90.45 with 83.33% visual pass. Remaining visual misses are mostly font/text,
JPEG 2000/image-codec, and function-based shading cases that belong to Prompt
04, Prompt 05, or later renderer fidelity iterations.
