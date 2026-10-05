# Matched 150-PDF benchmark - 2026-10-05

This is the benchmark presented in the repository README. Every engine receives
the same PDFs, operation, CPU affinity, warm-up policy, and output contract.
Every percentile contains one median per document, so a large PDF cannot drown
out the rest of the corpus by contributing more samples.

## Parsing

The resident contract opens the document bytes and resolves the page count.
Each engine completes one untimed corpus warm-up followed by three randomized
timed passes.

| Resident parse | Wellfriend | PDFium | Poppler | MuPDF | qpdf |
|---|---:|---:|---:|---:|---:|
| Qualified documents | 150/150 | 150/150 | 150/150 | 150/150 | 150/150 |
| P50 | 1.032 ms | 0.615 ms | 2.769 ms | 2.718 ms | 4.037 ms |
| P90 | 2.355 ms | 1.284 ms | 4.991 ms | 11.730 ms | 19.321 ms |
| P95 | 3.157 ms | 1.812 ms | 5.628 ms | 100.727 ms | 26.906 ms |
| P99 | 11.555 ms | 5.829 ms | 9.887 ms | 422.891 ms | 44.812 ms |
| Maximum | 43.615 ms | 47.796 ms | 24.623 ms | 439.354 ms | 48.510 ms |

All five engines resolve the same page count for all 150 documents.

The fresh-process profile includes process startup and one parse request.

| Fresh-process parse | Wellfriend | PDFium | Poppler | MuPDF | qpdf |
|---|---:|---:|---:|---:|---:|
| P50 | 16.396 ms | 18.771 ms | 51.699 ms | 29.110 ms | 38.756 ms |
| P90 | 70.040 ms | 66.875 ms | 91.186 ms | 102.120 ms | 93.334 ms |
| P95 | 104.810 ms | 183.659 ms | 170.529 ms | 280.849 ms | 216.425 ms |
| P99 | 470.489 ms | 533.304 ms | 533.090 ms | 1,083.218 ms | 730.800 ms |
| Maximum | 982.034 ms | 735.013 ms | 764.464 ms | 1,175.489 ms | 732.372 ms |

## Rendering

Each engine opens each PDF once, performs one untimed warm-up, and produces
three new page-one RGB rasters at 144 DPI. Final-raster caching is disabled.

| Retained-resource fresh raster | Wellfriend | PDFium | Poppler | MuPDF |
|---|---:|---:|---:|---:|
| Successful documents | 150/150 | 150/150 | 150/150 | 150/150 |
| P50 | 103.973 ms | 28.412 ms | 60.069 ms | 19.992 ms |
| P90 | 516.076 ms | 209.293 ms | 221.332 ms | 45.663 ms |
| P95 | 537.943 ms | 259.315 ms | 294.606 ms | 50.374 ms |
| P99 | 579.335 ms | 392.720 ms | 968.025 ms | 72.310 ms |
| Maximum | 749.481 ms | 703.016 ms | 1,076.739 ms | 75.102 ms |

All 600 engine-document pairs produce three identical raster hashes and
dimensions across their timed repetitions.

## Execution contract

- Host: `v72937`, Linux x86-64, four logical CPUs.
- Affinity: every adapter and controller request uses CPU 2.
- Corpus: 150 PDFs, 9,214 pages, 2.036 GiB.
- Corpus manifest SHA-256:
  `c5c357d38acd2a46e271ce42998abfc414811d9741ffd72c964ce2ced59e931d`.
- Renderer source commit:
  `0539f33972b677512aaf336573a6a8385d0cf3bd`.
- Wellfriend benchmark adapter SHA-256:
  `154da7ea9af5f56c0cd83d602a9e630f8333d0e73c6b03bfdbdf951b8e5c189a`.
- PDFium: `150.0.7857.0`.
- Poppler: `26.01.0-2ubuntu0.1`.
- MuPDF: `1.27.0+ds1-3ubuntu2`.
- qpdf: `12.3.2-1`.

## Reproduction

```bash
python3 scripts/pebq_parse_only.py \
  --corpus /root/raptor-corpus-150-20261003 \
  --adapter-dir /root/pebq-current-20261005/bin \
  --output /root/pebq-current-20261005/parse \
  --limit 150 --persistent-repetitions 3 --warmup-passes 1 \
  --fresh-repetitions 1 --cpu 2 --seed 20261005

python3 scripts/pebq_retained_benchmark.py \
  --corpus /root/raptor-corpus-150-20261003 \
  --adapter-dir /root/pebq-current-20261005/bin \
  --output /root/pebq-current-20261005/render \
  --limit 150 --dpi 144 --iterations 3 --cpu 2 --seed 20261005
```

## Evidence

| File | SHA-256 |
|---|---|
| `environment.json` | `ea259d9f693a4dea0b97d95fdd3e6aa5587cc5bb07e8beca8e925968cc00d59d` |
| `parse/summary.json` | `df2eaf1dc3231b81b43d36e334237a4bb6e51cdbdcb17407d49a7564207e7245` |
| `parse/environment.json` | `fa9ad2eb44dd3bf4585c2f7438baa6af80057ccc32be119cc7ed4d1ebcbe6dfb` |
| `parse/corpus-manifest.json` | `c5c357d38acd2a46e271ce42998abfc414811d9741ffd72c964ce2ced59e931d` |
| `parse/parse-persistent.jsonl` | `2372dcc1196c164d82125da5d9e32b997b5c17788a01a3efc4aa412085b50bce` |
| `parse/parse-fresh.jsonl` | `acf3b5d8c5be830d2b3a17d17715b85259f5b02fab320c1bf12e5e0b67b46ed6` |
| `render/retained-summary.json` | `41d1c0deb7a2ffcb26b5611366695701f683cf35dc52c37043ce602c4d899d78` |
| `render/render-retained-resources.jsonl` | `d05d07a2c72a1446fdbd61c772e26514806e49ec1d6d4559eba86f32c157f358` |

Adapter diagnostics are retained beside the raw results. PDFium and Wellfriend
complete both profiles without diagnostic output; the other engines record
their own recoverable malformed-document and font warnings while returning
successful results.
