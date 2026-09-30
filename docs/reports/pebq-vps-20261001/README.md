# PEBQ 100-PDF matched-contract qualification

Generated `2026-09-30T19:05:57Z` on `v72937`.
Every column executes the same declared contract. Results are disqualified when the
adapter fails or produces a page count/dimensions inconsistent with the contract.

The parsing profile is in-memory document open plus resolved page count. Persistent
timings exclude file reading and process startup; fresh-process timings include adapter
startup, file reading, parsing and JSON output. Rendering is page one at the declared DPI
to raw RGB; PNG/WebP encoding is not part of the renderer timer.

## Engine identity

| Benchmark | Wellfriend PDF | qpdf | MuPDF | PDFium | Poppler |
|---|---:|---:|---:|---:|---:|
| Version | 66f6b9c8c29d0e62025c9496c3b2f65dd9ce1045 | qpdf version 12.3.2 | mutool version 1.27.0 | PDFium 153.0.7999.0 | pdfinfo version 26.01.0 |

## Parsing — persistent native adapters

| Benchmark | Wellfriend PDF | qpdf | MuPDF | PDFium | Poppler |
|---|---:|---:|---:|---:|---:|
| Qualified page counts | 100/100 | 100/100 | 100/100 | 100/100 | 100/100 |
| P50 | 1.471 ms | 6.772 ms | 1.924 ms | 0.801 ms | 3.920 ms |
| P90 | 2.667 ms | 22.297 ms | 6.129 ms | 1.436 ms | 5.909 ms |
| P95 | 3.329 ms | 29.829 ms | 10.071 ms | 1.917 ms | 6.462 ms |
| P99 | 13.667 ms | 38.927 ms | 28.282 ms | 7.709 ms | 16.208 ms |
| Maximum | 49.275 ms | 49.733 ms | 90.889 ms | 49.147 ms | 25.847 ms |

## Parsing — fresh process end to end

| Benchmark | Wellfriend PDF | qpdf | MuPDF | PDFium | Poppler |
|---|---:|---:|---:|---:|---:|
| P50 | 13.046 ms | 31.631 ms | 22.775 ms | 14.214 ms | 46.148 ms |
| P90 | 27.809 ms | 64.538 ms | 50.218 ms | 30.056 ms | 64.311 ms |
| P95 | 34.694 ms | 79.602 ms | 71.876 ms | 41.058 ms | 72.978 ms |
| P99 | 64.228 ms | 110.839 ms | 109.914 ms | 70.942 ms | 95.507 ms |
| Maximum | 97.888 ms | 160.666 ms | 203.424 ms | 100.745 ms | 126.288 ms |

## Parsing — fresh-process peak RSS

| Benchmark | Wellfriend PDF | qpdf | MuPDF | PDFium | Poppler |
|---|---:|---:|---:|---:|---:|
| P50 | 7,316.000 KiB | 11,632.000 KiB | 11,432.000 KiB | 7,772.000 KiB | 18,284.000 KiB |
| P95 | 18,032.000 KiB | 23,288.000 KiB | 32,928.000 KiB | 18,300.000 KiB | 29,048.000 KiB |
| Maximum | 27,792.000 KiB | 38,212.000 KiB | 52,516.000 KiB | 28,260.000 KiB | 39,040.000 KiB |

## Parsing claim gate

- Shared qualified documents: **100**.
- Median paired Poppler/Wellfriend ratio: **2.639249×**.
- Geometric-mean paired ratio: **2.818896×**.
- Bootstrapped 95% interval for the median ratio: **[2.496132, 2.901311]**.
- Pre-registered 20× lower-bound claim: **FAIL**.

The fresh-process paired median ratio is **3.463471×**
with 95% interval **[3.269868, 3.662601]**.

## Rendering — persistent native raw-RGB page one

| Benchmark | Wellfriend PDF | MuPDF | PDFium | Poppler |
|---|---:|---:|---:|---:|
| Successful renders | 100/100 | 100/100 | 100/100 | 100/100 |
| P50 | 180.780 ms | 52.773 ms | 47.960 ms | 57.304 ms |
| P90 | 372.123 ms | 98.820 ms | 78.807 ms | 96.109 ms |
| P95 | 563.278 ms | 123.405 ms | 130.254 ms | 146.815 ms |
| P99 | 1,005.455 ms | 182.717 ms | 274.303 ms | 247.313 ms |
| Maximum | 1,095.182 ms | 314.309 ms | 435.279 ms | 298.590 ms |

## Rendering — fresh process end to end

| Benchmark | Wellfriend PDF | MuPDF | PDFium | Poppler |
|---|---:|---:|---:|---:|
| P50 | 278.780 ms | 113.775 ms | 97.190 ms | 146.901 ms |
| P90 | 526.965 ms | 172.594 ms | 141.576 ms | 200.437 ms |
| P95 | 662.000 ms | 210.200 ms | 232.862 ms | 240.241 ms |
| P99 | 1,244.165 ms | 295.490 ms | 387.669 ms | 385.911 ms |
| Maximum | 1,356.576 ms | 512.447 ms | 543.845 ms | 429.755 ms |

## Rendering — fresh-process peak RSS

| Benchmark | Wellfriend PDF | MuPDF | PDFium | Poppler |
|---|---:|---:|---:|---:|
| P50 | 30,660.000 KiB | 26,072.000 KiB | 23,220.000 KiB | 36,052.000 KiB |
| P95 | 45,464.000 KiB | 47,096.000 KiB | 38,520.000 KiB | 46,368.000 KiB |
| Maximum | 67,692.000 KiB | 78,392.000 KiB | 83,320.000 KiB | 59,824.000 KiB |

## Rendering quality — leave-one-engine-out consensus

| Benchmark | Wellfriend PDF | MuPDF | PDFium | Poppler |
|---|---:|---:|---:|---:|
| Dimension-qualified pages | 97/100 | 97/100 | 97/100 | 97/100 |
| SSIM P50 ↑ | 0.946905 | 0.964567 | 0.962659 | 0.854784 |
| SSIM P05 ↑ | 0.903857 | 0.936260 | 0.924754 | 0.787523 |
| Changed pixels >8 P50 ↓ | 8.418592% | 7.726159% | 9.243373% | 9.547941% |
| Mean ΔE2000 P50 ↓ | 1.530997 | 1.257197 | 1.370901 | 3.435389 |
| Mean FLIP P50 ↓ | 0.038033 | 0.030910 | 0.043193 | 0.063104 |
| RMSE P50 ↓ | 22.148278 | 18.154708 | 18.390884 | 42.584258 |

Consensus is a symmetric differential diagnostic, not an ISO visual oracle. A
renderer can agree with the other engines and still be wrong. Dimension mismatches
are failures and are never resized away.

### Strict dimension failures

- `arxiv_astro-ph.IM_2609.27676v1-b77c4db27da8.pdf` — MuPDF 1191×1588, PDFium 1191×1588, Poppler 1191×1587, Wellfriend PDF 1191×1588.
- `arxiv_cs.DL_2609.25327v1-e178a2e93b05.pdf` — MuPDF 1191×1684, PDFium 1191×1684, Poppler 1190×1684, Wellfriend PDF 1191×1684.
- `arxiv_physics.comp-ph_2609.28422v1-1f065a5ff5ec.pdf` — MuPDF 1191×1588, PDFium 1191×1588, Poppler 1191×1587, Wellfriend PDF 1191×1588.

### Visual evidence

The repository retains one native-output comparison sheet for every PDF. The top
row contains the four unscaled renderer outputs; the bottom row contains amplified
absolute-difference maps against the leave-one-engine-out consensus.

- [Pages 1–25 contact sheet](visual/contacts/contact-001-025.webp)
- [Pages 26–50 contact sheet](visual/contacts/contact-026-050.webp)
- [Pages 51–75 contact sheet](visual/contacts/contact-051-075.webp)
- [Pages 76–100 contact sheet](visual/contacts/contact-076-100.webp)
- [All 100 full comparison sheets](visual/pages/)

## Verdict

- All five native parser adapters returned the same page count on all 100 PDFs.
- The matched persistent parser evidence rejects the 20× Poppler claim.
- All four raster engines rendered all 100 pages, but only 97 had identical native dimensions.
- Wellfriend PDF is not the fastest renderer in this campaign; PDFium has the lowest median and tail latency.
- Consensus quality is diagnostic rather than ground truth and does not establish universal correctness.

## Reproducibility

- Corpus SHA-256 manifest: `61c378353c0f125e46348740089d00a84a70a9571994d7e6da50aea6d95f40f7`.
- DPI: `144`.
- Persistent repetitions: `10`.
- Fresh-process repetitions: `5`.
- CPU affinity: `2`.
- Retained raw observations: `13597`.
- Raw JSONL observations, adapter sources, binary hashes and the complete JSON summary
  are retained with this report.
