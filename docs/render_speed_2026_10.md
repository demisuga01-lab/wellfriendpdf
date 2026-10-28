# Rendering speed work, October 2026

This note records the October 2026 rendering speed work: what was slow, what
changed, how it was measured, and what remains. The goal was the median time
to render one page of a real-world corpus at 100 dpi, single-threaded,
including process start, document open, PNG encoding and output.

## Result

Corpus: the 100-document comparison set (8 regression PDFs plus 92 sampled
from the 500-document Wikimedia Commons corpus; mostly Internet Archive
scans), 380 pages (pages 1, n/4, n/2 and 3n/4 of each document), 100 dpi, one
thread (`RAYON_NUM_THREADS=1`), one CLI process per page.

| | Before | After |
|---|---|---|
| Median per page, wall clock | 0.50 s | 0.15 s |
| Median per page, CPU time | 0.34 s | 0.14 s |
| 25th percentile, wall | 0.21 s | 0.09–0.10 s |
| 90th percentile, wall | 1.20 s | 0.57 s |
| Total for 380 pages, wall | 222 s | ~92 s |

Other engines on the same pages (quiet machine): PDFium 0.066 s (document
already open, no encoding), MuPDF 0.30 s, Poppler `pdftoppm` 0.75 s.

Accuracy rose in the same period. Mean SSIM against the median of the other
three engines went from 0.928 to 0.935, with no failed pages. On the 156
Internet Archive pages it went from 0.916 to 0.928 (20 pages better and 5
worse by more than 0.02).

## Method

- **Timing.** `~/bench/mkprof_cpu.py` emits one
  `/usr/bin/time -f "%e %U %S"` invocation per page. The test VM is shared:
  another user's CI raised the load to 15–28 at times, which inflated wall
  times by 1.5–2×. CPU time (user + system) equals wall time for a
  single-threaded render on an idle machine and is far less sensitive to
  load. A/B comparisons were interleaved.
- **Profiling.** `perf record -F 999` over the whole page set (aggregate
  hotspots) and `perf record -F 9999` over single pages (breakdowns).
- **Quality.** `~/bench/fullcompare.py … render` computes SSIM of each
  engine's page against the per-pixel median of the others.
- **Correctness.** Every change keeps the full engine and CLI test suites
  green (5,258 tests). Changes to resampling, rounding and clipping are
  bit-exact by construction, and the existing byte-equality tests between the
  cached and direct image paths still pass.

## What was slow

The first corpus-wide profile (median 0.50 s):

| Share | Cost |
|---|---|
| 9.3% | `ImagePainter::area_average_sample`: per-pixel box arithmetic |
| ~20% | JPEG 2000 decoding (hayro): inverse wavelet, bit-plane decoding |
| 5.6% | `nearest_sample`: per-pixel index arithmetic |
| ~6% | zip deflate of already-compressed PNGs, PNG encoding |
| 3.2% | SHA-256 of the whole input file on every render |
| ~15% | kernel page faults from large fresh allocations |

The median band was almost entirely scanned books in two forms:

- **MRC pages** (Internet Archive): a low-resolution JPEG 2000 background,
  plus a full-resolution JPEG 2000 foreground masked by a JBIG2 soft mask
  (`/SMask` with `/Decode [1 0]`) or stencil mask (`/Mask`, `ImageMask true`);
- **large JPEG scans** (National Diet Library and newspapers).

## Changes

### 1. Reduced-resolution decoding of masked JPEG 2000 images

`render/page_renderer.rs`, `images/smask.rs`.

Before, any image with an `/SMask` or `/Mask` was flagged
`requires_full_image_postprocessing`, which disables reduced decoding. The
1906×3208 foreground of a typical MRC page was decoded at full resolution to
be drawn at about 640×1070 pixels.

Now:

- **Soft masks.** `image_smask_allows_shared_reduction` allows reduction when
  the image is JPEG or JPEG 2000 (`image_uses_reduction_codec`) and the soft
  mask has no `/Matte`. A matte unblend needs the exact pre-blended pixels.
  When the mask is not plain 8-bit gray (JBIG2, CCITT, 1-bit, DCT, or
  `/Decode` inverted), `SmaskLoader` decodes it whole with the same decoder as
  the unreduced path and box-averages it to the reduced image's size
  (`box_downscale_exact`).
- **Stencil masks.** `explicit_image_mask_allows_reduction` allows reduction
  for a JPEG/JPEG 2000 image with a `/Mask` stream. When the image came back
  reduced, the finer stencil is box-averaged down to the image size as
  coverage (inverted when the mask paints zeros) and combined as a soft mask,
  instead of enlarging the image back to the mask resolution.
- Raw (unfiltered) images are excluded on purpose: the planner may give them
  a windowed decode, which requires an exactly matching mask.

Effect: Nyasaland p361 went from 0.34 s to 0.11 s, with peak memory from
220 MB to 47 MB. Zeitschrift p209 went from 0.37 s to 0.21 s and Federal
Register p41 from 0.41 s to 0.20 s. SSIM on these pages rose: the box-averaged
mask gives anti-aliased text edges, where the old path enlarged the image with
nearest-neighbour sampling.

### 2. JPEG 2000 reduction target at the drawn size

`native_reduction_target_from_plan`. The codec is asked for an image at least
as large as the device size. Before, both codecs were asked for at least 2×.

- **JPEG 2000:** a resolution level is the wavelet low-pass of the full
  image. hayro skips `floor(log2(source / target))` levels, so the result is
  never smaller than requested. With 2×, typical 300–400 dpi scans viewed at
  screen resolution (about 3× the device size) were never reduced.
- **JPEG stays at 2×.** jpeg-decoder may pick a scale smaller than requested in
  one axis, and a final resample near 1× is not a minification, so it is
  bilinear-sampled and aliases fine print. 1.25× was tried; it cost 0.07–0.10
  SSIM on 1915 newsprint (Behar Herald) and on a Chinese scan (CNTS), so it
  was reverted.

### 3. Separable image sampling

`render/image_painter.rs`.

For every axis-aligned paint (and, in `paint_affine`, for flipped and
quarter-turned images, where u and v each depend on a single device axis),
the sample coordinate of each destination column and row is computed once
(`SeparableAxes`).

- **Area averaging.** `AreaAxisTaps` precomputes each column's and row's
  source box: centre ± half footprint, clamped and snapped exactly as in
  `area_average_sample`, as `(source index, overlap)` taps.
  `area_average_box` accumulates the same products in the same order (rows
  outer, columns inner), with premultiplied alpha weighting for RGBA. The
  output is byte-identical to the per-pixel path.
- **Nearest.** `paint_separable_nearest` precomputes the source column and
  row of every destination column and row with `nearest_sample`'s exact
  formula.
- **Cached pre-scale.** `scale_axis_aligned_default_rgb_for_target` (the
  cached path for opaque RGB/gray images) gained the same precomputation for
  magnification and for the mixed case where one axis enlarges and the other
  shrinks. Its both-axes-minify loop now indexes row slices with one bounds
  check per source row instead of a checked multiply and `Option` lookup per
  tap, after validating every tap once. Arithmetic and order are unchanged;
  `rotated_cached_reduction_matches_general_sampler` and
  `fractional_cached_reduction_matches_general_sampler` still pass.

Effect: a magnified 37×27-inch NDL page went from 0.35 s to 0.21 s;
per-pixel sampling disappeared from the corpus profile.

### 4. Fast exact rounding

`render/mod.rs`, `RoundByte`.

On baseline x86-64 (no SSE4.1), `f32::round`/`f64::round` compile to libm
calls. `value.round_byte()` computes `value.round().clamp(0.0, 255.0) as u8`
with a truncation and one comparison. It is exact for every input, including
NaN, negatives, half-way values and the largest value below 0.5; a test
sweeps −3 to 260 in 1/16 steps plus edge cases for `f32` and `f64`. It
replaced 43 hot call sites in the compositor, image painter, colour and
shading code.

### 5. Cheap cache identity instead of a whole-file SHA-256

`reader.rs::content_revision`, `render/document_view.rs`.

Render caches bind to a document revision. The revision was the first 8 bytes
of a SHA-256 of the whole file, so every render hashed the file and, for
path-backed input, read all of it. A 150 MB scan was read in full to render
one page. Now:

- in-memory input: XXH64 of the bytes (implemented in-crate, checked against
  reference vectors), about 10 GB/s;
- path-backed input never materialized: XXH64 of the path, length,
  modification time, the first and last 64 KiB, and 64 evenly spaced 4 KiB
  samples.

The public `fingerprint()` remains the full SHA-256, computed only when
asked.

### 6. Seeded fast hashing for object maps

`fast_hash.rs`. The xref map and the renderer's object-identity map are keyed
by `(object, generation)` and were hashed with SipHash, about 6% on text
pages. `ObjectMap` uses a multiply-rotate hasher with a per-process random
seed (from `getrandom`) and a MurmurHash3 `fmix64` finish. PDFs are untrusted
input, so the seed and avalanche keep a hostile file from choosing colliding
object numbers.

### 7. Analytic rectangular clips

`render/buffer.rs`.

A clip path that fills exactly an axis-aligned rectangle is common: producers
often write it as a 6-point polygon with collinear midpoints and fractional
coordinates. It used to go through the general 4×4-supersampled scanline
rasterizer over a full-page mask. `axis_aligned_rectangle_bounds` recognises
such a path exactly, testing only what the rasterizer sees:

- the subpath is explicitly closed (last point equal to the first);
- horizontal edges are ignored, as the rasterizer ignores them;
- every other edge is vertical and lies on one of two x positions;
- on each side, the vertical edges tile `[y_lo, y_hi)` exactly once, in one
  direction, with opposite directions on the two sides. Endpoint comparisons
  are exact equality, so no arithmetic can blur the decision.

`rectangle_supersampled` then writes, for each pixel, the count of covered
sub-rows × covered sub-columns with the same sample positions, edge tests and
byte conversion. A test compares it byte-for-byte with the rasterizer for
both fill rules, collinear points and off-page coordinates.

The mask classification scan (`from_alpha_bytes`, `refresh_solid_hint`) now
works per row: it finds the first and last non-zero sample and classifies
only that span with branch-free folds the compiler vectorizes.

### 8. Faster mask downscaling

`images/decoder.rs::box_downscale_exact`. Per destination row, it sums the
source rows into per-sample `u32` column totals (a contiguous add that
vectorizes), then totals each destination column's span. Same spans and
rounding as the direct box average, checked against it by a test.

### 9. Output path

- `cli/src/main.rs`: PNG, JPEG and WebP page entries are *stored* in the
  output zip, not deflated a second time.
- `render/buffer.rs::compact_rgba_to_rgb_in_place`: the RGBA→RGB conversion
  before PNG encoding copies in growing blocks that never overlap their
  destination (block ≤ p/3 at pixel p), so the inner loop vectorizes; it was a
  bounds-checked per-pixel loop.
- `cli/src/main.rs::tune_allocator_for_one_shot_process`: on Linux/glibc the
  CLI raises `M_MMAP_THRESHOLD` to 32 MiB (glibc's maximum on 64-bit) and the
  trim threshold and top pad. Large image buffers then reuse already-faulted
  heap pages instead of a fresh `mmap` each time. Page faults on a scan page
  went from 19,032 to 12,988, and CPU time from 142 ms to 135 ms.

### 10. Pathological clip-heavy page

TI LM358 p34 used 685 MB and 1.48 s, because every clip intersection
allocated and scanned a full-page mask. The row-span classification and the
analytic rectangles brought it to 0.86 s. Bounded clip masks are needed to go
further (see below).

## Tried and rejected

- **`-C target-cpu=x86-64-v3`** (AVX2, FMA, SSE4.1 rounding): slower in two
  interleaved A/B rounds on the shared VM (median 0.25 s against 0.165 s). Not
  adopted; it would also drop support for older CPUs.
- **JPEG reduction target 1.25×**: faster, but lost 0.05–0.10 SSIM on fine
  newsprint and Chinese scans. Reverted to 2×.

## Where the remaining time goes

Profile of a median MRC page after the changes (Maryland Medical Journal
p108, about 0.13 s CPU):

| Share | Cost |
|---|---|
| ~27% | JPEG 2000 decode (hayro: inverse wavelet, bit-plane coding, colour transform), already SIMD-dispatched |
| ~17% | box resampling of the background (cached pre-scale) and of the masked foreground |
| ~7% | JBIG2 mask decode |
| ~4% | mask box-downscale |
| ~6% | compositing (blend, rounding inside hayro's sample conversion) |
| ~8% | kernel page faults |

On text and vector pages, full-page clip masks (`ClipMask::from_alpha_bytes`,
`from_path`) and CMap range expansion into a `BTreeMap` are the largest
remaining items.

To reach 0.10 s per page, in order of expected payoff:

1. A faster JPEG 2000 decoder: an optimized vendored hayro, or OpenJPEG, which
   adds a C dependency.
2. One separable resampling kernel (f32 or fixed point) shared by the cached
   and direct paths, so their byte-equality is preserved by construction
   (about 10% on scans).
3. Clip masks stored for their bounding box instead of the whole page (large
   on clip-heavy pages; `ClipMask` has 61 construction sites).
4. JBIG2 decoded straight into reduced coverage, skipping the full-size 8-bit
   buffer and the second pass (about 5% on MRC pages).
5. Decoding a page's independent images in parallel. This helps normal
   multi-core use but not the single-thread benchmark; the renderer decodes
   images on demand during interpretation, so it needs a prefetch pass.

## Reproducing

On the test VM:

```
cd ~/bench
python -I mkprof_cpu.py ~/bench/bin/wellfriendpdf-<label> > prof.sh
TLOG=out.txt OUT=/tmp/o.zip sh prof.sh
awk '{print $1, $2+$3}' out.txt | sort -n -k2 | awk '{c[NR]=$2} END{print c[int(NR/2)]}'
python -I fullcompare.py results/full-<label>.jsonl ~/bench/bin/wellfriendpdf-<label> 8 render
python -I summarize_full.py results/full-<label>.jsonl
```
