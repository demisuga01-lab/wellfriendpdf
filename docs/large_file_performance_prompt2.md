# Large-File Performance Prompt 2 Report

Date: 2026-07-02

Base commit: `2d9901ef2a1fc4bc671f017ca7673600019403e4`

Prompt 1 baseline: `docs/large_file_performance_baseline.md`

Prompt 2 after-profile source: the working tree containing this report and the streaming reader changes.

Prompt 3 follow-up: `docs/large_file_performance_prompt3.md` records the target-scale streaming-content and bounded-parallel results.

## Headline

Target: process a single 3-4 GB PDF and documents with thousands of pages inside a hard 2 GB resident-memory envelope.

Prompt 1 ceiling under the 2 GB cap:

- Open/page-count failed at 2.0 GB because `from_path_with_password` copied the whole file into a heap `Vec<u8>`.
- Text/images/render failed at 1.5 GB synthetic size before first page output because the full-file buffer plus the first huge page content stream exceeded the cap.
- Tiny 5000-page files completed, but page-by-page text took 546.9s because page lookups repeatedly rebuilt the page tree.

Prompt 2 ceiling under the same 2 GB cap:

- Open/page-count now completes through the 3.0 GB synthetic fixture at about 20 MB peak working set.
- Text extraction, image extraction, and first-three-page render now complete on the 2.0 GB synthetic fixture. Peak working set is about 1.54 GB.
- Text extraction still fails on the 3.0 GB synthetic fixture before first page output. The remaining blocker is one huge page content allocation of 805,305,344 bytes, not the file-open path.
- 5000-page page-by-page text extraction now completes in 49.2s at about 23 MB peak working set. Aggregate text completes in 10.6s at about 186 MB.

Plainly: Prompt 2 removed the full-file heap buffer as the dominant bottleneck and proved bounded open/page-count behavior beyond 2 GB. The remaining gap to the 3-4 GB target is per-page content-stream materialization on size-heavy files with very large single-page streams. Prompt 3 should target streaming content tokenization/decode, bounded parallel page windows, and remaining throughput work.

## What Changed

1. Seekable file-backed source added while retaining `from_bytes`.
   - `crates/engine/src/reader.rs:102` defines `PdfSource`.
   - `crates/engine/src/reader.rs:107` defines `SeekableFileSource`.
   - `crates/engine/src/reader.rs:236` routes `from_path_with_password` through the seekable source and falls back to the old full read only for small files when streaming open fails.
   - `crates/engine/src/reader.rs:311` opens from the seekable source by reading the header/tail/xref windows instead of the whole input.

2. Object and xref reads now use bounded source windows.
   - `crates/engine/src/reader.rs:496` reads an object window starting at the xref offset.
   - `crates/engine/src/reader.rs:1039` reads xref chains from the source.
   - `crates/engine/src/reader.rs:1243` reads a bounded xref section window.

3. Object-stream cache is bounded.
   - `crates/engine/src/reader.rs:21` sets the default object-stream cache limit to 32 streams.
   - `crates/engine/src/reader.rs:61` implements the bounded cache.

4. Repeated page-tree traversal is cached.
   - `crates/engine/src/document.rs:14` adds a `OnceLock<Vec<PdfPage>>` page cache.
   - `crates/engine/src/document.rs:98` adds `page_count()`.
   - `crates/engine/src/document.rs:102` adds direct `get_page()`.
   - `crates/engine/src/document.rs:114` keeps page collection centralized behind the cache.

5. Text extraction has a streaming callback API and safer large-file scheduling.
   - `crates/engine/src/engine.rs:786` adds `for_each_page_text`.
   - `crates/engine/src/text/extractor.rs:13` caps parallel all-at-once text extraction to files at or below 512 MB.
   - `crates/engine/src/text/extractor.rs:75` keeps very large files on the serial path to avoid multiple huge content streams in flight.

6. Path and bytes behavior are covered by a regression test.
   - `crates/engine/tests/streaming_reader.rs:14` compares file-backed and in-memory readers for page counts, metadata, page bytes, and text.

Known scoped caveats:

- `crates/engine/src/reader.rs:144` and `crates/engine/src/reader.rs:364` still provide a lazy full-file materialization path for signing/writer compatibility. Parse/extract/render no longer use that path, but signing/incremental writing can still load the whole file.
- The current file source uses a `Mutex<File>`, so concurrent reads are safe but serialized. Prompt 3 should revisit this when adding bounded-window parallelism.
- Page content still materializes into one `Vec<u8>` per active page. This is now the dominant 3 GB blocker.

## Measurement Method

The Prompt 1 harness was reused unchanged:

- Probe worker: `crates/engine/examples/large_file_probe.rs`
- Memory wrapper: `scripts/large_file_profile.py`
- Synthetic generator: `scripts/generate_large_pdf_ladder.py`
- Cap: Windows Job Object, 2048 MB hard process/job memory limit.
- Memory metric: sampled peak working set, with private bytes also recorded.
- Runs are one large child process at a time.

Fresh before snapshot directory:

- `large-file-profile/results/prompt2-before/`

After snapshot directory:

- `large-file-profile/results/prompt2-after/`

Output-diff artifacts:

- `large-file-profile/results/prompt2-output-diff/`

Machine/tool provenance is unchanged from the Prompt 1 baseline:

- OS: Microsoft Windows 11 Home Single Language, 10.0.26200
- CPU: 13th Gen Intel(R) Core(TM) i5-13500HX
- RAM: 16,886,128,640 bytes
- Rust: `rustc 1.95.0`, `cargo 1.95.0`
- Python: `3.14.3`

## Before Snapshot

This is the fresh Prompt 2 before snapshot from the Prompt 1 commit, using the same 2 GB cap.

| Run | Result | Operation | Size | Pages | Done | Wall | Peak WS | Peak private | TTFP | Note |
|---|---:|---|---:|---:|---:|---:|---:|---:|---:|---|
| `before-p1000-text` | ok | extract-text | 0.5 MB | 1000 | 1000 | 23.0s | 8.2 MB | 4.4 MB | 43 ms | tiny page-axis fixture |
| `before-s1024-open` | ok | open | 1024 MB | n/a | 0 | 1.5s | 1027.9 MB | 1026.6 MB | n/a | full-file buffer |
| `before-s1024-text` | ok | extract-text | 1024 MB | 4 | 4 | 2.5s | 1795.8 MB | 1796.2 MB | 1254 ms | full file plus page stream |
| `before-s1536-open` | ok | open | 1536 MB | n/a | 0 | 2.0s | 1539.8 MB | 1539.6 MB | n/a | full-file buffer |
| `before-s1536-text` | cap | extract-text | 1536 MB | 4 | 0 | 6.5s | 1923.6 MB | 1924.4 MB | n/a | allocation of 402,651,648 bytes failed |
| `before-s2048-open` | cap | open | 2048 MB | n/a | 0 | 0.5s | 4.0 MB | 0.4 MB | n/a | `fs::read` out of memory |

## After Snapshot

| Run | Result | Operation | Size | Pages | Done | Wall | Peak WS | Peak private | TTFP | Note |
|---|---:|---|---:|---:|---:|---:|---:|---:|---:|---|
| `after-s1024-open` | ok | open | 1024 MB | n/a | 0 | 0.5s | 20.0 MB | 0.4 MB | n/a | file-backed open |
| `after-s1536-open` | ok | open | 1536 MB | n/a | 0 | 0.5s | 20.0 MB | 0.4 MB | n/a | file-backed open |
| `after-s2048-open` | ok | open | 2048 MB | n/a | 0 | 0.5s | 20.0 MB | 0.6 MB | n/a | file-backed open |
| `after-s3072-open` | ok | open | 3072 MB | n/a | 0 | 0.5s | 20.0 MB | 0.4 MB | n/a | file-backed open |
| `after-s1024-text` | ok | extract-text | 1024 MB | 4 | 4 | 2.5s | 772.5 MB | 257.2 MB | 549 ms | page mode |
| `after-s1536-text` | ok | extract-text | 1536 MB | 4 | 4 | 3.5s | 1156.2 MB | 1154.9 MB | 790 ms | page mode |
| `after-s2048-text` | ok | extract-text | 2048 MB | 4 | 4 | 4.5s | 1540.2 MB | 1026.7 MB | 1015 ms | page mode |
| `after-s3072-text` | cap | extract-text | 3072 MB | 4 | 0 | 6.0s | 1540.3 MB | 1539.7 MB | n/a | allocation of 805,305,344 bytes failed |
| `after-s1024-text-aggregate` | ok | extract-text | 1024 MB | 4 | 4 | 2.5s | 772.6 MB | 770.2 MB | n/a | aggregate mode |
| `after-s1536-text-aggregate` | ok | extract-text | 1536 MB | 4 | 4 | 3.5s | 1156.6 MB | 1154.9 MB | n/a | aggregate mode |
| `after-s2048-images` | ok | extract-images | 2048 MB | 4 | 4 | 5.0s | 1540.2 MB | 1539.7 MB | 1229 ms | page mode |
| `after-s2048-render-p1-3` | ok | render | 2048 MB | 4 | 3 | 3.5s | 1540.5 MB | 1026.8 MB | 1033 ms | first three pages |
| `after-p1000-text` | ok | extract-text | 0.5 MB | 1000 | 1000 | 2.0s | 8.4 MB | 4.8 MB | 15 ms | page-axis |
| `after-p5000-text` | ok | extract-text | 2.3 MB | 5000 | 5000 | 49.2s | 22.9 MB | 20.7 MB | 190 ms | page-axis page mode |
| `after-p5000-text-aggregate` | ok | extract-text | 2.3 MB | 5000 | 5000 | 10.6s | 185.6 MB | 183.8 MB | n/a | page-axis aggregate |
| `after-real-80mb-text` | ok | extract-text | 80.2 MB | 274 | 274 | 5.5s | 23.3 MB | 16.8 MB | 40 ms | real file |
| `after-real-290mb-text` | ok | extract-text | 290.4 MB | 398 | 398 | 0.5s | 84.0 MB | 0.4 MB | 113 ms | real file |
| `after-real-703mb-text` | ok | extract-text | 703.4 MB | 52 | 52 | 0.5s | 84.2 MB | 0.4 MB | 89 ms | real file |
| `after-real-703mb-render-p1-3` | ok | render | 703.4 MB | 52 | 3 | 3.0s | 631.2 MB | 380.7 MB | 557 ms | real file |

## Before/After Deltas

| Scenario | Prompt 1 | Prompt 2 | Result |
|---|---:|---:|---|
| 1.0 GB open peak WS | 1027.9 MB | 20.0 MB | full-file buffer removed |
| 1.5 GB open peak WS | 1539.8 MB | 20.0 MB | full-file buffer removed |
| 2.0 GB open | cap before parse | 20.0 MB peak, ok | new open ceiling is beyond 2 GB |
| 3.0 GB open | cap before parse | 20.0 MB peak, ok | open/page-count now reaches target-size class |
| 1.0 GB text peak WS | 1795.8 MB | 772.5 MB | about 1.0 GB reclaimed |
| 1.5 GB text | cap before page 1 | 1156.2 MB peak, ok | extraction ceiling increased |
| 2.0 GB text | cap before open | 1540.2 MB peak, ok | extraction now crosses 2 GB input |
| 3.0 GB text | cap before open | cap before page 1 | remaining blocker moved to page stream |
| 5000-page page-mode text | 546.9s / 19 MB | 49.2s / 22.9 MB | page-cache fixed the O(n^2)-like behavior |
| 5000-page aggregate text | 48.2s / 184 MB | 10.6s / 185.6 MB | throughput improved without memory growth |
| 703 MB real render p1-3 | 4.0s / 1335 MB | 3.0s / 631 MB | real render peak cut by about half |

## Correctness Checks

Full workspace gates:

- `cargo test --workspace`: passed.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `cargo build --release -p wellfriendpdf-cli`: passed after the final source changes.

Focused parity:

- `crates/engine/tests/streaming_reader.rs` passed as part of the workspace test run.
- It compares file-backed and in-memory readers over fixture PDFs for page count, metadata, page content bytes, and extracted page text.

Accuracy regression slices:

- Field slice: passed, `macro_field_f1 = 0.72503`, `macro_value_f1 = 0.81434`, precision `0.69231`, recall `0.84536`.
- Table slice: passed, `macro_shape_f1 = 0.96232`, `macro_cell_f1 = 0.98737`, `macro_cell_recall = 0.99689`, `macro_cell_precision = 0.98246`, `macro_teds_approx = 0.98111`.
- 200-file text slice, wellfriendpdf-only: `char_similarity = 0.92743`, `word_f1 = 1.0`, `line_recall = 1.0`, `spurious_line_ratio = 0.07633`, `reading_order = 0.96019`.

Large real-file before/after text diff:

| File | Bytes before | Bytes after | SHA-256 before | SHA-256 after | Result |
|---|---:|---:|---|---|---|
| `real-80mb-pp80` | 1,165,084 | 1,165,084 | `104A47F5535AA9222F897C5FCFE920511F22BEEC21C523FC8FD9BB11FCB6CEF5` | `104A47F5535AA9222F897C5FCFE920511F22BEEC21C523FC8FD9BB11FCB6CEF5` | identical |
| `real-703mb-atlas` | 52 | 52 | `AC2C2725ECAE4D38CE077D5367B6D6E80A68DA4E51DED7869A4135F3D7293958` | `AC2C2725ECAE4D38CE077D5367B6D6E80A68DA4E51DED7869A4135F3D7293958` | identical |

## Current Ceiling Under 2 GB

| Scenario | Prompt 2 ceiling | What breaks next |
|---|---:|---|
| Open/page-count, size-heavy PDFs | at least 3.0 GB tested | 4.0 GB not yet profiled in Prompt 2 |
| Text extraction, size-heavy PDFs | 2.0 GB synthetic size fixture | 3.0 GB fails on one 805 MB page content allocation |
| Image extraction, size-heavy PDFs | 2.0 GB synthetic size fixture | 3.0 GB not rerun after text exposed the same content-stream bottleneck |
| First-three-page render, size-heavy PDFs | 2.0 GB synthetic size fixture | 3.0 GB not rerun after text exposed the same content-stream bottleneck |
| Real files measured | 703 MB / 398 pages | no real-file failure in available set |
| Tiny high-page-count files | 5000 pages | completes under 25 MB page mode; aggregate under 200 MB |

## Bottleneck Attribution After Prompt 2

Confirmed fixed or reduced:

1. Full-file heap buffer is no longer dominant for path opens. A 3.0 GB file opens at about 20 MB peak working set.
2. Repeated page-tree traversal no longer dominates high page-count text extraction. The 5000-page page-mode run dropped from 546.9s to 49.2s.
3. Object-stream cache growth is no longer unbounded; it is capped by stream count.

Remaining dominant memory bottleneck:

1. Per-page content stream materialization. The 3.0 GB synthetic file has four pages with 805,305,344 bytes of uncompressed content each. Prompt 2 reaches page 1 and fails allocating that page stream. The failure has moved from "cannot open file" to "cannot hold this single active page stream."

Remaining dominant time bottlenecks:

1. Large content streams still require reading and tokenizing whole page content buffers.
2. Page-mode extraction is intentionally serial and memory conservative for very large files. It is bounded, but the 5000-page run is still slower than aggregate mode.
3. The file-backed source serializes file reads through one `Mutex<File>`. That is acceptable for Prompt 2 correctness, but Prompt 3 parallel extraction should avoid a global read lock becoming the throughput bottleneck.

## Prompt 3 Input List

1. Stream page content decode/tokenization so a single huge content stream does not require an 805 MB active allocation.
2. Add bounded-window parallel page extraction. Derive the worker window from measured per-page memory and keep the combined working set under the fixed 2 GB cap.
3. Preserve output order and verify serial-vs-parallel text equality on the real large-file samples.
4. Replace or supplement the single `Mutex<File>` source for concurrent reads if it limits bounded parallel throughput.
5. Extend the size ladder to 4.0 GB after streaming page content lands, with open/text/images/render all under the unchanged 2 GB cap.
6. Keep the lazy full-file materialization path out of parse/extract/render, and document or guard APIs that still need whole-file bytes for signing/writer compatibility.
7. Re-run the Prompt 1 harness unchanged after each major Prompt 3 change, including the real 80 MB, 290 MB, and 703 MB files to prove the common case did not regress.
