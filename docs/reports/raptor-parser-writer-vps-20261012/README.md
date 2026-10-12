# RAPTOR implementation pass: parser, writer, vector export, raster images, Office media (VPS, 12 October 2026)

Source: `main` at `904db7b` plus the working-tree changes listed below. Every build, test and timing ran on the IPv6 VPS (`v72937`, 4 vCPU QEMU guest without AVX2/SHA-NI, 11.7 GiB RAM). Nothing executed on the development machine.

| Binary (release profile, identical flags) | SHA-256 |
| --- | --- |
| baseline CLI (`904db7b`) | `94b9c80202316f0dc1162d4ff3a5b047bd10eb0e8337e87a3c02d4c5c2230fbe` |
| patched CLI (`n17`, final) | `55adfd46b2a50ecb7e1b87acf0f3ceec854e757bb1e0d46d3a8956669b0ec8d8` |
| baseline benchmark adapter (8 October publication) | `53c4ac24342a7caeeb833c47728535b786806e15aa667309e36241c721679dc8` |
| patched benchmark adapter (`n11`) | `604cc9ac47d0afc925e3a9ffd42cf514c1e007d5800b9aefffdcba93a97668c1` |

## Profiles that drove the changes

| Workload (baseline) | Dominant cost |
| --- | --- |
| merge / rotate of scans | 24–27% `sha256_compress` of the whole input at open; 32–39% kernel page zeroing from double-buffered stream reads, whole-graph retention and whole-output buffers |
| rotate, 1,398-page scan | Quadratic page lookup: each `get_page(n)` walked the page tree from the root |
| SVG/PS/EPS export | 36% re-parsing and eexec-decrypting the whole Type 1 font for every glyph; font stream re-inflated and ToUnicode CMap re-parsed for every text operator (emitter and vector-fallback classifier); each glyph outline computed two or three times |
| HTML/Markdown/JSON/DOCX (untagged) | Full semantic geometric-fallback analysis of every page, discarded, then the geometric model built again |
| DOCX of scans | 45% Deflate of 8-bit PNGs for 1-bit scans; 35% per-sample floating-point `/Decode` mapping |
| tables-json, tagged form with 1,947 object streams | Each page re-walked the structure tree; a 32-stream object-stream cache made every walk re-decode and re-parse almost all object streams |
| raster images | Box downscale recomputed horizontal weights per source row and accumulated gray three times; packed/Indexed/ICC samples decoded with a per-sample division and range-checked interpolation |
| resident open + page count | Classic xref parsing allocated a token per field and hashed each entry twice |

## Changes

| Change | File | Output effect |
| --- | --- | --- |
| Source digest computed on first revision/fingerprint request, not at open | `render/document_view.rs`, `engine.rs` | None |
| `get_page`: after 32 distinct indexed lookups, build the page projection once | `document.rs` | None |
| Streaming classic-xref writer (`ClassicStreamWriter`, `Vec` or `io::Write` sink) for whole-document rewrites | `writer.rs` | None (byte-identical) |
| Merge/split/extract defer stream bodies ≥16 KiB and read them only while serializing; new `build_merged_to` streams to any writer; `merge_paths_to_file` streams path-to-path through a temporary sibling and rename; exposed as CLI `merge`, C `wellfriendpdf_merge_pdf_files_to_path` and Python `merge_pdfs_to_path` | `writer.rs`, `reader.rs`, `lib.rs`, `cli/src/main.rs`, `wellfriendpdf-capi` (+ header), `wellfriendpdf-py` | None (byte-identical); merging into an input path is safe |
| Single-copy large stream reads with verified `endstream … endobj`; 16 KiB header probe; fallback to the general parser otherwise | `reader.rs`, `parser.rs` | None |
| Sorted offset index for object boundaries; binary-search page identity; allocation-free range reads | `reader.rs`, `render/document_view.rs` | None |
| Allocation-free xref tokens, single-hash xref inserts, bounded per-subsection reservation (SipHash kept: object numbers are attacker-controlled) | `reader.rs` | None |
| Object-stream cache bounded by objects and a 256 MiB retained-byte budget instead of 32 streams | `reader.rs` | None |
| Per-thread cache of parsed Type 1 programs (exact byte match) | `fonts/type1.rs` | None |
| Per-scope font-program and `FontResolver` caches; one outline computation per glyph (SVG, PS/EPS, vector-fallback classifier) | `render/svg.rs`, `render/postscript.rs`, `render/vector_fallback.rs` | None |
| Untagged documents go straight to the geometric document model | `docmodel.rs` | None |
| `/Decode` byte mapping and packed-sample decoding through per-component tables built with the original expressions | `images/decoder.rs`, `images/sample_decode.rs` (+ equivalence test) | None (bit-exact) |
| Box downscale with precomputed column weights and single gray accumulator; channel-specialised area average; SMask combine without an intermediate copy | `render/image_painter.rs`, `images/smask.rs` | None (pixel-identical) |
| Streaming rotate/crop/extract (`rotate_pages_to`, `crop_pages_to`, `ContentEngine::extract_pages_to`, `build_subset_to`, `rewrite_document_to`) and `AtomicFile` (temporary sibling + rename on commit); CLI `rotate`, `extract-pages`, `split`, `merge` stream through it; C `wellfriendpdf_document_rotate_pdf_to_path` / `wellfriendpdf_document_extract_pages_to_path`; Python `rotate_pdf_to_path` / `extract_pages_to_path` | `structural.rs`, `writer.rs`, `engine.rs`, `lib.rs`, CLI, C API (+ header), Python | None (byte-identical to the buffer APIs) |
| Path fill/stroke mask caches (page renderer and display-list replay) keyed by the exact bits of every rasterization input instead of 1/64-quantized offset, width, flatness and miter (and, in replay, linear terms) | `render/page_renderer.rs`, `render/display_list.rs` | Rendering no longer depends on cache history; 5 of 389 pages change by 38 pixels in total (max channel delta 19) where the baseline reused an approximate mask |
| Multi-page image export renders pages on the rayon pool (one retained cache per worker), results in page order | `utilities.rs` | Pixel-identical to single-core and to per-page renders |
| Diagnostic `RenderDocumentCache::diagnostic_reset` (hidden) and `render_order_probe` example used to locate non-transparent retained state | `render/page_renderer.rs`, `examples/render_order_probe.rs` | None |
| Office media: bilevel grayscale encoded as 1-bit PNG | `images/encoder.rs`, `engine.rs`, `office.rs` | Same decoded pixels; smaller files; media names/relationship IDs (hashes of PNG bytes) change |
| Rendered SVG/PS/EPS archives: Deflate level 1 | `cli/src/main.rs` | Same archive entries; larger archives |

## Results

All corpus timings: one run per binary per document, pinned to one core of the same host. Outputs are compared byte-for-byte (PDF, text, HTML), by unzipped entries (SVG/EPS), by decoded pixels (raster PNG), or entry-by-entry with PNG media compared by pixels and media-hash identifiers renumbered (DOCX). Where noted, the patched pass overlapped a compilation on other cores, so its timings are conservative.

### Parsing and document facilities

| Workload | Docs | Failures base/new | Equivalent outputs | P50 base → new | P95 base → new | Max base → new | Total-time ratio |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Resident open + page count (persistent adapter) | 150 | 0 / 0 | 150/150 page counts | 0.95 → 0.89 ms | 3.31 → 2.76–3.05 ms | 43.6 → 31.6–40.0 ms | 1.15× |
| merge (file with itself)* | 137 | 0 / 0 | 137/137 | 221 → 143 ms | 665 → 366 ms | 3,645 → 1,724 ms | 1.98× |
| rotate 90°, all pages* | 137 | 0 / 0 | 137/137 | 156 → 104 ms | 443 → 235 ms | 2,228 → 1,619 ms | 1.73× |
| extract pages 1–2* | 122 | 0 / 0 | 122/122 | 88 → 72 ms | 213 → 116 ms | 328 → 175 ms | 1.50× |
| text* | 122 | 0 / 0 | 122/122 | 342 → 300 ms | 1,196 → 1,315 ms | 4,303 → 4,163 ms | 1.04× |
| HTML* | 122 | 0 / 0 | 122/122 | 468 → 414 ms | 2,213 → 2,347 ms | 13,577 → 13,481 ms | 1.06× |
| DOCX* | 122 | 0 / 0 | 122/122 | 1,057 → 975 ms | 12,624 → 8,205 ms | 33,612 → 36,535 ms | 1.20× |

\* Patched pass overlapped a compilation. On an idle host the conversion corpus measured EPS 11.0×, extract 1.61×, DOCX 1.24×, HTML 1.10×, text 1.08× with the `work8` build.

Resident parse is reported for two alternating runs per adapter; the 8 October publication recorded PDFium at P50 0.72 ms, P95 2.15 ms, P99 6.15 ms.

### Visual and vector output

| Workload | Docs | Failures base/new | Equivalent outputs | P50 base → new | P95 base → new | Max base → new | Total-time ratio |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Raster pages 1–3 at 72 DPI (PNG, pixels compared, one core) | 137 | 0 / 0 | 132/137 identical; 5 pages differ by 38 pixels in total (exact cache keys, see below) | 918 → 855 ms | 2,935 → 2,386 ms | 6,419 → 6,007 ms | 1.18× |
| Raster pages 1–8, 20 multi-page documents, four vCPUs vs one | 20 | 0 / 0 | 160/160 identical to one-core output | | | | 2.23× (63.2 → 28.3 s) |
| SVG first page* | 137 | 0 / 0 | 137/137 | 2,418 → 298 ms | 12,804 → 711 ms | 115,127 → 1,570 ms | 13.6× |
| EPS first page* | 122 | 0 / 0 | 122/122 | 3,319 → 358 ms | 11,728 → 1,314 ms | 111,916 → 2,006 ms | 10.8× |

Page-1 rasters from the patched adapter match the 8 October adapter on 147/150 documents. The three differences were traced: one is nondeterminism in the 8 October adapter itself (its own second render differs and matches the patched result), and on the other two the pristine `904db7b` CLI and the patched CLI produce identical pixels, so the 8 October adapter binary differs from `904db7b` there rather than this change.

### Holdout verification (plan stage 8)

An independent holdout set was assembled on the VPS from the earlier RAPTOR corpus, the natural-real-world and corpus-selected qualification sets, the repository pdf.js test corpus and the comprehensive set: 174 unique files after SHA-256 deduplication, with all 217 files that duplicate the 150-document benchmark corpus excluded. It includes malformed and encrypted pdf.js cases. Both binaries ran every operation once per file on an idle host.

| Operation | Files | Failures base/new (same files, same exit code) | Equivalent outputs among successes | P95 base → new | Max base → new | Total-time ratio |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| merge | 174 | 8 / 8 | 166/166 | 93 → 77 ms | 759 → 291 ms | 1.09× |
| rotate | 174 | 10 / 10 | 164/164 | 79 → 73 ms | 379 → 164 ms | 1.04× |
| SVG first page | 174 | 7 / 7 | 167/167 | 866 → 313 ms | 5,658 → 1,203 ms | 2.03× |
| EPS first page | 174 | 7 / 7 | 167/167 | 981 → 355 ms | 5,527 → 1,172 ms | 1.98× |
| extract pages 1–2 | 174 | 8 / 8 | 166/166 | 77 → 68 ms | 181 → 124 ms | 1.04× |
| text | 174 | 13 / 13 | 161/161 | 156 → 170 ms | 1,879 → 1,966 ms | 1.01× |
| HTML | 174 | 7 / 7 | 167/167 | 214 → 193 ms | 7,606 → 6,055 ms | 1.14× |
| DOCX | 174 | 7 / 7 | 167/167 | 216 → 216 ms | 20,015 → 18,355 ms | 1.10× |
| raster pages 1–3 | 174 | 7 / 7 | 174/174 (pixels) | 557 → 525 ms | 1,415 → 1,310 ms | 1.00× |

Holdout files are small (33 MB in total, P50 below 110 ms for every operation), so whole-file costs removed by this work matter less there; no operation regressed in failures or output, and the timing differences on text P95/max are within run-to-run noise for sub-200 ms jobs.

### Individual documents

| Document | Operation | Baseline | Patched | qpdf (same host) |
| --- | --- | ---: | ---: | ---: |
| NASA 19960055177 (223 MB, 1,398 pages) | merge | 13.0–17.0 s, 1.59 GB RSS | 3.7–4.9 s, 243 MB RSS | 2.6–2.9 s |
| NASA 19980010868 (266 MB, 1,518 pages) | merge | 17.3 s (8 Oct) | 6.0–7.2 s, 247 MB RSS | 3.0–3.3 s |
| NASA 19960055177 | rotate | 24.71 s, 815 MB RSS | 1.61 s, 108 MB RSS (streaming) | 2.30 s (8 Oct) |
| NASA 19960055177 | extract pages 1–2 | 3.01 s | 0.38 s | |
| NASA 19960055177 | SVG first page | 2.60 s | 0.12 s | |
| NASA 19960055177 | text | 33.5 s | 13.2 s | |
| NASA 19710069510 (64 MB, 107 pages) | merge | 2.25–2.35 s, 447 MB | 0.69–1.08 s, 91 MB | |
| NASA 19710069510 | DOCX | 40.8 s, 22.0 MB | 17.0 s, 17.3 MB | |
| arXiv 2609.28414 | SVG / PS / EPS first page | 6.6 s each | 0.30 s (SVG) | |
| i1040gi (4 MB, 126 pages, tagged) | tables-json | 122.2 s | 53.7 s | |

## Remaining gaps

- **Raster rendering** is 1.18× faster on one core and 2.23× on four, now deterministic, but still far from MuPDF/PDFium single-core speed (8 October all-page P50 5.3 s versus 0.9–1.1 s). The remaining cost is spread over path coverage, clipping, image scaling and soft masks, plus the whole-file SHA-256 that every render contract binds to; closing it needs the plan's rasteriser redesign (bounded clips, prepared filters, tile execution) and a decision on the revision-identity contract, which 125 call sites and persisted render plans depend on.
- **Resident open/page count** improved 1.15× but remains behind PDFium (P50 0.89 versus 0.72 ms). The remaining cost is xref hashing with randomized SipHash, kept deliberately to avoid attacker-chosen collisions.
- **Merge** of the largest scans is within 1.5–2× of qpdf; remaining work is serialization of every copied stream.
- **tables-json on tagged documents** still walks the structure tree once per page; a shared walk changes results for cross-page tables, so it needs a semantic-extractor redesign.
- **HTML on the 1,398-page scan** (75 s) has a flat profile.
- Plan stage 7: streaming output now covers merge, rotate, extract-pages/split (CLI, Rust, C, Python) and crop (Rust). Remaining byte-only surfaces: WASM (no file system), the server (no structural write endpoints of this kind), CLI `pages-crop` (uses the separate `crop_pdf_pages` implementation), and encrypt/optimize/linearize/repair, which have full-output contracts.
- **Rendering determinism (stage 6) — fixed.** The baseline's retained render cache was not output-transparent: on `f1040.pdf` and `f1040se.pdf`, page 2 rendered after page 1 differed from page 2 rendered alone (4 and 2 pixels). Bisecting the cache with `render_order_probe` isolated the path-stroke mask cache together with its cross-page dependency state: masks were keyed by 1/64-quantized sub-pixel offset and width but rasterized from exact values, so a hit could return a mask built for a neighbouring offset. Keying every rasterization input exactly (all four path-mask caches) makes all 427 benchmark pages (pages 1–3 of every document) render identically with a shared or fresh cache, which in turn makes parallel page rendering deterministic: 160/160 pages identical between one and four cores, 2.23× faster with four vCPUs (final build).

## Verification

- Benchmark corpus: 411/411 rotate/merge/SVG outputs identical, 610/610 conversion outputs equivalent, 137/137 raster outputs pixel-identical, 150/150 page counts identical; zero failures on either binary.
- Holdout corpus: 1,566 operations; every success equivalent to the baseline and every failure on the same file with the same exit code.
- Streaming bindings: C programs and a Python script compared every `*_to_path` result with its buffer counterpart (merge, rotate, extract) byte for byte, rotated a file in place, rejected an unwritable output and left no `.wfpartial-*` files.
- Bindings: a C program linked against the built `libwellfriendpdf_capi.so` and the built Python extension both merged two corpus files with output byte-identical to the baseline CLI (C) and to the existing `merge_pdfs` bytes (Python), merged a file into itself successfully, and rejected an empty input list. `wellfriendpdf-capi` and `wellfriendpdf-py` compile cleanly.
- Full engine library test suite on the VPS for the final source (`n17`): 4,353 passed, 0 failed (includes a new bit-exact equivalence test for the sample-decode tables).
