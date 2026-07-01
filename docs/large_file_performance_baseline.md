# Large-File Performance Baseline

Date: 2026-07-02

Commit measured: `01c592b718f17513083d32556c5b206e9159454d`

This is the Prompt 1 measurement-only baseline. No reader/parser/engine implementation was changed.

Prompt 2 follow-up: `docs/large_file_performance_prompt2.md` records the streaming-reader re-architecture results, before/after profile, correctness checks, and remaining Prompt 3 gap. This document remains the pre-change baseline.

## Headline Gap

Target: process a single 3-4 GB PDF and documents with thousands of pages within a hard 2 GB resident-memory envelope.

Current measured ceiling under the same 2 GB cap:

- Real files: the largest real file tested, 703 MB / 52 pages, opens, counts pages, extracts text/images, and renders the first three pages under the cap.
- Synthetic size axis: open/page-count succeeds at 1.5 GB, then fails at 2.0 GB and 3.0 GB during the initial full-file read. Text/image extraction and first-three-page render complete at 1.0 GB, then fail at 1.5 GB before first page output.
- Synthetic page-count axis: 5000 tiny pages stay under 20 MB for serial page-by-page extraction, but take about 9 minutes because current page APIs repeatedly rebuild the full page list.

Plainly: Oxide is not yet close to the 3-4 GB target. The primary gap is the full-file heap buffer; the secondary gap is full content-stream materialization and repeated page-tree materialization.

## Current Read-Path Evidence

The current path is whole-file in memory:

- `crates/engine/src/reader.rs:56-70` stores `data: Vec<u8>` in `PdfReader`.
- `crates/engine/src/reader.rs:74-82` implements `from_path_with_password` as `fs::read(path)?` followed by `from_bytes_with_password`.
- `crates/engine/src/reader.rs:95-138` parses header/xref/trailer from that byte slice and then stores the original `Vec<u8>` in the reader.
- `crates/engine/src/reader.rs:246-262` resolves uncompressed objects by creating a parser over `&self.data` at the xref offset, so there is no streaming or mmap path today.

Objects are partially lazy but caches are unbounded:

- `crates/engine/src/reader.rs:246-284` parses individual objects on demand from xref entries.
- `crates/engine/src/reader.rs:54` defines the object-stream cache as nested `HashMap`s.
- `crates/engine/src/reader.rs:403-424` inserts decoded object streams into that cache without a capacity or byte budget.

Pages are rebuilt eagerly on repeated calls:

- `crates/engine/src/document.rs:83-115` traverses the page tree and returns a new `Vec<PdfPage>`.
- `crates/engine/src/engine.rs:401-427`, `crates/engine/src/engine.rs:411-417`, and later page helpers call `get_pages()` repeatedly for validation, lookup, page boxes, rotation, resources, and content.

Content and outputs materialize in memory:

- `crates/engine/src/document.rs:118-188` decodes every content stream for a page into `Vec<u8>` and appends it into a single page buffer.
- `crates/engine/src/text/extractor.rs:48-87` returns one whole `String` for selected pages; parallel extraction also collects page strings before joining.
- `crates/cli/src/main.rs:925-945` collects all page text and then writes/prints the full output.
- `crates/cli/src/main.rs:1624-1651` collects image references before encoding.
- `crates/cli/src/main.rs:1790-1817` collects rendered page bytes before writing the ZIP.

## Hypotheses Before Profiling

1. Full-file buffer keeps at least 1x input size resident for every operation.
2. Large page content streams add another large per-page allocation during text/image/render operations.
3. Page count itself is not memory-heavy, but repeated `get_pages()` calls make high page-count extraction scale poorly.
4. Whole-document convenience paths and CLI paths accumulate output and can exceed memory sooner than page-by-page probes.
5. Object-stream cache growth is unbounded and must be fixed, even though these synthetic fixtures do not stress object streams heavily.

The measurements below confirm all five except that object-stream cache dominance was not directly triggered by the chosen fixtures.

## Measurement Method

Harness:

- Probe worker: `crates/engine/examples/large_file_probe.rs`
- Memory wrapper: `scripts/large_file_profile.py`
- Synthetic generator: `scripts/generate_large_pdf_ladder.py`

Every run was a separate child process. On Windows, the wrapper applies a Job Object with `JOB_OBJECT_LIMIT_PROCESS_MEMORY`, `JOB_OBJECT_LIMIT_JOB_MEMORY`, and `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, with a 2048 MB limit. It samples process working set/private bytes via `GetProcessMemoryInfo` and records JSON events plus CSV memory samples. A 128 MB cap smoke test failed with Python `MemoryError`, confirming the cap is enforced.

Machine:

- OS: Microsoft Windows 11 Home Single Language, 10.0.26200
- CPU: 13th Gen Intel(R) Core(TM) i5-13500HX
- RAM: 16,886,128,640 bytes
- Rust: `rustc 1.95.0`, `cargo 1.95.0`
- Python: `3.14.3`

Note: very short runs are quantized by the 250-500 ms sampling interval. Peak working set is the primary memory column.

## Test Files

### Real Files

| ID | Path | Size | Pages | Notes |
|---|---:|---:|---:|---|
| `real-703mb-atlas` | `E:\pdf_heavy\osl_anatomical-atlas-human-natural-size_elfQSW375a1830z.pdf` | 703 MB | 52 | Largest real file tested |
| `real-290mb-flower` | `E:\pdf_heavy\1000hintsonflowe00croyuoft_raw_jp2.pdf` | 290 MB | 398 | Highest real page count tested |
| `real-150mb-nasa-ocr` | `E:\pdf_heavy\NASA_heavy_ocr.pdf` | 150 MB | 86 | Open/page-count only |
| `real-149mb-nasa` | `E:\pdf_heavy\NASA_heavy.pdf` | 149 MB | 86 | Text/images/render sample |
| `real-80mb-pp80` | `E:\pdf_big\PP-80.pdf` | 80 MB | 274 | Common-case sample |

### Synthetic Files

Size-axis files contain 4 pages, one Helvetica font, and one large uncompressed content stream per page. Page-axis files contain many small pages with 256-byte content streams.

| ID | Size | Pages | Stream bytes/page | Axis |
|---|---:|---:|---:|---|
| `synthetic-size-50mb` | 50 MB | 4 | 13,105,664 | File size |
| `synthetic-size-200mb` | 200 MB | 4 | 52,427,264 | File size |
| `synthetic-size-500mb` | 500 MB | 4 | 131,070,464 | File size |
| `synthetic-size-1024mb` | 1024 MB | 4 | 268,433,920 | File size |
| `synthetic-size-1536mb` | 1536 MB | 4 | 402,651,648 | File size |
| `synthetic-size-2048mb` | 2048 MB | 4 | 536,869,376 | File size |
| `synthetic-size-3072mb` | 3072 MB | 4 | 805,305,344 | File size |
| `synthetic-pages-50p` | <1 MB | 50 | 256 | Page count |
| `synthetic-pages-1000p` | <1 MB | 1000 | 256 | Page count |
| `synthetic-pages-5000p` | 2.3 MB | 5000 | 256 | Page count |

## Results: Real Files

Render is first three pages only.

| File | Operation | Result | Wall time | Peak WS | TTFP | Pages done |
|---|---|---:|---:|---:|---:|---:|
| 703 MB / 52p | open | ok | 1.0s | 707 MB | n/a | n/a |
| 703 MB / 52p | page-count | ok | 0.5s | 708 MB | n/a | 52 |
| 703 MB / 52p | extract-text | ok | 1.0s | 708 MB | 721 ms | 52 |
| 703 MB / 52p | extract-images | ok | 1.5s | 744 MB | 728 ms | 52 |
| 703 MB / 52p | render p1-3 | ok | 4.0s | 1335 MB | 1185 ms | 3 |
| 290 MB / 398p | open | ok | 0.5s | 295 MB | n/a | n/a |
| 290 MB / 398p | extract-text | ok | 3.5s | 296 MB | 318 ms | 398 |
| 290 MB / 398p | extract-images | ok | 3.5s | 299 MB | 276 ms | 398 |
| 290 MB / 398p | render p1-3 | ok | 3.0s | 813 MB | 1098 ms | 3 |
| 149 MB / 86p | extract-text | ok | 0.5s | 154 MB | 108 ms | 86 |
| 149 MB / 86p | render p1-3 | ok | 7.5s | 682 MB | 2030 ms | 3 |
| 80 MB / 274p | extract-text page | ok | 8.5s | 103 MB | 84 ms | 274 |
| 80 MB / 274p | extract-text aggregate | ok | 1.5s | 232 MB | n/a | 274 |
| 80 MB / 274p | extract-images | ok | 3.0s | 94 MB | 85 ms | 274 |

## Results: Synthetic Size Axis

Render is first three pages only. Failures at 1.5 GB happen after open but before first page completion; failures at 2.0 GB and 3.0 GB happen during initial open/full-file read.

| Size | Operation | Result | Wall time | Peak WS | TTFP | Pages done | Failure point |
|---|---|---:|---:|---:|---:|---:|---|
| 50 MB | open | ok | 0.3s | 54 MB | n/a | n/a | n/a |
| 50 MB | extract-text | ok | 0.5s | 92 MB | 84 ms | 4 | n/a |
| 50 MB | extract-images | ok | 0.5s | 92 MB | 49 ms | 4 | n/a |
| 50 MB | render p1-3 | ok | 0.5s | 93 MB | 95 ms | 3 | n/a |
| 200 MB | open | ok | 0.3s | 204 MB | n/a | n/a | n/a |
| 200 MB | extract-text | ok | 0.5s | 355 MB | 205 ms | 4 | n/a |
| 200 MB | extract-images | ok | 0.5s | 354 MB | 186 ms | 4 | n/a |
| 200 MB | render p1-3 | ok | 0.5s | 355 MB | 210 ms | 3 | n/a |
| 500 MB | open | ok | 0.5s | 504 MB | n/a | n/a | n/a |
| 500 MB | extract-text | ok | 1.5s | 880 MB | 578 ms | 4 | n/a |
| 500 MB | extract-images | ok | 1.5s | 879 MB | 600 ms | 4 | n/a |
| 500 MB | render p1-3 | ok | 1.0s | 880 MB | 600 ms | 3 | n/a |
| 1024 MB | open | ok | 1.0s | 1028 MB | n/a | n/a | n/a |
| 1024 MB | extract-text page | ok | 3.0s | 1796 MB | 1334 ms | 4 | n/a |
| 1024 MB | extract-text aggregate | cap | 4.5s | 1030 MB | n/a | 0 | allocation of 268 MB content buffer after open |
| 1024 MB | extract-images | ok | 2.5s | 1796 MB | 1277 ms | 4 | n/a |
| 1024 MB | render p1-3 | ok | 2.5s | 1796 MB | 1333 ms | 3 | n/a |
| 1536 MB | open | ok | 1.5s | 1540 MB | n/a | n/a | n/a |
| 1536 MB | page-count | ok | 1.0s | 1540 MB | n/a | 4 | n/a |
| 1536 MB | extract-text page | cap | 6.5s | 1924 MB | n/a | 0 | page 1 content allocation of 402 MB |
| 1536 MB | extract-images | cap | 6.5s | 1924 MB | n/a | 0 | page 1 content allocation of 402 MB |
| 1536 MB | render p1-3 | cap | 7.0s | 1924 MB | n/a | 0 | page 1 content allocation of 402 MB |
| 2048 MB | open | cap | 0.3s | 4 MB | n/a | 0 | `fs::read` reports out of memory |
| 3072 MB | open | cap | 0.5s | 4 MB | n/a | 0 | `fs::read` reports out of memory |

## Results: Synthetic Page-Count Axis

These files are tiny on disk. They isolate page/object-count cost from file-size cost.

| Pages | Operation | Result | Wall time | Peak WS | TTFP | Pages done |
|---:|---|---:|---:|---:|---:|---:|
| 50 | page-count | ok | 0.3s | 5 MB | n/a | 50 |
| 50 | extract-text page | ok | 0.5s | 5 MB | 3 ms | 50 |
| 50 | extract-images page | ok | 0.5s | 5 MB | 8 ms | 50 |
| 50 | render p1-3 | ok | 0.5s | 9 MB | 76 ms | 3 |
| 1000 | page-count | ok | 0.3s | 7 MB | n/a | 1000 |
| 1000 | extract-text page | ok | 21.1s | 8 MB | 25 ms | 1000 |
| 1000 | extract-text aggregate | ok | 2.0s | 44 MB | n/a | 1000 |
| 1000 | extract-images page | ok | 21.1s | 8 MB | 24 ms | 1000 |
| 1000 | render p1-3 | ok | 0.5s | 10 MB | 39 ms | 3 |
| 5000 | page-count | ok | 0.3s | 16 MB | n/a | 5000 |
| 5000 | extract-text page | ok | 546.9s | 19 MB | 115 ms | 5000 |
| 5000 | extract-text aggregate | ok | 48.2s | 184 MB | n/a | 5000 |
| 5000 | extract-images page | ok | 539.5s | 19 MB | 115 ms | 5000 |
| 5000 | render p1-3 | ok | 1.0s | 20 MB | 200 ms | 3 |

The page-count result is the clearest time bottleneck: 1000 to 5000 pages is 5x more pages but about 26x more wall time in page-by-page text/images. That is consistent with repeated whole-tree `get_pages()` calls inside per-page APIs.

## Bottleneck Attribution

### Dominant Memory Bottlenecks

1. Full-file heap buffer.
   - Evidence: open peak working set scales almost 1:1 with file size: 500 MB file -> 504 MB peak; 1024 MB -> 1028 MB; 1536 MB -> 1540 MB.
   - Code: `from_path_with_password` uses `fs::read` and `PdfReader` stores the `Vec<u8>`.
   - Effect: 2048 MB and 3072 MB synthetic files fail before page counting or extraction begins.

2. Page content stream materialization.
   - Evidence: 1024 MB synthetic open is 1028 MB, but text/images/render are about 1796 MB because a 268 MB page content stream is decoded/materialized on top of the file buffer.
   - Evidence: 1536 MB extraction/render fail before first page output with `memory allocation of 402651648 bytes failed`.
   - Code: `get_page_content_bytes` decodes streams into a `Vec<u8>` and appends into `out`.

3. Aggregate output and parallel page buffers.
   - Evidence: aggregate text on the 1024 MB synthetic fails after open with `memory allocation of 268433920 bytes failed`, while page-by-page text completes. The aggregate path can have multiple page buffers/results in flight.
   - Evidence: real 80 MB aggregate text peaks at 232 MB vs 103 MB for page-progress extraction.
   - Code: `TextExtractor::extract` collects page strings, and CLI extraction/render collect full outputs before writing.

4. Render/image decode spikes.
   - Evidence: real 703 MB open peaks at 707 MB, while rendering only the first three pages peaks at 1335 MB.
   - Evidence: real 290 MB render p1-3 peaks at 813 MB.
   - Interpretation: rendered pixel buffers, decoded images, and encoded PNG bytes can dominate after the full-file buffer.

5. Unbounded object stream cache.
   - Evidence: code stores decoded object streams in an unbounded nested `HashMap`.
   - Measurement note: not dominant in this fixture set, but it is a required Prompt 2 fix because object-stream-heavy files can grow cache with document object count.

### Dominant Time Bottlenecks

1. Initial open time scales linearly with file size because `fs::read` must copy the full file before any output.
2. Time-to-first-page scales with file size on size-axis files: about 84 ms at 50 MB, 578 ms at 500 MB, and 1334 ms at 1024 MB.
3. Page-by-page extraction scales worse than linearly with page count: 1000 pages in 21s, 5000 pages in 547s. Repeated full page-tree traversal is the likely cause.
4. Aggregate text is faster on page-count synthetic files because it uses the all-pages path with parallel work, but it has higher memory and loses time-to-first-page.

## Current Ceiling Under 2 GB

| Scenario | Current ceiling | What breaks next |
|---|---:|---|
| Open/page-count, size-heavy PDFs | about 1.5 GB | 2.0 GB `fs::read` fails before parse |
| Text/image extraction, size-heavy PDFs | about 1.0 GB | 1.5 GB fails allocating first 402 MB page content buffer |
| First-three-page render, size-heavy PDFs | about 1.0 GB | 1.5 GB fails allocating first 402 MB page content buffer |
| Real files measured | 703 MB / 398 pages | No real-file failure in available set |
| Tiny high-page-count files | 5000 pages | Completes but page-by-page time is unacceptable |

## Prompt 2 Priority List

1. Replace path open with a seekable source (`from_path` backed by mmap or buffered random access) so file size does not imply an equal-sized heap `Vec<u8>`. Keep `from_bytes` for small/WASM callers.
2. Route parser/object reads through the seekable source by offset. `xref` already carries object offsets; use that instead of parsing from a monolithic slice.
3. Bound object and object-stream caches by entries or bytes with eviction. The current object-stream cache is unbounded.
4. Cache or lazily index page-tree traversal so per-page operations do not rebuild a full `Vec<PdfPage>` repeatedly. Provide direct `get_page(n)`/iterator semantics over a bounded page window.
5. Stream page content and extraction output. Existing all-at-once APIs should be retained as convenience wrappers, but the core extraction path must emit per-page/chunk output without holding every page result.
6. Limit in-flight page work by memory budget. Current aggregate/parallel paths can hold multiple large page content buffers at once.
7. For images/rendering, decode/render/write/release one page or one image at a time by default, with explicit bounded parallelism later.

These changes should be validated by rerunning this exact harness and comparing the same real/synthetic ladders under the unchanged 2 GB cap.
