# PEBQ 100-PDF matched-contract qualification

## Renderer optimization result

This report qualifies renderer revision
`c3260044461cdb30a1355361ccad74f77df245d2`. The implementation contains a bounded
parsed-Type-1 program cache, copy-on-write shared clip planes, a one-byte
antialias coverage plane, precomputed axis-aligned bilinear coordinates, and a
single full-page coordinate invariant for retained render bounds and image
decode planning.

The bounds invariant prevents a concrete visual defect: images nested inside an
offset transparency-group surface can use incompatible local and full-page
coordinate spaces and become culled. The current renderer keeps
the retained bounds global and converts to surface-local coordinates only when
mapping the clipped source region.

### Current process-resident, document-cold render timing

The distribution contains one median per PDF.

| Process-resident, document-cold page-one render | Wellfriend PDF |
|---|---:|
| P50 | 114.979 ms |
| P90 | 206.626 ms |
| P95 | 271.934 ms |
| P99 | 533.881 ms |
| Maximum | 554.989 ms |

The campaign uses the fixed 100-PDF corpus manifest, 144-DPI raw-RGB contract,
CPU affinity, process-resident/document-cold native adapters, and three
repetitions per document.

### Correctness evidence

On `arxiv_cs.CR_2609.28239v1-03731b7996f9.pdf`, the current raster contains all
four image panels. Its leave-one-engine-out SSIM is `0.894591`, mean FLIP is
`0.068472`, and mean absolute channel delta is `9.054367`.

![Current four-renderer comparison with all image panels retained](visual/pages/015-arxiv-cs-cr-2609-28239v1-03731b7996f9-pdf.webp)

Aggregate quality medians remain unchanged at the reported precision because
the correction affects a small subset of the 97 dimension-qualified pages. The complete
page-level observations and all 100 comparison sheets are retained below.
Consensus agreement is diagnostic, not ground truth.

Manual inspection covers the current page and the two lowest-SSIM remaining
Wellfriend sheets (`arxiv_cs.IR_2609.28007v1-88f04e6c2113.pdf` and
`f1040sb.pdf`). They are readable and structurally complete. Their remaining
diagnostic differences are dominated by font/stroke rasterization policy and
antialiasing, not a missing nested-image panel.
The four 25-page contact sheets contain no additional grossly blank or
unrecognizable Wellfriend page-one output at contact-sheet scale.

### Qualification boundary

- All five document-open adapters return the same page count for 100/100 inputs.
- Wellfriend, MuPDF, PDFium, and Poppler each produce 100/100 rasters.
- Three Poppler pages differ by one native output pixel from the other engines
  and remain excluded rather than resized into quality scoring.
- PDFium remains faster at every reported persistent render percentile. This
  report does not claim renderer leadership, universal correctness, or Adobe
  superiority.
- The qualified source revision is
  `c3260044461cdb30a1355361ccad74f77df245d2`; it is not a packaged release.
  Source and adapter hashes are retained in `summary.json` and `environment.json`.

### Verification state

- `cargo check -p wellfriendpdf-engine --lib` passes on the VPS.
- The release `pebq_adapter` build passes and has adapter SHA-256
  `c0f428cdbe6ce8d5efe80fd2cc439cc4cec1a98bc2e6f0aa151001fe3e6172ed`.
- Six focused renderer regressions pass: Type-1 cache accounting, shared clip
  copy-on-write, bilinear sample identity, pixel-window render bounds, tile-origin
  image planning, and nested-image transparency-group rendering.
- The engine test executable reports 4,325 tests total; this qualification covers
  the six focused regressions, not the complete engine suite.
- The retained campaign contains 1,500 process-resident parse rows, 500 fresh
  parse rows, 1,200 persistent render rows, 400 fresh render rows, and 97 strict
  dimension-qualified page-quality rows.

Generated `2026-10-02T11:05:42Z` on `v72937`.
Every column executes the same declared contract. Results are disqualified when the
adapter fails or produces a page count/dimensions inconsistent with the contract.

The parsing profile is in-memory document open plus resolved page count. The
process stays resident, while every request reopens the document. These timings
exclude file reading and process startup. Fresh-process timings include adapter
startup, file reading, parsing, and JSON output. Rendering produces page one at
the declared DPI as raw RGB; PNG/WebP encoding is outside the renderer timer.

## Engine identity

| Benchmark | Wellfriend PDF | qpdf | MuPDF | PDFium | Poppler |
|---|---:|---:|---:|---:|---:|
| Version | c3260044461cdb30a1355361ccad74f77df245d2 | qpdf version 12.3.2 | mutool version 1.27.0 | PDFium 153.0.7999.0 | pdfinfo version 26.01.0 |

## Document open + resolved page count - process-resident, document-cold

The primary distribution contains one median per PDF, so each of the 100
documents has equal weight.

| Benchmark | Wellfriend PDF | qpdf | MuPDF | PDFium | Poppler |
|---|---:|---:|---:|---:|---:|
| Qualified page counts | 100/100 | 100/100 | 100/100 | 100/100 | 100/100 |
| P50 | 1.186 ms | 5.240 ms | 1.704 ms | 0.666 ms | 3.203 ms |
| P90 | 2.017 ms | 15.951 ms | 5.442 ms | 1.193 ms | 4.932 ms |
| P95 | 2.473 ms | 24.242 ms | 9.440 ms | 1.539 ms | 5.527 ms |
| P99 | 8.746 ms | 27.682 ms | 21.883 ms | 4.770 ms | 11.764 ms |
| Maximum | 35.692 ms | 29.779 ms | 34.941 ms | 37.239 ms | 13.123 ms |

### Raw document-open observations

The diagnostic distribution below contains all 300 observations per engine.
It is not the primary cross-document score.

| Benchmark | Wellfriend PDF | qpdf | MuPDF | PDFium | Poppler |
|---|---:|---:|---:|---:|---:|
| P50 | 1.230 ms | 5.484 ms | 1.759 ms | 0.697 ms | 3.211 ms |
| P90 | 2.218 ms | 19.378 ms | 6.176 ms | 1.319 ms | 5.217 ms |
| P95 | 2.689 ms | 24.847 ms | 9.468 ms | 1.653 ms | 6.249 ms |
| P99 | 11.771 ms | 31.091 ms | 34.941 ms | 9.143 ms | 12.349 ms |
| Maximum | 48.555 ms | 32.652 ms | 67.045 ms | 37.406 ms | 19.110 ms |

Nearest-rank P99 is observation 297 of 300, so three observations can exceed
it. For Wellfriend PDF, those three observations all belong to `i1040gi.pdf`:
24.415, 35.692, and 48.555 ms. Its document median is 35.692 ms. Poppler's
median on the same document is 13.123 ms. The retained rows therefore identify
a stable hard-document path rather than a system-wide timing spike.

## Document open + resolved page count — fresh process end to end

| Benchmark | Wellfriend PDF | qpdf | MuPDF | PDFium | Poppler |
|---|---:|---:|---:|---:|---:|
| P50 | 11.864 ms | 30.184 ms | 20.550 ms | 13.133 ms | 42.573 ms |
| P90 | 22.939 ms | 62.584 ms | 46.813 ms | 26.957 ms | 62.131 ms |
| P95 | 30.342 ms | 82.271 ms | 70.927 ms | 43.486 ms | 71.478 ms |
| P99 | 56.773 ms | 93.677 ms | 117.206 ms | 73.293 ms | 94.145 ms |
| Maximum | 60.892 ms | 152.110 ms | 145.312 ms | 104.658 ms | 103.376 ms |

## Document open + resolved page count — fresh-process peak RSS

| Benchmark | Wellfriend PDF | qpdf | MuPDF | PDFium | Poppler |
|---|---:|---:|---:|---:|---:|
| P50 | 7,516.000 KiB | 11,296.000 KiB | 11,172.000 KiB | 7,600.000 KiB | 17,804.000 KiB |
| P95 | 18,360.000 KiB | 22,932.000 KiB | 32,564.000 KiB | 18,092.000 KiB | 28,424.000 KiB |
| Maximum | 28,092.000 KiB | 37,924.000 KiB | 52,192.000 KiB | 28,056.000 KiB | 38,392.000 KiB |

## Document-open claim gate

- Shared qualified documents: **100**.
- Median paired Poppler/Wellfriend ratio: **2.679562×**.
- Geometric-mean paired ratio: **2.80237×**.
- Bootstrapped 95% interval for the median ratio: **[2.376752, 2.943126]**.
- Pre-registered 20× lower-bound claim: **FAIL**.

The fresh-process paired median ratio is **3.43634×**
with 95% interval **[3.1816, 3.939697]**.

## Rendering - process-resident, document-cold raw-RGB page one

The primary distribution contains one median per PDF. Every request performs a
new render, and this profile contains no final-raster cache reuse.

| Benchmark | Wellfriend PDF | MuPDF | PDFium | Poppler |
|---|---:|---:|---:|---:|
| Successful renders | 100/100 | 100/100 | 100/100 | 100/100 |
| P50 | 114.979 ms | 48.451 ms | 41.370 ms | 47.973 ms |
| P90 | 206.626 ms | 88.260 ms | 66.382 ms | 83.988 ms |
| P95 | 271.934 ms | 107.667 ms | 100.459 ms | 126.741 ms |
| P99 | 533.881 ms | 136.061 ms | 203.666 ms | 215.028 ms |
| Maximum | 554.989 ms | 242.380 ms | 304.614 ms | 247.404 ms |

### Raw renderer observations

| Benchmark | Wellfriend PDF | MuPDF | PDFium | Poppler |
|---|---:|---:|---:|---:|
| P50 | 115.739 ms | 48.451 ms | 41.458 ms | 49.017 ms |
| P90 | 214.608 ms | 91.522 ms | 73.247 ms | 88.513 ms |
| P95 | 288.682 ms | 109.718 ms | 108.857 ms | 135.783 ms |
| P99 | 545.612 ms | 156.913 ms | 271.614 ms | 219.783 ms |
| Maximum | 603.857 ms | 269.918 ms | 350.352 ms | 283.435 ms |

All 400 engine-document pairs publish stable dimensions and raster hashes across
their three repetitions. This determinism check detects repeat-dependent output;
it does not prove that a binary corresponds to source without a reproducible
build attestation.

## Rendering — fresh process end to end

| Benchmark | Wellfriend PDF | MuPDF | PDFium | Poppler |
|---|---:|---:|---:|---:|
| P50 | 208.930 ms | 102.230 ms | 89.238 ms | 137.412 ms |
| P90 | 356.413 ms | 152.173 ms | 137.889 ms | 186.059 ms |
| P95 | 377.433 ms | 198.872 ms | 191.385 ms | 217.495 ms |
| P99 | 817.058 ms | 246.819 ms | 315.342 ms | 365.251 ms |
| Maximum | 839.589 ms | 391.249 ms | 424.959 ms | 394.465 ms |

## Rendering — fresh-process peak RSS

| Benchmark | Wellfriend PDF | MuPDF | PDFium | Poppler |
|---|---:|---:|---:|---:|
| P50 | 30,548.000 KiB | 25,952.000 KiB | 23,196.000 KiB | 35,544.000 KiB |
| P95 | 44,636.000 KiB | 46,832.000 KiB | 38,464.000 KiB | 45,608.000 KiB |
| Maximum | 70,028.000 KiB | 78,196.000 KiB | 83,252.000 KiB | 58,976.000 KiB |

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

- All five native document-open adapters return the same page count on all 100 PDFs.
- The matched process-resident, document-cold open/page-count evidence rejects the 20× Poppler claim.
- All four raster engines render all 100 pages; 97 have identical native dimensions.
- Wellfriend PDF is not the fastest renderer in this campaign; reference leadership varies by percentile.
- Consensus quality is diagnostic rather than ground truth and does not establish universal correctness.

## Reproducibility

- Corpus SHA-256 manifest: `61c378353c0f125e46348740089d00a84a70a9571994d7e6da50aea6d95f40f7`.
- DPI: `144`.
- Process-resident document-cold repetitions: `3`.
- Fresh-process repetitions: `1`.
- CPU affinity: `2`.
- Retained raw observations: `3697`.
- Raw JSONL observations, adapter sources, binary hashes and the complete JSON summary
  are retained with this report.
