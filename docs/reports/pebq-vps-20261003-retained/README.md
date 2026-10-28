# PEBQ retained-resource fresh-raster diagnostic

## Contract

Each engine opens each of the fixed 100 PDFs once, performs one untimed warm-up,
then executes three timed page-one renders at 144 DPI. Every timed iteration
creates a new raw RGB raster. Final-raster reuse is disabled. All workers use
CPU 2 on VPS `v72937`.

This profile measures warm document/resource execution. It does not represent
document-cold rendering or a final-image cache hit.

## Timing

Each percentile contains one median per PDF, so each document has equal weight.

| Benchmark | Wellfriend PDF | MuPDF | PDFium | Poppler |
|---|---:|---:|---:|---:|
| Successful documents | 100/100 | 100/100 | 100/100 | 100/100 |
| P50 | 85.802 ms | 14.315 ms | 23.754 ms | 46.582 ms |
| P90 | 128.807 ms | 28.546 ms | 36.831 ms | 75.748 ms |
| P95 | 159.532 ms | 34.325 ms | 72.795 ms | 107.362 ms |
| P99 | 218.646 ms | 45.863 ms | 200.248 ms | 205.693 ms |
| Maximum | 239.446 ms | 82.233 ms | 212.789 ms | 236.042 ms |

Repeated raster dimensions and hashes are stable for 400/400 engine-document
pairs. Wellfriend PDF dimensions and hashes match the qualified 2026-10-02
Wellfriend outputs for 100/100 shared PDFs.

## Verdict

Wellfriend PDF is not the fastest engine in this profile. Its median is 5.99x
MuPDF's median, 3.61x PDFium's median, and 1.84x Poppler's median. The result
does not support a single-digit full-page rendering claim.

The current optimization reuses bounded compiled plans and resources without
reusing final pixels. The remaining cost is real raster work. A sampled
hard-document profile assigns the largest visible shares to bilinear image
sampling, clip-mask materialization, pixel compositing, color conversion, and
image decode. Further speed work targets those stages and exact parallel tile
execution rather than benchmark-specific shortcuts.

## Evidence and identity

- Base Git revision: `3d9062b1508ed56acad8c49a4b09440bc59c2e8a`.
- Wellfriend adapter SHA-256: `9bdeef36cef80d065f9c93e5ecbd28d16b1957b33483a504d0960fe3af0d8a4f`.
- MuPDF adapter SHA-256: `55f6961d911b559488462b37d687faf71b3805ea5f66f7e5adf958c7dc8bd59f`.
- PDFium adapter SHA-256: `ed0f75e9d1df4a7ad42aa2a8c28f10daedcba4264767b882970ec63e126deb5f`.
- Poppler adapter SHA-256: `932fcaa6b6edf72f1b016449d54a7ee3e57671a1cd9eebaba08af375a956deee`.
- Raw JSONL SHA-256: `a80989bf8b38f496e2e6fa32b7d3f1e690bfca090541dc019ef3676d62db8399`.
- Summary SHA-256: `10efd1a00151015ec59c5caa2054c3f140d347ede226e83eb0a22a792d7167ef`.
- Native-adapter source SHA-256: `2482dae30b0022bdeec089c0096ef3d1d3da4260e720c2d969c10783f438c554`.
- Rust adapter source SHA-256: `725733f174df81664c0f9ce73f7741c40788fd6918ab277de29f722477a32c87`.
- Renderer source hashes: `page_renderer.rs`
  `e703d687416a163589a48a22c029ab8ba08996d2940c5f1848e26a30f9a21e30`;
  `image_painter.rs`
  `70898675e2040a39be4c839e28dd4d9bc53440763d5fc1cfc6eedc2ac9b18ff1`.

The [raw observations](render-retained-resources.jsonl) and
[machine-readable summary](retained-summary.json) are part of this report.
These hashes bind the recorded files and binaries. They do not constitute a
reproducible-build proof that independently rebuilds each binary from source.
