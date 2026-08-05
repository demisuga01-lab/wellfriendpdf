# PDFium-level renderer closure gap register

Starting commit: `3139236f3ada008a1ab2844b0da5195cccaa1729`.

Prompt hash: `6ba827188bb426a5e008aa83122e3c21bb581c50877de3d24f0f4f7871d51563`.

This campaign starts from the previous zero-failure 5,044-PDF rendering corpus result. That result proves command-level completion on the corpus, not PDFium-level performance or visual parity.

Current source-only follow-up:

- Removed the remaining `DisplayOp::ContentRun`/`DisplayRunKind` raw-content
  bridge from the retained display-list program and page-renderer replay path.
- Wired the generic CPU display-list device through bounded cached fill/stroke
  path-mask replay before falling back to direct path painting.
- Added edge-bucketed scanline routing for large/complex path rasterization so
  scan conversion can reuse tile-local row edge buckets instead of scanning all
  subpaths for every row/sample.
- Added path-raster edge-bucket build/link/row counters to the render-corpus
  report so VPS validation can prove edge-bucketed scan conversion is active.
- Added lazy compressed clip visible-run caching and invalidation, so binary
  and sparse clip queries reuse row runs rather than rescanning the full mask.
- Routed binary-mask intersect/union against antialiased masks through
  run-windowed dense mutation instead of the old full-page
  `opacity_byte`-per-pixel loops.
- Changed solid all-visible/all-clipped clip masks to structural compressed
  masks that allocate no dense byte plane until partial mutation is required.
  Solid hints are compacted back to the structural form after operations that
  prove the mask became fully visible or fully clipped.
- Routed binary-clipped one-to-one RGB image row writes through the same
  cached visible-run iterator instead of scanning the clip mask row on every
  image write.
- Added warmed nested render-state cache transfer/absorption for soft masks and
  transparency groups, including glyph, glyph-mask, Type3, path-mask, image,
  scaled-image, soft-mask, shading, Form, pattern, and pooled offscreen-buffer
  caches. Child render states now start from the warmed parent raster caches
  and return them after replay.
- Added tight-window soft-mask group rendering that allocates and renders the
  bounded Form/clip pixel window and stores it as an origin-aware alpha mask
  with explicit outside-window default alpha, avoiding full-page alpha-mask
  allocation for bounded soft-mask groups.
- Added operation-level compositor backend reporting for solid fill,
  source-over, alpha-mask, glyph-mask, soft-mask, and separable-blend hot paths.
- Added the internal `wellfriendpdf-render-simd` backend crate so
  architecture-specific AVX2/SSE2/NEON kernels are isolated behind a safe API
  while `wellfriendpdf-engine` keeps `#![forbid(unsafe_code)]`.
- Wired native SIMD into solid opaque-run fills, normal source-over over opaque
  destinations, full-row source-over over opaque destinations, and the
  soft-mask opaque-destination row path, with debug scalar-equivalence guards
  in the SIMD crate.
- Added safe portable-wide source-over compositing for varying-alpha
  glyph/alpha/path mask rows. Scalar tails remain for short rows,
  non-opaque destinations, separable blends, high-quality fallbacks, and
  operation classes that still require the exact general compositor.
- Routed direct general fill/stroke fallback through the bounded scanline path
  for large or complex flattened paths, removing the active signed-area
  accumulator fallback surface for pathological vector pages.
- Routed all bounded glyph, Type3, and cached path alpha-mask rasterization
  through edge-bucket scanline masks so warm replay no longer enters
  bounding-box-sized accumulator grids for medium-sized mask geometry.
- Promoted glyph masks, Type3 masks/rendered glyphs, path fill/stroke masks,
  and offscreen buffers into the caller-owned `RenderDocumentCache`, so warm
  display-list replay can reuse raster artifacts across repeated page renders
  instead of rebuilding those masks inside each transient render state.
- Routed cached rendered Type3 glyph replay through a row-slice RGBA compositor
  with all-visible/binary-clip fast paths before falling back to per-pixel
  compositing for partial clips, soft masks, high-quality, and non-normal
  blend modes.
- Extended render-corpus cache reporting with glyph-hit/miss/eviction counters
  plus glyph-mask, Type3-mask/rendered-glyph, path-mask, and offscreen-pool
  byte/entry counts.
- Extended glyph-cache reporting with source family buckets for monochrome,
  color, bitmap, SVG-blocked, unsupported-bitmap, and other glyph families, so
  later VPS evidence can distinguish ordinary outline reuse from color/Type3
  and unsupported scaler/cache behavior.

These follow-up changes have not yet been compiled or validated. The next
required evidence is VPS-only compile/static validation, focused renderer
probes, then the Wellfriend renderer corpus run.

Current measured truth:

| Path | Files | Pages | Failures | Median ms | P95 ms | P99 ms |
|---|---:|---:|---:|---:|---:|---:|
| PDFium via pypdfium2 | 5,044 | 116,975 | 0 | 127.5 | 792.2 | 1,621.4 |
| Wellfriend document-cache path | 5,044 | 116,975 | 0 | 572.8 | 3,224.5 | 10,503.9 |
| Wellfriend retained display-list path | 5,044 | 116,975 | 0 | 970.4 | 4,269.2 | 11,114.6 |

The retained display-list path is still slower than the immediate/document-cache path. That remains the main renderer architecture defect.

Code changes in this step:

- Added row-slice `PixelBuffer` crop/blit primitives for already-rasterized tile movement.
- Replaced per-pixel progressive tile assembly with row-slice blitting.
- Replaced per-pixel tile crop extraction with row-slice crop copying.
- Added solid clip short-circuits for opaque rectangle fills.
- Added document-scoped display-list retention by page and DPI.
- Added normal opaque-pixel direct replacement when clip, blend, soft-mask, and knockout state prove exact source-over replacement.
- Added byte-fill paths for uniform full-buffer and row-run fills.
- Added an opaque RGB/gray image-paint path that writes sampled pixels directly when page state has no clip/composite/mask side effects.
- Added a bounded decoded Image XObject cache to `RenderDocumentCache` with a 512 MiB per-document cap.
- Added `render-corpus --repeat-page-renders` so retained warm replay can be measured inside one opened document/cache.
- Added transparent-page decision caching for retained replay.
- Added integer axis-aligned rectangle fill replay so common PDF rectangle paths bypass general flattened-path rasterization.
- Added row-span scanline clip fill and solid-hint refresh for clip construction, intersection, and union.
- Changed cached glyph outlines to shared `Arc<Path>` storage so cache hits do not clone full glyph paths.
- Added compat-mode row-span compositing for translucent normal fills.
- Added a compat-mode normal transparency-group composite fast path for unclipped buffers.
- Added magnified nearest-neighbour image run filling for opaque RGB/gray images.
- Added compat-mode opaque-background flattening with integer source-over math.
- Added analytic-AA row-run coalescing so fully covered path/glyph cells dispatch
  to row fills while partial antialiasing edges stay on per-pixel coverage.
- Added compat-mode transparency-group row shortcuts that skip fully transparent
  rows and copy fully opaque rows directly.
- Changed single-worker `render-corpus` evidence generation to stream JSONL
  records as files finish instead of retaining every file record until the end.
- Added retained full-page display-list raster caching keyed by page, DPI,
  render mode, visibility, prepress state, and tile identity.
- Added retained display-list tile raster caching for repeated viewport/zoom
  tiles.
- Added `render-corpus` split timing for display-list compilation and replay,
  plus display-list cache-hit, raster-cache-hit, and fallback counters.
- Added embedded Type1 built-in encoding parsing for PFA/PFB font programs and
  wired it into simple-font resolution when the PDF font dictionary has no
  `/Encoding`.
- Fixed Type1 OtherSubr handling so empty `pop` operations no longer inject
  zero operands, and implemented the Standard flex OtherSubr sequence as two
  cubic Bezier segments instead of treating flex control-point moves as real
  subpath moves.
- Enabled the existing bounded light grid-fitting path for Type1 body-text
  glyph outlines.
- Added a clipped normal transparency-group compositor fast path that converts
  partial clip masks into visible row runs before compositing, avoiding the
  previous per-pixel clip/blend dispatch for clipped page-sized groups while
  retaining identical source-over math.
- Hardened the reference-renderer comparison harness: pypdfium2/PDFium and
  PyMuPDF/MuPDF renders now run in isolated helper subprocesses with per-file
  timeouts, all PIL/reference handles are closed deterministically, and long
  corpus runs write compact progress JSON without retaining source paths.
- Added explicit image `/Mask` handling for color-key masks and referenced
  stencil/grayscale masks, producing alpha-bearing image samples before normal
  painting instead of silently ignoring the mask.
- Corrected image `/SMask` decoding to use the soft-mask stream's own declared
  dimensions, bit depth, and color space before alpha combination. Mismatched
  masks now fail closed by preserving the unmasked image rather than decoding
  with the main image's dimensions.
- Added a safe portable vector path for compat-mode translucent solid fills
  over opaque destination row runs. The path uses the `wide` crate, keeps a
  scalar tail, and is covered by a focused scalar-parity test.
- Added a document-cache entry for computed graphics-state soft mask groups,
  keyed by SMask identity, page, output dimensions, render mode, subtype, and
  CTM. Repeated page renders reuse the computed `AlphaMask`.
- Added a document-cache entry for decoded Type 4-7 mesh shading streams keyed
  by indirect shading object and shading type. This avoids repeated mesh stream
  decode on retained/repeated renders.

VPS validation for this first slice:

| Stage | Status | Evidence |
|---|---|---|
| `git diff --check` | pass | `/mnt/wellpdf-block/results/pdfium-renderer-closure-20260801T100439Z/diff-check.log` |
| `cargo fmt --all --check` | pass | `/mnt/wellpdf-block/results/pdfium-renderer-closure-20260801T100439Z/vps_cargo_fmt_apply_clip_hint.log` |
| Focused renderer unit tests | pass | `/mnt/wellpdf-block/results/pdfium-renderer-closure-20260801T100439Z/cargo-test-*.log` |
| `cargo check --workspace --all-targets --jobs 1` | pass | `/mnt/wellpdf-block/results/pdfium-renderer-closure-20260801T100439Z/vps_cargo_check_renderer_fast_paths_gate.log` |
| CLI release build | pass | `/mnt/wellpdf-block/results/pdfium-renderer-closure-20260801T100439Z/vps_renderer_cli_release_build_gate.log` |
| AA row-run compositor focused test | pass | `/mnt/wellpdf-block/results/pdfium-renderer-closure-20260801T100439Z/vps_test_aa_color_compositor.log` |
| Opaque transparency-row copy focused test | pass | `/mnt/wellpdf-block/results/pdfium-renderer-closure-20260801T100439Z/vps_test_group_row_copy.log` |
| Current `cargo check --workspace --all-targets --jobs 1` | pass | `/mnt/wellpdf-block/results/pdfium-renderer-closure-20260801T100439Z/vps_cargo_check_aa_runs.log` |
| Current CLI release build | pass | `/mnt/wellpdf-block/results/pdfium-renderer-closure-20260801T100439Z/vps_renderer_cli_release_build_aa_runs.log` |
| Streaming JSONL harness smoke | pass, 3/3 records | `/mnt/wellpdf-block/results/pdfium-renderer-closure-20260801T100439Z/render_streaming_jsonl_smoke2.log` |
| 100-file immediate render smoke | pass, 0 failures | `render-100-immediate-summary.json` |
| 100-file display-list render smoke | pass, 0 failures | `render-100-display-list-summary.json` |
| 50-file warm repeat immediate smoke | pass, 0 failures | `render_repeat50_real5000_immediate_gate-summary.json` |
| 50-file warm repeat display-list smoke | pass, 0 failures | `render_repeat50_real5000_display_list_gate-summary.json` |
| Full 5,044-file first-page immediate pass | pass, 0 failures | `render_5044_first_immediate_aa3-summary.json` |
| Full 5,044-file first-page display-list pass | pass, 0 failures | `render_5044_first_display_list_aa3-summary.json` |
| Safe vector blend focused test | pass | `cargo-test-wide-simd-v3.log` |
| Soft-mask group cache focused test | pass | `cargo-test-softmask-cache-v3.log` |
| Renderer package clippy after vector/cache changes | pass | `cargo-clippy-renderer-package-after-softmask-wide.log` |
| CLI release build after vector/cache changes | pass | `release-build-after-softmask-wide.log` |
| Latest 5,044-file repeat immediate pass | pass, 0 failures | `render-corpus-5044-immediate-after-softmask-wide-summary.json` |
| Latest 5,044-file repeat display-list pass | pass, 0 failures | `render-corpus-5044-displaylist-after-softmask-wide-summary.json` |
| Latest 5,044-file immediate/display-list raw-hash parity | pass, 0 mismatches | `render-hash-parity-5044-after-softmask-wide.json` |

The 100-file all-page smoke still shows display-list slower than immediate: immediate median `768.7 ms`, display-list median `808.8 ms`. After retained-list caching, decoded image caching, glyph-outline sharing, rectangle/clip/span fast paths, and transparency flatten/composite fast paths, the corrected 50-file first-page warm-repeat probe on `/mnt/wellpdf-block/corpus/real-5000-20260730T082710Z/pdfs` showed immediate median `84.3 ms` and display-list median `79.6 ms`; render-only median was `68.9 ms` for immediate and `67.7 ms` for display-list. This proves retained warm replay can beat the immediate path on that corrected smoke probe, but it is not a stable 2x guarantee.

The latest 5,044-file repeat-first-page pass has zero failures on both paths.
Immediate warm median is `15.0 ms` / p95 `64.1 ms`; retained display-list
warm median is `0.26 ms` / p95 `0.41 ms`. The display-list run recorded
`5,003` display-list cache hits and `5,003` retained raster-cache hits, with
`82` display-list fallbacks. Raw rendered output hashes matched between the
immediate and display-list runs for all compared files. Evidence:
`render_5044_repeat2_immediate_split_metrics.summary.json` SHA256
`c87917c602d6c49cce4a1631da4f19d0654c9aae70a611b0d860f7945689045b`,
`render_5044_repeat2_display_list_split_metrics.summary.json` SHA256
`4458998e72bbb31667258dc47f775deb93288560d011676882efec8a4db11baf`, and
`render_5044_immediate_vs_display_list_rawhash.json` SHA256
`4f0abd04e5f2e418c928ec2d45c5a8bbff36a845da0dbda1c3d097eec4cebf07`.

The latest repeat-first-page pass after the safe vector blend path, soft-mask
group cache, and mesh-shading stream cache rendered all 5,044 files with zero
failures on both paths. Immediate median was `77.18 ms` / p95 `231.15 ms`;
retained display-list median was `56.57 ms` / p95 `174.92 ms`. The retained
path recorded `5,003` raster-cache hits and `82` compatibility fallbacks, and
raw rendered output hashes matched for all compared immediate/display-list
records. Evidence: `render-corpus-5044-immediate-after-softmask-wide-summary.json`
SHA256 `ae9ed27f256da459c87dd8f03487fa4f419ddab0f23d83472b3b041a06bf802e`,
`render-corpus-5044-displaylist-after-softmask-wide-summary.json` SHA256
`a5c3681789c5f267676b668a385146a1b41fc81229e45e0e6d0847a47e253f8f`, and
`render-hash-parity-5044-after-softmask-wide.json` SHA256
`6083491c55240fa79326a4c4a249fe19e54e93d18382b6ec0953637423d22f51`.

That clears the repeat first-page warm-replay performance threshold, but it
does not close the complete PDFium-level renderer gap: broad external
PDFium/MuPDF/Poppler pixel-differential parity remains open.

A 100-file all-page repeat probe after the retained raster cache shows the
same trend on multi-page work: immediate rendered `7,516` page samples with
median file time `1,438.2 ms`, while display-list rendered the same `7,516`
page samples with median file time `842.2 ms` and `3,758` retained raster-cache
hits. Raw rendered output hashes matched for all compared files. Evidence:
`render_100_all_repeat2_immediate_current.summary.json` SHA256
`a77d75d98c8e5561e0dc000bc4ee172f3bf6d97f160c624dd48c1f5f6412e3a7`,
`render_100_all_repeat2_display_list_current.summary.json` SHA256
`6ff2c726d0b125be75f8b11cd05cbcd328ea369593152d5af47d8e7ef9411ad7`, and
`render_100_all_immediate_vs_display_list_rawhash.json` SHA256
`bf8a29c2cbb75ebce4cccea61d5a06cad7c3505a826958689649ff40d83a4763`.

The full 5,044-file all-page retained-vs-immediate campaign has not been rerun
after page-cache changes.

Reference-renderer visual comparison is also not closed. A compact 25-file
first-page probe using pypdfium2/PDFium, PyMuPDF/MuPDF, and Poppler completed
with zero tool failures and no size mismatches. Before the Type1 fixes, the
changed-pixel percentage at threshold 8 was median `10.92%` versus PDFium,
`9.20%` versus MuPDF, and `10.74%` versus Poppler. After Type1 builtin
encoding, OtherSubr/flex, and light grid-fitting changes, the 25-file medians
are `10.07%` versus PDFium, `6.69%` versus MuPDF, and `10.13%` versus Poppler.
A 100-file first-page probe after the same fixes completed with zero failures:
median `10.11%`, p95 `17.91%` versus PDFium; median `7.29%`, p95 `12.92%`
versus MuPDF; and median `10.08%`, p95 `17.64%` versus Poppler. Evidence:
`reference-compare-25-type1-hinting.json` SHA256
`1d4b3abc5cfa219a52447273bd704e5bdb2e9ed38bca4b50e71e00ff6b5ea4a1` and
`reference-compare-100-type1-hinting.json` SHA256
`cbf537451c4648a90315515f8f209af4ced807cd2d7a9884b39e7e38a4be9ea4`.

The first full classified 5,044-file first-page reference comparison completed
with zero Wellfriend render failures and zero reference-renderer failures. It
reported median changed-pixel percentages of `12.23%` versus PDFium, `8.72%`
versus MuPDF, and `12.18%` versus Poppler. A same-host 100-file inter-reference
probe measured the reference engines disagreeing with each other at similar
magnitude: PDFium vs MuPDF median `9.53%`, PDFium vs Poppler median `11.42%`,
and MuPDF vs Poppler median `8.59%`. That means the threshold-8 changed-pixel
metric is too strict to treat as literal single-engine parity, but it is still
useful as a regression and defect-finding signal.

The first full 5,044-file reference-compare rerun after the Type1 fixes exposed
two harness defects rather than renderer output failures: the original in-process
pypdfium2 path leaked native handles, and the first leak-fixed run could hang
inside an in-process reference renderer without a per-reference timeout. Both are
now recorded as failed validation attempts and addressed by the isolated helper
subprocess path above. The replacement 5,044-file first-page comparison after
explicit image `/Mask` support and corrected `/SMask` stream metadata completed
with zero Wellfriend render failures and zero reference-renderer failures. It
reported unchanged first-page medians: `12.23%` versus PDFium, `8.72%` versus
MuPDF, and `12.18%` versus Poppler. Evidence:
`reference-compare-5044-current-renderer.json` SHA256
`e581e28566ba7979dfde401b5406344eb058189a268e547cabf7d065fa217657` and
summary SHA256 `24c8cc02e85454631cc5f8ef7ebb15378ecb14c1ed80af34a36847d0bf20e178`.
The current renderer snapshot also passes `cargo fmt --all --check`,
`git diff --check`, `cargo check --workspace --all-targets --jobs 1`,
`cargo clippy --workspace --all-targets --jobs 1 -- -D warnings`, and
`cargo test --workspace --all-targets --jobs 1` on the VPS after the image-mask
changes.

The final isolated two-shard 5,044-file first-page comparison completed with
zero Wellfriend render failures and zero reference-renderer failures against
PDFium, MuPDF, and Poppler. The retained summary is
`render-metrics-5044-two-shard.json` SHA256
`8079766913b8051f9155116e8a14fd7f0dc1ac28917019f0cb626e6de160b241`, backed by
the VPS artifact `reference-compare-5044-two-shard.json` SHA256
`70ca632def37a59bb8b941fb482baecb7bdef4c3b680925ab9613827d3bfc6dd`.

That final run reports zero tool failures while still recording non-zero
threshold-8 visual differences. The changed-pixel averages are `13.12%` versus
PDFium, `10.00%` versus MuPDF, and `12.93%` versus Poppler, with p95 values of
`20.80%`, `15.55%`, and `20.64%` respectively. This is the closure boundary:
the renderer failure gap on the required 5,044-real-PDF first-page gate is
closed, but the evidence does not claim universal pixel identity, all-page
corpus parity, or complete PDFium graphics-feature equivalence.

Final VPS gates after the recursive-pattern guard, bounded clip fallback,
source-test corrections, and SVG threshold correction passed:
`cargo fmt --all --check`, `git diff --check`,
`cargo check --workspace --all-targets --jobs 1`,
`cargo clippy --workspace --all-targets --jobs 1 -- -D warnings`, and
`cargo test --workspace --all-targets --jobs 1`.

The complete mandatory gap register is tracked in
[gap-register.json](../../evidence/renderer-pdfium-closure/gap-register.json).
The final closure evidence is tracked in
[final-renderer-closure.json](../../evidence/renderer-pdfium-closure/final-renderer-closure.json).

Current verdict:
`renderer_failure_gap_closed_on_required_5044_first_page_gate_with_explicit_pixel_parity_limits`.

## Full PDFium-level rendering objective status

The final mega-prompt is broader than the zero-failure first-page renderer
closure above. It requires retained replay performance gates, all-page visual
differentials, correctness-qualified same-host PDFium parity ratios, native
display replay coverage, SIMD and scan-converter closure, tile-first progressive
rendering across bindings, allocation measurements, and a VPS pull/rebuild from
the final pushed commit.

Those broader requirements are not yet proven complete. The current resumed
work is continuing under
[objective-completion-audit.json](../../evidence/renderer-pdfium-closure/objective-completion-audit.json).
The first resumed production slice extends the compositor hot path with:

- a SIMD-compatible source-over row path for RGBA rows over opaque destinations;
- a SIMD-compatible transparent-page flatten path over opaque backgrounds;
- single-pass alpha-row classification for transparent-row skip and opaque-row
  copy decisions;
- a SIMD-compatible uniform-alpha source-over row path for opaque RGBA sources
  over opaque destinations;
- a source-over soft-mask row fast path for all-visible paint clips;
- a source-over soft-mask row-run fast path through binary clip masks, with
  fallback preserved for partial clips and undersized masks;
- soft-mask row classification that skips all-transparent mask rows and routes
  all-opaque mask rows through the unmasked source-over row compositor for both
  all-visible and binary-clipped row runs;
- a binary clip visible-run iterator that scans clip mask rows directly and
  feeds normal group-composite row runs without per-pixel `is_visible` dispatch;
- binary clip visible-run iteration for opaque and translucent rectangular fill
  fast paths, avoiding per-pixel clip queries before row-run painting;
- an opaque-source source-over row copy path that bypasses destination alpha
  scanning when a fully opaque source row is composited at full group alpha;
- an axis-aligned one-to-one 8-bit RGB image row path that writes exact opaque
  source rows into an unclipped normal destination without floating-point
  per-pixel sampling;
- a native named-shading display-list bridge that captures supported `sh`
  operations as typed high-level replay ops and dispatches them through the
  canonical page renderer, while preserving a typed compatibility fallback for
  missing shading resources;
- a native pattern path replay bridge that removes the pattern-triggered
  page-wide display-list fallback, preserves the original path-construction
  operators for pattern paints, replays available pattern paints as typed
  high-level display-list ops, and keeps unaffected vector path paints native;
- decoded image XObject cache LRU admission/eviction under the existing byte
  budget, so large-image documents can continue caching later useful decoded
  images without unbounded growth or stop-caching-on-full behavior;
- a binary-clipped one-to-one RGB image row path that writes exact source rows
  through visible clip spans and preserves fractional antialias clips on the
  general blending path;
- graphics-state cleanup for pattern colours, clearing stale pattern names when
  later device colour or non-pattern colour-space operators take over, so
  ordinary path paints do not remain falsely classified as pattern paints;
- a Form XObject decoded-program cache that decodes and parses reusable Form
  streams once per object revision, then replays the cached operation program
  for repeated invocations while still applying each invocation's CTM,
  inherited resources, BBox clip, transparency handling, and recursion limits;
- granular alpha/blend ExtGState display-list replay, removing the page-wide
  retained-list compatibility run for ordinary alpha/blend graphics state while
  keeping soft-mask ExtGState as an explicit unsupported boundary until exact
  group replay can prove correctness;
- native image display-list bounds for Image XObject and inline-image replay,
  allowing retained tile rendering to skip images whose transformed unit-square
  bounds do not intersect the current tile viewport while still routing
  intersecting or unbounded images through the canonical image painter;
- native text display-list bounds for text-showing operators, allowing retained
  tile replay to skip off-tile glyph painting while still advancing text state
  with invisible rendering; clipping text modes remain uncullable because their
  clip side effects must stay exact;
- native pattern-path display-list bounds, allowing retained tile replay to skip
  pattern-backed path paints outside the current tile while preserving canonical
  pattern replay for intersecting tiles;
- Form XObject BBox tile culling, using the cached Form program's BBox and
  Matrix with each invocation CTM to skip Form replay when the transformed BBox
  cannot affect the current tile;
- bounded LRU retention for decoded mesh-shading streams, replacing unbounded
  shading stream retention with byte-accounted reuse and eviction;
- axis-aligned scaled-image caching for exact integer-phase opaque gray/RGB
  XObject draws, storing device-size RGB scale results by image identity, target
  dimensions, and render mode, then replaying them through the one-to-one row
  writer and binary-clip path. Fractional-phase, affine, alpha/masked,
  interpolated, and JPX-compatibility images stay on the canonical sampler;
- Type3 geometry-cache key isolation, using the same resource-dictionary
  identity as the Type3 charproc program cache so same-named fonts with
  different Type3 dictionaries cannot share stale retained glyph geometry;
- optional-content fallback narrowing for retained display lists: ordinary
  marked-content spans and property dictionaries no longer force a page-wide
  immediate-render fallback, while BDC/DP entries that actually reference OCG,
  OCMD, or unresolved optional-content property resources keep the typed
  fallback;
- a separable blend-mode rectangle fast path for opaque source paints over
  opaque destinations in Multiply/Screen/etc. modes. It uses direct row slices
  for unclipped, all-visible, and binary-clipped runs, and keeps high-quality,
  masked, knockout, partial-clip, or non-opaque-destination cases on the
  canonical per-pixel compositor;
- clip-visible-bounds pruning for function, axial, and radial shading replay.
  Sparse clips now bound the shading raster loop to the minimal non-zero clip
  rectangle before the existing exact per-pixel clip opacity check, while empty
  clips return immediately and all-visible clips keep full-page behavior.

A focused dirty-snapshot runtime smoke rebuilt the CLI on the VPS after these
slices and reran `render_compare_page1` over the first 100 real PDFs from the
5,044-file corpus with zero failures. This is a regression smoke only; it does
not replace the required all-page, same-host PDFium/MuPDF/Poppler parity gates.

A dirty-snapshot retained-replay probe also compared immediate versus
display-list repeated first-page renders over the first 100 real PDFs. The
probe passed its 2x median / 1.5x p95 warm-replay threshold check and is
retained as focused evidence only; it does not replace the full specified
retained-replay workload or all-page parity gates.

The Form XObject decoded-program cache focused gate passed on the VPS:
`focused_form_program_cache.log` SHA256
`37d52d4f86c1ab0f2240a1d707c5ce5a705c619ce39a3d7446802e28b2a2c942`.
The same dirty snapshot then passed `cargo check -p wellfriendpdf-engine
--all-targets --jobs 1` SHA256
`aeb88e773f9801345f093f7dba3f6e6baa44ce4207b2911f2f79789a41792cb2` and
`cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings`
SHA256 `1953d5e514a69491b4763195baf692a492f53294d6b94f94a1e318e03463e477`.

The alpha/blend ExtGState display-list slice passed focused VPS validation:
`focused_alpha_extgstate_display_list.log` SHA256
`2015ba2f947d9826d945bebad665d0898bc8912c8579c04d2a8ae848bcc37256`,
followed by engine `cargo check` SHA256
`0c386a4f63b0ad4a92463c88609536f7302ce154abf137eb70b843981d3e43bb` and
engine `cargo clippy -D warnings` SHA256
`3efba3123819892a1fc9a73854d800e4e7d2342e0cafdd07ab535233be30afe9`.

The native image display-list bounds slice passed focused VPS validation:
`focused_image_bounds_culling.log` SHA256
`c55f0137f3ab6c8e6c79ff8c0c6c2f378866f0e66ba1271d8175c560712c3dcb`,
followed by engine `cargo check` SHA256
`32e69c5e4fbb3cc6354326f485ea55fe094a65f0d13b42e25d1b89aaf6815a5d` and
engine `cargo clippy -D warnings` SHA256
`28e0e9b2b8cacee19dc5566aeb67d988dd29c311b55c48acdc8304175eadbb14`.

The native text display-list bounds slice passed focused VPS validation:
`focused_text_bounds_culling.log` SHA256
`077674a8000dadbf624a483fa6c5e1baf2295b877567be8f3aa44abb662ecf16`,
followed by engine `cargo check` SHA256
`58bf99de1a3d78e8cf5bc8b42ebaef1250997654f2de3cb19577910a1e3fd7b1` and
engine `cargo clippy -D warnings` SHA256
`ed952bb4284d0487f49dcab3479b8c890f9ad2e3751a16d9c8068045e9852ce7`.

The native pattern-path display-list bounds slice passed focused VPS validation:
`focused_pattern_bounds_culling_final.log` SHA256
`82e84af7be73203a880431880dee8988f24f76a6df7479a25e1bdb413a3ad1fd`,
followed by engine `cargo check` SHA256
`bf6b87d15dd02f21b8accbbe8bbe3ded5696a4db77a4e6cecc3f63747a5f910d` and
engine `cargo clippy -D warnings` SHA256
`c565b3ce3c63570c9481dd8bc2bfab43c644bfccdf138e3b1d4d4c68933cd8fb`.

The Form XObject BBox tile-culling slice passed focused VPS validation:
`focused_form_bbox_culling_retry.log` SHA256
`88617fe22963365bc70a407d6fae882c454260fdb63ff8c148e73db30532e0bf`,
followed by engine `cargo check` SHA256
`6ad67a046dee4f8b67bcc8ad8a9f1311e1757b818bc51164eac07d740bfc708d` and
engine `cargo clippy -D warnings` SHA256
`107dd2adf2784fac16bdfdad2d8e67c73f111945de77d4091c5e1eced1148ab0`.

The bounded LRU shading-mesh cache slice passed focused VPS validation:
`focused_shading_mesh_lru.log` SHA256
`8e753516c02f2a372cac18334181d9b1e01dfd73264c999d0c92887bf9f084ef`,
followed by engine `cargo check` SHA256
`955f8a346f4b2cbe5bf742ae4e3aa9c6040ab9fb318d42d1b96d5b193644c18b` and
engine `cargo clippy -D warnings` SHA256
`ff3475a455170b530b3a1c52ff404aee5a830eb5915b6777fe92ec45c837ad28`.

The axis-aligned scaled-image cache slice passed focused VPS validation:
`focused_scaled_image_cache_final2.log` SHA256
`f041b708f2a32d3c9031fc62f5babdfa1d74882ec793fee954a1c31d3dcfb850`,
followed by engine `cargo check` SHA256
`1aba7b9dde9248606b1ac4ab0f47c8459cb3214890778af0522661113251d71d` and
engine `cargo clippy -D warnings` SHA256
`3d3ac52bc9716786326e0f2e89e45e34c5a4bd83ff53561409e1b46d22125569`.
A cumulative 100-real-PDF first-page PDFium/MuPDF/Poppler reference smoke
also passed with zero failures:
`reference_compare_100_after_slices.json` SHA256
`41723fc04a9b633ef939c201c17865045b3f25c538c769871ed6cf2fcb11d9ac`.

The Type3 geometry-cache key isolation slice passed focused VPS validation:
`focused_type3_cache_key.log` SHA256
`0ad4d977da2c39eaf38d1634b420b06f000cbbd84bd3996e1e923bb292cd579c`,
followed by engine `cargo check` SHA256
`d26b5f01b1c57df369c363ea3cf0c3bcc1d0b41cbc25710dc320402041f1db9a` and
engine `cargo clippy -D warnings` SHA256
`ac928eac54f23fc6eced54220d291b75b282497c60623968793181ebeacb03a1`.

The optional-content fallback narrowing and separable blend-mode rectangle
fast-path slices passed focused VPS validation:
`focused_marked_content_display_list_tests.log` SHA256
`6a98d988b1003c7927d929dfe82c00dec49f728d50ebf5b8832c8672dc1dab38` and
`focused_blend_fastpath_tests.log` SHA256
`476f7bdcc9ccf63f6977d22a037f9593be91ee3aaee38af0b9d33faf16d2f986`,
followed by engine `cargo check` SHA256
`5c9994201b603641f3765bbf016ce474b1882c738a31191817dc1e362076c99e` and
engine `cargo clippy -D warnings` SHA256
`b72c586c6c3483de382554b5b51f73f55bb68e52a491f3eaf81c6b3ff092a04c`.
The cumulative 100-real-PDF first-page reference smoke after the scaled-image
and Type3 slices also passed with zero failures:
`reference_compare_100_after_scaled_type3.json` SHA256
`91080d732011beba0e27240952d81e744b4916ef76a38f50bf8e55ee6a0f8537`.

The clip-visible-bounds shading slice passed focused VPS validation:
`focused_clip_visible_bounds_test.log` SHA256
`93d5d0488964d00e5a7390be8ad748d158e309f26859744cc6a59f23ffd362b1`,
followed by engine `cargo check` SHA256
`6a58b7d70a308143a64d493e2acd950408315f154437f26dc02fd4533177d8d3` and
engine `cargo clippy -D warnings` SHA256
`b5d1fbd459db71f727bcae026ab671a1f6aff74bd18b992a6d26f61fcec0ddbc`.
A cumulative 100-real-PDF first-page PDFium/MuPDF/Poppler reference smoke
after this slice also passed with zero failures:
`reference_compare_100_after_clip_visible_bounds.json` SHA256
`e29d2c17762d36bc2bbe2420d3f54580d6e229d4b1f3c216a56029e865df5d06`.

The native high-level display-list dispatch slice passed focused VPS
validation by routing retained native shading, Form XObject, and image XObject
ops through direct native handlers instead of generic replay dispatch:
`cargo_check_after_native_high_level_dispatch.log` SHA256
`a3adadaee400979985e6e9df794157af38a0835369f11365848e0287f1101ca4`,
`cargo_clippy_after_native_high_level_dispatch.log` SHA256
`059a45577313aa2b4a671140d68cc4293b939ff024ab7ea3ec6ecca18c37c2ac`,
and `cargo_build_cli_after_native_high_level_dispatch.log` SHA256
`be268a17bc00730834a2f1abf3a2a26aeb625f5e02628865618209cf2dbf000d`.
A cumulative 100-real-PDF first-page PDFium/MuPDF/Poppler reference smoke
after this slice also passed with zero failures:
`reference_compare_100_after_native_high_level_dispatch.json` SHA256
`58213480c596074d4ffa669677a79c1bb3b2c67b58d136c66e4a0a2507176817`.

The native inline-image display-list dispatch slice passed focused VPS
validation by routing retained inline-image ops through the direct image-data
handler for the common ID/data pair:
`cargo_check_after_native_inline_image_dispatch.log` SHA256
`37e91c99a80d98311c828ba6e16a32c08526134a2daa95ec4c6bf4e6625d8169`,
`cargo_clippy_after_native_inline_image_dispatch.log` SHA256
`e466b2f2dbbcf04d1d2f4069ff04251ca00d7e8f2285a19e4fce8e6096596cf8`,
and `cargo_build_cli_after_native_inline_image_dispatch.log` SHA256
`97abe7fbad665019e6f06efc91ecd67d8a11d51d280dd9855ad5e43424c8456e`.
A cumulative 100-real-PDF first-page PDFium/MuPDF/Poppler reference smoke
after this slice also passed with zero failures:
`reference_compare_100_after_native_inline_image_dispatch.json` SHA256
`f0febc8a3dc8efd4b93b09c61f3267097c08b51cd57d2c26e328f0168e124234`.

The native text and native pattern-path direct replay slices passed focused
VPS validation by routing retained text-showing and replayable pattern path
ops around generic operation dispatch:
`focused_native_text_direct_replay.log` SHA256
`7735f188e2cd8ff393d76966a9fe2232f0140fcbaf12d6a88da8ac4bb7d96218`,
`focused_native_pattern_direct_replay.log` SHA256
`413a4f5da1c292d84b0109fc826530c6f9ac66baac1d77b8db5cf53dfd35ccf0`,
followed by engine `cargo check` SHA256
`482a437dd5dc61d16d9c495e20bdc214626772407c8e6bbfae08062337127061`
and engine `cargo clippy -D warnings` SHA256
`9c0b9dc60af638a47bf4575edb7cebab6a876db790fb591fb2c5547f3d8a794c`.

The clip scanline allocation-reduction slice passed focused VPS validation:
`focused_clip_scanline_allocation_reduction.log` SHA256
`04bff819c1533ecbab5d6f395c67cb7b3070f6f5fcb88200ba2e0cf9c28f51e1`,
followed by engine `cargo check` SHA256
`24c2f85918b442c67bd05001d84776a0733aab85321ba91c1c5a55bdd5556670`
and engine `cargo clippy -D warnings` SHA256
`e99a17c6cc5d1a3a0b36624939260a6c1bd8ca2487d045ae9ff28514e9e9e59b`.
A latest 100-real-PDF first-page warm-repeat probe passed its focused warm
retained replay ratio gate:
`retained_warm_replay_ratio_100_latest.json` SHA256
`c1169d2841932a6ba60186d253f4613e6d2caca1e045d9d18812c1142fc4c1ac`.
A cumulative 100-real-PDF first-page PDFium/MuPDF/Poppler reference smoke
after these slices also passed with zero failures:
`reference_compare_100_after_native_text_pattern_scanline_final.json` SHA256
`5122ae2d5e7bcf09e84155860dee788ae78622fcc5d3b40eaec2d79eafc9ad55`.

The page-numbered all-pages reference harness passed a 10-file smoke covering
354 attempted real pages with zero failures:
`reference_compare_all_pages_limit10_latest.json` SHA256
`e9fb14722fae790be91aca83603cce8c805fea8fc7b333ddf765e52ff394c9fc`.

The tiling-pattern program-cache slice passed focused VPS validation by caching
decoded and parsed tiling-pattern content programs while preserving per-tile
resources, pattern matrix, clipping, forced-color handling, cancellation, and
recursion guards:
`focused_tiling_pattern_program_cache.log` SHA256
`e44be66c4a90890940eb91be8e35b14b6d22a8fb47e2b5577d3a14ee6202f863`,
followed by workspace `cargo check` SHA256
`a27d6dc446078e8b46974b7ab8f0d7f184617b4f91b077eef56198d1efbd2706`
and workspace `cargo clippy -D warnings` SHA256
`af0c3eff1968d208079949a9e65feb01a9e5b893920f9dcc4b2cca74015c10e7`.
A cumulative 100-real-PDF first-page reference smoke after the tiling-pattern
program-cache slice passed with zero failures:
`reference_compare_100_after_tiling_pattern_program_cache.json` SHA256
`8da66c02a60892abc4aa21ec934e7aeeb4ff6544715f2cc19728ffb77c8f1292`.

The native Form XObject bounds slice passed focused VPS validation by carrying
source Form `/BBox` and `/Matrix` metadata through `PageResources`, retaining
it in native display-list Form ops, and skipping off-tile retained Form replay
before stream-program dispatch:
`focused_native_form_bounds.log` SHA256
`0f6ac4bb4a6e4a06d60ff6f6fad722bcbf73558fce3eaa67b8cbfc35ea10ebce`,
followed by workspace `cargo check` SHA256
`8e4eeedf31df04ed2a83e61ff0fc010fd383737019ccdb100967019bf65d99b3`
and workspace `cargo clippy -D warnings` SHA256
`82d20f70563a8a60db8f566174fc97f2d608fdbaf6b23adf9809cfd1202a9e9e`.
A cumulative 100-real-PDF first-page reference smoke after the native Form
bounds slice passed with zero failures:
`reference_compare_100_after_native_form_bounds.json` SHA256
`225d4361c3438774136d8d5d397daabb9aa0ae5ebdc75ac05e6eaa3c92f1c659`.

The native shading clip-bounds slice passed focused VPS validation by tracking
display-list clip bounds across save/restore and clip operators, then using
those bounds to skip off-tile native shading replay when the active clip cannot
intersect the tile:
`focused_shading_clip_bounds.log` SHA256
`81c7a7544fcdf7bff4db3a79e297a5fb17630a26bffd743ab00d03a965bb03c0`,
followed by workspace `cargo check` SHA256
`04d01eba093935d9074575246cd31125805db094b1275b72af10100788399b19`
and workspace `cargo clippy -D warnings` SHA256
`f16b0ee01e94ef8874102648ccace9401af11768958199e74c023e0d443727cd`.
A cumulative 100-real-PDF first-page reference smoke after the native shading
clip-bounds slice passed with zero failures:
`reference_compare_100_after_shading_clip_bounds.json` SHA256
`5c65d8d0182037d6d5af3dab36548e28987e2d1559179d1cffe4f90a43ae5586`.

The display-list clip-op bounds slice passed focused VPS validation by carrying
retained bounds on clip operations and installing an empty tile clip when those
bounds do not intersect the replay viewport, avoiding off-tile clip-mask
construction:
`focused_clip_op_bounds.log` SHA256
`5c2ec5c43684647f7f3f564d004049f2aed863e8c6bc7072f1509881b3170695`,
followed by workspace `cargo check` SHA256
`e7cd47a382acd5a644526d637540c9f7f271986d9fa9a11d117d92386845aaab`
and workspace `cargo clippy -D warnings` SHA256
`0c22b8934604284a7945bcab0265ad649bc3f570986e63d0c88e56aec50c1162`.
A cumulative 100-real-PDF first-page reference smoke after the clip-op bounds
slice passed with zero failures:
`reference_compare_100_after_clip_op_bounds.json` SHA256
`43f813ac373d2bf17fbbe0913c09dba0817b62d17fccf81890302ce4145beb84`.
A latest retained warm-replay 100-file first-page repeat probe after the
clip-op bounds slice passed the focused median and P95 speed-ratio gate:
`retained_warm_replay_ratio_100_after_clip_op_bounds.json` SHA256
`2e04a02aee0cf4e129e167b5ea2853786c21ec3eae93c3ddce0a1a2327b3f0ea`.

The reference-comparison harness now supports bounded parallel page workers.
The workers=4 all-pages 10-file smoke covered 354 attempted real pages with
zero failures:
`reference_compare_all_pages_limit10_workers4_smoke.json` SHA256
`1d620482f883bed92ac06fdd8ce2a1de1bf9590a6a6bca0a061c6dec2e9f00d8`.

Current full-objective verdict:
`not_complete_against_full_pdfium_level_objective`.

## Source-only code-gap follow-up after the last VPS slice

No local build, test, render, or PDF processing was run for this follow-up.
The changes below are source-level implementation work that must be compiled
and validated on the VPS before they can be counted as measured closure:

- retained display-list raw-content bridging remains removed: source scans show
  no `DisplayOp::ContentRun` or `DisplayRunKind` in the retained display-list
  program or page-renderer replay files;
- the generic CPU display-list device keeps the same cached path-mask
  fill/stroke replay path as the canonical page renderer before bounded direct
  path fallback;
- binary and complex clip fallback construction now returns run-backed
  `ClipMask` values instead of materializing a dense page mask one row span at
  a time;
- `ClipMask` intersection, union, rectangular updates, and tile-window copies
  now stay in the compressed visible-run representation for binary masks,
  avoiding dense full-page materialization for ordinary rectangle/clip boolean
  operations;
- anti-aliased clip construction now compacts exact binary coverage results
  into the same run-backed `ClipMask` representation while retaining dense
  coverage only when real partial antialias samples exist;
- retained display-list clips, Form BBox clips, and tiling-pattern tile BBox
  clips now use run-backed rectangle masks when the transformed box is
  axis-aligned;
- patterned stroke device clips now use `rasterize_flat_binary_clip_mask`, so
  stroke outlines used only as binary clipping masks avoid page-sized alpha
  buffers;
- the scanline edge-bucket rasterizer stores row edge membership in a flat
  offset/link table instead of per-row vectors, reducing allocator pressure on
  pathological vector pages;
- the complex `ClipMask::from_path` binary fallback now also uses a flat
  offset/link start table rather than per-row start vectors;
- display-list native pattern and inline-image replay no longer falls back to a
  raw retained-content dispatch bridge when an impossible unexpected operator
  appears in those native op bundles; it records a debug skip instead;
- Form XObject recursion now tracks object/generation keys on a stack, so
  direct or indirect Form cycles are refused before repeated replay reaches the
  coarse nesting limit;
- architecture-specific SIMD kernels are isolated in the internal
  `wellfriendpdf-render-simd` crate so `wellfriendpdf-engine` can keep
  `#![forbid(unsafe_code)]` while still dispatching active AVX2/SSE2/NEON
  compositor kernels through safe row-level APIs;
- compositor reporting now separates active operation coverage from detected
  hardware: solid fill, source-over, alpha/glyph masks, and supported
  soft-mask opaque-destination rows can report the native backend, while
  separable-blend and exact general fallback paths remain `portable_wide` or
  scalar as appropriate;
- the compressed clip cache now stores visible runs in one flat run array plus
  row offsets instead of `Vec<Vec<...>>`, reducing per-row allocation pressure
  for large rectangular, tile-local, and binary path clips;
- binary clip intersection and union now operate directly on the flat run
  caches instead of cloning both sides back into nested per-row vectors;
- general page-path fill, custom-compositor fill, bounded glyph masks, Type3
  masks, and cached path alpha masks now route through the edge-bucket
  scanline rasterizer by default. The old accumulator counters are retained as
  report compatibility fields, but the signed-area accumulator implementation
  is no longer present as an active retained-display-list mask path;
- decoded image, scaled image, soft-mask group, shading mesh, Form program, and
  tiling-pattern program cache hit/miss/entry/byte reporting is now exposed
  through the render-corpus JSON cache report, alongside glyph, Type3,
  path-mask, and offscreen-pool reporting.
- font-program byte-cache and font-resolver/scaler cache hit/miss/entry/byte
  reporting is now exposed through the same cache report, giving VPS probes a
  direct way to evaluate font-instance lifecycle reuse instead of inferring it
  only from glyph-mask outcomes.
- soft-mask group caching is now bounded and LRU-tracked with byte accounting,
  replacing the previous plain map with cache admission/eviction hooks for
  repeated transparency groups.

VPS compile boundary for this source follow-up:

- `cargo fmt --all --check` passed on
  `/home/ubuntu/wellpdf/tmp/renderer-gap-20260804T032756Z/repo`;
- `cargo check --workspace --all-targets --jobs 1` passed on the same VPS
  snapshot with peak RSS 2,439,948 KiB;
- no local build, test, render, or PDF processing was run.

Current source-code implementation posture:
`renderer_pdfium_gap_source_paths_patched_pending_vps_compile_and_corpus_validation`.
