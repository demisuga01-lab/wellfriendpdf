# PDFium-level renderer closure gap register

Starting commit: `3139236f3ada008a1ab2844b0da5195cccaa1729`.

Prompt hash: `6ba827188bb426a5e008aa83122e3c21bb581c50877de3d24f0f4f7871d51563`.

This campaign starts from the previous zero-failure 5,044-PDF rendering corpus result. That result proves command-level completion on the corpus, not PDFium-level performance or visual parity.

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
