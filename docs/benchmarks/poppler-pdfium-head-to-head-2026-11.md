# Renderer head-to-head: Wellfriend vs Poppler vs PDFium vs MuPDF (2026-11)

Single-threaded, all-pages, 150 DPI PNG rendering of 126 PDFs (24,011 pages), run on a shared VPS on
2026-11-10/11. Every tool rendered every page of every document. Each tool process was single-threaded and
pinned to one vCPU. Raw results stay on the VPS under `/srv/wf-bench/results/`; nothing here is committed
evidence beyond this summary.

## Headline

| Tool | Pages rendered (of 24,011) | Total wall, sum of per-doc walls (s) | Aggregate pages/s | Median ms/page, 33 books | Median ms/page, 93 small docs | Peak RSS max / median (MB) | Failed docs |
|---|---:|---:|---:|---:|---:|---:|---:|
| Wellfriend | 24,011 | 12,461 | 1.93 | 414 | 20.5 | 5,263 / 37 | 2 |
| Poppler (pdftoppm) | 24,010 | 45,296 | 0.53 | 1,357 | 74.5 | 434 / 30 | 3 |
| PDFium (pypdfium2) | 24,011 | 11,877 | 2.02 | 389 | 97.0 | 1,718 / 40 | 2 |
| MuPDF (mutool draw) | 24,010 | 12,426 | 1.93 | 451 | 24.9 | 1,244 / 23 | 3 |

- **Throughput.** Wellfriend, PDFium and MuPDF all land at about 1.9-2.0 pages/s on this scan-heavy corpus.
  Their differences (under 5% in total wall) are much smaller than the run-to-run noise measured on this
  machine (see Caveats). Poppler is about 3.6x slower, because of JPX decoding on the archive.org books.
- **Small documents.** On the 93 fixture and corpus files, Wellfriend has the lowest per-document cost.
  PDFium's figure includes about 90 ms of Python interpreter and pypdfium2 start-up per document.
- **Memory.** Wellfriend has the highest peak RSS on large books (up to 5.3 GB) because `render -o x.zip`
  keeps the whole ZIP of PNGs in memory. RSS tracks output size: America00Ogil_ peaks at 4.0 GB for a
  3.5 GB ZIP. The comparators write one PNG at a time and stay under 1.8 GB.
- **Robustness.** Wellfriend failed only the two documents that every tool failed (both need a password or
  are unparseable). Poppler aborted on `bug_jpx.pdf` (OpenJPEG assertion). MuPDF could not open
  `encrypted-attachment.pdf`.
- **Fidelity (SSIM on 157 sample pages).**
  - Wellfriend vs Poppler: median 0.974, the same as the PDFium vs Poppler baseline (0.974).
  - Wellfriend vs MuPDF: median 0.987, the closest pair of all.
  - The low-SSIM tail comes from downsampled halftone scans, where every pair of tools disagrees. A visual
    check found no gross rendering errors in Wellfriend; details are in the SSIM section.

## Tools and versions

| Tool | Version / build | Invocation |
|---|---|---|
| Wellfriend | `wellfriendpdf 0.1.0` (engine 0.1.0, no optional features), final branch binary `/srv/wf-cmyk-test/wf-new`, sha256 `e51a0b2c231467af3b18bf82246152c59c2e5fecd89cb23da06f909cbc029a9a`, copied to `/srv/wf-bench/bin/wellfriendpdf-new`. Source tree `/srv/wf-cmyk` at `2ed4d908` ("Merge parity/r4-decode-render") plus the branch's later changes | `wellfriendpdf --threads 1 render IN.pdf -o OUT.zip --format png --dpi 150 --pages all` (default `--render-quality compat`, crop box, opaque white background) |
| Poppler | `pdftoppm` 26.07.0 (Ubuntu `poppler-utils 26.07.0-2ubuntu1.1`), libopenjp2 2.5.4, libpng 1.6.59, lcms2 | `pdftoppm -r 150 -png IN.pdf OUTPREFIX` |
| PDFium | PDFium 159.0.8149.0 via pypdfium2 5.15.2, Python 3.14.8, Pillow 13.0.0, numpy 2.5.4 | `/srv/wf-bench/pdfium_render.py`: `PdfDocument`, `page.render(scale=150/72).to_pil()`, `Image.save(png)` with Pillow defaults (zlib level 6, no optimize), self-timing each page |
| MuPDF | `mutool` 1.27.0 (Ubuntu `mupdf-tools 1.27.0+ds1-3ubuntu3`, installed for this run). This build prints `warning: ICC support is not available` | `mutool draw -q -r 150 -o OUT/p-%05d.png IN.pdf` |

All four tools write 8-bit RGB PNGs (checked on a greyscale scan page). Average PNG size per page over the
whole run was 2.00 MB for Wellfriend, 1.17 MB for Poppler, 1.34 MB for PDFium and 1.33 MB for MuPDF.
Wellfriend's encoder compresses less.

An earlier Wellfriend binary, `/root/wf-cmyk-target/release/wellfriendpdf` (sha256 `b846b7ca…`), was used
for the first hour. It is not part of the published column. Its partial rows (fixtures, corpus and four
books) are kept in `results/wellfriend-base-b846-partial/`. Its SSIM results are identical to the final
binary's on all 157 sample pages.

## Machine

| Item | Value |
|---|---|
| Host | shared VPS `136.65.4.82` (GCP), Ubuntu 26.10, kernel 7.3.0-1004-gcp |
| CPU | AMD EPYC 9B45, 8 vCPU = 4 physical cores x 2 SMT threads (siblings 0/4, 1/5, 2/6, 3/7), 32 MiB L3 |
| RAM / disk | 30 GB RAM, no swap; root filesystem shared with other agents (12-31 GB free during the run) |
| Other load | Other agents' cargo builds and tests were running during part of the window. Load average reached about 13 at 21:30-22:15 UTC |

## Corpus

| Set | Path | Files | Pages | Size |
|---|---|---:|---:|---:|
| fixtures | `/srv/wf-cmyk/crates/engine/tests/fixtures/**/*.pdf` | 18 | 33 | 1.5 MB |
| corpus | `/srv/wf-cmyk/tests/corpus/pdfs/**/*.pdf` (pdfjs, existing, generated) | 75 | 596 | 8.8 MB |
| realpdf | `/srv/realpdf/*.pdf` | 21 | 16,912 | 6.3 GB |
| realpdf200 | `/srv/realpdf200/*.pdf` | 12 | 6,470 | 2.0 GB |
| **Total** | | **126** | **24,011** | **8.4 GB** |

- **Page counts.** Counts come from `pdfinfo`. Two corpus files have no countable pages because no tool can
  open them: `issue15893_reduced.pdf` and `print_protection.pdf`.
- **Duplicates.** Five fixtures are byte-identical (same MD5) to files in `corpus/existing`: basicapi,
  form_160f, image_only, multi_stream and tracemonkey. They were rendered twice, as listed.
- **Excluded.** `/srv/realpdf200/r6/` was left out. Its 10 files are byte-identical copies of the top-level
  realpdf200 books, plus a trailing `% wellfriend loadtest r200-r6` comment.
- **Book content.** The 33 books are mostly archive.org scans in the mixed raster content (MRC) layout:
  per page, two JPX layers of about 28 MP (greyscale or RGB) plus a JBIG2 mask. A few use JPEG (EA1989,
  dli.language.*), and a few are born-digital atlases or catalogues.
- **Skew.** Books are 97% of all pages, so the totals measure large scanned-image decoding and
  downsampling, not vector or text rendering.

## Method

**Per document and tool:**
- Command line: `taskset -c CORE /usr/bin/time -v -o time.txt timeout -k 30 14400 <tool command>`.
- Wall time: Python `perf_counter` around the whole process, so it includes process start-up, document
  open and PNG encoding of every page.
- Peak RSS: the "Maximum resident set size" reported by GNU time.
- Pages rendered: PNG files written, or PNG entries in Wellfriend's ZIP.
- Clean-up: the output was deleted right after counting.
- A document counts as failed on a nonzero exit, on a timeout, or when fewer pages were rendered than
  `pdfinfo` reports.
- No timeouts occurred.

**Threads and pinning:**
- Every render process was single-threaded (`--threads 1` for Wellfriend; the comparators are
  single-threaded) and pinned to one vCPU.
- Each worker rendered one document at a time.
- **Phase 1** (about 21:00-22:16 UTC): one worker per tool, on vCPUs 5 (Wellfriend), 6 (Poppler), 7 (PDFium)
  and 4 (MuPDF). vCPUs 0-3 belonged to another agent's builds.
- **Phase 2** (22:16-01:05 UTC): vCPUs 0-3 were freed. A supervisor (`supervisor.py`) started extra
  workers on idle vCPUs for the tool with the most unrendered pages: Wellfriend capped at 2 workers
  (vCPUs 0 and 1), then up to 7 Poppler workers.
  - Workers of the same tool share a claim directory, so each document was rendered exactly once.
  - All final Wellfriend rows (binary `wf-new`) come from this phase.
  - PDFium ran entirely on vCPU 7.
  - MuPDF ran on vCPU 4, plus 7 docs on vCPU 7.
- **Phase 3** (01:25-02:18 UTC): two Poppler documents were rerun. They had died at 01:02 when the shared
  disk filled up (see Notes on failures).

**Metric definitions:**
- **Total wall** is the sum of per-document wall times. It is not the elapsed time of the campaign, which
  was about 5.3 h with up to 8 workers.
- **Aggregate pages/s** = pages rendered / total wall.
- **Median ms/page** = median over documents of (document wall / pages). This is shown separately for
  books and small documents, because one-page files measure process start-up.
- **Median true per-page ms** (whole run, all pages pooled):

  | Tool | Median true per-page ms | Source |
  |---|---:|---|
  | PDFium | 337 | self-timed render + PNG save |
  | MuPDF | 441 | interval between successive output-file mtimes |
  | Poppler | 1,571 | interval between successive output-file mtimes |
  | Wellfriend | n/a | the CLI writes one ZIP at the end, so per-page times cannot be observed |

**SSIM:**
- Sample: the middle page of each of the 93 fixture and corpus files, and the first and middle page of
  each of the 33 books. That makes 159 pages, of which 157 have a known page count.
- Each tool rendered each page alone at 150 DPI (Wellfriend `--pages N`, `pdftoppm -f N -l N -singlefile`,
  `mutool draw … N`, PDFium single page).
- Images were converted to luminance with Pillow `L` (ITU-R 601), after compositing any alpha onto white.
- SSIM: Gaussian window 11x11 with sigma 1.5, K1 = 0.01, K2 = 0.03, mean over the valid region (Wang et
  al. 2004; numpy implementation in `ssim_sample.py`).
- When sizes differed, both images were cropped to the common top-left area. Pairs that differ by more
  than 2 px (a different page box or rotation) are excluded from the statistics and listed below.
- The sample was rendered twice, once with each Wellfriend binary. The results are identical.

## Results: throughput and memory

All documents (126):

| Tool | Docs | Pages expected | Pages rendered | Median ms/page, all docs (doc wall / pages) | Median true per-page ms | Total wall (s) | Aggregate pages/s | Peak RSS max (MB) | Peak RSS median (MB) | PNG MB/page | Failed docs |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Wellfriend | 126 | 24,011 | 24,011 | 31.4 | n/a | 12,461 | 1.93 | 5,263 | 37 | 2.00 | 2 |
| Poppler | 126 | 24,011 | 24,010 | 100.0 | 1,571 (mtime) | 45,296 | 0.53 | 434 | 30 | 1.17 | 3 |
| PDFium | 126 | 24,011 | 24,011 | 107.0 | 337 (self-timed) | 11,877 | 2.02 | 1,718 | 40 | 1.34 | 2 |
| MuPDF | 126 | 24,011 | 24,010 | 31.0 | 441 (mtime) | 12,426 | 1.93 | 1,244 | 23 | 1.33 | 3 |

Common-success subset: the 122 documents that every tool rendered completely. This removes the two
documents that every tool failed, plus `bug_jpx.pdf` and `encrypted-attachment.pdf`.

| Tool | Pages | Total wall (s) | Aggregate pages/s | Median ms/page |
|---|---:|---:|---:|---:|
| Wellfriend | 24,009 | 12,461 | 1.93 | 32.3 |
| Poppler | 24,009 | 45,296 | 0.53 | 101.5 |
| PDFium | 24,009 | 11,876 | 2.02 | 105.7 |
| MuPDF | 24,009 | 12,426 | 1.93 | 31.0 |

Per corpus set, common-success documents only. Each cell is pages/s, with total wall in seconds in brackets.

| Set | Docs | Pages | Wellfriend | Poppler | PDFium | MuPDF |
|---|---:|---:|---:|---:|---:|---:|
| fixtures | 18 | 33 | 31.91 (1) | 4.95 (7) | 12.99 (3) | 26.87 (1) |
| corpus | 71 | 594 | 41.07 (14) | 5.80 (102) | 24.08 (25) | 40.50 (15) |
| realpdf | 21 | 16,912 | 2.01 (8,431) | 0.47 (35,813) | 2.08 (8,147) | 1.79 (9,424) |
| realpdf200 | 12 | 6,470 | 1.61 (4,015) | 0.69 (9,373) | 1.75 (3,702) | 2.17 (2,986) |

Per book: wall ms/page and peak RSS in MB.

| Book | Pages | WF ms/p | Poppler ms/p | PDFium ms/p | MuPDF ms/p | WF RSS | Poppler RSS | PDFium RSS | MuPDF RSS |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| realpdf200/1923ATFSpecimenBook | 1174 | 380 | 1082 | 302 | 203 | 4068 | 84 | 295 | 435 |
| realpdf200/2015.227013.Linguistic-Survey | 474 | 246 | 618 | 301 | 243 | 834 | 42 | 218 | 325 |
| realpdf200/2015.263319.New-English | 199 | 308 | 875 | 389 | 278 | 689 | 42 | 221 | 304 |
| realpdf200/LinguisticSurveyOfIndiaVolVPartI… | 455 | 650 | 1539 | 468 | 791 | 1085 | 226 | 249 | 489 |
| realpdf200/Vol2BritishEmpireInAsia | 114 | 191 | 574 | 354 | 304 | 656 | 33 | 212 | 305 |
| realpdf200/Vol2China | 146 | 191 | 667 | 443 | 248 | 743 | 33 | 243 | 322 |
| realpdf200/Vol4Italy | 144 | 192 | 610 | 356 | 303 | 755 | 33 | 244 | 315 |
| realpdf200/Vol5Netherlands | 132 | 185 | 680 | 370 | 329 | 715 | 34 | 235 | 320 |
| realpdf200/atlas_animals | 303 | 1578 | 6890 | 5155 | 495 | 5263 | 128 | 399 | 453 |
| realpdf200/catalogofcopyrig361lib | 1542 | 634 | 1861 | 432 | 891 | 2378 | 194 | 300 | 540 |
| realpdf200/dictionaryhindus030502mbp | 1143 | 694 | 669 | 217 | 214 | 1705 | 38 | 206 | 289 |
| realpdf200/libraryofcongres013803mbp | 644 | 1154 | 1357 | 362 | 452 | 1331 | 40 | 208 | 297 |
| realpdf/America00Ogil_ | 796 | 973 | 4092 | 650 | 1269 | 4035 | 434 | 392 | 1244 |
| realpdf/Bolt_Action_Rifles_3rd_Edition | 532 | 267 | 1266 | 541 | 416 | 1878 | 39 | 580 | 378 |
| realpdf/CompleteAtlasOfTheWorld3rdEdition2016 | 434 | 1117 | 1457 | 653 | 352 | 2362 | 58 | 838 | 358 |
| realpdf/EA1989 | 270 | 284 | 1171 | 832 | 329 | 1364 | 40 | 364 | 329 |
| realpdf/Newsletter_1993_V67N04_ | 140 | 931 | 2229 | 1145 | 696 | 1075 | 166 | 336 | 537 |
| realpdf/Summer_1965_Foreign_Amateur_Callbook | 304 | 402 | 1268 | 901 | 550 | 2961 | 51 | 603 | 440 |
| realpdf/UkrMovEnts | 833 | 33 | 693 | 112 | 88 | 1559 | 32 | 383 | 347 |
| realpdf/anatomyofhumanbo1918gray_ | 1402 | 483 | 2321 | 552 | 460 | 3442 | 145 | 373 | 519 |
| realpdf/cassellscarpentr00hasl_ | 612 | 414 | 2152 | 363 | 1207 | 1109 | 183 | 177 | 510 |
| realpdf/cassellscarpentr00hasl_bw_ | 612 | 329 | 1016 | 318 | 679 | 1013 | 85 | 125 | 372 |
| realpdf/con00ciseoxforddicfowlrich_ | 1084 | 309 | 2103 | 337 | 516 | 2675 | 137 | 203 | 565 |
| realpdf/dli.language.1095 | 1224 | 40 | 617 | 228 | 92 | 2638 | 37 | 419 | 324 |
| realpdf/dli.language.1096 | 1361 | 168 | 660 | 281 | 295 | 2691 | 47 | 1718 | 335 |
| realpdf/englishmalayalam00tobirich_ | 1414 | 662 | 3822 | 1219 | 886 | 3961 | 178 | 401 | 601 |
| realpdf/in.ernet.dli.2015.274158 | 774 | 138 | 1306 | 203 | 187 | 1510 | 39 | 506 | 323 |
| realpdf/indexofarmymotio00unitrich_ | 542 | 730 | 1982 | 458 | 1285 | 1228 | 178 | 164 | 507 |
| realpdf/jacaranda-atlas | 274 | 988 | 1426 | 835 | 451 | 1945 | 202 | 778 | 624 |
| realpdf/registermotorv1919cali_ | 1052 | 783 | 1798 | 241 | 323 | 1852 | 108 | 179 | 612 |
| realpdf/sinhaleseenglish00clourich_ | 1676 | 574 | 3305 | 504 | 619 | 2584 | 162 | 210 | 472 |
| realpdf/studentstaffdire9394univ_ | 672 | 1236 | 4035 | 528 | 1024 | 2602 | 188 | 228 | 545 |
| realpdf/zuluenglishdicti00brya_ | 904 | 668 | 2779 | 310 | 506 | 1824 | 141 | 192 | 457 |

Patterns in the per-book table:

- **Wellfriend is fastest, by 1.3-2.7x over the next tool,** on:
  - the realpdf200 `Vol*` atlases;
  - UkrMovEnts (33 ms/page, against 88 for MuPDF);
  - dli.language.1095;
  - in.ernet.dli.
- **Wellfriend is 2.3-3.2x slower than the best comparator** on:
  - atlas_animals (1,578 ms/page against MuPDF's 495);
  - CompleteAtlas (1,117 against 352);
  - dictionaryhindus, libraryofcongres and registermotorv (783-1,154 ms/page against 214-362);
  - studentstaffdire.

  These are the best leads for optimisation work.
- **Wellfriend's RSS rises with page count.** The cause is the in-memory ZIP. It is not decode state.

## Results: SSIM (luminance, 150 DPI)

157 sample pages. The statistics exclude pages that could not be rendered or whose sizes differ by more
than 2 px.

| Pair | Pages compared | Mean | Median | Min | Pages < 0.90 | Pages < 0.95 | Size mismatches (>0 px / >2 px) |
|---|---:|---:|---:|---:|---:|---:|---:|
| Wellfriend vs Poppler | 151 | 0.9215 | 0.9739 | 0.4890 | 47 | 59 | 57 / 5 |
| Wellfriend vs PDFium | 157 | 0.9282 | 0.9561 | 0.5262 | 41 | 75 | 17 / 0 |
| Wellfriend vs MuPDF | 156 | 0.9563 | 0.9867 | 0.0011 | 16 | 34 | 52 / 0 |
| PDFium vs Poppler (reference) | 151 | 0.9316 | 0.9744 | 0.5204 | 35 | 57 | 59 / 5 |
| MuPDF vs Poppler (reference) | 150 | 0.9273 | 0.9750 | 0.0001 | 37 | 59 | 10 / 5 |

Worst pages:

| Wellfriend vs Poppler | SSIM | Wellfriend vs PDFium | SSIM | Wellfriend vs MuPDF | SSIM |
|---|---:|---|---:|---|---:|
| realpdf200/Vol2China p73 | 0.4890 | realpdf200/1923ATFSpecimenBook p587 | 0.5262 | corpus/pdfjs/xobject-image p1 | 0.0011 |
| realpdf200/Vol4Italy p72 | 0.5230 | realpdf/zuluenglishdicti00brya_ p1 | 0.5346 | realpdf/jacaranda-atlas p1 | 0.2631 |
| corpus/pdfjs/xobject-image p1 | 0.5465 | realpdf200/Vol2China p73 | 0.5361 | realpdf/zuluenglishdicti00brya_ p1 | 0.7069 |
| realpdf200/Vol2BritishEmpireInAsia p57 | 0.5575 | realpdf/studentstaffdire9394univ_ p1 | 0.6739 | realpdf/CompleteAtlasOfTheWorld… p217 | 0.7451 |
| realpdf200/Vol5Netherlands p66 | 0.5900 | realpdf200/Vol4Italy p72 | 0.6796 | realpdf/con00ciseoxforddicfowlrich_ p1 | 0.7649 |
| realpdf/zuluenglishdicti00brya_ p1 | 0.6002 | realpdf200/Vol5Netherlands p66 | 0.6953 | corpus/pdfjs/issue5801 p1 | 0.8327 |
| realpdf/con00ciseoxforddicfowlrich_ p1 | 0.6102 | realpdf200/catalogofcopyrig361lib p1 | 0.7236 | realpdf200/atlas_animals p152 | 0.8395 |
| realpdf200/Vol2BritishEmpireInAsia p1 | 0.6478 | realpdf/Summer_1965_Foreign_Amateur_Callbook p152 | 0.7260 | corpus/pdfjs/XiaoBiaoSong p1 | 0.8401 |
| realpdf/UkrMovEnts p1 | 0.6565 | realpdf200/Vol2BritishEmpireInAsia p57 | 0.7346 | realpdf200/Vol4Italy p72 | 0.8451 |
| realpdf200/Vol5Netherlands p1 | 0.6572 | realpdf/con00ciseoxforddicfowlrich_ p1 | 0.7445 | realpdf/con00ciseoxforddicfowlrich_ p542 | 0.8628 |

### How to read these SSIM numbers

- **Low SSIM is mostly resampling, not rendering errors.** Most low values are scanned pages
  (half-toned photographs, colour scans) that each tool downsamples from about 400-500 DPI to 150 DPI with
  a different filter.
  - Example: Vol2China p73 scores 0.49 for Wellfriend vs Poppler, 0.52 for PDFium vs Poppler and 0.95 for
    Wellfriend vs MuPDF.
  - A side-by-side visual check of that page showed all four renders are equivalent to the eye; only the
    halftone texture differs.
  - Wellfriend's image scaling is closest to MuPDF's, which explains the high Wellfriend vs MuPDF median.
- **`pdfjs/xobject-image` p1 shows a real difference.** Wellfriend and PDFium draw a red image. Poppler
  draws it white and MuPDF draws it black. Wellfriend agrees with PDFium here.
- **`jacaranda-atlas` p1 (CMYK cover):** the dark background converts differently in every tool:
  - Wellfriend: brown-black;
  - Poppler: navy;
  - PDFium: dark teal;
  - MuPDF: pure black. MuPDF's build has no ICC support.
- **1923ATFSpecimenBook p587:** PDFium's image downsampling is visibly softer than the other three, hence
  0.53 against PDFium but 0.94 against Poppler.
- **Pages excluded for size mismatch above 2 px.** Poppler picks a different page size for these; all
  other pairs agree with each other:
  - `pdfjs/bug1802506` p1: Poppler 1275x1650, the others 558x304;
  - `pdfjs/freeculture` p176: Poppler 1000x1413, the others 826x1239;
  - `jacaranda-atlas` p137: 118 px apart;
  - `Summer_1965…` p1 and p152: 3-4 px apart.
- **1 px size differences are common.** Poppler and MuPDF round page sizes differently from Wellfriend and
  PDFium. Those pages were cropped to the common area and kept.

## Notes on failures

| Document | Wellfriend | Poppler | PDFium | MuPDF |
|---|---|---|---|---|
| corpus/pdfjs/print_protection.pdf | fail: "PDF is password-protected" | fail: incorrect password | fail: incorrect password | fail: cannot authenticate password |
| corpus/pdfjs/issue15893_reduced.pdf | fail: "indirect object header is missing obj keyword" | fail: incorrect password | fail: incorrect password | fail: cannot authenticate password |
| corpus/pdfjs/bug_jpx.pdf | ok | crash (SIGABRT, OpenJPEG `opj_int_ceildiv` assertion) | ok | ok |
| corpus/pdfjs/encrypted-attachment.pdf | ok | ok | ok | fail: cannot authenticate password |

Wellfriend has no failure that another tool did not share. Its message for `issue15893_reduced.pdf` names
a parse error, while the others report an encryption or password problem.

**Infrastructure incident (not a tool failure).** At 01:02 UTC the shared root filesystem filled up
because of other agents' files. Two in-flight Poppler renders died with ENOSPC:
- America00Ogil_, at page 780 of 796;
- atlas_animals, at page 265 of 303.

Their partial timing lines were removed and both were rerun from scratch in phase 3. Every other result
row was checked: all have status ok and the full page count. Any other render in flight at that moment
completed, though its I/O may have been slowed.

## Caveats

- **Shared, noisy machine.** The VPS was shared with other agents' builds, and 8 vCPUs are only 4
  physical cores with SMT.
  - Every worker shared a physical core with another busy thread: a build in phase 1, another benchmark
    worker in phase 2.
  - Pinning keeps work off each other's vCPU, not off each other's core, L3 cache or memory bandwidth.
  - Same-work noise is large. The old and final Wellfriend binaries produce identical pixels, yet their
    per-book walls differed by -30% to +50% in different phases: America00Ogil_ took 517 s against 775 s,
    and catalogofcopyrig361lib 1,331 s against 978 s.
  - Treat differences under about 1.5x between Wellfriend, PDFium and MuPDF as noise. The 3.6x Poppler
    gap is real.
- **The tools did not run under identical conditions.**
  - Wellfriend ran entirely in phase 2, with 8 busy vCPUs.
  - PDFium and MuPDF ran mostly in phases 1 and 2.
  - Poppler spanned all three phases.
- **Encoders differ.** Every wall time includes PNG encoding, and the encoders differ: Wellfriend's own
  encoder (2.00 MB/page), libpng (Poppler), Pillow/zlib level 6 (PDFium) and MuPDF's own encoder. A
  stronger encoder costs time; Wellfriend writes about 50% more bytes.
- **Output path differs.** Wellfriend builds a ZIP in memory and writes it at the end. The comparators
  write one file per page. This is what inflates Wellfriend's RSS.
- **PDFium wall includes Python.** Its wall times include Python start-up, pypdfium2 import and the
  `to_pil()` copy. That is about 90 ms per document, which dominates small files, and some per-page
  overhead. PDFium's self-timed per-page median (337 ms) is the cleaner number.
- **Per-page medians are not comparable across tools.** Poppler and MuPDF per-page times come from output
  file mtimes; PDFium's are self-timed; Wellfriend's are not available.
- **Scan-heavy skew.** 97% of the pages are archive.org MRC scans (JPX plus JBIG2), so the totals mostly
  measure JPX/JBIG2 decoding and image downsampling. Poppler's slowness is mostly OpenJPEG decoding two
  28 MP JPX layers per page at full resolution. Vector and text rendering is represented only by the
  627-page small-document sets.
- **Comparator builds.** All comparators are distribution builds or wheels; none was compiled locally.
  MuPDF lacks ICC support, which affects CMYK and ICC-based colour.
- **SSIM is a coarse signal.** It runs on luminance only, so colour differences such as the CMYK cover
  are under-weighted. It measures agreement between tools, not correctness against a ground truth.

## CMYK output: Wellfriend native CMYK vs `pdftoppm -jpegcmyk`

PDFium has no CMYK raster output and this MuPDF build has no ICC support, so only Poppler is compared.
Same machine, one pinned single-threaded process per run, 150 dpi, all pages, JPEG output; wall and peak
RSS from `/usr/bin/time`. `wf native` is `render --format jpg --cmyk` (plate compositing, two passes per
page: C/M/Y, then K), `wf convert` is `--cmyk-method convert` (RGB render plus the uncalibrated
formula), `wf rgb` is the ordinary RGB JPEG render for reference. Run on 2026-11-10 while other agents'
builds loaded the box, so treat differences below about 1.3x as noise.

| Document | Pages | Poppler -jpegcmyk ms/page (RSS MB) | wf native ms/page (RSS MB) | wf convert ms/page | wf rgb jpg ms/page |
|---|---:|---:|---:|---:|---:|
| pdfjs/TAMReview.pdf | 23 | 130 (46) | 195 (81) | 109 | 129 |
| pdfjs/freeculture.pdf | 352 | 97 (51) | 155 (737) | 78 | 45 |
| fixtures/tracemonkey.pdf | 14 | 105 (41) | 132 (86) | 97 | 65 |
| pdfjs/knockout_groups_test.pdf | 3 | 290 (59) | 356 (91) | 203 | 173 |

Native CMYK costs about two RGB renders (the plate passes) and is 1.2-1.6x slower than Poppler's CMYK
mode on these files; the high RSS on `freeculture.pdf` is the CLI keeping all encoded pages in memory
until it writes the ZIP (the RGB render shows the same). Colour accuracy against Poppler's CMYK output is
covered in `docs/native_cmyk_and_separation_rendering.md` (synthetic overprint/spot page within JPEG noise;
corpus pages track the RGB renderer's own differences to Poppler).

## Reproducing / rerunning only the Wellfriend column

The scripts are in `/srv/wf-bench/` on the VPS:

| Script | Purpose |
|---|---|
| `make_corpus.py` | builds `corpus.tsv` |
| `run_tool.py` | per-tool runner: resumable, claim-based, takes `--tool` and `--bin` |
| `run_all.sh` | starts one worker per tool on vCPUs 5, 6, 7, 4 |
| `supervisor.py` | fills idle vCPUs with extra workers |
| `pdfium_render.py` | PDFium renderer |
| `ssim_sample.py` | SSIM sample run |
| `summarize.py` | builds the summary tables |
| `stop_all.sh` | stops runners and their in-flight renders |
| `vis.py` | side-by-side visual check |

Requirements: `/usr/bin/time` (package `time`) and `pdftoppm` are installed. Recreate the venv
(`python3 -m venv /srv/wf-bench-venv && /srv/wf-bench-venv/bin/pip install pypdfium2 numpy pillow`) only
if you also rerun SSIM.

```sh
# throughput column for a new binary, tagged v2, two single-threaded pinned workers on vCPUs 4 and 5
ssh root@136.65.4.82 'cp NEW_BINARY /srv/wf-bench/bin/wellfriendpdf-v2 && cd /srv/wf-bench && \
  nohup python3 supervisor.py --tools wellfriend --tag v2 --cores 4,5 --max wellfriend=2 \
  --wf-bin /srv/wf-bench/bin/wellfriendpdf-v2 > /srv/wf-parity-logs/bench-supervisor-v2.log 2>&1 < /dev/null &'
# when the log prints "ALL DONE": compare the v2 Wellfriend rows against the stored comparator rows
ssh root@136.65.4.82 'cd /srv/wf-bench && python3 summarize.py --wf-tag v2 --ssim ssim-v2.csv'
# optional SSIM for the new binary (needs the venv; skip MuPDF if mupdf-tools was removed)
ssh root@136.65.4.82 'cd /srv/wf-bench && nohup /srv/wf-bench-venv/bin/python ssim_sample.py --core 6 \
  --wf-bin /srv/wf-bench/bin/wellfriendpdf-v2 --tools wellfriend,poppler,pdfium \
  --out /srv/wf-bench/results/ssim-v2.csv > /srv/wf-parity-logs/bench-ssim-v2.log 2>&1 < /dev/null &'
```

Raw results are on the VPS under `/srv/wf-bench/results/`:

| File | Contents |
|---|---|
| `full-TOOL[@CORE].csv` | per-document rows, one file per worker |
| `full-TOOL[@CORE]-pages.jsonl` | per-page timings |
| `ssim.csv` | SSIM sample results |
| `summary.json`, `summary.md`, `summary-stdout.txt` | summary tables |
| `corpus.tsv` | corpus list with page counts |
| `wellfriend-base-b846-partial/` | the superseded binary's rows |

## Re-run on the final build (main 8c53a5c, 2026-11-11)

After rounds 6–8 (XFA data DOM/events, native CMYK fallbacks removed,
SVG embedded fonts, additive spot overprint) the head-to-head was re-run on
the test VPS with a release build of `8c53a5c`, Poppler `pdftoppm` 26.07,
PDFium via pypdfium2 and MuPDF `mutool` 1.27. Same harness (`run_tool.py`),
150 dpi, one process per document pinned to its own core, single-threaded.

Timing set: the 93 small documents (`fixtures` + `corpus` sets, 629 pages).
The large `realpdf`/`realpdf200` books were not re-timed (Poppler alone took
about 12.6 CPU-hours on them in the first run); they are covered by the
SSIM sample below.

| Tool | Documents rendered | Pages | Total wall (s) | Median ms/page | Peak RSS max / median (MB) |
|---|---:|---:|---:|---:|---:|
| **Wellfriend** | 91/93 | 629 | **11.8** | **17.8** | 673 / 39 |
| Poppler 26.07 | 90/93 | 628 | 101.8 | 66.6 | 64 / 28 |
| PDFium | 91/93 | 629 | 26.8 | 93.2 | 99 / 41 |
| MuPDF 1.27 | 90/93 | 628 | 14.6 | 24.6 | 92 / 23 |

Failures: `issue15893_reduced` and `print_protection` fail in every tool
(broken / password-protected). Poppler also fails `bug_jpx`; MuPDF also fails
`encrypted-attachment`.

Visual similarity (SSIM on the 157-page sample across all sets, books
included):

| Pair | Mean | Median |
|---|---:|---:|
| Wellfriend vs Poppler | 0.910 | 0.972 |
| Wellfriend vs PDFium | 0.928 | 0.956 |
| Wellfriend vs MuPDF | 0.956 | 0.987 |
| PDFium vs Poppler (baseline) | 0.921 | 0.971 |
| MuPDF vs Poppler (baseline) | 0.915 | 0.966 |

Wellfriend agrees with Poppler as closely as PDFium and MuPDF do, and on
this set renders about 8.6x faster than Poppler, 2.3x faster than PDFium and
1.2x faster than MuPDF in total wall time. Its peak RSS is higher on one
document (673 MB, a page with very large images); the median is in line with
the other engines.

## Text extraction on hard-font PDFs (final build, 2026-11-12)

Four PDFs whose fonts carry little or no Unicode information, extracted
with `pdftotext` (Poppler 26.07), pypdfium2 `get_text_range` (PDFium),
`mutool draw -F txt` (MuPDF 1.27) and `wellfriendpdf extract-text`.

| PDF | Poppler | PDFium | MuPDF | Wellfriend |
|---|---|---|---|---|
| TAMReview (letters+digits / garbage chars) | 10,195 / 46,378 | 8,735 / 39,963 | 7,677 / 50,731 | **47,689 / 0** |
| issue5801 (Polish weekday table) | garbage | garbage | garbage | **correct text** ("piątek, czwartek, … poniedziałek") |
| ThuluthFeatures (real Arabic letters) | 19 | 19 | 0 | **337** |
| issue5874 (real Arabic letters / invented CJK chars) | 41 / 0 | 52 / 26 | 41 / 0 | 41 / 0 |

On `issue5874` no engine recovers more than the same 41 Arabic letters: the
remaining glyphs are ligatures with no Unicode mapping in the file (PDFium's
extra letters come with 26 invented CJK characters). Wellfriend marks the
unrecoverable glyphs with U+FFFD instead of inventing characters.

## Damaged CCITT Group 4 image (final build, 2026-11-12)

`ccitt_EndOfBlock_false.pdf` with 4 bytes flipped in the middle of its G4
stream (file length unchanged), rendered at 72 dpi; each engine is compared
with its own render of the undamaged file.

| Engine | Rows identical to its clean render | Clean ink pixels still inked |
|---|---:|---:|
| Poppler 26.07 | 92.8% | 90.7% |
| PDFium | 92.8% | 91.6% |
| MuPDF 1.27 | 93.2% | 99.9% |
| Wellfriend | 92.9% | 90.9% |

All four keep the rows decoded before the damage. Group 4 (T.6) has no
end-of-line markers, so no decoder can resynchronise after it. MuPDF's higher
ink figure with no more identical rows suggests it paints the damaged region
dark rather than recovering it. This is a single small image, so it is a spot
check rather than a broad measurement.

### Broader sweep: 24 damaged G4 samples

The same test repeated on 6 Group 4 images (from the pdf.js corpus and from
PDFium's pixel tests), each with 4 bytes damaged at 20%, 40%, 60% and 80% of
the stream (ASCIIHex streams get hex-digit swaps so they stay valid). The score
is the share of rows identical to that engine's own clean render, averaged over
the 24 samples.

| Engine | Rows recovered |
|---|---:|
| Poppler 26.07 | 89.2% |
| PDFium | 92.2% |
| MuPDF 1.27 | 88.7% |
| Wellfriend | **92.2%** |

Wellfriend ties PDFium for the best recovery. None of the test files use
Group 3 (K >= 0), so G3 resynchronisation was not measured.

## PDFium's own pixel tests (main c2e326b, 2026-11-12)

Every page in PDFium's `testing/resources/pixel` suite that has a default
`*_expected.pdf.N.png` (230 pages, `.in` templates expanded with PDFium's
`fixup_pdf_template.py`) was rendered at 72 dpi by Wellfriend and by Poppler's
`pdftoppm`, and compared with PDFium's expected image (luminance SSIM, 7x7
box window). This measures how closely each engine reproduces PDFium's
behaviour, including on PDFium's deliberately malformed regression files.

| | Mean SSIM | Median | Pages < 0.90 | Render failures |
|---|---:|---:|---:|---:|
| Wellfriend before (7d8b831) | 0.898 | 0.988 | 35 | 15 |
| Wellfriend now | **0.973** | **0.994** | **12** | **0** |
| Poppler 26.07 | 0.931 | 0.989 | 28 | 4 |

Wellfriend is closer to PDFium than Poppler on 58 pages; Poppler is closer on 21.
No page scored lower than before the fixes. What changed:

- Compatibility rendering falls back to immediate replay when the cached
  display-list path meets a malformed operator, a missing resource or an
  unterminated inline image, so the bad object is skipped rather than the page
  failing (exact/HighQuality modes still refuse).
- A panic inside the JPEG 2000 decoder crate is caught and turned into an
  error (bug_1258634).
- The object parser recovers bare words, cut-off arrays and dictionaries,
  wrong or missing `endstream`, junk after `stream`, duplicate objects in object
  streams and doubled minus signs; the xref rebuild finds object headers in the
  middle of a line. bug_828206 and bug_830221, which Poppler also fails, now render.
- Images: over-long data is trimmed, missing BitsPerComponent defaults to 1
  for JBIG2/CCITT, short bilevel images pad white, short soft masks are
  transparent, malformed /Matte and color-key masks are ignored, out-of-range
  Indexed palettes draw, and indirect /Filter arrays resolve.
- Drawing: negative dash phases, calculator-function outputs from the top of
  the stack, large pattern cells over small fills, pattern opacity, single-point
  subpaths and zero-area fills drawn as hairlines (as PDFium does).

Known remaining gaps against PDFium: Lab and function-based Indexed shading
interpolation and folded tensor patches (`shade`, `shade-tensor`), text painted
with a pattern (bug_1772), non-embedded TrueType Symbol fonts (bug_1442723), a
Type 3 image-mask glyph painted with a pattern (bug_40721524), truncated JBIG2
partial decode (bug_867501), and bug_554151, where PDFium fills a nearly empty
612x104000 image black while Wellfriend fills it white as Poppler does.

## PDFium pixel tests and performance after rounds 10 and 11 (main 7264092, 2026-11-12)

Same harness as the previous section, run on a release build of 7264092.

| | Mean SSIM | Median | Pages < 0.90 | Render failures | Pages where Poppler is closer |
|---|---:|---:|---:|---:|---:|
| Wellfriend c2e326b | 0.973 | 0.994 | 12 | 0 | 21 |
| Wellfriend 7264092 | **0.989** | **0.998** | **3** | **0** | **4** |
| Poppler 26.07 | 0.931 | 0.989 | 28 | 4 | - |

Wellfriend is closer to PDFium than Poppler on 79 pages.

What round 10 added (all compat-mode, matching PDFium's source):
- shadings drawn with PDFium's own algorithm, including its Coons/tensor patch
  subdivision and Lab/Indexed mesh colours;
- text stroked with a pattern; pattern fills reaching Type 3 glyphs and their image masks;
- a non-embedded symbolic TrueType "Symbol" font uses the Symbol face;
- JBIG2 segments walked the way PDFium walks them, including zero-length segments
  (hayro-jbig2 vendored under `vendor/hayro-jbig2`);
- rows past the end of a short image are zero samples (black or palette entry 0), as in PDFium and MuPDF;
- JPX images whose /ColorSpace contradicts the JP2 colour box are not drawn;
- translucent fill-and-stroke painted as a knockout pair; form alpha inside tiling
  cells; no stroke adjustment inside pattern cells or soft-mask groups;
- inline images without `EI` dropped, `scn` naming a pattern in a device colour space.

Remaining pages below Poppler or below 0.90 are either drawn by PDFium's test
harness rather than the PDF (form highlight, open drop-down, selected text,
popup) or have expected PNGs that current PDFium itself no longer reproduces
(bug_42270524, bug_42270980, bug_42270979_3, bug_42271010).

Round 11 (performance, from PDFium's design):
- **Optional PDFium rectangle snapping** (`--rect-snapping pdfium`, contract field
  `rect_snapping`, C API / Python / WASM / .NET / Java): axis-aligned filled rectangles
  are snapped to whole pixels exactly as `CFX_RenderDevice::DrawPath` does, so
  touching table cells and chart bars show no seams. Pixel-identical to PDFium on a
  19,200-cell table and a 1,000-bar chart; in HighQuality mode it also renders those
  pages 2.5x to 7x faster. Default output is unchanged.
- **One annotation pass:** the page fingerprint, AcroForm and widget font tables are
  computed once per render pass instead of once per widget. The annotation pass of
  `prefilled_f1040` falls from 28.6 to 12.8 ms of CPU time.
- **Compact content operations:** operators are interned and up to two operands are
  stored inline. On the dense atlas pages, memory per operation falls from 177 to 83
  bytes and allocations per operation from 2.04 to 0.06; parse time falls from 269 to
  234 ms.

Every page of `tests/corpus/pdfs` renders byte-identically to the previous build
with default options; rounds 10 and 11 together changed only the 12 pages listed in
round 10 (all closer to PDFium).

A/B on the VPS, 91 bench documents (157 pages, up to 40 pages each), 150 dpi, one
pinned core, best of two runs each:

| Build | Total CPU time | Median peak RSS | Max peak RSS |
|---|---:|---:|---:|
| c2e326b | 6.38 s | 37 MB | 172 MB |
| 7264092 | **5.63 s** | 39 MB | 174 MB |

Tests on the VPS for 7264092: workspace check clean; engine 5,712 passed, 0 failed;
CLI + C API 162 passed, 0 failed.

## PDFium pixel tests after round 12: PDFium render profile (2026-11-12)

PDFium's pixel expectations are produced by its `pdfium_test` harness, which
installs a form-field highlight (`FPDF_SetFormFieldHighlightColor(form, 0,
0xFFE4DD)`, a Windows COLORREF, so RGB 221,228,255, with alpha 100) and
replays the test's `.evt` mouse and keyboard events before drawing. Round 12
adds an opt-in render profile that renders the way PDFium does
(`render --profile pdfium|pdfium-test`, contract field `profile`, C API
`wellfriendpdf_render_contract_with_profile`, Python/WASM `with_profile`,
.NET/Java `WithProfile`, server field `profile`):

- `pdfium`: PDFium rectangle snapping, no stroke adjustment (new contract
  field `stroke_adjustment: Off`; PDFium's AGG rasterizer never snaps
  strokes), PDFium text gamma, PDFium's `CFX_ImageTransformer` for skewed
  and rotated images and PDFium's note icon for Text annotations;
- `pdfium-test`: `pdfium` plus pdfium_test's form highlight.

The harness (`/srv/wf-bench/pdfium-pixel-r12.py`) scores both the default
rendering and `MODE=pdfium-test`, which also passes the field the `.evt`
events leave focused (last click or Tab; Enter in a single-line text field
closes its edit window). Typed text, open drop-downs, pop-ups and text
selections are interaction state and are not emulated.

| | Mean SSIM | Median | Pages < 0.90 | Render failures | Pages where Poppler is closer |
|---|---:|---:|---:|---:|---:|
| Wellfriend 7264092 | 0.989 | 0.998 | 3 | 0 | 4 |
| Wellfriend round 12, default | 0.989 | 0.999 | 3 | 0 | 2 |
| Wellfriend round 12, `--profile pdfium-test` | **0.993** | **1.000** | **2** | **0** | **1** |
| Poppler 26.07 | 0.931 | 0.989 | 28 | 4 | - |

With the profile Wellfriend is closer to PDFium's expectation than Poppler on
109 pages; image_transformer_other goes from 0.879 to 0.984 and bug_1963 from
0.930 to 0.994, and 114 pages score 1.000 (94 before).

Default rendering also changed where the old output was wrong:
- form highlight (when requested) is PDFium's whole-pixel `DrawShadow`, on
  AcroForm fields only, never on push buttons or signatures, and not on the
  focused field; the focus rectangle is drawn around buttons only;
- text fields show only `/MaxLen` characters (bug_1072440) and indirect
  field values are resolved (bug_40643646's field was not drawn at all);
- a zero-area fill whose subpath has curves but lies on one line is drawn
  whole (bug_42270524, 0.990 -> 1.000; a PDFium bug that PDFium itself
  suppresses).
Every page of `tests/corpus/pdfs` (598 pages, 50 dpi) still renders
byte-identically to 7264092 with default options.

Remaining pages, all with evidence that they are not rendering gaps:
- bug_40643646 (0.872 with the profile): most of the page is a pop-up that
  pdfium_test opens by hovering over the note (`mousemove` event); PDFium
  never draws closed pop-ups. The test is suppressed in PDFium itself.
- bug_42270979_3 (0.886, Poppler 0.886): the curve
  `0 0 m 2 10000 20 9000 40 -10000 c` at 5x crosses the page at
  x = 136.0..136.2 (exact evaluation), where Wellfriend and Poppler draw it;
  the expected PNG (Skia) draws it at x ~ 105 and current PDFium (AGG) at
  ~139. Suppressed for AGG in PDFium.
- bug_42271010 (0.943, Poppler 0.953): suppressed in PDFium (pypdfium2
  renders the page solid red). Wellfriend's ramp seam is one row later than
  the expectation because its axial shading samples pixel corners as PDFium's
  shading code does, and the expected gradient colours are not a linear
  interpolation of the function's stops (mid-ramp 467c1f instead of 008000).
- bug_42270980 (default 0.925, Poppler 0.937; profile 0.966): suppressed in
  PDFium; the expectation places the image on `GetClosestRect`, the default
  renderer samples pixel centres like Splash.

Tests on the VPS: workspace check clean; engine 5,727 passed, 0 failed;
CLI + C API 162 passed, 0 failed; .NET binding and tests build; Java binding
compiles.

## Speed and memory techniques: PDFium and Poppler vs Wellfriend

Each technique below was found by reading PDFium's and Poppler's source and
checked against Wellfriend's code. "Measured" gives the before/after from the
round that added it.

| Technique | PDFium | Poppler | Wellfriend | Measured |
|---|---|---|---|---|
| Clip regions bounded to the clip's box; rectangle clips kept as a box | `CFX_AggClipRgn` | `SplashClip` rect + paths | yes | nested-clip stress 14.9 s -> 0.8 s, 4.7 GB -> 107 MB |
| Coverage spans blended straight into the page, no per-path mask | AGG scanline renderer | Splash pipe | yes | tracemonkey 1.6 s -> 0.59 s |
| Axis-aligned rectangle fast path (no rasteriser) | `DrawPath` -> `FillRect` | Splash `fillRect` | yes (default Splash-style; `--rect-snapping pdfium` for PDFium's rule) | HighQuality table 179 -> 71 ms |
| Shared, reference-counted objects and resources instead of deep copies | `RetainPtr` | `Object` refcount | yes (`Arc`) | |
| Document-level font, glyph-outline and glyph-bitmap caches | `CPDF_DocPageData`, `CFX_GlyphCache` | `SplashFontEngine`, glyph cache | yes | |
| Soft masks shared between graphics states, not copied on save | yes | yes | yes | soft-mask stress 1.7 s -> 0.52 s |
| Reduced-size JPEG decode when the image is drawn small | libjpeg scaled IDCT | no | yes (native reduced IDCT) | |
| JPEG 2000 resolution-level skipping | `CJPX_Decoder` resolution levels | no | yes, when the full image exceeds limits | |
| Annotations drawn in the same pass, form data prepared once | `CPDF_AnnotList` in one `CPDF_RenderContext` | `Annots::draw` in same `Gfx` | yes | f1040 annotation pass 28.6 -> 12.8 ms |
| Compact content operations (interned operators, inline operands) | `CPDF_PageObject` tree | streamed, not stored | yes | 177 -> 83 bytes/op, 2.04 -> 0.06 allocs/op |
| Streaming page output instead of collecting every page | page by page | page by page | yes (streaming ZIP, reorder buffer) | 303-page atlas peak 5.4 GB -> 378 MB |
| Parallel page rendering, one cache per worker | single-threaded per document | single-threaded | yes (one thread per worker, workers split one 256 MiB retained-cache budget) | 93 docs 11.8 s vs Poppler 101.8 s, PDFium 26.8 s; atlas at 8 workers 2.68 -> 0.98 GB (round 14) |
| Image masks shrunk without an expanded copy | 1 bpp `CPDF_DIB` | `ImageStream` | yes (`StencilMask`, round 14) | freeculture 671 -> 245 MB |
| Image masks kept at one bit per pixel | 1 bpp `CPDF_DIB` | `ImageStream` | yes (packed `RawImage`, round 15) | freeculture 245 -> 169 MB |
| Huge decoded images not kept between pages | `kHugeImageSize` (60,000,000 bytes) | no image cache | yes (single-pass renders, round 14) | |
| SIMD blending and colour conversion | AGG + Skia SIMD | limited | yes (`render-simd` crate) | |
| Link-time optimisation in release builds | yes | distro-dependent | yes (`lto = "thin"`) | |
| Tuned memory allocator | PartitionAlloc | system malloc | opt-in `mimalloc` feature (CLI, C API); default stays glibc with tuned settings | mimalloc: -9.5% CPU but +30-40% peak RSS, no multi-thread wall gain (see "Global allocator") |

## Global allocator (round 12, main 15cd153, 2026-11-12)

PDFium allocates through PartitionAlloc, and Wellfriend uses the system allocator
(glibc on Linux, which the CLI tunes with `mallopt`). Round 12 measured mimalloc
0.1.52 (mimalloc v3) and jemalloc (tikv-jemallocator 0.6) as the CLI's global
allocator. All three builds use the same source and toolchain. Every page of
`tests/corpus/pdfs` (75 PDFs, 596 page images at 50 dpi) rendered byte-identically
with each allocator, with the same exit codes.

Single-threaded, using the bench A/B above (91 documents, 157 pages, 150 dpi, one
pinned core, best of two). The CPU column is the change against 7264092 over
several A/B runs. The A/A run (system allocator against 7264092) moved by up to 2%.

| Allocator | CPU time | Median peak RSS | Max peak RSS |
|---|---:|---:|---:|
| system (glibc, tuned) | 0% (A/A: -1% to +2%) | 37 MB | 172 MB |
| mimalloc | **-7% to -12%** (4 runs, mean -9.5%) | 51 MB (+38%) | 222-224 MB (+30%) |
| mimalloc, `MIMALLOC_ALLOW_THP=0` | -3% (3 runs) | 39 MB | 206 MB (+20%) |
| jemalloc | -2% to -4% (3 runs) | 37-39 MB | 183-185 MB (+7%) |

mimalloc's single-threaded gain comes mostly from committing memory in large
(transparent huge page) blocks. This is also the source of its higher peak RSS.
Without THP, or with lazy arena commit (`MIMALLOC_ARENA_EAGER_COMMIT=0`), the RSS
cost mostly disappears, and so does most of the gain.

Multi-threaded: `atlas_animals` (303 pages, 168 MB), 150 dpi PNG, `/usr/bin/time -v`.
Runs were interleaved, with 2 to 5 repeats per setting.

| Threads | Allocator | User CPU | System CPU | Peak RSS |
|---|---|---:|---:|---:|
| 8 (default) | system | 244-248 s | 2.5-2.9 s | 2.72-2.93 GB |
| 8 (default) | mimalloc | 244-250 s | 1.5-26 s | 2.40-2.48 GB (-14%) |
| 8 (default) | jemalloc | 244-248 s | 15.5-16.8 s | 1.89-1.90 GB (-33%) |
| 4 | system | 180-241 s | 1.3-2.2 s | 1.52-1.59 GB |
| 4 | mimalloc | 191-212 s | 7.8-12.3 s | 1.33-1.36 GB (-13%) |
| 4 | jemalloc | 202-206 s | 13.6-13.7 s | 1.03 GB (-34%) |
| 2 | system (lean glibc mode) | 172-236 s | 11.7-14.7 s | 0.55-0.56 GB |
| 2 | mimalloc | 204-237 s | 1.1-1.3 s | 0.79-0.86 GB (+43% to +54%) |
| 2 | jemalloc | 209-237 s | 15.0-16.7 s | 0.57-0.58 GB (+3% to +4%) |

Wall time on 8 threads ranged from 42 to 62 s for every allocator, because the
shared VPS was running other builds (load 12 to 17). No allocator was measurably
faster on wall time. User CPU time was also the same within that noise. jemalloc
spends 13 to 17 s of system time (page purging) on every run, against 1 to 3 s for
the system allocator on 4 and 8 threads.

Decision: the default stays the system allocator. mimalloc is the only candidate
with a clear CPU gain, and that gain comes with materially higher peak RSS on the
small documents it speeds up (+38% median, +30% max). It also gives a 43-54% higher
peak when the CLI is given two threads, because that is where the CLI's lean glibc
settings apply. On large multi-threaded renders its lower peak is not matched by a
faster wall time. jemalloc saves memory on many threads but costs system CPU time,
and it does not build for MSVC. mimalloc stays available as a non-default
`mimalloc` cargo feature on `wellfriendpdf-cli` and on `wellfriendpdf-capi`, for
deployments that prefer CPU time over peak memory. When it is enabled, the CLI
skips its glibc tunables. The C API feature changes only the library's own Rust
allocations. Every buffer it hands out is freed through its own `*_free`
functions, so the host's allocator is never mixed with mimalloc.

## Peak memory (round 14, 2026-11-13)

The r13 head-to-head left Wellfriend fastest on the 93 small documents but
with the highest peak RSS: 671 MB max (PDFium 97, Poppler 64, MuPDF 92), and
2.7 GB for `atlas_animals` (303 scanned pages, 150 dpi) with 8 threads.
Round 14 found the causes with heaptrack and per-page runs and removed them.
Rendered output is unchanged: every page of `tests/corpus/pdfs` at 50 dpi
(75 PDFs, 596 pages) and 424 pages of large scanned and image books at
150 dpi (atlas_animals, freeculture, six realpdf200 books) hash identically
to `ea7832d` (same engine as main `a8be500`).

**The 671 MB document: `freeculture.pdf`.** Pages 171 and 255 each draw a
scanned CCITT stencil mask (8992x10384 and 9258x12259 at 2,856 and 2,182 ppi)
a few hundred pixels wide. The renderer decoded the mask to one byte per
pixel (113 MB), expanded it to RGBA for the painter (454 MB), and kept the
decoded mask in the document cache for the following pages. Page 255 alone
peaked at 588 MB.

| What | PDFium | Poppler | Wellfriend before | Wellfriend now |
|---|---|---|---|---|
| Stencil mask storage | 1 bpp `CPDF_DIB`, scanlines decoded on demand | `ImageStream`, one line at a time | 8 bpp + RGBA copy | 8 bpp, painted directly |
| Shrinking a mask | `CFX_ImageStretcher` streams rows | `scaleMask` box filter, streamed | area average over the RGBA copy | same area average, read through the mask |
| Huge decoded images between pages | not realised above `kHugeImageSize` (60,000,000 bytes) | never cached | kept up to 128 MB | dropped at page end above 60,000,000 bytes (single pass) |

The direct mask painter (`StencilMask`, `ImagePainter::paint_stencil_mask`)
reads each sample as the painted or unpainted RGBA pixel the expansion held,
with the same taps, weights and accumulation order, so it is byte-identical;
magnified masks and unsmoothed rendering still expand (they are small). A
soft-masked image is now combined from the cached decoded image instead of a
copy (`images.pdf`: 170 -> 147 MB).

**Multi-thread scaling (`atlas_animals`).** heaptrack at 8 workers showed
950 MB of live heap and about 0.9 GB more of freed but resident memory:

- `par_bridge` ran pages on rayon workers; a worker waiting inside a page
  (decoders run nested parallel work on the pool) stole and started another
  page, so ten pages (each with a 55 MB page raster) were in flight for eight
  workers. Pages now run on one plain thread per worker; nested work still
  uses the rayon pool.
- Every page also built a 40 MB device-size copy of its scan for the
  scaled-image cache, which could never cache it (32 MB entry limit). Such
  copies are no longer built; the general painter paints the same pixels.
  This also saves about a third of the CPU time on scanned books.
- Each worker kept full-size caches. The workers now split one 256 MiB
  retained-cache budget (`RenderDocumentCache::set_retained_byte_limit`,
  at least 32 MiB each), applied when each page finishes.
- With more than two workers the CLI kept glibc's one-shot retention
  (32 MiB mmap threshold, 1 GiB trim threshold), so every arena kept its own
  high-water mark. The lean settings (1 MiB, 64 MiB, no top pad) now apply at
  every worker count: about 7 s more system time on atlas at 8 workers, 0.9 GB
  less peak RSS.

`atlas_animals`, 150 dpi PNG to `/dev/null`, `/usr/bin/time`, same session,
before (`ea7832d`) and after (round 14):

| Workers | Wall (s) | User + sys CPU (s) | Peak RSS |
|---:|---:|---:|---:|
| 8 | 34.5 -> **23.7** | 270 -> **184** | 2.68 GB -> **0.98 GB** |
| 4 | 42.1 -> **27.2** | 168 -> **110** | 1.57 GB -> **0.67 GB** |
| 1 | 173 -> **107** | 174 -> **107** | 0.33 GB -> **0.32 GB** |

A repeat in a later, busier session (load 6 to 14) gave, for 8 workers,
42.5 -> 31.1 s wall, 264 -> 182 s CPU and 2.65 -> 1.00 GB; for 4 workers
55.9 -> 30.6 s, 216 -> 123 s and 1.54 -> 0.68 GB; for 1 worker 196 -> 113 s,
194 -> 113 s and 0.33 -> 0.32 GB.

About 170 MB of every figure is the 168 MB input file, which Wellfriend reads
into memory (pypdfium2 on the same file: 416 MB single-process; pdftoppm
132 MB).

**93-document Wellfriend column** (fixtures + corpus sets, 150 dpi, one pinned
core, `run_tool.py`; the r13 comparator columns are unchanged):

| Build | Docs rendered | Pages | Total wall (s) | CPU (s) | Peak RSS max / median (MB) | Largest |
|---|---:|---:|---:|---:|---:|---|
| r13 run (`69828bc`) | 91/93 | 629 | 11.8 | 10.6 | 671 / 39 | freeculture 671, images 173 |
| `ea7832d`, same session | 91/93 | 629 | 12.1 | 10.0 | 671 / 36 | freeculture 671, images 170 |
| round 14 | 91/93 | 629 | 11.6 | 9.2 | **245 / 38** | freeculture 245, images 147 |

PDFium's max on this set is 97 MB, Poppler's 64 MB, MuPDF's 92 MB. The
remaining freeculture peak is the 113 MB one-byte-per-pixel mask; decoding
stencils to 1 bpp, as PDFium does, would remove most of it.

**Round 15: masks at one bit per pixel.** Image masks are now decoded and
cached packed, one bit per pixel (`RawImage::is_packed_bilevel`), as PDFium
keeps a 1 bpc `CPDF_DIB`: CCITT masks are decoded straight to bits (full image
or source window), unfiltered and Flate/LZW 1 bpc masks keep their stream bits,
and other masks (JBIG2, ...) are packed after decoding. The mask painters read
a bit as the sample 0 or 255 it expanded to, so the output is unchanged: every
page of `tests/corpus/pdfs` at 50 dpi (596 pages) and the 424 pages at 150 dpi
above hash identically to round 14. freeculture page 255 alone: 149 -> 54 MB.
93-document Wellfriend column, same session (other jobs running, so the wall
times are high for both):

| Build | Docs rendered | Pages | Total wall (s) | CPU (s) | Peak RSS max / median (MB) | Largest |
|---|---:|---:|---:|---:|---:|---|
| round 14 | 91/93 | 629 | 23.0 | 13.9 | 246 / 37.6 | freeculture 246, images 146 |
| round 15 | 91/93 | 629 | 21.3 | 13.6 | **169 / 38.2** | freeculture 169, images 147 |

1 bpc DeviceGray images still decode to one byte per pixel; the general image
painter reads 8-bit samples.
