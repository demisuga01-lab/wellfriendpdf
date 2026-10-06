# PEBQ matched benchmark — 6 October 2026

This report binds the README benchmark to the renderer implementation shipped
in commit `44169d4a2d41f10f1318b7f1849a41eaaf197e19`.

The corpus contains 150 public and interoperability PDFs. Every adapter runs on
the same Linux VPS, CPU affinity, shuffled document order, PDF bytes, page and
DPI contract. Parsing opens in-memory bytes and resolves the page count.
Rendering opens each document once, performs one untimed resource warm-up, and
then produces five new page-one RGB rasters at 144 DPI. Final-raster reuse is
disabled, so the renderer must repaint every sample.

## Document open and resolved page count

Each percentile summarizes one median per document.

| Measurement | Wellfriend | PDFium | Poppler | MuPDF | qpdf |
|---|---:|---:|---:|---:|---:|
| Qualified documents | 150/150 | 150/150 | 150/150 | 150/150 | 150/150 |
| P50 | 1.024 ms | 0.609 ms | 2.765 ms | 2.527 ms | 4.058 ms |
| P90 | 2.422 ms | 1.277 ms | 4.456 ms | 9.738 ms | 17.557 ms |
| P95 | 3.484 ms | 1.844 ms | 4.714 ms | 107.373 ms | 25.498 ms |
| P99 | 9.690 ms | 5.122 ms | 13.590 ms | 408.382 ms | 48.894 ms |
| Maximum | 35.030 ms | 34.778 ms | 16.653 ms | 528.445 ms | 55.196 ms |

## Retained-resource rendering with fresh rasters

| Measurement | Wellfriend | PDFium | Poppler | MuPDF |
|---|---:|---:|---:|---:|
| Successful documents | 150/150 | 150/150 | 150/150 | 150/150 |
| P50 | 74.206 ms | 31.342 ms | 62.058 ms | 21.077 ms |
| P90 | 144.425 ms | 218.372 ms | 226.310 ms | 41.842 ms |
| P95 | 192.349 ms | 271.324 ms | 320.914 ms | 46.313 ms |
| P99 | 516.290 ms | 430.015 ms | 1,047.497 ms | 75.575 ms |
| Maximum | 628.836 ms | 792.267 ms | 1,176.852 ms | 104.838 ms |

All four renderers complete 150/150 documents. Repeated dimensions and raster
hashes are deterministic for all 600 engine-document pairs.

## Optimization evidence

The phase-aware image reduction was also measured directly against the prior
Wellfriend adapter on identical inputs and CPU placement.

| Document | Prior adapter | Current adapter | Pixel result |
|---|---:|---:|---|
| 70.2 MiB scanned NASA PDF | 512.501 ms | 34.050 ms | Identical dimensions and hash |
| Ordinary digital PDF | 97.580 ms | 98.424 ms | Identical hash |
| Dense vector/text PDF | 435.258 ms | 457.579 ms | Identical hash |

This optimization closes the pathological scan-minification path. Dense
vector/text pages remain the renderer's principal performance target.

## Reproduction identity

- Host: `v72937`, four-core KVM x86-64, Linux 7.0.0-31.
- CPU affinity: `2`.
- Corpus manifest SHA-256: `c5c357d38acd2a46e271ce42998abfc414811d9741ffd72c964ce2ced59e931d`.
- Wellfriend adapter SHA-256: `fdb1496b576489607debc088cf64fc05ddb66ff25c52b94fe7bc0cc9bcedb34b`.
- MuPDF adapter SHA-256: `55f6961d911b559488462b37d687faf71b3805ea5f66f7e5adf958c7dc8bd59f`.
- PDFium adapter SHA-256: `ed0f75e9d1df4a7ad42aa2a8c28f10daedcba4264767b882970ec63e126deb5f`.
- Poppler adapter SHA-256: `932fcaa6b6edf72f1b016449d54a7ee3e57671a1cd9eebaba08af375a956deee`.
- qpdf adapter SHA-256: `fdcbb4c6de1e9d0456facd8737adddecb0b40ef1af15049a599793f596a600f9`.
- Retained-render JSONL SHA-256: `4fa09ce969340656cb4b5e91f9e8ec5d409c3d2ccf482a008513c0a54f090d45`.
- Persistent-parse JSONL SHA-256: `ac21d7d8128f206bb501dfa800b6c35ba18601308c2e506ce53f786552899a5e`.
- Machine summary SHA-256: `3929d72bf5af5bd7a274d2987405996a85d566b96f8a1585908cd8a00f598263`.

The run uses `scripts/pebq_benchmark.py`, the Rust adapter in
`crates/engine/examples/pebq_adapter.rs`, and the native adapters in
`tools/pebq/native_adapter.cpp`. The benchmark host was also serving unrelated
workloads during the campaign, so the tables are reproducible run evidence,
not a claim about dedicated-hardware limits.
