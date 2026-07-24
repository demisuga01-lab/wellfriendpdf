# Large-File Performance Prompt 3 Report

Date: 2026-07-02

Base commit: `d85852b2f45f6b4dc26551f9ca5f47d1612df9aa`

Prompt 1 baseline: `docs/large_file_performance_baseline.md`

Prompt 2 report: `docs/large_file_performance_prompt2.md`

Prompt 3 result: the 3-4 GB / 1000s-pages target is met for open, text extraction, image-location/extraction, and first-three-page render on the synthetic target ladder under the fixed 2 GB cap. The largest tested target-size input is a 4.0 GB synthetic PDF with four 1.0 GB uncompressed page content streams.

## Headline

Target: process a single 3-4 GB PDF and documents with thousands of pages inside a hard 2 GB resident-memory envelope.

Prompt 2 reached:

- 3.0 GB open/page-count at about 20 MB peak working set.
- 2.0 GB text/images/render at about 1.54 GB peak working set.
- 3.0 GB text failed before page 1 with `memory allocation of 805305344 bytes failed`.
- 5000-page text completed, but page mode was still 46-49s.

Prompt 3 now reaches:

- 4.0 GB open: ok, 0.5s, 20.1 MB peak working set.
- 4.0 GB text, page mode: ok, 14.5s, 20.1 MB peak working set, first page at 3654 ms.
- 4.0 GB text, bounded aggregate/parallel mode: ok, 5.5-6.0s, 20.1 MB peak working set.
- 4.0 GB image extraction: ok, 14.5s, 20.2 MB peak working set.
- 4.0 GB render p1-3: ok, 11.0s, 20.1 MB peak working set.
- 5000-page text, page mode: ok, 39.1s, 24.3 MB peak working set.
- 5000-page text, aggregate mode: ok, 16.0s, 63.5 MB peak working set.

Plainly: within the fixed 2 GB cap, Wellfriend now processes the 3-4 GB target-size synthetic PDFs and 5000-page synthetic PDFs. The remaining caveat is scope: this proves the parser/extraction/render path for huge uncompressed content streams and high page counts. It does not prove every possible 4 GB real-world image/filter mix, and signing/incremental writer compatibility APIs can still materialize the full source bytes by design.

## What Changed

1. Single unfiltered file-backed content streams are parsed from a bounded range reader instead of copied into one `Vec<u8>`.
   - `crates/engine/src/parser.rs:27` adds `IndirectStreamHeader`.
   - `crates/engine/src/parser.rs:142` parses an indirect stream header without reading stream bytes.
   - `crates/engine/src/reader.rs:119` adds `PdfRangeReader`.
   - `crates/engine/src/reader.rs:558` exposes `unfiltered_stream_range`.
   - `crates/engine/src/document.rs:214` exposes `single_unfiltered_content_stream`.
   - `crates/engine/src/engine.rs:405` routes `get_page_content` through the streaming tokenizer when that safe fast path is available.

2. Content tokenization can consume a `Read` stream.
   - `crates/engine/src/content/tokenizer.rs:36` adds `StreamingContentTokenizer`.
   - `crates/engine/src/content/tokenizer.rs:739` implements the iterator.
   - `crates/engine/src/content/tokenizer.rs:809` verifies the streaming tokenizer matches the existing slice tokenizer on a mixed token stream.

3. File-backed reads no longer serialize behind one seek lock.
   - `crates/engine/src/reader.rs:216` reads source ranges through `read_exact_at`.
   - `crates/engine/src/reader.rs:226` and `crates/engine/src/reader.rs:243` use platform positional reads on Unix/Windows.
   - `crates/engine/src/reader.rs:260` keeps a clone-and-seek fallback for other platforms.

4. Text extraction parallelism is bounded by a page window.
   - `crates/engine/src/text/extractor.rs:76` processes aggregate extraction in bounded chunks.
   - `crates/engine/src/text/extractor.rs:123` derives the window from available parallelism and a conservative memory budget.
   - `crates/cli/src/main.rs:925` applies the same bounded window to CLI default text extraction.
   - Tunables: `WELLFRIENDPDF_TEXT_PARALLEL_PAGES`, `WELLFRIENDPDF_TEXT_PARALLEL_MEMORY_MB`, and `WELLFRIENDPDF_TEXT_PARALLEL_PAGE_MB`.

## Measurement Method

The Prompt 1 harness was reused unchanged:

- Probe worker: `crates/engine/examples/large_file_probe.rs`
- Memory wrapper: `scripts/large_file_profile.py`
- Synthetic generator: `scripts/generate_large_pdf_ladder.py`
- Cap: Windows Job Object, 2048 MB hard process/job memory limit.
- Memory metric: sampled peak working set, with private bytes also recorded.
- Runs are one large child process at a time.

Result directories:

- Starting point: `large-file-profile/results/prompt3-before/`
- Content-streaming fix: `large-file-profile/results/prompt3-after-content-streaming/`
- Final ladder: `large-file-profile/results/prompt3-final/`
- Serial-vs-parallel diff: `large-file-profile/results/prompt3-parallel-diff/`
- Malformed large input: `large-file-profile/results/prompt3-robustness/`

Machine/tool provenance is unchanged from the Prompt 1 baseline:

- OS: Microsoft Windows 11 Home Single Language, 10.0.26200
- CPU: 13th Gen Intel(R) Core(TM) i5-13500HX
- RAM: 16,886,128,640 bytes
- Rust: `rustc 1.95.0`, `cargo 1.95.0`
- Python: `3.14.3`

## Starting Point

Fresh Prompt 3 starting profile from the Prompt 2 checkpoint:

| Run | Result | Operation | Size | Pages done | Wall | Peak WS | TTFP | Note |
|---|---:|---|---:|---:|---:|---:|---:|---|
| `before3-s3072-open` | ok | open | 3072 MB | 0 | 0.5s | 20.1 MB | n/a | file-backed open |
| `before3-s3072-text` | cap | extract-text | 3072 MB | 0 | 6.0s | 1539.8 MB | n/a | allocation of 805,305,344 bytes failed |
| `before3-p5000-text` | ok | extract-text | 2.3 MB | 5000 | 46.1s | 23.6 MB | 219 ms | page-count axis |

## Final Target Ladder

All rows were run under the fixed 2048 MB Job Object cap.

| Run | Result | Operation | Size / Pages | Done | Wall | Peak WS | Peak private | TTFP | Throughput |
|---|---:|---|---:|---:|---:|---:|---:|---:|---:|
| `final-s4096-open` | ok | open | 4096 MB / 4p | 0 | 0.5s | 20.1 MB | 0.4 MB | n/a | n/a |
| `final-s4096-text` | ok | extract-text page | 4096 MB / 4p | 4 | 14.5s | 20.1 MB | 0.9 MB | 3654 ms | 282 MB/s |
| `final-s4096-text-aggregate` | ok | extract-text aggregate | 4096 MB / 4p | 4 | 5.5s | 20.1 MB | 3.0 MB | n/a | 744 MB/s |
| `final-s4096-images` | ok | extract-images | 4096 MB / 4p | 4 | 14.5s | 20.2 MB | 0.9 MB | 3607 ms | 282 MB/s |
| `final-s4096-render-p1-3` | ok | render p1-3 | 4096 MB / 4p | 3 | 11.0s | 20.1 MB | 1.3 MB | 3607 ms | n/a |
| `final-p5000-text` | ok | extract-text page | 2.3 MB / 5000p | 5000 | 39.1s | 24.3 MB | 21.2 MB | 109 ms | 127.9 pages/s |
| `final-p5000-text-aggregate` | ok | extract-text aggregate | 2.3 MB / 5000p | 5000 | 16.0s | 63.5 MB | 60.1 MB | n/a | 311.9 pages/s |

The 3.0 GB content-streaming fix point also completed:

| Run | Result | Operation | Size / Pages | Done | Wall | Peak WS | TTFP |
|---|---:|---|---:|---:|---:|---:|---:|
| `after3-s3072-text` | ok | extract-text | 3072 MB / 4p | 4 | 11.0s | 20.2 MB | 2699 ms |
| `after3-s3072-images` | ok | extract-images | 3072 MB / 4p | 4 | 11.0s | 20.1 MB | 2674 ms |
| `after3-s3072-render-p1-3` | ok | render p1-3 | 3072 MB / 4p | 3 | 8.5s | 20.1 MB | 2671 ms |

## Real Common-Case Check

| Run | Result | Operation | Size / Pages | Done | Wall | Peak WS | TTFP |
|---|---:|---|---:|---:|---:|---:|---:|
| `final-real-80mb-text` | ok | extract-text | 80.2 MB / 274p | 274 | 5.5s | 23.7 MB | 93 ms |
| `final-real-290mb-text` | ok | extract-text | 290.4 MB / 398p | 398 | 1.0s | 84.1 MB | 211 ms |
| `final-real-703mb-text` | ok | extract-text | 703.4 MB / 52p | 52 | 0.5s | 84.4 MB | 166 ms |
| `final-real-703mb-render-p1-3` | ok | render | 703.4 MB / 52p | 3 | 3.0s | 631.6 MB | 654 ms |

Compared with Prompt 1, common-case memory is dramatically lower. Compared with Prompt 2, the real-file times are within the same sampling-scale envelope; the 703 MB render remains about 631 MB peak, down from 1335 MB in Prompt 1.

## Serial-vs-Parallel Proof

The 4.0 GB aggregate text run was executed twice under the 2 GB cap:

| Run | Window | Result | Wall | Peak WS | SHA-256 |
|---|---:|---:|---:|---:|---|
| `serial-s4096-aggregate` | `WELLFRIENDPDF_TEXT_PARALLEL_PAGES=1` | ok | 14.5s | 20.1 MB | `A39C1DF7EC5CEA6252050340F7D3A979C77D163ECF6B79FF175708F2E6FC7A72` |
| `parallel-s4096-aggregate` | default bounded window | ok | 6.0s | 20.1 MB | `A39C1DF7EC5CEA6252050340F7D3A979C77D163ECF6B79FF175708F2E6FC7A72` |

Result: output is byte-identical, in order, and the bounded parallel run is faster without increasing peak working set.

## Correctness Checks

Full/source gates:

- `cargo build --release -p wellfriendpdf-cli`: passed.
- Focused engine tokenizer and file-backed reader tests: passed.
- `cargo test -p wellfriendpdf-cli`: passed.
- `cargo test --workspace`: passed.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `git diff --check`: passed; output contained only CRLF conversion warnings for modified files.

Accuracy regression slices:

- Field slice: passed, `macro_field_f1 = 0.72503`, `macro_value_f1 = 0.81434`, precision `0.69231`, recall `0.84536`.
- Table slice independent re-run: passed, `macro_shape_f1 = 0.96232`, `macro_cell_f1 = 0.98737`, `macro_cell_recall = 0.99689`, `macro_cell_precision = 0.98246`, `macro_teds_approx = 0.98111`.
- 200-file text slice, wellfriendpdf-only: `char_similarity = 0.92743`, `word_f1 = 1.0`, `line_recall = 1.0`, `spurious_line_ratio = 0.07633`, `reading_order = 0.96019`.

Large real-file Prompt 2 vs Prompt 3 text diff:

| File | Bytes Prompt 2 | Bytes Prompt 3 | SHA-256 Prompt 2 | SHA-256 Prompt 3 | Result |
|---|---:|---:|---|---|---|
| `real-80mb-pp80` | 1,165,084 | 1,165,084 | `104A47F5535AA9222F897C5FCFE920511F22BEEC21C523FC8FD9BB11FCB6CEF5` | `104A47F5535AA9222F897C5FCFE920511F22BEEC21C523FC8FD9BB11FCB6CEF5` | identical |
| `real-703mb-atlas` | 52 | 52 | `AC2C2725ECAE4D38CE077D5367B6D6E80A68DA4E51DED7869A4135F3D7293958` | `AC2C2725ECAE4D38CE077D5367B6D6E80A68DA4E51DED7869A4135F3D7293958` | identical |

## Robustness At Scale

A malformed 3.0 GB zero-filled file was opened under the 2 GB cap:

| Run | Result | Wall | Peak WS | Error |
|---|---:|---:|---:|---|
| `malformed-zero-3072-open` | clean error | 0.5s | 4.1 MB | `malformed PDF: missing PDF header` |

It did not panic, time out, or hit the memory cap.

## Operating Envelope

Fixed cap: 2048 MB resident/process memory.

| Workload | Maximum tested under cap | Peak WS | Time | Status |
|---|---:|---:|---:|---|
| Open/page-count, size-heavy PDF | 4.0 GB / 4 pages | 20.1 MB | 0.5s | target met |
| Text extraction, size-heavy PDF | 4.0 GB / 4 pages | 20.1 MB | 14.5s page, 5.5-6.0s aggregate | target met |
| Image extraction, size-heavy PDF | 4.0 GB / 4 pages | 20.2 MB | 14.5s | target met for no-image huge content fixture |
| First-three-page render, size-heavy PDF | 4.0 GB / 4 pages | 20.1 MB | 11.0s | target met for huge content fixture |
| Page-count axis | 5000 pages | 24.3 MB page, 63.5 MB aggregate | 39.1s page, 16.0s aggregate | target met |
| Real files available | 703 MB / 398 pages | 631.6 MB max observed render | 0.5-5.5s text, 3.0s render p1-3 | no real-file failure |

## Remaining Caveats

1. The 4 GB target proof is synthetic and stresses huge uncompressed content streams. It is not a substitute for a real 4 GB customer PDF with many large decoded images or complex filters.
2. Filtered content streams still decode to a `Vec<u8>` through the existing filter layer. That is acceptable for the measured target fixture, but a future pass should add streaming Flate/LZW content tokenization.
3. The signing/writer compatibility path can still lazily materialize the whole source file via `file_bytes()`. Parse/extract/render avoid it.
4. Image decode memory is still governed by existing pixel/decompression caps. A true image-heavy 4 GB file should be added to the real corpus when available.
5. The bounded text window is conservative by default. Self-hosters can tune `WELLFRIENDPDF_TEXT_PARALLEL_PAGES`, `WELLFRIENDPDF_TEXT_PARALLEL_MEMORY_MB`, and `WELLFRIENDPDF_TEXT_PARALLEL_PAGE_MB`.
